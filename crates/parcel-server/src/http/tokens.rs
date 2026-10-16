//! `/api/v1/tokens` — `pcl_…` API tokens for the CLI and agents.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use parcel_types::{CreateTokenRequest, CreatedToken, ErrorBody, TokenList};
use time::{Duration, OffsetDateTime};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::auth::{AuthUser, AuthVia, TOKEN_FORMAT};
use crate::db::tokens::{self, NewToken};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_tokens, create_token))
        .routes(routes!(revoke_current_token))
        .routes(routes!(revoke_token))
}

/// Mint and store a token for `user_id`; returns the row and the raw secret.
/// Takes any executor so the device flow can mint inside its redeem
/// transaction.
pub(crate) async fn mint<'e>(
    db: impl sqlx::PgExecutor<'e>,
    user_id: Uuid,
    name: &str,
    created_via: &str,
    expires_at: Option<OffsetDateTime>,
) -> anyhow::Result<CreatedToken> {
    let generated = TOKEN_FORMAT.generate();
    let row = tokens::insert(
        db,
        &NewToken {
            user_id,
            name,
            hash: &generated.hash,
            prefix_display: &generated.prefix_display,
            created_via,
            expires_at,
        },
    )
    .await?;
    Ok(CreatedToken {
        token: row.into(),
        secret: generated.raw,
    })
}

#[utoipa::path(
    get,
    path = "/tokens",
    tag = "tokens",
    description = "The caller's active API tokens (secrets are never returned).",
    security(("bearer" = [])),
    responses((status = 200, body = TokenList), (status = 401, body = ErrorBody))
)]
async fn list_tokens(
    State(state): State<AppState>,
    caller: AuthUser,
) -> AppResult<Json<TokenList>> {
    let rows = tokens::list_active(&state.db, caller.user.id).await?;
    Ok(Json(TokenList {
        tokens: rows.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/tokens",
    tag = "tokens",
    description = "Mint an API token. The `secret` is returned exactly once.",
    request_body = CreateTokenRequest,
    security(("bearer" = [])),
    responses(
        (status = 201, body = CreatedToken),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
async fn create_token(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(body): Json<CreateTokenRequest>,
) -> AppResult<(StatusCode, Json<CreatedToken>)> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::BadRequest("name must be 1–100 characters".into()));
    }
    let expires_at = match body.expires_in_days {
        None => None,
        Some(days @ 1..=3650) => Some(OffsetDateTime::now_utc() + Duration::days(i64::from(days))),
        Some(_) => {
            return Err(AppError::BadRequest(
                "expires_in_days must be between 1 and 3650".into(),
            ))
        }
    };
    let created = mint(&state.db, caller.user.id, name, "web", expires_at).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[utoipa::path(
    delete,
    path = "/tokens/current",
    tag = "tokens",
    description = "Revoke the API token this request is authenticated with (`parcel logout`).",
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, body = ErrorBody, description = "The request was not made with an API token"),
        (status = 401, body = ErrorBody),
    )
)]
async fn revoke_current_token(
    State(state): State<AppState>,
    caller: AuthUser,
) -> AppResult<StatusCode> {
    let AuthVia::Token(id) = caller.via else {
        return Err(AppError::BadRequest(
            "this request was not made with an API token".into(),
        ));
    };
    tokens::revoke(&state.db, caller.user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/tokens/{id}",
    tag = "tokens",
    description = "Revoke one of the caller's tokens.",
    params(("id" = String, Path, description = "Token id")),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 404, body = ErrorBody),
    )
)]
async fn revoke_token(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::NotFound("token not found".into()))?;
    if tokens::revoke(&state.db, caller.user.id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("token not found".into()))
    }
}
