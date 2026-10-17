//! Router assembly and server bootstrap.
//!
//! One process answers two origins. Requests whose `Host` is the content
//! origin go to the sandboxed content router; everything else goes to the
//! app (API, OpenAPI, Swagger UI, SPA). The split happens before any route
//! matching, so no API route is ever reachable on the content host and no
//! user file is ever reachable on the app host.
//!
//! Layer order, outermost first: request id → trace span → host dispatch →
//! (app: security headers → body limit → routes) | (content: routes).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use anyhow::Context;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::{CONTENT_TYPE, HOST};
use axum::routing::get;
use axum::Router;
use stridelabs_observability::{RequestId, RequestIdLayer, REQUEST_ID_FIELD, REQUEST_SPAN_NAME};
use tower::util::Oneshot;
use tower::{Service, ServiceExt};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::config::Config;
use crate::error::AppError;
use crate::http::{auth, health, openapi, spa};
use crate::state::AppState;
use crate::{content, db, gc};

/// Build the complete application. Tests drive this with `oneshot`.
pub fn build_app(state: AppState) -> Router {
    let app = app_router(state.clone());
    let content = content::router(state.clone());
    let content_authority = authority_of(&state.config.content_origin);

    let dispatch = HostDispatch {
        app,
        content,
        content_authority: content_authority.into(),
    };

    Router::new()
        .fallback_service(dispatch)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request<Body>| {
                    let request_id = req
                        .extensions()
                        .get::<RequestId>()
                        .map(|r| r.0.as_str())
                        .unwrap_or("-");
                    // `requestId` is the field `LogFormat::Cloud` lifts onto
                    // every line inside the span; it writes no other span
                    // field, so the method and path show only in text/json.
                    tracing::info_span!(
                        REQUEST_SPAN_NAME,
                        method = %req.method(),
                        path = %redacted_path(req.uri().path()),
                        { REQUEST_ID_FIELD } = %request_id,
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(RequestIdLayer)
}

/// Routes each request to the app or the content router by its `Host`.
#[derive(Clone)]
struct HostDispatch {
    app: Router,
    content: Router,
    content_authority: Arc<str>,
}

impl Service<Request<Body>> for HostDispatch {
    type Response = axum::response::Response;
    type Error = Infallible;
    type Future = Oneshot<Router, Request<Body>>;

    fn poll_ready(&mut self, _cx: &mut TaskContext<'_>) -> Poll<Result<(), Infallible>> {
        // Both routers are always ready; readiness is checked per call by `oneshot`.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let router = if is_content_host(&req, &self.content_authority) {
            self.content.clone()
        } else {
            self.app.clone()
        };
        router.oneshot(req)
    }
}

fn app_router(state: AppState) -> Router {
    let (api_v1, api_doc) = openapi::router().split_for_parts();
    let api_doc = openapi::finalize(api_doc);
    // Pre-rendered once so `/openapi.json` is byte-identical to the
    // committed file and to `parcel-server openapi`.
    let spec_json: std::sync::Arc<str> = stridelabs_http::openapi::to_pretty_json(&api_doc).into();

    let app = Router::new()
        .merge(health::router())
        .route(
            "/openapi.json",
            get(move || {
                let spec_json = spec_json.clone();
                async move { ([(CONTENT_TYPE, "application/json")], spec_json.to_string()) }
            }),
        )
        .merge(api_v1)
        .merge(auth::router());
    let app = match crate::mcp::router(&state) {
        Some(mcp) => app.merge(mcp),
        None => app,
    };
    let app = install_swagger_ui(app, &state.config, &api_doc);
    let app = match state.config.static_dir.clone() {
        Some(dir) => spa::install(app, &dir, &state.config),
        None => app.fallback(|| async { AppError::NotFound("not found".into()) }),
    };

    // Publishes stream their own size checks (see `publish::read_multipart`);
    // this outer cap only bounds what a single request may send at all.
    let body_cap = usize::try_from(state.config.limits.max_upload_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(1024 * 1024);

    app.with_state(state)
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(body_cap))
        .layer(stridelabs_http::security_headers())
}

#[cfg(feature = "swagger-ui")]
fn install_swagger_ui(
    app: Router<AppState>,
    config: &Config,
    api_doc: &utoipa::openapi::OpenApi,
) -> Router<AppState> {
    if config.app_env == crate::config::AppEnv::Production {
        return app;
    }
    app.merge(Router::<AppState>::from(
        utoipa_swagger_ui::SwaggerUi::new("/docs").url("/docs/openapi.json", api_doc.clone()),
    ))
}

#[cfg(not(feature = "swagger-ui"))]
fn install_swagger_ui(
    app: Router<AppState>,
    _config: &Config,
    _api_doc: &utoipa::openapi::OpenApi,
) -> Router<AppState> {
    app
}

/// `host[:port]` as a browser would send it in `Host` (default port omitted).
fn authority_of(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn is_content_host(req: &Request<Body>, content_authority: &str) -> bool {
    let host = req
        .headers()
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.to_string()));
    let Some(host) = host else { return false };
    let host = host.to_ascii_lowercase();
    let host = host
        .strip_suffix(":443")
        .or_else(|| host.strip_suffix(":80"))
        .unwrap_or(&host);
    host == content_authority
}

/// Content URLs carry a capability token as their second segment; keep it
/// out of logs.
fn redacted_path(path: &str) -> String {
    // Content files (`/v/`) and page data (`/d/`) both carry a capability.
    for prefix in ["/v/", "/d/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return match rest.split_once('/') {
                Some((_, tail)) => format!("{prefix}<token>/{tail}"),
                None => format!("{prefix}<token>"),
            };
        }
    }
    path.to_string()
}

/// Bind, migrate, start GC, and serve until a shutdown signal arrives.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    let addr: SocketAddr = config.bind_addr;
    tracing::info!(
        app_env = config.app_env.as_str(),
        auth_mode = config.auth_mode.as_str(),
        app_origin = %config.app_origin_str(),
        content_origin = %config.content_origin_str(),
        "starting parcel"
    );
    let state = AppState::new(config)?;
    if state.config.run_migrations {
        db::migrate(&state.db).await?;
    } else {
        tracing::info!("RUN_MIGRATIONS=false; expecting `parcel-server migrate` to have run");
    }
    gc::spawn(state.clone());

    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "parcel listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(stridelabs_http::shutdown_signal())
        .await
        .context("server error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens() {
        assert_eq!(
            redacted_path("/v/abc.def/index.html"),
            "/v/<token>/index.html"
        );
        assert_eq!(redacted_path("/v/abc"), "/v/<token>");
        assert_eq!(redacted_path("/d/abc.def/votes/a"), "/d/<token>/votes/a");
        assert_eq!(redacted_path("/d/abc.def"), "/d/<token>");
        assert_eq!(redacted_path("/api/v1/me"), "/api/v1/me");
    }

    #[test]
    fn authorities() {
        let u = url::Url::parse("https://Content.Example.com").unwrap();
        assert_eq!(authority_of(&u), "content.example.com");
        let u = url::Url::parse("http://127.0.0.1:8106").unwrap();
        assert_eq!(authority_of(&u), "127.0.0.1:8106");
    }
}
