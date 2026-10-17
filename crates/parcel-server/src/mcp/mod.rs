//! The MCP server at `<app origin>/mcp`: parcel's tools for agents (Claude,
//! Claude Code, …), on stridelabs-mcp.
//!
//! Two credentials reach it. An MCP client signs in through the MCP provider
//! (slauth) and sends an access token minted for this resource (`aud` = the
//! resource URL), which works nowhere else in the app; the web session and
//! the REST API never accept one. A client that can only send a header can
//! use a `pcl_` API token instead, under the same rules as the REST API.
//! Either way the token is verified and stripped before any tool runs, and
//! each tool resolves its caller to a parcel user and then calls the same
//! functions as the REST handlers, so access, limits and sharing rules are
//! the API's own.

mod tools;

use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::header::CONTENT_LENGTH;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use stridelabs_mcp::{
    AuthMethod, McpAuthLayer, McpVerifier, Principal, ProtectedResource, ToolError, VerifyError,
};
use uuid::Uuid;

use crate::auth::{self, AuthUser, AuthVia};
use crate::db::users;
use crate::error::AppError;
use crate::http::parcels::upload_permit;
use crate::state::AppState;

pub use tools::ParcelMcp;

/// Read and change parcels; both are allowed for connectors (slauth's
/// `apps.yaml`), so a connected agent can publish.
pub const SCOPE_READ: &str = "parcel:read";
pub const SCOPE_WRITE: &str = "parcel:write";

/// The largest MCP request. rmcp reads and parses a whole request before a
/// tool runs, and a publish then decodes its files, so one request costs a
/// few times its size; far below the multipart upload limit, which the CLI
/// is for.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Requests bigger than this (or of unknown length) wait for an upload
/// permit before rmcp reads them, so concurrent publishes stay bounded by
/// `PARCEL_MAX_CONCURRENT_UPLOADS` as on the REST API; small ones don't.
const GATED_BODY_BYTES: u64 = 1024 * 1024;

fn max_body_bytes(state: &AppState) -> usize {
    usize::try_from(state.config.limits.max_upload_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(1024 * 1024)
        .min(MAX_BODY_BYTES)
}

/// Hold an upload permit for the life of a large request.
async fn upload_gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let length = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if length.is_some_and(|n| n <= GATED_BODY_BYTES) {
        return next.run(request).await;
    }
    match upload_permit(&state).await {
        Ok(_permit) => next.run(request).await,
        Err(e) => e.into_response(),
    }
}

/// The MCP endpoint and its RFC 9728 metadata, to merge into the app router,
/// or `None` when the MCP server is off.
pub fn router(state: &AppState) -> Option<Router<AppState>> {
    let mcp = state.config.mcp.as_ref()?;
    let resource = ProtectedResource::new(
        &state.config.mcp_resource_url(),
        mcp.issuer.as_str(),
        [SCOPE_READ, SCOPE_WRITE],
    )
    .expect("config loading validated the MCP resource URL");

    let tokens = {
        let state = state.clone();
        move |raw: String| {
            let state = state.clone();
            async move { verify_api_token(&state, &raw).await }
        }
    };
    let verifier = McpVerifier::new(&resource, mcp.jwks_url.as_str(), state.http.clone())
        .with_credential(auth::TOKEN_FORMAT.prefix(), tokens);
    let config = stridelabs_mcp::server_config(&resource, Vec::<String>::new())
        .with_max_request_body_bytes(max_body_bytes(state));

    let handler_state = state.clone();
    let endpoint = stridelabs_mcp::router(McpAuthLayer::new(verifier), config, move || {
        Ok(ParcelMcp::new(handler_state.clone()))
    })
    .layer(middleware::from_fn_with_state(state.clone(), upload_gate));
    Some(
        Router::new()
            .merge(stridelabs_mcp::metadata_router(&resource))
            .merge(endpoint),
    )
}

/// A `pcl_` token's principal subject: `pcl:<parcel user id>`, so a later
/// credential type's subjects can't be mistaken for parcel user ids.
const TOKEN_SUBJECT: &str = "pcl:";

/// A `pcl_` token as an MCP credential: the REST API's checks (revoked,
/// expired, owner disabled, paused for want of a web sign-in), with every
/// tool's scopes, as on the API.
async fn verify_api_token(state: &AppState, raw: &str) -> Result<Principal, VerifyError> {
    match auth::token_user(state, raw).await {
        Ok(caller) => Ok(Principal::new(
            format!("{TOKEN_SUBJECT}{}", caller.user.id),
            [SCOPE_READ, SCOPE_WRITE],
            AuthMethod::Token,
        )),
        Err(AppError::Unauthorized(message)) => Err(VerifyError::invalid(message)),
        Err(e) => {
            tracing::warn!(error = %e, "MCP token check failed");
            Err(VerifyError::unavailable("the token store is unavailable"))
        }
    }
}

/// The parcel user a verified principal acts as.
pub(crate) async fn caller(state: &AppState, principal: &Principal) -> Result<AuthUser, ToolError> {
    match principal.auth_method {
        AuthMethod::Jwt => {
            let provider = state
                .config
                .mcp
                .as_ref()
                .map(|m| m.provider.as_str())
                .ok_or_else(|| ToolError::internal("MCP is not configured"))?;
            auth::mcp_user(
                state,
                provider,
                &principal.subject,
                principal.email.as_deref(),
                principal.name.as_deref().unwrap_or_default(),
            )
            .await
            .map_err(tool_error)
        }
        // Minted by `verify_api_token` from a token row just checked; the
        // user may have been disabled since.
        AuthMethod::Token => {
            let id = principal
                .subject
                .strip_prefix(TOKEN_SUBJECT)
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or_else(|| ToolError::denied("unsupported credential"))?;
            let user = users::find_by_id(&state.db, id)
                .await
                .map_err(ToolError::internal)?
                .filter(|u| u.disabled_at.is_none())
                .ok_or_else(|| ToolError::denied("this account is disabled"))?;
            Ok(AuthUser {
                user,
                via: AuthVia::Mcp,
                provider: None,
            })
        }
        _ => Err(ToolError::denied("unsupported credential")),
    }
}

/// An API error as the model should see it. Client errors carry messages
/// written for the caller already (they are the REST API's); anything else
/// is internal and stays out of the reply.
pub(crate) fn tool_error(e: AppError) -> ToolError {
    match e {
        AppError::Unauthorized(m) | AppError::Forbidden(m) | AppError::TooManyRequests(m) => {
            ToolError::denied(m)
        }
        AppError::BadRequest(m) | AppError::NotFound(m) | AppError::Conflict(m) => {
            ToolError::user(m)
        }
        AppError::Custom {
            status, message, ..
        } if status.is_client_error() => ToolError::user(message),
        other => {
            tracing::error!(error = %other, "MCP tool failed");
            ToolError::internal(other)
        }
    }
}

/// How long one tool call may run. A publish waits up to 30 s for an
/// upload slot and then writes blobs, so it gets more than the default.
pub(crate) const PUBLISH_TIMEOUT: Duration = Duration::from_secs(120);
