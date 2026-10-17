//! What the built-SPA mount (`http::spa`) answers: hashed assets cached
//! forever, the app shell revalidated on every load, client-side routes
//! served the shell, and unknown `/api/*` paths kept as JSON 404s. Pinned
//! since tower-http 0.7, whose `ServeDir` adds strong `ETag`s and 304s.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use common::*;

const SHELL: &str = "<!doctype html><title>parcel</title>";
const ASSET: &str = "console.log('app');";
const ASSET_PATH: &str = "/assets/app-abc123.js";

/// The app plus the directory it serves, which lives as long as this does.
struct Spa {
    app: TestApp,
    _dir: tempfile::TempDir,
}

async fn spa() -> Spa {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), SHELL).unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app-abc123.js"), ASSET).unwrap();
    let static_dir = dir.path().to_str().unwrap().to_string();
    let app = TestApp::with_config(|c| c.static_dir = Some(static_dir)).await;
    Spa { app, _dir: dir }
}

async fn get(app: &TestApp, path: &str, if_none_match: Option<&str>) -> Response {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::HOST, "parcel.test");
    if let Some(etag) = if_none_match {
        req = req.header(header::IF_NONE_MATCH, etag);
    }
    app.send(req.body(Body::empty()).unwrap()).await
}

fn header_of(response: &Response, name: header::HeaderName) -> &str {
    response
        .headers()
        .get(&name)
        .unwrap_or_else(|| panic!("no {name} header"))
        .to_str()
        .unwrap()
}

/// A strong validator, then a 304 for it that keeps the caching headers.
async fn revalidates(app: &TestApp, path: &str, cache_control: &str) {
    let first = expect_status(get(app, path, None).await, StatusCode::OK).await;
    let etag = header_of(&first, header::ETAG).to_string();
    assert!(!etag.starts_with("W/"), "{path}: weak ETag {etag}");

    let again = get(app, path, Some(&etag)).await;
    assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{path}");
    assert_eq!(header_of(&again, header::ETAG), etag, "{path}");
    assert_eq!(
        header_of(&again, header::CACHE_CONTROL),
        cache_control,
        "{path}"
    );
    assert!(body_bytes(again).await.is_empty(), "{path}");
}

#[tokio::test]
async fn hashed_assets_are_immutable() {
    let spa = spa().await;
    let app = &spa.app;
    let response = expect_status(get(app, ASSET_PATH, None).await, StatusCode::OK).await;
    assert_eq!(
        header_of(&response, header::CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
    assert!(header_of(&response, header::CONTENT_TYPE).contains("javascript"));
    assert!(header_of(&response, header::CONTENT_SECURITY_POLICY).contains("frame-src"));
    assert_eq!(body_bytes(response).await, ASSET.as_bytes());

    revalidates(app, ASSET_PATH, "public, max-age=31536000, immutable").await;
}

#[tokio::test]
async fn the_shell_is_revalidated_on_every_load() {
    let spa = spa().await;
    let app = &spa.app;
    for path in ["/", "/p/abc123", "/tokens"] {
        let response = expect_status(get(app, path, None).await, StatusCode::OK).await;
        assert_eq!(
            header_of(&response, header::CACHE_CONTROL),
            "no-cache",
            "{path}"
        );
        assert!(
            header_of(&response, header::CONTENT_TYPE).starts_with("text/html"),
            "{path}"
        );
        assert_eq!(body_bytes(response).await, SHELL.as_bytes(), "{path}");
        revalidates(app, path, "no-cache").await;
    }
}

#[tokio::test]
async fn a_trailing_slash_never_serves_the_file() {
    // tower-http 0.7 404s a file path with a trailing slash (0.6 served the
    // file); the mount's fallback then answers with the shell.
    let spa = spa().await;
    let app = &spa.app;
    let response = get(app, &format!("{ASSET_PATH}/"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, SHELL.as_bytes());
}

#[tokio::test]
async fn unknown_api_paths_stay_json_404s() {
    let spa = spa().await;
    let app = &spa.app;
    let response = get(app, "/api/v1/no-such-route", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(header_of(&response, header::CONTENT_TYPE).starts_with("application/json"));
}
