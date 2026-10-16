//! Health probes. Plain axum on purpose: never part of the OpenAPI document.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
}

/// The process is up.
async fn live() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Ready to serve: the database answers. (The blob store is not probed — a
/// GCS hiccup should degrade content requests, not pull the instance.)
async fn ready(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match crate::db::ping(&state.db).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "blob_backend": state.blobs.backend() })),
        ),
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable" })),
            )
        }
    }
}
