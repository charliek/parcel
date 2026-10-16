//! Parcel data on the content origin: what `window.parcel.data` (the page
//! helper) calls.
//!
//! The page's own capability token authorizes these requests, exactly as it
//! does its files: any valid token reads, an owner-scope token (minted only
//! for the parcel's owner) writes. A public-scope token stops working once
//! the parcel stops being (effectively) public, a member-scope one once its
//! user loses access. Requests come from the sandbox's opaque `null`
//! origin without credentials, so plain `Access-Control-Allow-Origin: *`
//! CORS is all the browser needs.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, CACHE_CONTROL, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use parcel_types::{DataItem, DataItemList, DataSummary, Visibility};
use time::OffsetDateTime;
use tower_http::set_header::SetResponseHeaderLayer;

use super::member_still_allowed;
use super::token::{self, ContentClaims, Scope, TokenError};
use crate::db::data::Write;
use crate::db::parcels;
use crate::error::{not_found, AppError, AppResult};
use crate::http::data::{self as shared, ListQuery};
use crate::http::views::effective_visibility;
use crate::state::AppState;

pub fn router(state: &AppState) -> Router<AppState> {
    // Leave room for whitespace around a maximal value; `parse_value`
    // applies the exact limit.
    let body_limit = usize::try_from(state.config.limits.max_data_value_bytes)
        .unwrap_or(usize::MAX)
        .saturating_mul(2)
        .saturating_add(8192);
    Router::new()
        .route("/d/{token}", get(summary).options(preflight))
        .route(
            "/d/{token}/{collection}",
            get(list).delete(clear).options(preflight),
        )
        .route(
            "/d/{token}/{collection}/{key}",
            get(get_item)
                .put(put_item)
                .patch(patch_item)
                .delete(delete_item)
                .options(preflight),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(header("*", ACCESS_CONTROL_ALLOW_ORIGIN))
        .layer(header("no-store", CACHE_CONTROL))
        .layer(header("nosniff", X_CONTENT_TYPE_OPTIONS))
}

fn header(
    value: &'static str,
    name: axum::http::HeaderName,
) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
}

async fn preflight() -> impl axum::response::IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            (ACCESS_CONTROL_ALLOW_METHODS, "GET, PUT, PATCH, DELETE"),
            (ACCESS_CONTROL_ALLOW_HEADERS, "content-type"),
            (ACCESS_CONTROL_MAX_AGE, "600"),
        ],
    )
}

/// Verify the token and that the parcel is still visible to its scope.
async fn authorize(state: &AppState, raw: &str) -> Result<ContentClaims, AppError> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let claims =
        token::verify(&state.config.content_signing_key, raw, now).map_err(|e| match e {
            TokenError::Expired => AppError::Forbidden(
                "this page's link has expired; reopen the parcel from Parcel".into(),
            ),
            TokenError::Malformed | TokenError::BadSignature => not_found(),
        })?;
    let (visibility, owner_disabled) = parcels::grant_state(&state.db, &claims.parcel_id)
        .await?
        .ok_or_else(not_found)?;
    let holds = match claims.scope {
        Scope::Owner => !owner_disabled,
        Scope::Public => effective_visibility(&state.config, visibility) == Visibility::Public,
        Scope::Member(user) => member_still_allowed(state, &claims.parcel_id, user).await?,
    };
    if !holds {
        return Err(AppError::Forbidden(
            "the owner has changed who can see this parcel".into(),
        ));
    }
    Ok(claims)
}

async fn authorize_write(state: &AppState, raw: &str) -> Result<ContentClaims, AppError> {
    let claims = authorize(state, raw).await?;
    if claims.scope != Scope::Owner {
        return Err(AppError::Forbidden(
            "only the parcel's owner can change its data; viewers can read it".into(),
        ));
    }
    Ok(claims)
}

async fn summary(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> AppResult<Json<DataSummary>> {
    let claims = authorize(&state, &token).await?;
    let writable = claims.scope == Scope::Owner;
    Ok(Json(
        shared::summary(&state, &claims.parcel_id, writable).await?,
    ))
}

async fn list(
    State(state): State<AppState>,
    Path((token, collection)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<DataItemList>> {
    let claims = authorize(&state, &token).await?;
    Ok(Json(
        shared::list_items(&state, &claims.parcel_id, &collection, q).await?,
    ))
}

async fn clear(
    State(state): State<AppState>,
    Path((token, collection)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let claims = authorize_write(&state, &token).await?;
    shared::delete_items(&state, &claims.parcel_id, &collection, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_item(
    State(state): State<AppState>,
    Path((token, collection, key)): Path<(String, String, String)>,
) -> AppResult<Json<DataItem>> {
    let claims = authorize(&state, &token).await?;
    Ok(Json(
        shared::get_item(&state, &claims.parcel_id, &collection, &key).await?,
    ))
}

async fn put_item(
    State(state): State<AppState>,
    Path((token, collection, key)): Path<(String, String, String)>,
    body: Bytes,
) -> AppResult<Json<DataItem>> {
    let claims = authorize_write(&state, &token).await?;
    let value = shared::parse_value(&state, &body)?;
    Ok(Json(
        shared::write_item(
            &state,
            &claims.parcel_id,
            &collection,
            &key,
            Write::Replace(&value),
        )
        .await?,
    ))
}

async fn patch_item(
    State(state): State<AppState>,
    Path((token, collection, key)): Path<(String, String, String)>,
    body: Bytes,
) -> AppResult<Json<DataItem>> {
    let claims = authorize_write(&state, &token).await?;
    let patch = shared::parse_value(&state, &body)?;
    shared::merge_body(&patch)?;
    Ok(Json(
        shared::write_item(
            &state,
            &claims.parcel_id,
            &collection,
            &key,
            Write::Merge(&patch),
        )
        .await?,
    ))
}

async fn delete_item(
    State(state): State<AppState>,
    Path((token, collection, key)): Path<(String, String, String)>,
) -> AppResult<StatusCode> {
    let claims = authorize_write(&state, &token).await?;
    shared::delete_items(&state, &claims.parcel_id, &collection, Some(&key)).await?;
    Ok(StatusCode::NO_CONTENT)
}
