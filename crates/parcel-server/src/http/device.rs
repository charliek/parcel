//! `/api/v1/cli/device` — the CLI's device login (RFC 8628 in spirit).
//!
//! 1. The CLI starts a login and gets a secret `device_code` plus a short
//!    `user_code` for a human to confirm.
//! 2. The signed-in user opens `/device?code=…` in the web app and approves.
//! 3. The CLI's next poll redeems the approval exactly once and receives a
//!    freshly minted `pcl_…` token — minted at that moment, so no raw token
//!    is ever stored.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use parcel_types::{
    DeviceInfo, DevicePollRequest, DevicePollResponse, DeviceStartRequest, DeviceStartResponse,
    DeviceStatus, ErrorBody,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthUser;
use crate::db::error::DbContext;
use crate::db::{device, users};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// No vowels (RFC 8628 §6.1: avoids spelling words) and no lookalikes.
const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
const CODE_TTL: time::Duration = time::Duration::minutes(10);
const POLL_INTERVAL_SECS: i32 = 5;
/// Ceiling on live pending logins across all clients: the start endpoint is
/// unauthenticated, so this is what bounds the table.
const MAX_PENDING: i64 = 1000;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(device_start))
        .routes(routes!(device_poll))
        .routes(routes!(device_describe))
        .routes(routes!(device_approve))
        .routes(routes!(device_deny))
}

fn hash_device_code(code: &str) -> String {
    hex::encode(Sha256::digest(code.as_bytes()))
}

fn new_user_code() -> String {
    let bytes: [u8; 8] = rand::random();
    let chars: String = bytes
        .iter()
        .map(|b| USER_CODE_ALPHABET[(*b as usize) % USER_CODE_ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// Accept `bcdf ghjk`, `BCDFGHJK`, `bcdf-ghjk` …; `None` if it can't be a code.
pub fn normalize_user_code(input: &str) -> Option<String> {
    let chars: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (chars.len() == 8 && chars.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)))
        .then(|| format!("{}-{}", &chars[..4], &chars[4..]))
}

#[utoipa::path(
    post,
    path = "/cli/device",
    tag = "device",
    description = "Start a CLI login. Show `user_code` to the user and send them to \
        `verification_uri_complete`, then poll `/cli/device/token`.",
    request_body = DeviceStartRequest,
    responses((status = 200, body = DeviceStartResponse), (status = 400, body = ErrorBody))
)]
async fn device_start(
    State(state): State<AppState>,
    Json(body): Json<DeviceStartRequest>,
) -> AppResult<Json<DeviceStartResponse>> {
    let client_name = body
        .client_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("parcel CLI");
    if client_name.chars().count() > 120 {
        return Err(AppError::BadRequest(
            "client_name must be at most 120 characters".into(),
        ));
    }
    if device::count_pending(&state.db).await? >= MAX_PENDING {
        return Err(AppError::TooManyRequests(
            "too many CLI logins are in progress; try again in a few minutes".into(),
        ));
    }
    let device_code = B64.encode(rand::random::<[u8; 32]>());
    let expires_at = OffsetDateTime::now_utc() + CODE_TTL;

    // A user-code collision is astronomically unlikely but cheap to retry.
    let mut row = None;
    for _ in 0..3 {
        match device::insert(
            &state.db,
            &hash_device_code(&device_code),
            &new_user_code(),
            client_name,
            POLL_INTERVAL_SECS,
            expires_at,
        )
        .await
        {
            Ok(r) => {
                row = Some(r);
                break;
            }
            Err(e) if crate::publish::is_unique_violation(&e) => continue,
            Err(e) => return Err(AppError::Internal(e)),
        }
    }
    let row = row.ok_or_else(|| AppError::Internal(anyhow::anyhow!("user code collisions")))?;
    let verification_uri = format!("{}/device", state.config.app_origin_str());
    Ok(Json(DeviceStartResponse {
        device_code,
        verification_uri_complete: format!("{verification_uri}?code={}", row.user_code),
        user_code: row.user_code,
        verification_uri,
        expires_in: CODE_TTL.whole_seconds(),
        interval: i64::from(POLL_INTERVAL_SECS),
    }))
}

#[utoipa::path(
    post,
    path = "/cli/device/token",
    tag = "device",
    description = "Poll a CLI login. Returns `pending` (or `slow_down`) until the user decides; \
        `approved` carries the new API token exactly once.",
    request_body = DevicePollRequest,
    responses((status = 200, body = DevicePollResponse))
)]
async fn device_poll(
    State(state): State<AppState>,
    Json(body): Json<DevicePollRequest>,
) -> AppResult<Json<DevicePollResponse>> {
    let status_only = |status| {
        Json(DevicePollResponse {
            status,
            token: None,
            email: None,
        })
    };
    let Some(row) = device::poll(&state.db, &hash_device_code(body.device_code.trim())).await?
    else {
        return Ok(status_only(DeviceStatus::Expired));
    };
    let now = OffsetDateTime::now_utc();
    if row.expires_at <= now {
        return Ok(status_only(DeviceStatus::Expired));
    }
    match row.status.as_str() {
        "denied" => Ok(status_only(DeviceStatus::Denied)),
        "redeemed" => Ok(status_only(DeviceStatus::Expired)),
        "approved" => {
            let user_id = row
                .user_id
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("approved without user")))?;
            let user = users::find_by_id(&state.db, user_id)
                .await?
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("approving user vanished")))?;
            // Claim the approval and mint the token in one transaction, so a
            // failure in between leaves the approval redeemable rather than
            // burned. Exactly one concurrent poll can win the claim.
            let mut tx = state
                .db
                .begin()
                .await
                .db_context("beginning device redeem")?;
            if !device::redeem(&mut *tx, row.id).await? {
                // Another poll won the race and already took the token.
                return Ok(status_only(DeviceStatus::Expired));
            }
            let created =
                crate::http::tokens::mint(&mut *tx, user_id, &row.client_name, "device", None)
                    .await?;
            tx.commit().await.db_context("committing device redeem")?;
            tracing::info!(user = %user_id, "CLI device login redeemed");
            Ok(Json(DevicePollResponse {
                status: DeviceStatus::Approved,
                token: Some(created.secret),
                email: Some(user.email),
            }))
        }
        _ => {
            let too_fast = row.last_polled_at.is_some_and(|prev| {
                now - prev < time::Duration::seconds(i64::from(row.interval_secs) - 1)
            });
            Ok(status_only(if too_fast {
                DeviceStatus::SlowDown
            } else {
                DeviceStatus::Pending
            }))
        }
    }
}

async fn pending(state: &AppState, raw_code: &str) -> AppResult<device::DeviceRow> {
    let code = normalize_user_code(raw_code)
        .ok_or_else(|| AppError::NotFound("unknown or expired code".into()))?;
    device::find_pending_by_user_code(&state.db, &code)
        .await?
        .ok_or_else(|| AppError::NotFound("unknown or expired code".into()))
}

#[utoipa::path(
    get,
    path = "/cli/device/{user_code}",
    tag = "device",
    description = "Describe a pending CLI login for the approval page.",
    params(("user_code" = String, Path, description = "The code shown in the terminal")),
    security(("bearer" = [])),
    responses((status = 200, body = DeviceInfo), (status = 404, body = ErrorBody))
)]
async fn device_describe(
    State(state): State<AppState>,
    _caller: AuthUser,
    Path(user_code): Path<String>,
) -> AppResult<Json<DeviceInfo>> {
    let row = pending(&state, &user_code).await?;
    Ok(Json(DeviceInfo {
        user_code: row.user_code,
        client_name: row.client_name,
        created_at: row.created_at,
        expires_at: row.expires_at,
    }))
}

#[utoipa::path(
    post,
    path = "/cli/device/{user_code}/approve",
    tag = "device",
    description = "Approve a pending CLI login as the signed-in user.",
    params(("user_code" = String, Path, description = "The code shown in the terminal")),
    security(("bearer" = [])),
    responses((status = 204, description = "Approved"), (status = 404, body = ErrorBody))
)]
async fn device_approve(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_code): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &caller, &user_code, true).await
}

#[utoipa::path(
    post,
    path = "/cli/device/{user_code}/deny",
    tag = "device",
    description = "Deny a pending CLI login.",
    params(("user_code" = String, Path, description = "The code shown in the terminal")),
    security(("bearer" = [])),
    responses((status = 204, description = "Denied"), (status = 404, body = ErrorBody))
)]
async fn device_deny(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(user_code): Path<String>,
) -> AppResult<StatusCode> {
    decide(&state, &caller, &user_code, false).await
}

async fn decide(
    state: &AppState,
    caller: &AuthUser,
    user_code: &str,
    approve: bool,
) -> AppResult<StatusCode> {
    let row = pending(state, user_code).await?;
    if device::decide(&state.db, &row.user_code, caller.user.id, approve).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("unknown or expired code".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes() {
        let code = new_user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(normalize_user_code(&code), Some(code.clone()));
        assert_eq!(
            normalize_user_code(&code.to_lowercase().replace('-', " ")),
            Some(code)
        );
        assert_eq!(normalize_user_code("ABCD-EFGH"), None); // vowels never appear
        assert_eq!(normalize_user_code("BCDF"), None);
    }
}
