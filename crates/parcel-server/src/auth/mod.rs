//! Request authentication.
//!
//! Three ways in, one result ([`AuthUser`]):
//!
//! * a slauth access token (RS256 JWT, `aud=parcel`) — the web app;
//! * a parcel API token (`pcl_…`) — the CLI and agents;
//! * under `AUTH_MODE=dev`, no header at all — a fixed local user.
//!
//! [`MaybeAuthUser`] is the optional form for routes that also serve public
//! parcels. It never downgrades a *bad* credential to anonymous: a request
//! that sends an invalid token gets a 401, not a public view, so a client
//! with an expired token learns about it.

use std::sync::LazyLock;

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use stridelabs_auth::{bearer_token, PatFormat};
use uuid::Uuid;

use crate::config::AuthMode;
use crate::db::{tokens, users, users::User};
use crate::error::AppError;
use crate::state::AppState;

/// Parcel's API-token format: `pcl_` + 32 alphanumerics; `pcl_` + 8 is the
/// non-secret display prefix.
pub static TOKEN_FORMAT: LazyLock<PatFormat> =
    LazyLock::new(|| PatFormat::new("pcl_").expect("`pcl_` is a valid token prefix"));

/// How the caller authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVia {
    Jwt,
    /// The id of the `api_tokens` row used.
    Token(Uuid),
    Dev,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: User,
    pub via: AuthVia,
}

/// Optional authentication: `None` only when no credential was sent.
pub struct MaybeAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        authenticate(parts, state)
            .await?
            .ok_or_else(|| AppError::Unauthorized("sign in or pass a bearer token".into()))
    }
}

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        Ok(MaybeAuthUser(authenticate(parts, state).await?))
    }
}

async fn authenticate(parts: &Parts, state: &AppState) -> Result<Option<AuthUser>, AppError> {
    let dev = state.config.auth_mode == AuthMode::Dev;
    if !parts.headers.contains_key(AUTHORIZATION) {
        return if dev {
            dev_user(state).await.map(Some)
        } else {
            Ok(None)
        };
    }
    let raw = bearer_token(parts)?;

    if TOKEN_FORMAT.has_prefix(raw) {
        return token_user(state, raw).await.map(Some);
    }
    if dev {
        // The dev bypass has no JWT issuer to check against; any other
        // bearer is simply the dev user, matching the header-less case.
        return dev_user(state).await.map(Some);
    }

    let verified = state.verifier.verify_bearer(raw).await.inspect_err(|e| {
        // The client gets one generic 401; the specific reason is logged.
        tracing::debug!(error = %e, "rejected bearer token");
    })?;
    let user = users::upsert_by_sub(&state.db, &verified.sub, &verified.email, &verified.name)
        .await
        .map_err(AppError::Internal)?;
    Ok(Some(AuthUser {
        user,
        via: AuthVia::Jwt,
    }))
}

async fn token_user(state: &AppState, raw: &str) -> Result<AuthUser, AppError> {
    let hash = stridelabs_auth::pat::hash(raw);
    let token = tokens::find_active_by_hash(&state.db, &hash)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Unauthorized("invalid, revoked or expired token".into()))?;
    let user = users::find_by_id(&state.db, token.user_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Unauthorized("invalid, revoked or expired token".into()))?;

    // Last-used is informational; keep it off the request path.
    let db = state.db.clone();
    let token_id = token.id;
    tokio::spawn(async move {
        if let Err(e) = tokens::touch_last_used(&db, token_id).await {
            tracing::debug!(error = %e, "failed to stamp token last_used_at");
        }
    });

    Ok(AuthUser {
        user,
        via: AuthVia::Token(token.id),
    })
}

async fn dev_user(state: &AppState) -> Result<AuthUser, AppError> {
    let email = &state.config.dev_auth_email;
    let user = users::upsert_by_sub(
        &state.db,
        &format!("dev:{email}"),
        email,
        &state.config.dev_auth_name,
    )
    .await
    .map_err(AppError::Internal)?;
    Ok(AuthUser {
        user,
        via: AuthVia::Dev,
    })
}
