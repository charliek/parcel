//! `/api/v1/parcels` — list, create, read, update, delete, view.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use parcel_types::slugs::normalize_slug;
use parcel_types::tags::{normalize_set, normalize_tags};
use parcel_types::{media, paths};
use parcel_types::{
    ArchivedFilter, CreateParcelMetadata, ErrorBody, ListScope, ParcelAccess, ParcelDetail,
    ParcelList, ParcelListItem, ParcelSummary, PublishResult, ShareRole, SharedInfo,
    UpdateParcelRequest, ViewGrant,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AuthUser, MaybeAuthUser};
use crate::content::{self, token::Scope};
use crate::db::parcels::{self, ListFilter, ParcelRow, Patch, PatchRefused, SearchRow, TagsChange};
use crate::db::users;
use crate::error::{busy, slug_error, slug_taken, tag_error, AppError, AppResult};
use crate::http::views::{self, owned, readable, Audience};
use crate::publish;
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_parcels, create_parcel))
        .routes(routes!(get_parcel, update_parcel, delete_parcel))
        .routes(routes!(view_parcel))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Search: a case-insensitive substring of the id, title, description or slug,
    /// or full text (title, description, tags and the entry page's text;
    /// web-search syntax: `"exact phrase"`, `-word`, `or`). Results are then
    /// ordered by relevance, and carry `match` when the page text matched.
    pub q: Option<String>,
    /// `mine` (default), `shared` (shared with you by email) or `all`.
    #[param(inline)]
    pub scope: Option<ListScope>,
    /// Only parcels carrying this tag; repeat for several (all must match).
    #[serde(default)]
    #[param(style = Form, explode)]
    pub tag: Vec<String>,
    /// `exclude` (default), `include` or `only`.
    #[param(inline)]
    pub archived: Option<ArchivedFilter>,
    /// Page size (1–200, default 50).
    pub limit: Option<i64>,
    /// Rows to skip (default 0).
    pub offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/parcels",
    tag = "parcels",
    description = "List parcels: the caller's own by default, or those shared with them \
        (`scope=shared`), or both (`scope=all`); most recently updated first, or by relevance \
        with `q`. Filter by tags (`tag=a&tag=b`: all of them) and archive state. Items shared \
        with the caller carry `shared`.",
    params(ListQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = ParcelList),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
async fn list_parcels(
    State(state): State<AppState>,
    caller: AuthUser,
    axum_extra::extract::Query(q): axum_extra::extract::Query<ListQuery>,
) -> AppResult<Json<ParcelList>> {
    Ok(Json(list(&state, &caller, &q).await?))
}

/// A list's filters for `caller`, checked and normalized (`GET /parcels`,
/// `GET /tags`, and the MCP tools).
pub(crate) fn list_filter(
    caller: &AuthUser,
    scope: Option<ListScope>,
    tags: &[String],
    archived: Option<ArchivedFilter>,
    q: Option<&str>,
) -> AppResult<ListFilter> {
    let query = q.map(str::trim).filter(|q| !q.is_empty());
    if query.is_some_and(|q| q.chars().count() > parcels::MAX_QUERY_CHARS) {
        return Err(AppError::BadRequest(format!(
            "q must be at most {} characters",
            parcels::MAX_QUERY_CHARS
        )));
    }
    Ok(ListFilter {
        scope: scope.unwrap_or_default(),
        user_id: caller.user.id,
        email: caller.user.email.clone(),
        tags: normalize_tags(tags).map_err(tag_error)?,
        archived: archived.unwrap_or_default(),
        query: query.map(str::to_string),
    })
}

/// Parcels the caller can list (`GET /parcels`, and the MCP `list_parcels` tool).
pub(crate) async fn list(
    state: &AppState,
    caller: &AuthUser,
    q: &ListQuery,
) -> AppResult<ParcelList> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let filter = list_filter(caller, q.scope, &q.tag, q.archived, q.q.as_deref())?;
    let (rows, total) = parcels::search(&state.db, &filter, limit, offset).await?;
    Ok(ParcelList {
        parcels: rows.into_iter().map(|r| list_item(state, r)).collect(),
        total,
    })
}

fn list_item(state: &AppState, row: SearchRow) -> ParcelListItem {
    let shared = match (row.share_role, row.shared_at) {
        (Some(role), Some(shared_at)) => Some(SharedInfo {
            owner_name: users::display_name(
                row.owner_name.as_deref().unwrap_or_default(),
                row.owner_email.as_deref().unwrap_or_default(),
            ),
            role: ShareRole::parse(&role).unwrap_or_default(),
            shared_at,
        }),
        _ => None,
    };
    ParcelListItem {
        parcel: views::summary(&state.config, &row.parcel, Audience::SignedIn),
        matched: row.headline.as_deref().and_then(crate::search::segments),
        shared,
    }
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

/// What a publish returns. Only the owner publishes, so the version
/// carries its source.
pub(crate) async fn publish_result(
    state: &AppState,
    id: &str,
    version: i32,
    created: bool,
) -> AppResult<PublishResult> {
    let row = parcels::get(&state.db, id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    let v = parcels::get_version(&state.db, id, version)
        .await?
        .ok_or_else(crate::error::not_found)?;
    let files = parcels::version_files(&state.db, id, version).await?;
    Ok(PublishResult {
        parcel: views::summary(&state.config, &row, Audience::SignedIn),
        version: views::version_detail(&v, files, ParcelAccess::Owner),
        created,
    })
}

#[utoipa::path(
    post,
    path = "/parcels",
    tag = "parcels",
    description = "Create a parcel from a multipart upload. Send an optional `metadata` part \
        (JSON, `CreateParcelMetadata`) and one `file` part per file whose filename is the \
        published path (e.g. `index.html`, `assets/app.js`). With a `slug` the caller already \
        uses (or used, before a rename), this publishes a new version of that parcel instead \
        and answers 200 with `created: false`.",
    request_body(content = crate::http::openapi::PublishUpload, content_type = "multipart/form-data"),
    security(("bearer" = [])),
    responses(
        (status = 201, body = PublishResult, description = "Created"),
        (status = 200, body = PublishResult, description = "A new version of the parcel with this slug"),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
        (status = 409, body = ErrorBody, description = "`base_version` is no longer current, or no parcel has the slug yet"),
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
    let published = publish::create(&state, caller.user.id, meta, uploads).await?;
    let id = &published.id;
    let status = if published.created {
        tracing::info!(parcel = %id, user = %caller.user.id, "parcel created");
        StatusCode::CREATED
    } else {
        tracing::info!(parcel = %id, version = published.version, "version published by slug");
        StatusCode::OK
    };
    Ok((
        status,
        Json(publish_result(&state, id, published.version, published.created).await?),
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
    detail_of(state, caller, row, access).await
}

/// [`detail`] for a parcel whose access is already decided.
pub(crate) async fn detail_of(
    state: &AppState,
    caller: Option<&AuthUser>,
    row: ParcelRow,
    access: ParcelAccess,
) -> AppResult<ParcelDetail> {
    let id = row.id.as_str();
    let (versions, files) = tokio::try_join!(
        parcels::list_versions(&state.db, id),
        parcels::version_files(&state.db, id, row.current_version),
    )?;
    Ok(ParcelDetail {
        parcel: views::summary(&state.config, &row, Audience::of(caller)),
        is_owner: access == ParcelAccess::Owner,
        access,
        owner_name: views::owner_name(state, &row, caller).await?,
        versions: versions
            .iter()
            .map(|v| views::version_summary(v, access))
            .collect(),
        pages: views::pages(&row.entry, &files),
        files: files.into_iter().map(Into::into).collect(),
    })
}

#[utoipa::path(
    patch,
    path = "/parcels/{id}",
    tag = "parcels",
    description = "Change a parcel's title, description or visibility (which counts as an \
        update), or organize it: `tags` replaces the tag set, `add_tags`/`remove_tags` edit it, \
        `archived` archives or unarchives, `slug` names it (`\"\"` clears; the old slug keeps \
        redirecting until another of your parcels takes it). Organizing doesn't change \
        `updated_at`. Files change only through new versions.",
    params(("id" = String, Path, description = "Parcel id")),
    request_body = UpdateParcelRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = ParcelSummary),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody, description = "Another of your parcels has this slug"),
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

/// Change an owned parcel's title, description, visibility, tags or archive state.
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
    let tags = tags_change(body)?;
    let slug = match body.slug.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(raw) => Some(Some(normalize_slug(raw).map_err(slug_error)?)),
    };
    let patch = Patch {
        title,
        description,
        visibility: body.visibility,
        tags,
        archived: body.archived,
        slug: slug.as_ref().map(Option::as_deref),
    };
    match parcels::apply_patch(&state.db, caller.user.id, id, &patch).await? {
        Ok(()) => {}
        Err(PatchRefused::ParcelGone) => return Err(crate::error::not_found()),
        Err(PatchRefused::Tags(e)) => return Err(tag_error(e)),
        Err(PatchRefused::SlugTaken) => return Err(slug_taken()),
    }
    let row = parcels::get(&state.db, id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    Ok(views::summary(&state.config, &row, Audience::SignedIn))
}

/// A PATCH's tag change: a replacement or an edit, never both. Tags are
/// normalized before anything is counted.
fn tags_change(body: &UpdateParcelRequest) -> AppResult<TagsChange> {
    let editing = !body.add_tags.is_empty() || !body.remove_tags.is_empty();
    match &body.tags {
        Some(_) if editing => Err(AppError::BadRequest(
            "send either `tags` (the new set) or `add_tags`/`remove_tags`, not both".into(),
        )),
        Some(tags) => Ok(TagsChange::Replace(
            normalize_tags(tags).map_err(tag_error)?,
        )),
        None if editing => {
            let add = normalize_set(&body.add_tags).map_err(tag_error)?;
            let remove = normalize_set(&body.remove_tags).map_err(tag_error)?;
            if let Some(both) = add.intersection(&remove).next() {
                return Err(AppError::BadRequest(format!(
                    "tag {both:?} is in both add_tags and remove_tags"
                )));
            }
            Ok(TagsChange::Edit { add, remove })
        }
        None => Ok(TagsChange::Keep),
    }
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
    /// Page to open (one of the version's `pages`; default: its entry).
    pub path: Option<String>,
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/view",
    tag = "parcels",
    description = "A short-lived signed URL on the sandboxed content origin for one page of a \
        version (its entry unless `path` names another of its `pages`) — what the viewer iframe \
        loads. The URL's capability covers the whole version, so the page's relative links to \
        its other files and pages work.",
    params(("id" = String, Path, description = "Parcel id"), ViewQuery),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = ViewGrant),
        (status = 404, body = ErrorBody, description = "No such parcel or version, or `path` is not a page of that version"),
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
    let page = match q.path.as_deref() {
        None => v.entry.clone(),
        Some(path) => {
            // A path no publish could store (a NUL byte, `..`) can't name a
            // page; checking first also keeps it away from Postgres, which
            // refuses NUL in text and would turn the miss into a 500.
            let file = match paths::validate_path(path) {
                Ok(()) => parcels::find_file(&state.db, &id, version, path).await?,
                Err(_) => None,
            };
            match file {
                Some(f) if media::is_entry_candidate(&f.path) => f.path,
                _ => {
                    return Err(AppError::NotFound(format!(
                        "version {version} has no page {path:?}"
                    )))
                }
            }
        }
    };
    let (content_url, expires_at) = content::grant_url(&state, &id, version, &page, scope);
    Ok(Json(ViewGrant {
        parcel: views::summary(&state.config, &row, Audience::of(caller.as_ref())),
        version: views::version_summary(&v, access),
        is_owner: access == ParcelAccess::Owner,
        access,
        owner_name: views::owner_name(&state, &row, caller.as_ref()).await?,
        content_url,
        expires_at,
    }))
}
