//! Request authentication.
//!
//! Three ways in, one result ([`AuthUser`]):
//!
//! * a web session cookie, issued at `/auth/callback` after an OIDC sign-in
//!   (see [`oidc`] and [`cookie`]) — the web app;
//! * a parcel API token (`pcl_…`) — the CLI and agents;
//! * under `AUTH_MODE=dev`, no credential at all — a fixed local user.
//!
//! [`MaybeAuthUser`] is the optional form for routes that also serve public
//! parcels. It never downgrades a *bad* credential to anonymous: an invalid
//! token or an expired session gets a 401 (which also clears the cookie),
//! not a public view, so the client learns it was signed out.

pub mod cookie;
pub mod oidc;

use std::sync::LazyLock;

use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use stridelabs_auth::{bearer_token, PatFormat};
use time::OffsetDateTime;
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
    /// A web session cookie.
    Session,
    /// The id of the `api_tokens` row used.
    Token(Uuid),
    /// An MCP access token from the MCP provider (only on `/mcp`).
    Mcp,
    Dev,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: User,
    pub via: AuthVia,
    /// For sessions: the provider id it signed in with.
    pub provider: Option<String>,
}

/// Optional authentication: `None` only when no credential was sent.
pub struct MaybeAuthUser(pub Option<AuthUser>);

/// An authentication failure, which also clears a session cookie that
/// stopped working so the browser doesn't keep sending it.
pub struct AuthRejection {
    error: AppError,
    clear_cookie: Option<HeaderValue>,
}

impl From<AppError> for AuthRejection {
    fn from(error: AppError) -> Self {
        Self {
            error,
            clear_cookie: None,
        }
    }
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let mut response = self.error.into_response();
        if let Some(c) = self.clear_cookie {
            cookie::append(response.headers_mut(), [c]);
        }
        response
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, AuthRejection> {
        authenticate(parts, state)
            .await?
            .ok_or_else(|| AppError::Unauthorized("sign in or pass a bearer token".into()).into())
    }
}

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, AuthRejection> {
        Ok(MaybeAuthUser(authenticate(parts, state).await?))
    }
}

async fn authenticate(parts: &Parts, state: &AppState) -> Result<Option<AuthUser>, AuthRejection> {
    let dev = state.config.auth_mode == AuthMode::Dev;
    if parts.headers.contains_key(AUTHORIZATION) {
        let raw = bearer_token(parts).map_err(AppError::from)?;
        if TOKEN_FORMAT.has_prefix(raw) {
            return Ok(Some(token_user(state, raw).await?));
        }
        if dev {
            // The dev bypass treats any other bearer as the dev user,
            // matching the header-less case.
            return Ok(Some(dev_user(state).await?));
        }
        return Err(AppError::Unauthorized(
            "unsupported bearer token: pass a parcel API token (pcl_…)".into(),
        )
        .into());
    }
    if dev {
        return Ok(Some(dev_user(state).await?));
    }
    session_user(parts, state).await
}

/// The signed-in browser, if its session cookie is present and usable.
async fn session_user(parts: &Parts, state: &AppState) -> Result<Option<AuthUser>, AuthRejection> {
    let config = &state.config;
    let Some(raw) = cookie::read(&parts.headers, cookie::names(config).session) else {
        return Ok(None);
    };
    // Only the app's own pages (and top-level navigations someone typed or
    // followed) speak with the session; a request from any other site —
    // including sibling subdomains, which SameSite treats as same-site —
    // is anonymous.
    let fetch_site = parts
        .headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok());
    if !matches!(fetch_site, None | Some("same-origin" | "none")) {
        return Ok(None);
    }
    let ended = || AuthRejection {
        error: AppError::Unauthorized("your session has ended; sign in again".into()),
        clear_cookie: Some(cookie::clear_session(config)),
    };
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let session = cookie::open_session(config, raw, now).ok_or_else(ended)?;
    let user = users::find_by_id(&state.db, session.user_id)
        .await
        .map_err(AppError::Internal)?
        .filter(|u| u.disabled_at.is_none() && u.session_epoch == session.epoch)
        .ok_or_else(ended)?;
    // A provider removed from config takes its sessions with it.
    if config.provider(&session.provider).is_none() {
        return Err(ended());
    }

    // CSRF: a state-changing request riding the cookie must come from the
    // app itself. Browsers send `Origin` on every non-GET fetch.
    let safe = matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe {
        let origin = parts.headers.get(ORIGIN).and_then(|v| v.to_str().ok());
        if origin != Some(config.app_origin_str().as_str()) {
            return Err(AppError::Forbidden("cross-origin request refused".into()).into());
        }
    }
    Ok(Some(AuthUser {
        user,
        via: AuthVia::Session,
        provider: Some(session.provider),
    }))
}

pub(crate) async fn token_user(state: &AppState, raw: &str) -> Result<AuthUser, AppError> {
    let hash = stridelabs_auth::pat::hash(raw);
    let invalid = || AppError::Unauthorized("invalid, revoked or expired token".into());
    let token = tokens::find_active_by_hash(&state.db, &hash)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(invalid)?;
    let user = users::find_by_id(&state.db, token.user_id)
        .await
        .map_err(AppError::Internal)?
        .filter(|u| u.disabled_at.is_none())
        .ok_or_else(invalid)?;
    // A token outlives its owner's account at the provider; requiring a
    // recent web sign-in is what lets offboarding there reach it.
    if let Some(max_age) = state.config.token_login_max_age {
        let age = OffsetDateTime::now_utc() - user.last_login_at;
        if state.config.auth_mode != AuthMode::Dev && age > max_age {
            return Err(AppError::Unauthorized(format!(
                "this token is paused: sign in to Parcel on the web ({}) to keep using it \
                 (tokens need a web sign-in at least every {} days)",
                state.config.app_origin_str(),
                max_age.as_secs() / 86400
            )));
        }
    }

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
        provider: None,
    })
}

/// The person behind an MCP access token that `provider` issued for
/// `subject`. An identity seen before is that user. The first connection
/// signs the person in as the web callback would, under the same email and
/// domain rules, since connecting is an interactive sign-in at the provider
/// (with its consent screen); later calls only look the identity up.
pub async fn mcp_user(
    state: &AppState,
    provider_id: &str,
    subject: &str,
    email: Option<&str>,
    name: &str,
) -> Result<AuthUser, AppError> {
    let provider = state
        .config
        .provider(provider_id)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("MCP provider not configured")))?;
    let email = email.and_then(|e| parcel_types::normalize_email(e).ok());
    let domain_refused = || AppError::Forbidden("your email domain can't sign in to Parcel".into());
    let user = match users::find_by_identity(&state.db, provider_id, subject)
        .await
        .map_err(AppError::Internal)?
    {
        // The provider's allowed domains apply on every call, as they do
        // at every web sign-in, so narrowing them cuts connections off too.
        // The token's email counts only from a provider trusted to vouch
        // for it; otherwise the one a verified sign-in stored.
        Some(user) => {
            let current = match email.as_deref() {
                Some(e) if provider.trust_email => e,
                _ => &user.email,
            };
            if !provider.allows_domain(parcel_types::email_domain(current)) {
                return Err(domain_refused());
            }
            user
        }
        None => {
            let email = email.ok_or_else(|| {
                AppError::Unauthorized("this sign-in carries no usable email".into())
            })?;
            // Access tokens carry no `email_verified`, so a first connection
            // can only trust the address when the provider is trusted to.
            if !provider.trust_email {
                return Err(AppError::Unauthorized(format!(
                    "sign in to Parcel on the web ({}) once before connecting",
                    state.config.app_origin_str()
                )));
            }
            if !provider.allows_domain(parcel_types::email_domain(&email)) {
                return Err(domain_refused());
            }
            let user = users::sign_in(&state.db, provider_id, subject, &email, name)
                .await
                .map_err(AppError::Internal)?;
            tracing::info!(user = %user.id, provider = %provider_id, "signed in through MCP");
            user
        }
    };
    if user.disabled_at.is_some() {
        return Err(AppError::Forbidden("this account is disabled".into()));
    }
    Ok(AuthUser {
        user,
        via: AuthVia::Mcp,
        provider: Some(provider_id.to_string()),
    })
}

async fn dev_user(state: &AppState) -> Result<AuthUser, AppError> {
    let email = state.config.dev_auth_email.to_lowercase();
    let user = match users::find_by_identity(&state.db, "dev", &email)
        .await
        .map_err(AppError::Internal)?
    {
        Some(user) => user,
        None => users::sign_in(
            &state.db,
            "dev",
            &email,
            &email,
            &state.config.dev_auth_name,
        )
        .await
        .map_err(AppError::Internal)?,
    };
    Ok(AuthUser {
        user,
        via: AuthVia::Dev,
        provider: None,
    })
}
