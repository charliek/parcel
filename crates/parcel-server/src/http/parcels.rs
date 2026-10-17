//! `/api/v1/parcels` — list, create, read, update, delete, view.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use parcel_types::{
    CreateParcelMetadata, ErrorBody, ParcelAccess, ParcelDetail, ParcelList, ParcelSummary,
    PublishResult, UpdateParcelRequest, ViewGrant,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AuthUser, MaybeAuthUser};
use crate::content::{self, token::Scope};
use crate::db::parcels;
use crate::error::{busy, AppError, AppResult};
use crate::http::views::{self, owned, readable};
use crate::publish;
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_parcels, create_parcel))
        .routes(routes!(get_parcel, update_parcel, delete_parcel))
        .routes(routes!(view_parcel))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Case-insensitive match on title, description or id.
    pub q: Option<String>,
    /// Page size (1–200, default 50).
    pub limit: Option<i64>,
    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/parcels",
    tag = "parcels",
    description = "List the caller's parcels, most recently updated first.",
    params(ListQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = ParcelList),
        (status = 401, body = ErrorBody),
    )
)]
async fn list_parcels(
    State(state): State<AppState>,
    caller: AuthUser,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<ParcelList>> {
    Ok(Json(list(&state, &caller, &q).await?))
}

/// The caller's parcels (`GET /parcels`, and the MCP `list_parcels` tool).
pub(crate) async fn list(
    state: &AppState,
    caller: &AuthUser,
    q: &ListQuery,
) -> AppResult<ParcelList> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let (rows, total) =
        parcels::list_for_owner(&state.db, caller.user.id, q.q.as_deref(), limit, offset).await?;
    Ok(ParcelList {
        parcels: rows
            .iter()
            .map(|r| views::summary(&state.config, r))
            .collect(),
        total,
    })
}

/// Wait briefly for an upload slot; publishes hold their bytes in memory.
pub(crate) async fn upload_permit(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, AppError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.upload_permits.clone().acquire_owned(),
    )
    .await
    .map_err(|_| busy("the server is busy with other uploads; retry shortly"))?
    .map_err(|e| AppError::Internal(e.into()))
}

pub(crate) async fn publish_result(
    state: &AppState,
    id: &str,
    version: i32,
) -> AppResult<PublishResult> {
    let row = parcels::get(&state.db, id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    let v = parcels::get_version(&state.db, id, version)
        .await?
        .ok_or_else(crate::error::not_found)?;
    let files = parcels::version_files(&state.db, id, version).await?;
    Ok(PublishResult {
        parcel: views::summary(&state.config, &row),
        version: views::version_detail(&v, files),
    })
}

#[utoipa::path(
    post,
    path = "/parcels",
    tag = "parcels",
    description = "Create a parcel from a multipart upload. Send an optional `metadata` part \
        (JSON, `CreateParcelMetadata`) and one `file` part per file whose filename is the \
        published path (e.g. `index.html`, `assets/app.js`).",
    request_body(content = crate::http::openapi::PublishUpload, content_type = "multipart/form-data"),
    security(("bearer" = [])),
    responses(
        (status = 201, body = PublishResult),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
        (status = 413, body = ErrorBody, description = "A size, count or quota limit was exceeded"),
    )
)]
async fn create_parcel(
    State(state): State<AppState>,
    caller: AuthUser,
    multipart: Multipart,
) -> AppResult<(StatusCode, Json<PublishResult>)> {
    let _permit = upload_permit(&state).await?;
    let (meta, uploads) =
        publish::read_multipart::<CreateParcelMetadata>(multipart, &state.config.limits, |m| {
            views::check_visibility(&state.config, m.visibility)
        })
        .await?;
    let id = publish::create(&state, caller.user.id, meta, uploads).await?;
    tracing::info!(parcel = %id, user = %caller.user.id, "parcel created");
    Ok((
        StatusCode::CREATED,
        Json(publish_result(&state, &id, 1).await?),
    ))
}

#[utoipa::path(
    get,
    path = "/parcels/{id}",
    tag = "parcels",
    description = "A parcel with its version history and current files. Public parcels are \
        readable without credentials, internal ones by anyone signed in, private ones by their \
        owner and the people it is shared with.",
    params(("id" = String, Path, description = "Parcel id")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = ParcelDetail),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_parcel(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ParcelDetail>> {
    Ok(Json(detail(&state, caller.as_ref(), &id).await?))
}

/// A parcel the caller may read, with its history and current files.
pub(crate) async fn detail(
    state: &AppState,
    caller: Option<&AuthUser>,
    id: &str,
) -> AppResult<ParcelDetail> {
    let (row, access) = readable(state, id, caller).await?;
    let versions = parcels::list_versions(&state.db, id).await?;
    let files = parcels::version_files(&state.db, id, row.current_version).await?;
    Ok(ParcelDetail {
        parcel: views::summary(&state.config, &row),
        is_owner: access == ParcelAccess::Owner,
        access,
        owner_name: views::owner_name(state, &row, caller).await?,
        versions: versions.iter().map(views::version_summary).collect(),
        files: files.into_iter().map(Into::into).collect(),
    })
}

#[utoipa::path(
    patch,
    path = "/parcels/{id}",
    tag = "parcels",
    description = "Change a parcel's title, description or visibility. Files change only \
        through new versions.",
    params(("id" = String, Path, description = "Parcel id")),
    request_body = UpdateParcelRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = ParcelSummary),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn update_parcel(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateParcelRequest>,
) -> AppResult<Json<ParcelSummary>> {
    Ok(Json(update_metadata(&state, &caller, &id, &body).await?))
}

/// Change an owned parcel's title, description or visibility.
pub(crate) async fn update_metadata(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    body: &UpdateParcelRequest,
) -> AppResult<ParcelSummary> {
    owned(state, id, caller).await?;
    views::check_visibility(&state.config, body.visibility)?;
    let title = body.title.as_deref().map(str::trim);
    if title.is_some_and(|t| t.is_empty() || t.chars().count() > 200) {
        return Err(AppError::BadRequest(
            "title must be 1–200 characters".into(),
        ));
    }
    let description = body.description.as_deref().map(str::trim);
    if description.is_some_and(|d| d.chars().count() > 4000) {
        return Err(AppError::BadRequest(
            "description must be at most 4000 characters".into(),
        ));
    }
    parcels::update_metadata(&state.db, id, title, description, body.visibility).await?;
    let row = parcels::get(&state.db, id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    Ok(views::summary(&state.config, &row))
}

#[utoipa::path(
    delete,
    path = "/parcels/{id}",
    tag = "parcels",
    description = "Delete a parcel and all its versions. Its URL stops working immediately.",
    params(("id" = String, Path, description = "Parcel id")),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn delete_parcel(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    delete(&state, &caller, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete an owned parcel and all its versions.
pub(crate) async fn delete(state: &AppState, caller: &AuthUser, id: &str) -> AppResult<()> {
    owned(state, id, caller).await?;
    parcels::delete(&state.db, id).await?;
    tracing::info!(parcel = %id, user = %caller.user.id, "parcel deleted");
    Ok(())
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ViewQuery {
    /// Version to view (default: current).
    pub version: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/view",
    tag = "parcels",
    description = "A short-lived signed URL on the sandboxed content origin for one version's \
        entry page — what the viewer iframe loads.",
    params(("id" = String, Path, description = "Parcel id"), ViewQuery),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = ViewGrant),
        (status = 404, body = ErrorBody),
    )
)]
async fn view_parcel(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> AppResult<Json<ViewGrant>> {
    let (row, access) = readable(&state, &id, caller.as_ref()).await?;
    let version = q.version.unwrap_or(row.current_version);
    let v = parcels::get_version(&state.db, &id, version)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version} not found")))?;
    let scope = match (access, &caller) {
        (ParcelAccess::Owner, _) => Scope::Owner,
        (ParcelAccess::Viewer | ParcelAccess::Commenter, Some(c)) => Scope::Member(c.user.id),
        _ => Scope::Public,
    };
    let (content_url, expires_at) = content::grant_url(&state, &id, version, &v.entry, scope);
    Ok(Json(ViewGrant {
        parcel: views::summary(&state.config, &row),
        version: views::version_summary(&v),
        is_owner: access == ParcelAccess::Owner,
        access,
        owner_name: views::owner_name(&state, &row, caller.as_ref()).await?,
        content_url,
        expires_at,
    }))
}
