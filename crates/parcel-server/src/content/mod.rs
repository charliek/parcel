//! The content origin: serves parcel files to the sandboxed viewer.
//!
//! This router only ever answers on `PARCEL_CONTENT_ORIGIN`'s host (see
//! `http::server`'s host dispatch), and every response it produces carries
//! the sandbox headers, so user HTML can never run with a real origin:
//!
//! * `Content-Security-Policy: sandbox …` — an opaque origin even when the
//!   URL is opened top-level, so a parcel cannot read or set cookies/storage
//!   for this host or its parent domain;
//! * `frame-ancestors` — only the app may frame it;
//! * `Referrer-Policy: no-referrer` — the capability token in the URL never
//!   leaks to sites a parcel links to;
//! * `Access-Control-Allow-Origin: *` — module scripts, fonts and `fetch()`
//!   of sibling files are CORS requests from the opaque (`null`) origin.

pub mod markdown;
pub mod token;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_SECURITY_POLICY,
    CONTENT_TYPE, ETAG, IF_NONE_MATCH, LOCATION, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use time::OffsetDateTime;

use crate::db::parcels;
use crate::state::AppState;
use crate::storage::blob_key;
use token::{ContentClaims, Scope, TokenError};

/// Sandbox flags shared by the CSP header and the viewer iframe. Never
/// `allow-same-origin`: that is what keeps the document's origin opaque.
pub const SANDBOX_FLAGS: &str = "allow-scripts allow-forms allow-popups \
     allow-popups-to-escape-sandbox allow-modals allow-downloads";

/// Everything but RFC 3986 unreserved characters is escaped in a segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encode a published path for use in a URL, keeping `/` separators.
pub fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// Mint a capability token for one version and build the URL of its entry.
pub fn grant_url(
    state: &AppState,
    parcel_id: &str,
    version: i32,
    entry: &str,
    scope: Scope,
) -> (String, OffsetDateTime) {
    let expires = OffsetDateTime::now_utc() + state.config.content_token_ttl;
    let token = token::sign(
        &state.config.content_signing_key,
        &ContentClaims {
            parcel_id: parcel_id.to_string(),
            version,
            expires_at: expires.unix_timestamp(),
            scope,
        },
    );
    let url = format!(
        "{}/v/{token}/{}",
        state.config.content_origin_str(),
        encode_path(entry)
    );
    (url, expires)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // So uptime checks against the content domain have something to hit.
        .route("/health/live", get(|| async { "ok" }))
        .route("/v/{token}", get(redirect_to_dir))
        .route("/v/{token}/", get(redirect_to_entry))
        .route("/v/{token}/{*path}", get(serve))
        .fallback(|State(state): State<AppState>| async move {
            error_page(
                &state,
                StatusCode::NOT_FOUND,
                "Not found",
                "There is nothing here.",
            )
        })
        .with_state(state)
}

/// Read a blob through the in-memory cache.
pub async fn load_blob(state: &AppState, sha256: &str) -> anyhow::Result<Option<Bytes>> {
    if let Some(bytes) = state.blob_cache.get(sha256).await {
        return Ok(Some(bytes));
    }
    let Some(bytes) = state.blobs.get(&blob_key(sha256)).await? else {
        return Ok(None);
    };
    state
        .blob_cache
        .insert(sha256.to_string(), bytes.clone())
        .await;
    Ok(Some(bytes))
}

enum Denied {
    Expired,
    Invalid,
}

fn check_token(state: &AppState, token: &str) -> Result<ContentClaims, Denied> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    token::verify(&state.config.content_signing_key, token, now).map_err(|e| match e {
        TokenError::Expired => Denied::Expired,
        TokenError::Malformed | TokenError::BadSignature => Denied::Invalid,
    })
}

fn denied_page(state: &AppState, denied: Denied) -> Response {
    match denied {
        Denied::Expired => error_page(
            state,
            StatusCode::FORBIDDEN,
            "This link has expired",
            "Content links are temporary. Reopen the parcel from Parcel to get a fresh one.",
        ),
        Denied::Invalid => error_page(
            state,
            StatusCode::NOT_FOUND,
            "Not found",
            "This content link is not valid.",
        ),
    }
}

async fn redirect_to_dir(Path(token): Path<String>) -> Response {
    redirect(&format!("/v/{token}/"))
}

async fn redirect_to_entry(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let claims = match check_token(&state, &token) {
        Ok(c) => c,
        Err(d) => return denied_page(&state, d),
    };
    match parcels::version_entry(&state.db, &claims.parcel_id, claims.version).await {
        Ok(Some(entry)) => redirect(&format!("/v/{token}/{}", encode_path(&entry))),
        Ok(None) => not_found(&state),
        Err(e) => internal(&state, e),
    }
}

async fn serve(
    State(state): State<AppState>,
    Path((token, path)): Path<(String, String)>,
    request_headers: HeaderMap,
) -> Response {
    let claims = match check_token(&state, &token) {
        Ok(c) => c,
        Err(d) => return denied_page(&state, d),
    };

    let lookup = if path.is_empty() || path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path
    };
    let row = match parcels::content_lookup(&state.db, &claims.parcel_id, claims.version, &lookup)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&state),
        Err(e) => return internal(&state, e),
    };
    // A public grant stops working the moment the owner makes the parcel
    // private, rather than lingering until the token expires.
    if claims.scope == Scope::Public && row.visibility != "public" {
        return error_page(
            &state,
            StatusCode::FORBIDDEN,
            "This parcel is private",
            "The owner has made this parcel private.",
        );
    }

    let render_markdown = row.path == row.entry && parcel_types::media::is_markdown(&row.path);
    let etag = if render_markdown {
        format!("\"{}-md\"", row.blob_sha256)
    } else {
        format!("\"{}\"", row.blob_sha256)
    };
    if request_headers
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        apply_file_headers(&state, response.headers_mut(), &etag);
        return response;
    }

    let bytes = match load_blob(&state, &row.blob_sha256).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            tracing::error!(sha256 = %row.blob_sha256, "referenced blob missing from store");
            return internal_message(&state);
        }
        Err(e) => return internal(&state, e),
    };

    let (body, content_type) = if render_markdown {
        let text = String::from_utf8_lossy(&bytes);
        let name = parcel_types::paths::file_name(&row.path);
        (
            Bytes::from(markdown::render(&text, name)),
            "text/html; charset=utf-8".to_string(),
        )
    } else {
        (bytes, row.content_type)
    };

    let mut response = Response::new(Body::from(body.clone()));
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&content_type) {
        headers.insert(CONTENT_TYPE, v);
    }
    headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
    apply_file_headers(&state, headers, &etag);
    response
}

fn apply_file_headers(state: &AppState, headers: &mut HeaderMap, etag: &str) {
    apply_sandbox_headers(state, headers);
    if let Ok(v) = HeaderValue::from_str(etag) {
        headers.insert(ETAG, v);
    }
    // The URL pins an immutable version, but it is also a capability that
    // must not land in shared caches.
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("cross-origin"),
    );
}

fn apply_sandbox_headers(state: &AppState, headers: &mut HeaderMap) {
    let csp = format!(
        "sandbox {SANDBOX_FLAGS}; frame-ancestors {}",
        state.config.app_origin_str()
    );
    if let Ok(v) = HeaderValue::from_str(&csp) {
        headers.insert(CONTENT_SECURITY_POLICY, v);
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        response.headers_mut().insert(LOCATION, v);
    }
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

fn not_found(state: &AppState) -> Response {
    error_page(
        state,
        StatusCode::NOT_FOUND,
        "Not found",
        "This file is not part of the parcel.",
    )
}

fn internal(state: &AppState, e: anyhow::Error) -> Response {
    tracing::error!(error = %format!("{e:#}"), "content request failed");
    internal_message(state)
}

fn internal_message(state: &AppState) -> Response {
    error_page(
        state,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something went wrong",
        "The file could not be loaded. Try again shortly.",
    )
}

fn error_page(state: &AppState, status: StatusCode, title: &str, detail: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title>\
         <style>:root{{color-scheme:light dark}}body{{font:16px/1.5 system-ui,sans-serif;\
         display:grid;place-items:center;min-height:100vh;margin:0;padding:16px;text-align:center}}\
         p{{opacity:.7}}</style></head><body><div><h1>{title}</h1><p>{detail}</p></div></body></html>",
        title = markdown::escape_html(title),
        detail = markdown::escape_html(detail),
    );
    let mut response = (status, html).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_sandbox_headers(state, response.headers_mut());
    response
}

#[cfg(test)]
mod tests {
    use super::encode_path;

    #[test]
    fn encodes_segments_but_keeps_slashes() {
        assert_eq!(encode_path("a b/c#d.html"), "a%20b/c%23d.html");
        assert_eq!(encode_path("index.html"), "index.html");
    }
}
