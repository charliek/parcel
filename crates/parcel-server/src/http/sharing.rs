//! Sharing with named people: `/api/v1/parcels/{id}/shares` and the
//! caller's `/api/v1/shared` list.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use parcel_types::{
    email_domain, normalize_email, ErrorBody, PutShareRequest, Share, ShareList, SharedParcel,
    SharedParcelList,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthUser;
use crate::db::shares::{self, PutRefused, ShareRow, MAX_SHARES_PER_PARCEL};
use crate::db::users::display_name;
use crate::error::{not_found, AppError, AppResult};
use crate::http::views::{self, owned};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_shares))
        .routes(routes!(put_share, delete_share))
        .routes(routes!(list_shared_with_me))
}

fn share(row: ShareRow) -> Share {
    Share {
        role: row.role(),
        email: row.email,
        name: row.name,
        created_at: row.created_at,
    }
}

fn parse_email(raw: &str) -> Result<String, AppError> {
    normalize_email(raw).map_err(AppError::BadRequest)
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/shares",
    tag = "sharing",
    description = "The people a parcel is shared with, by email. Owner only.",
    params(("id" = String, Path, description = "Parcel id")),
    security(("bearer" = [])),
    responses(
        (status = 200, body = ShareList),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn list_shares(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ShareList>> {
    Ok(Json(list(&state, &caller, &id).await?))
}

/// Who an owned parcel is shared with.
pub(crate) async fn list(state: &AppState, caller: &AuthUser, id: &str) -> AppResult<ShareList> {
    owned(state, id, caller).await?;
    let rows = shares::list(&state.db, id).await?;
    Ok(ShareList {
        shares: rows.into_iter().map(share).collect(),
    })
}

#[utoipa::path(
    put,
    path = "/parcels/{id}/shares/{email}",
    tag = "sharing",
    description = "Share a parcel with someone by email, or change their role. They get access \
        the next time they open it signed in with that (verified) email, including on their \
        first sign-in. Owner only.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("email" = String, Path, description = "Their email (any case)"),
    ),
    request_body = PutShareRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = Share),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn put_share(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, email)): Path<(String, String)>,
    Json(body): Json<PutShareRequest>,
) -> AppResult<Json<Share>> {
    Ok(Json(put(&state, &caller, &id, &email, body.role).await?))
}

/// Share an owned parcel with `email` as `role`, or change their role.
pub(crate) async fn put(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    email: &str,
    role: parcel_types::ShareRole,
) -> AppResult<Share> {
    owned(state, id, caller).await?;
    let email = parse_email(email)?;
    if email == caller.user.email.to_lowercase() {
        return Err(AppError::BadRequest(
            "that's you: owners always have access".into(),
        ));
    }
    if !state.config.domain_can_sign_in(email_domain(&email)) {
        return Err(AppError::BadRequest(format!(
            "nobody at @{} can sign in to this Parcel, so sharing with them would do nothing",
            email_domain(&email)
        )));
    }
    match shares::put(&state.db, id, &email, role).await? {
        Ok(row) => {
            tracing::info!(parcel = %id, user = %caller.user.id, role = role.as_str(), "parcel shared");
            Ok(share(row))
        }
        Err(PutRefused::ParcelGone) => Err(not_found()),
        Err(PutRefused::TooMany) => Err(AppError::BadRequest(format!(
            "a parcel can be shared with at most {MAX_SHARES_PER_PARCEL} people; \
             make it internal to open it to everyone signed in"
        ))),
    }
}

#[utoipa::path(
    delete,
    path = "/parcels/{id}/shares/{email}",
    tag = "sharing",
    description = "Stop sharing with someone. Pages they have open stop loading at once. \
        Owner only.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("email" = String, Path, description = "Their email (any case)"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "No longer shared with them"),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn delete_share(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, email)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    remove(&state, &caller, &id, &email).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop sharing an owned parcel with `email`.
pub(crate) async fn remove(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    email: &str,
) -> AppResult<()> {
    owned(state, id, caller).await?;
    let email = parse_email(email)?;
    if !shares::delete(&state.db, id, &email).await? {
        return Err(AppError::NotFound(format!(
            "this parcel isn't shared with {email}"
        )));
    }
    tracing::info!(parcel = %id, user = %caller.user.id, "parcel unshared");
    Ok(())
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SharedQuery {
    /// Page size (1–200, default 50).
    pub limit: Option<i64>,
    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/shared",
    tag = "sharing",
    description = "Parcels other people shared with the caller by email, most recently updated \
        first. (Internal parcels anyone signed in can open aren't listed.)",
    params(SharedQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = SharedParcelList),
        (status = 401, body = ErrorBody),
    )
)]
async fn list_shared_with_me(
    State(state): State<AppState>,
    caller: AuthUser,
    Query(q): Query<SharedQuery>,
) -> AppResult<Json<SharedParcelList>> {
    Ok(Json(shared_with_me(&state, &caller, &q).await?))
}

/// Parcels other people shared with the caller.
pub(crate) async fn shared_with_me(
    state: &AppState,
    caller: &AuthUser,
    q: &SharedQuery,
) -> AppResult<SharedParcelList> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let (rows, total) =
        shares::shared_with(&state.db, &caller.user.email, caller.user.id, limit, offset).await?;
    Ok(SharedParcelList {
        parcels: rows
            .into_iter()
            .map(|r| SharedParcel {
                parcel: views::summary(&state.config, &r.parcel),
                owner_name: display_name(&r.owner_name, &r.owner_email),
                role: parcel_types::ShareRole::parse(&r.role).unwrap_or_default(),
                shared_at: r.shared_at,
            })
            .collect(),
        total,
    })
}
