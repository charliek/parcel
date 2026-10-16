//! Shared integration-test harness: a fail-loud real Postgres (database
//! `parcel_test`, created on demand), a mock slauth JWKS endpoint, an
//! in-memory blob store, and the full router driven with `oneshot`.
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
use parcel_server::config::{AuthMode, Config};
use parcel_server::state::AppState;
use parcel_server::storage::MemoryBlobStore;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::PgPool;
use stridelabs_auth::test_support::{TestClaims, TestKey};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const ISSUER: &str = "https://auth.parcel.test";
pub const APP_ORIGIN: &str = "https://parcel.test";
pub const CONTENT_HOST: &str = "content.parcel.test";

const ADMIN_URL_DEFAULT: &str = "postgres://postgres:localdev@localhost:5439/postgres";
const TEST_DB: &str = "parcel_test";

/// The database URL for tests: `DATABASE_URL` if set, else the compose
/// Postgres's `parcel_test` database.
pub fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("postgres://postgres:localdev@localhost:5439/{TEST_DB}"))
}

/// Create `parcel_test` if it doesn't exist (only when using the default URL).
async fn ensure_test_database() {
    if std::env::var("DATABASE_URL").is_ok_and(|u| !u.is_empty()) {
        return;
    }
    let admin = stridelabs_testing::require_postgres(ADMIN_URL_DEFAULT).await;
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
        match sqlx::query(&format!("CREATE DATABASE {TEST_DB}"))
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
    ensure_test_database().await;
    let base = database_url();
    let admin = stridelabs_testing::require_postgres(&base).await;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let schema = format!(
        "t_{now}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
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
            let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {old} CASCADE"))
                .execute(&admin)
                .await;
        }
    }
    admin.close().await;

    let separator = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{separator}options=-c%20search_path%3D{schema}");
    let pool = stridelabs_testing::require_postgres(&url).await;
    parcel_server::db::migrate(&pool)
        .await
        .expect("run migrations");
    pool
}

pub fn test_config(jwks_url: &str) -> Config {
    let mut config = Config {
        database_url: database_url(),
        app_origin: url::Url::parse(APP_ORIGIN).unwrap(),
        content_origin: url::Url::parse(&format!("https://{CONTENT_HOST}")).unwrap(),
        auth_mode: AuthMode::Slauth,
        blob_backend: parcel_server::config::BlobBackend::Memory,
        ..Config::default()
    };
    config.slauth.issuer = ISSUER.into();
    config.slauth.jwks_url = jwks_url.into();
    config.slauth.audience = "parcel".into();
    config
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub blobs: Arc<MemoryBlobStore>,
    pub pool: PgPool,
    _jwks: MockServer,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(|_| {}).await
    }

    /// Build an app, letting the test adjust config (limits, auth mode…).
    pub async fn with_config(adjust: impl FnOnce(&mut Config)) -> Self {
        let jwks = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                TestKey::primary().jwks_json().to_owned(),
                "application/json",
            ))
            .mount(&jwks)
            .await;
        let mut config = test_config(&format!("{}/.well-known/jwks.json", jwks.uri()));
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
            _jwks: jwks,
        }
    }

    pub async fn send(&self, request: Request<Body>) -> Response {
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

/// A fresh user's slauth access token.
pub fn user_token() -> String {
    let sub = uuid::Uuid::new_v4().to_string();
    token_for(&sub, &format!("{sub}@example.com"))
}

pub fn token_for(sub: &str, email: &str) -> String {
    TestKey::primary().mint(
        &TestClaims::new(ISSUER, "parcel")
            .subject(sub)
            .email(email)
            .name("Test User")
            .build(),
    )
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
