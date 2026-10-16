//! `/api/v1/info` (public server description) and `/api/v1/me`.

use axum::extract::State;
use axum::Json;
use parcel_types::{AuthInfo, ErrorBody, Me, ServerInfo};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthUser;
use crate::db::parcels;
use crate::error::AppResult;
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_info))
        .routes(routes!(get_me))
}

#[utoipa::path(
    get,
    path = "/info",
    tag = "account",
    description = "Unauthenticated description of this server: origins, the slauth settings \
        the web app signs in with, limits, and allowed file extensions.",
    responses((status = 200, body = ServerInfo))
)]
async fn get_info(State(state): State<AppState>) -> Json<ServerInfo> {
    let c = &state.config;
    Json(ServerInfo {
        name: "parcel".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        app_origin: c.app_origin_str(),
        content_origin: c.content_origin_str(),
        auth: AuthInfo {
            mode: c.auth_mode.as_str().into(),
            issuer: c.slauth.issuer.clone(),
            client_id: c.slauth.client_id.clone(),
            audience: c.slauth.audience.clone(),
        },
        limits: c.limits.clone(),
        allowed_extensions: parcel_types::media::allowed_extensions()
            .into_iter()
            .map(String::from)
            .collect(),
    })
}

#[utoipa::path(
    get,
    path = "/me",
    tag = "account",
    description = "The signed-in user, what they are storing, and the limits that apply.",
    security(("bearer" = [])),
    responses((status = 200, body = Me), (status = 401, body = ErrorBody))
)]
async fn get_me(State(state): State<AppState>, caller: AuthUser) -> AppResult<Json<Me>> {
    let usage = parcels::usage(&state.db, caller.user.id).await?;
    Ok(Json(Me {
        id: caller.user.id.to_string(),
        email: caller.user.email,
        name: caller.user.name,
        usage,
        limits: state.config.limits.clone(),
    }))
}
