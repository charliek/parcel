//! Serving the built SPA (production; in dev, Vite serves the app).
//!
//! Every path the API doesn't claim falls through to the static directory,
//! and anything that isn't a file there gets `index.html` so client-side
//! routes (`/p/abc…`, `/tokens`) load. Unknown `/api/*` paths stay JSON 404s
//! instead of silently returning the app shell.

use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY};
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::config::Config;
use crate::error::AppError;
use crate::state::AppState;

pub fn install(app: Router<AppState>, dir: &str, config: &Config) -> Router<AppState> {
    let index = std::path::Path::new(dir).join("index.html");
    let serve = ServeDir::new(dir)
        .append_index_html_on_directories(false)
        .fallback(ServeFile::new(index));
    let csp = spa_csp(config);
    let static_service = Router::new()
        .fallback_service(serve)
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let csp = csp.clone();
            async move { with_static_headers(req, next, &csp).await }
        }));
    app.route("/api/{*rest}", any(api_not_found))
        .fallback_service(static_service)
}

async fn api_not_found() -> AppError {
    AppError::NotFound("no such API route".into())
}

/// The SPA only talks to itself (sign-in is server-side redirects), and
/// only frames the content origin.
fn spa_csp(config: &Config) -> String {
    let content = config.content_origin_str();
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; \
         frame-src {content}; frame-ancestors 'none'; base-uri 'self'; object-src 'none'"
    )
}

async fn with_static_headers(req: Request, next: Next, csp: &str) -> Response {
    // Vite fingerprints everything under /assets, so those never change.
    let immutable = req.uri().path().starts_with("/assets/");
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    if let Ok(v) = HeaderValue::from_str(csp) {
        headers.insert(CONTENT_SECURITY_POLICY, v);
    }
    response
}
