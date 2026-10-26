//! Database errors stay value-free on every surface (plan 002 §3.1): a
//! trigger in the test's own schema fails a representative write with a
//! message, DETAIL and HINT that are all canaries, and neither the response
//! nor anything logged may carry them, nor sqlx's own rendering of a server
//! error. Each test also proves the failure *was* logged, so a missed log
//! line can't pass for a clean one.
//!
//! Every test runs on a current-thread runtime: `Captured` subscribes for
//! this thread only.

mod common;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use common::*;
use parcel_server::config::McpConfig;
use parcel_server::db::error::DbError;
use parcel_server::db::parcels;
use parcel_types::{CreatedToken, PublishResult, ViewGrant};
use serde_json::{json, Value};
use sqlx::AssertSqlSafe;
use tracing::subscriber::DefaultGuard;

/// What the trigger raises, and what must never appear anywhere.
const CANARIES: [&str; 3] = ["canary-m", "canary-d", "canary-h"];
/// sqlx's `Display` of a server error, which carries its message.
const SQLX_TEXT: &str = "error returned from database";
/// The trigger's SQLSTATE (`RAISE EXCEPTION`'s default) as `DbError` renders it.
const SUMMARY: &str = "database error SQLSTATE P0001";
/// The PL/pgSQL statement that raises the canaries.
const RAISE: &str = "RAISE EXCEPTION 'canary-m' USING DETAIL = 'canary-d', HINT = 'canary-h'";

/// Everything logged on this thread until it drops.
struct Logs {
    captured: Captured,
    _guard: DefaultGuard,
}

fn capture() -> Logs {
    let captured = Captured::default();
    let _guard = captured.install();
    Logs { captured, _guard }
}

/// Fail every row `when` (`"BEFORE UPDATE ON parcels"`, …) with the
/// canaries, and capture the logs from here on. The function and trigger
/// land in this test's schema (the pool's `search_path`), so no other test
/// sees them; that is checked.
async fn arm(app: &TestApp, when: &str) -> Logs {
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE FUNCTION canary_raise() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN {RAISE}; END $$;
         CREATE TRIGGER canary {when} FOR EACH ROW EXECUTE FUNCTION canary_raise();"
    )))
    .execute(&app.pool)
    .await
    .expect("arm the canary trigger");
    assert_scoped(app, "canary").await;
    capture()
}

/// The trigger `name` is on a table in this test's schema (other tests'
/// triggers, under the same name, sit in theirs).
async fn assert_scoped(app: &TestApp, name: &str) {
    let (schema, current): (String, String) = sqlx::query_as(
        "SELECT n.nspname::text, current_schema()::text FROM pg_trigger t
         JOIN pg_class c ON c.oid = t.tgrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE t.tgname = $1 AND n.nspname = current_schema()",
    )
    .bind(name)
    .fetch_one(&app.pool)
    .await
    .expect("the trigger is in this test's schema");
    assert!(current.starts_with("t_"), "{current}");
    assert_eq!(schema, current);
}

/// A response's status, headers and body (as text), for checking.
struct Seen {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

async fn seen(response: axum::response::Response) -> Seen {
    let status = response.status();
    let headers = response.headers().clone();
    let body = String::from_utf8_lossy(&body_bytes(response).await).into_owned();
    Seen {
        status,
        headers,
        body,
    }
}

/// The REST API's generic failure (500 and the fixed body, nothing else),
/// logged as an internal error under `context` and otherwise value-free.
fn assert_internal_500(logs: &Logs, seen: &Seen, context: &str) {
    assert_eq!(
        seen.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        seen.body
    );
    let body: Value = serde_json::from_str(&seen.body).expect("a JSON error body");
    assert_eq!(
        body,
        json!({"error": {"message": "internal server error", "type": "Internal Server Error"}})
    );
    assert_value_free(logs, seen, "internal error", context);
}

/// Neither a canary nor sqlx's own text is in `text`.
fn assert_clean(text: &str) {
    for needle in CANARIES.iter().chain([&SQLX_TEXT]) {
        assert!(!text.contains(needle), "{needle:?} leaked:\n{text}");
    }
}

/// The failure was logged as `event`, naming `context` and the SQLSTATE,
/// and neither the logs nor `response` carry a canary or sqlx's text.
fn assert_value_free(logs: &Logs, response: &Seen, event: &str, context: &str) {
    let logged = logs.captured.text();
    let line = logged
        .lines()
        .find(|l| l.contains(event))
        .unwrap_or_else(|| panic!("no {event:?} line in:\n{logged}"));
    assert!(line.contains(&format!("{context}: {SUMMARY}")), "{line}");
    let headers = format!("{:?}", response.headers);
    for text in [&logged, &response.body, &headers] {
        assert_clean(text);
    }
}

async fn publish(app: &TestApp, who: &Person) -> String {
    let form = Form::new().file("index.html", PAGE);
    let response = app
        .post_multipart("/api/v1/parcels", Some(&who.token), &form)
        .await;
    let published: PublishResult = expect_json(response, StatusCode::CREATED).await;
    published.parcel.id
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_tag_rename_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    patched(&app, &ada, &id, json!({"tags": ["kind:plan"]})).await;
    let logs = arm(&app, "BEFORE UPDATE ON parcels").await;

    let response = seen(
        app.post_json(
            "/api/v1/tags/rename",
            Some(&ada.token),
            &json!({"from": "kind:plan", "to": "kind:report"}),
        )
        .await,
    )
    .await;
    assert_internal_500(&logs, &response, "renaming tag");
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_patch_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    let logs = arm(&app, "BEFORE UPDATE ON parcels").await;

    let response = seen(patch(&app, &ada, &id, json!({"title": "Renamed"})).await).await;
    assert_internal_500(&logs, &response, "updating parcel metadata");
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_publish_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let logs = arm(&app, "BEFORE INSERT ON parcels").await;

    let form = Form::new().file("index.html", PAGE);
    let response = seen(
        app.post_multipart("/api/v1/parcels", Some(&ada.token), &form)
            .await,
    )
    .await;
    assert_internal_500(&logs, &response, "inserting parcel");
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_new_version_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    let logs = arm(&app, "BEFORE INSERT ON parcel_versions").await;

    let form = Form::new().file("index.html", "<h1>Two</h1>");
    let response = seen(
        app.post_multipart(
            &format!("/api/v1/parcels/{id}/versions"),
            Some(&ada.token),
            &form,
        )
        .await,
    )
    .await;
    assert_internal_500(&logs, &response, "inserting version");
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_comment_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    let logs = arm(&app, "BEFORE INSERT ON comments").await;

    let response = seen(
        app.post_json(
            &format!("/api/v1/parcels/{id}/comments"),
            Some(&ada.token),
            &json!({"body": "Looks good"}),
        )
        .await,
    )
    .await;
    assert_internal_500(&logs, &response, "inserting comment");
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_share_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let bob = person("bob");
    let id = publish(&app, &ada).await;
    let logs = arm(&app, "BEFORE INSERT ON parcel_shares").await;

    let response = seen(
        app.put_json(
            &format!("/api/v1/parcels/{id}/shares/{}", bob.email),
            Some(&ada.token),
            &json!({"role": "viewer"}),
        )
        .await,
    )
    .await;
    assert_internal_500(&logs, &response, "saving share");
}

/// Through `/mcp` an internal failure is an `isError` result (HTTP 200)
/// whose text says nothing about the cause; the server logs it itself.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_mcp_tool_call_is_value_free() {
    let app = TestApp::with_config(|c| {
        let issuer = c.providers[0].issuer.clone();
        c.mcp = Some(McpConfig {
            provider: PROVIDER.into(),
            jwks_url: format!("{issuer}/jwks.json"),
            issuer,
        });
    })
    .await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    patched(&app, &ada, &id, json!({"tags": ["kind:plan"]})).await;
    let token: CreatedToken = expect_json(
        app.post_json("/api/v1/tokens", Some(&ada.token), &json!({"name": "mcp"}))
            .await,
        StatusCode::CREATED,
    )
    .await;
    let logs = arm(&app, "BEFORE UPDATE ON parcels").await;

    let request = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "parcel.test")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .header(header::AUTHORIZATION, format!("Bearer {}", token.secret))
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                   "params": {"name": "rename_tag",
                              "arguments": {"from": "kind:plan", "to": "kind:report"}}})
            .to_string(),
        ))
        .unwrap();
    let response = seen(app.send(request).await).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert_value_free(&logs, &response, "MCP tool failed", "renaming tag");
}

/// A page's data write on the content origin fails like the API does: the
/// same generic 500, still with the origin's CORS header.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_page_data_write_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    let base = format!("/d/{}", view_token(&app, &ada, &id).await);
    let logs = arm(&app, "BEFORE INSERT ON parcel_data").await;

    let response = seen(
        app.content_request(Method::PUT, &format!("{base}/votes/a"), Some(r#"{"n":1}"#))
            .await,
    )
    .await;
    assert_internal_500(&logs, &response, "writing data item");
    assert_eq!(response.headers["access-control-allow-origin"], "*");
}

/// Serving a file on the content origin renders its own sandboxed error
/// page and logs the chain itself. A trigger can't fail a read, so the
/// files table is swapped for a view whose filter raises the canaries.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_content_file_read_is_value_free() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let id = publish(&app, &ada).await;
    let token = view_token(&app, &ada, &id).await;
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE FUNCTION canary_refuse() RETURNS boolean LANGUAGE plpgsql AS $$
         BEGIN {RAISE}; END $$;
         ALTER TABLE version_files RENAME TO version_files_real;
         CREATE VIEW version_files AS SELECT * FROM version_files_real WHERE canary_refuse();"
    )))
    .execute(&app.pool)
    .await
    .unwrap();
    // Resolved through the pool's `search_path`, as the server's queries are.
    let swapped: bool = sqlx::query_scalar(
        "SELECT c.relkind = 'v' AND c.relnamespace = current_schema()::regnamespace
         FROM pg_class c WHERE c.oid = 'version_files'::regclass",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(swapped, "the swap stays in this test's schema");
    let logs = capture();

    let response = seen(
        app.content_request(Method::GET, &format!("/v/{token}/index.html"), None)
            .await,
    )
    .await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.body.contains("Something went wrong"),
        "{}",
        response.body
    );
    let csp = response.headers["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.starts_with("sandbox "), "{csp}");
    assert!(!csp.contains("allow-same-origin"), "{csp}");
    assert_value_free(&logs, &response, "content request failed", "content lookup");
}

/// Against a real server: a `DbError` keeps the SQLSTATE and the
/// constraint's name (which the id retry and slug 409s decide on) and
/// drops the message, DETAIL and HINT.
#[tokio::test(flavor = "current_thread")]
async fn a_db_error_keeps_code_and_constraint_only() {
    let app = TestApp::new().await;
    let raw = sqlx::raw_sql(AssertSqlSafe(format!(
        "DO $$ BEGIN {RAISE}, ERRCODE = '23505', CONSTRAINT = 'parcels_pkey'; END $$"
    )))
    .execute(&app.pool)
    .await
    .unwrap_err();
    assert!(
        raw.to_string().contains("canary-m"),
        "sqlx's text carries it"
    );

    let error = DbError::new("inserting parcel", &raw);
    assert_eq!(error.code(), Some("23505"));
    assert_eq!(error.constraint(), Some(parcels::PARCEL_PKEY));
    assert!(error.is_unique_violation());
    assert!(!error.is_retryable());
    assert!(std::error::Error::source(&error).is_none());
    let chained = anyhow::Error::new(error).context("publishing");
    assert_eq!(
        parcels::violated_constraint(&chained),
        Some(parcels::PARCEL_PKEY)
    );
    for rendered in [
        format!("{chained}"),
        format!("{chained:#}"),
        format!("{chained:?}"),
    ] {
        assert_clean(&rendered);
    }
    assert!(format!("{chained:#}").contains("inserting parcel: database error SQLSTATE 23505"));
}

/// The capability token in the owner's view grant for parcel `id`.
async fn view_token(app: &TestApp, who: &Person, id: &str) -> String {
    let grant: ViewGrant = expect_json(
        app.get(&format!("/api/v1/parcels/{id}/view"), Some(&who.token))
            .await,
        StatusCode::OK,
    )
    .await;
    let url = url::Url::parse(&grant.content_url).unwrap();
    url.path().split('/').nth(2).unwrap().to_string()
}
