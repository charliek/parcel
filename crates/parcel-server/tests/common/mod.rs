//! Shared integration-test harness: a fail-loud real Postgres (database
//! `parcel_test`, created on demand), a mock OIDC provider (discovery +
//! JWKS; tests mount its token endpoint), an in-memory blob store, and the
//! full router driven with `oneshot`.
//!
//! Most tests don't care how a user signed in: [`user_token`] returns a
//! stand-in credential that [`TestApp::send`] swaps for a real signed
//! session cookie (and the `Origin` a browser would send). Sign-in itself is
//! tested end to end in `tests/auth.rs`.
//!
//! Each `TestApp` runs in its own Postgres schema (see [`migrated_pool`]).
//! `cargo test` REQUIRES the compose
//! Postgres (`docker compose up -d`); with none reachable these helpers panic
//! with instructions rather than skipping.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use parcel_server::auth::cookie::{self, Session};
use parcel_server::config::{AuthMode, Config, ProviderConfig};
use parcel_server::db::users;
use parcel_server::state::AppState;
use parcel_server::storage::MemoryBlobStore;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use stridelabs_auth::test_support::TestKey;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The mock provider's id in `AUTH_PROVIDERS`.
pub const PROVIDER: &str = "test";
pub const CLIENT_ID: &str = "parcel";
/// Prefix of the stand-in credential [`user_token`] returns.
const TEST_USER: &str = "test-user:";
pub const APP_ORIGIN: &str = "https://parcel.test";
pub const CONTENT_HOST: &str = "content.parcel.test";

const ADMIN_URL_DEFAULT: &str = "postgres://postgres:localdev@localhost:5439/postgres";
const TEST_DB: &str = "parcel_test";

/// Whether the suite runs against a `DATABASE_URL` of the caller's choosing.
fn database_url_set() -> bool {
    std::env::var("DATABASE_URL").is_ok_and(|u| !u.is_empty())
}

/// The database URL for tests: `DATABASE_URL` if set, else the compose
/// Postgres's `parcel_test` database. A set `DATABASE_URL` must name a
/// loopback host (this harness creates and drops schemas there), or the
/// suite panics before connecting, without printing the URL.
pub fn database_url() -> String {
    if database_url_set() {
        stridelabs_testing::require_local_url("DATABASE_URL")
    } else {
        format!("postgres://postgres:localdev@localhost:5439/{TEST_DB}")
    }
}

/// Create `parcel_test` if it doesn't exist (only when using the default URL).
async fn ensure_test_database() {
    if database_url_set() {
        return;
    }
    let admin = stridelabs_testing::require_postgres_at(ADMIN_URL_DEFAULT).await;
    for attempt in 0..10 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(TEST_DB)
                .fetch_one(&admin)
                .await
                .expect("query pg_database");
        if exists {
            return;
        }
        // Parallel test binaries can race here; "already exists" and
        // "template1 is being accessed" both resolve on retry.
        match sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {TEST_DB}")))
            .execute(&admin)
            .await
        {
            Ok(_) => return,
            Err(_) if attempt < 9 => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await
            }
            Err(e) => panic!("could not create {TEST_DB}: {e}"),
        }
    }
}

/// A pool on a fresh, migrated schema of its own.
///
/// Every `TestApp` gets its own schema (via the connection's `search_path`),
/// so tests are fully isolated: each pairs a private database namespace with
/// a private in-memory blob store, and content-addressed dedup can never
/// match a blob row whose bytes live in another test's store. Schemas older
/// than an hour are dropped as new ones are created, which bounds growth
/// without racing any test that could still be running.
pub async fn migrated_pool() -> PgPool {
    let pool = schema_pool().await;
    parcel_server::db::migrate(&pool)
        .await
        .expect("run migrations");
    pool
}

/// A pool on a fresh, empty schema (the migration tests apply migrations
/// one by one).
pub async fn schema_pool() -> PgPool {
    ensure_test_database().await;
    // `database_url` refuses a non-local `DATABASE_URL` before the `DROP
    // SCHEMA … CASCADE` below can reach it.
    let base = database_url();
    // `_at`, never `require_postgres`: that one prefers a set `DATABASE_URL`
    // over its argument, which would drop the per-test `search_path` below.
    let admin = stridelabs_testing::require_postgres_at(&base).await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    // DDL can't bind identifiers, so the names below are spliced in: this
    // one (and the `old` ones, which only ever match it) is `t_<secs>_<hex>`.
    let schema = format!(
        "t_{now}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create test schema");
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT nspname::text FROM pg_namespace WHERE nspname LIKE 't\\_%'")
            .fetch_all(&admin)
            .await
            .expect("list test schemas");
    for old in existing {
        let created = old.split('_').nth(1).and_then(|t| t.parse::<i64>().ok());
        if created.is_some_and(|t| t < now - 3600) {
            let _ = sqlx::query(AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {old} CASCADE"
            )))
            .execute(&admin)
            .await;
        }
    }
    admin.close().await;

    let separator = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{separator}options=-c%20search_path%3D{schema}");
    stridelabs_testing::require_postgres_at(&url).await
}

pub fn test_config(issuer: &str) -> Config {
    Config {
        database_url: database_url(),
        app_origin: url::Url::parse(APP_ORIGIN).unwrap(),
        content_origin: url::Url::parse(&format!("https://{CONTENT_HOST}")).unwrap(),
        auth_mode: AuthMode::Oidc,
        providers: vec![ProviderConfig {
            id: PROVIDER.into(),
            label: "Test IdP".into(),
            issuer: issuer.into(),
            client_id: CLIENT_ID.into(),
            client_secret: None,
            allowed_domains: Vec::new(),
            trust_email: false,
            scopes: "openid email profile".into(),
            provider_logout: false,
        }],
        blob_backend: parcel_server::config::BlobBackend::Memory,
        ..Config::default()
    }
}

/// Serve a discovery document and JWKS for an issuer at `server`.
pub async fn mount_provider(server: &MockServer) {
    let issuer = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks.json"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "end_session_endpoint": format!("{issuer}/logout"),
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            TestKey::primary().jwks_json().to_owned(),
            "application/json",
        ))
        .mount(server)
        .await;
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub blobs: Arc<MemoryBlobStore>,
    pub pool: PgPool,
    /// The mock OIDC provider (its issuer is `idp.uri()`).
    pub idp: MockServer,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(|_| {}).await
    }

    /// Build an app, letting the test adjust config (limits, auth mode…).
    pub async fn with_config(adjust: impl FnOnce(&mut Config)) -> Self {
        let idp = MockServer::start().await;
        mount_provider(&idp).await;
        let mut config = test_config(&idp.uri());
        adjust(&mut config);
        let pool = migrated_pool().await;
        let blobs = Arc::new(MemoryBlobStore::default());
        let state =
            AppState::with_parts(config, pool.clone(), reqwest::Client::new(), blobs.clone());
        let router = parcel_server::http::server::build_app(state.clone());
        TestApp {
            router,
            state,
            blobs,
            pool,
            idp,
        }
    }

    /// A signed session cookie (`name=value`) for the user with this
    /// provider subject and email, signing them in first.
    pub async fn session_cookie(&self, sub: &str, email: &str) -> String {
        let user = users::sign_in(&self.pool, PROVIDER, sub, email, "Test User")
            .await
            .expect("sign in test user");
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let session = Session {
            user_id: user.id,
            epoch: user.session_epoch,
            issued_at: now,
            expires_at: now + 3600,
            provider: PROVIDER.into(),
        };
        format!(
            "{}={}",
            cookie::names(&self.state.config).session,
            cookie::seal_session(&self.state.config, &session)
        )
    }

    pub async fn send(&self, mut request: Request<Body>) -> Response {
        // Swap a stand-in credential for what a signed-in browser sends.
        let stand_in = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .and_then(|v| v.strip_prefix(TEST_USER))
            .map(String::from);
        if let Some(who) = stand_in {
            let (sub, email) = who.split_once(':').expect("test-user:<sub>:<email>");
            let cookie = self.session_cookie(sub, email).await;
            let headers = request.headers_mut();
            headers.remove(header::AUTHORIZATION);
            headers.insert(header::COOKIE, cookie.parse().unwrap());
            if !headers.contains_key(header::ORIGIN) {
                headers.insert(header::ORIGIN, APP_ORIGIN.parse().unwrap());
            }
        }
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("infallible")
    }

    pub async fn get(&self, uri: &str, token: Option<&str>) -> Response {
        self.send(build(Method::GET, uri, token, Body::empty(), None))
            .await
    }

    pub async fn delete(&self, uri: &str, token: Option<&str>) -> Response {
        self.send(build(Method::DELETE, uri, token, Body::empty(), None))
            .await
    }

    pub async fn post_json(&self, uri: &str, token: Option<&str>, body: &Value) -> Response {
        self.send(build(
            Method::POST,
            uri,
            token,
            Body::from(body.to_string()),
            Some("application/json"),
        ))
        .await
    }

    pub async fn patch_json(&self, uri: &str, token: Option<&str>, body: &Value) -> Response {
        self.send(build(
            Method::PATCH,
            uri,
            token,
            Body::from(body.to_string()),
            Some("application/json"),
        ))
        .await
    }

    pub async fn put_json(&self, uri: &str, token: Option<&str>, body: &Value) -> Response {
        self.send(build(
            Method::PUT,
            uri,
            token,
            Body::from(body.to_string()),
            Some("application/json"),
        ))
        .await
    }

    /// Send any request to the content host. `url` is a full content URL or
    /// a path.
    pub async fn content_request(&self, method: Method, url: &str, body: Option<&str>) -> Response {
        let path = if url.starts_with('/') {
            url.to_string()
        } else {
            let parsed = url::Url::parse(url).expect("content url");
            match parsed.query() {
                Some(q) => format!("{}?{q}", parsed.path()),
                None => parsed.path().to_string(),
            }
        };
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, CONTENT_HOST);
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        self.send(
            req.body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
                .unwrap(),
        )
        .await
    }

    pub async fn post_multipart(&self, uri: &str, token: Option<&str>, form: &Form) -> Response {
        let (content_type, body) = form.encode();
        self.send(build(
            Method::POST,
            uri,
            token,
            Body::from(body),
            Some(&content_type),
        ))
        .await
    }

    /// GET a content-origin URL (as returned in a `ViewGrant`) through the
    /// router, with the content host's `Host` header.
    pub async fn get_content(&self, url: &str, extra: &[(&str, &str)]) -> Response {
        let parsed = url::Url::parse(url).expect("content url");
        assert_eq!(
            parsed.host_str(),
            Some(CONTENT_HOST),
            "not a content URL: {url}"
        );
        let mut req = Request::builder()
            .method(Method::GET)
            .uri(parsed.path())
            .header(header::HOST, CONTENT_HOST);
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        self.send(req.body(Body::empty()).unwrap()).await
    }
}

fn build(
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Body,
    content_type: Option<&str>,
) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "parcel.test");
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(ct) = content_type {
        req = req.header(header::CONTENT_TYPE, ct);
    }
    req.body(body).unwrap()
}

/// A fresh user's credential (a stand-in for a web session; see the
/// module docs).
pub fn user_token() -> String {
    let sub = uuid::Uuid::new_v4().to_string();
    token_for(&sub, &format!("{sub}@example.com"))
}

pub fn token_for(sub: &str, email: &str) -> String {
    format!("{TEST_USER}{sub}:{email}")
}

pub async fn body_bytes(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec()
}

pub async fn json<T: DeserializeOwned>(response: Response) -> T {
    let bytes = body_bytes(response).await;
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("invalid JSON ({e}): {}", String::from_utf8_lossy(&bytes)))
}

/// Assert a status, printing the body on mismatch.
pub async fn expect_status(response: Response, status: StatusCode) -> Response {
    if response.status() == status {
        return response;
    }
    let got = response.status();
    let body = body_bytes(response).await;
    panic!(
        "expected {status}, got {got}: {}",
        String::from_utf8_lossy(&body)
    );
}

/// A multipart/form-data body, built by hand so tests control every byte.
#[derive(Default, Clone)]
pub struct Form {
    parts: Vec<(String, Option<String>, String, Vec<u8>)>,
}

impl Form {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn metadata(mut self, value: Value) -> Self {
        self.parts.push((
            "metadata".into(),
            None,
            "application/json".into(),
            value.to_string().into_bytes(),
        ));
        self
    }

    pub fn file(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.parts.push((
            "file".into(),
            Some(path.into()),
            "application/octet-stream".into(),
            bytes.into(),
        ));
        self
    }

    pub fn encode(&self) -> (String, Vec<u8>) {
        let boundary = "parcel-test-boundary-7d3f";
        let mut body = Vec::new();
        for (name, filename, ctype, bytes) in &self.parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match filename {
                Some(f) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(format!("Content-Type: {ctype}\r\n\r\n").as_bytes());
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }
}

pub const PAGE: &str = "<!doctype html><html><head><title>Hello Parcel</title></head>\
    <body><h1>Hi</h1><script src=\"app.js\"></script></body></html>";
