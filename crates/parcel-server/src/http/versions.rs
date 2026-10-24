//! `/api/v1/parcels/{id}/versions` — history, new versions, restore, raw files.

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use parcel_types::{ErrorBody, NewVersionMetadata, PublishResult, VersionDetail, VersionList};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AuthUser, MaybeAuthUser};
use crate::content::{self, SANDBOX_FLAGS};
use crate::db::parcels;
use crate::error::{AppError, AppResult};
use crate::http::parcels::{publish_result, upload_permit};
use crate::http::views::{self, owned, parse_version, readable};
use crate::publish::{self, VersionSource};
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_versions, create_version))
        .routes(routes!(get_version))
        .routes(routes!(restore_version))
        .routes(routes!(get_file))
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/versions",
    tag = "versions",
    description = "Every version of a parcel, newest first.",
    params(("id" = String, Path, description = "Parcel id")),
    security((), ("bearer" = [])),
    responses((status = 200, body = VersionList), (status = 404, body = ErrorBody))
)]
async fn list_versions(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<VersionList>> {
    // The access decides whether the versions carry their source (the owner's only).
    let (_, access) = readable(&state, &id, caller.as_ref()).await?;
    let versions = parcels::list_versions(&state.db, &id).await?;
    Ok(Json(VersionList {
        versions: versions
            .iter()
            .map(|v| views::version_summary(v, access))
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/parcels/{id}/versions",
    tag = "versions",
    description = "Publish a new version (the URL stays the same). In `merge` mode (default) \
        uploaded files are added or replaced, others are kept, and `remove` drops paths; in \
        `replace` mode the version is exactly the uploaded files. `base_version` makes the \
        publish fail with 409 if someone else published first.",
    params(("id" = String, Path, description = "Parcel id")),
    request_body(content = crate::http::openapi::NewVersionUpload, content_type = "multipart/form-data"),
    security(("bearer" = [])),
    responses(
        (status = 201, body = PublishResult),
        (status = 400, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody, description = "`base_version` is no longer current"),
        (status = 413, body = ErrorBody),
    )
)]
async fn create_version(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
    multipart: Multipart,
) -> AppResult<(StatusCode, Json<PublishResult>)> {
    owned(&state, &id, &caller).await?;
    let _permit = upload_permit(&state).await?;
    let (meta, uploads) =
        publish::read_multipart::<NewVersionMetadata>(multipart, &state.config.limits, |m| {
            views::check_visibility(&state.config, m.visibility)
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(publish_version(&state, &caller, &id, meta, uploads).await?),
    ))
}

/// Publish a new version from checked uploads. The caller has already
/// checked that it owns the parcel (`owned`, before buffering any bytes)
/// and that `meta.visibility` is allowed; `add_version` re-checks ownership
/// under the row lock.
pub(crate) async fn publish_version(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    meta: NewVersionMetadata,
    uploads: Vec<publish::Upload>,
) -> AppResult<PublishResult> {
    let version = publish::add_version(
        state,
        id,
        caller.user.id,
        VersionSource::Upload { meta, uploads },
    )
    .await?;
    tracing::info!(parcel = %id, version, "version published");
    publish_result(state, id, version, false).await
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/versions/{version}",
    tag = "versions",
    description = "One version and its files. `version` is a number or `latest`.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("version" = String, Path, description = "Version number or `latest`"),
    ),
    security((), ("bearer" = [])),
    responses((status = 200, body = VersionDetail), (status = 404, body = ErrorBody))
)]
async fn get_version(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path((id, version)): Path<(String, String)>,
) -> AppResult<Json<VersionDetail>> {
    let (row, access) = readable(&state, &id, caller.as_ref()).await?;
    let version = parse_version(&version, row.current_version)?;
    let v = parcels::get_version(&state.db, &id, version)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version} not found")))?;
    let files = parcels::version_files(&state.db, &id, version).await?;
    Ok(Json(views::version_detail(&v, files, access)))
}

#[utoipa::path(
    post,
    path = "/parcels/{id}/versions/{version}/restore",
    tag = "versions",
    description = "Publish a new version whose files are a copy of an earlier one.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("version" = i32, Path, description = "Version to restore"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 201, body = PublishResult),
        (status = 404, body = ErrorBody),
        (status = 413, body = ErrorBody),
    )
)]
async fn restore_version(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, from_version)): Path<(String, i32)>,
) -> AppResult<(StatusCode, Json<PublishResult>)> {
    owned(&state, &id, &caller).await?;
    let new_version = publish::add_version(
        &state,
        &id,
        caller.user.id,
        VersionSource::Restore { from_version },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(publish_result(&state, &id, new_version, false).await?),
    ))
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/versions/{version}/files/{*path}",
    tag = "versions",
    description = "Raw bytes of one file, as an attachment. This is how the CLI and agents read \
        a parcel's source; browsers view parcels through the sandboxed content origin instead.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("version" = String, Path, description = "Version number or `latest`"),
        ("path" = String, Path, description = "Published path, e.g. `index.html` or `assets/app.js`"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "File bytes", content_type = "application/octet-stream", body = Vec<u8>),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_file(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path((id, version, path)): Path<(String, String, String)>,
) -> AppResult<Response> {
    let (file, bytes) = read_file(&state, caller.as_ref(), &id, &version, &path).await?;

    let mut response = Response::new(Body::from(bytes.clone()));
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&file.content_type) {
        headers.insert(CONTENT_TYPE, v);
    }
    headers.insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    // This is the APP origin: user HTML must never render here. Attachment
    // disposition plus a sandbox CSP make a direct visit inert.
    let name = parcel_types::paths::file_name(&file.path).replace(['"', '\\'], "_");
    if let Ok(v) = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        name.chars()
            .filter(|c| c.is_ascii() && !c.is_ascii_control())
            .collect::<String>()
    )) {
        headers.insert(CONTENT_DISPOSITION, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("sandbox {SANDBOX_FLAGS}")) {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, v);
    }
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    Ok(response)
}

/// One file of a version the caller may read: its row and bytes.
/// `version` is a number or `latest`.
pub(crate) async fn read_file(
    state: &AppState,
    caller: Option<&AuthUser>,
    id: &str,
    version: &str,
    path: &str,
) -> AppResult<(parcels::FileRow, bytes::Bytes)> {
    let file = file_row(state, caller, id, version, path).await?;
    let bytes = load_file(state, &file).await?;
    Ok((file, bytes))
}

/// The row of one file of a version the caller may read.
pub(crate) async fn file_row(
    state: &AppState,
    caller: Option<&AuthUser>,
    id: &str,
    version: &str,
    path: &str,
) -> AppResult<parcels::FileRow> {
    let (row, _) = readable(state, id, caller).await?;
    let version = parse_version(version, row.current_version)?;
    parcels::find_file(&state.db, id, version, path)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{path:?} not found in version {version}")))
}

pub(crate) async fn load_file(
    state: &AppState,
    file: &parcels::FileRow,
) -> AppResult<bytes::Bytes> {
    content::load_blob(state, &file.blob_sha256)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("blob {} missing", file.blob_sha256)))
}
