//! `/api/v1/tags` — the tag vocabulary with facet counts, and renaming or
//! deleting a tag across all of the caller's parcels.

use axum::extract::State;
use axum::Json;
use parcel_types::tags::{normalize_tag, MAX_TAG_CHARS};
use parcel_types::{
    ArchivedFilter, DeleteTagRequest, ErrorBody, ListScope, RenameTagRequest, TagCount, TagList,
    TagsMode, TagsUpdated,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthUser;
use crate::db::parcels::{self, SearchMode};
use crate::db::tags;
use crate::error::{tag_error, AppError, AppResult};
use crate::http::parcels::list_filter;
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_tags))
        .routes(routes!(rename_tag))
        .routes(routes!(delete_tag))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TagsQuery {
    /// Whose parcels to count: `mine` (default), `shared` or `all`.
    #[param(inline)]
    pub scope: Option<ListScope>,
    /// Count only parcels carrying this tag; repeat for several (all must match).
    #[serde(default)]
    #[param(style = Form, explode)]
    pub tag: Vec<String>,
    /// Count only parcels matching this search (as `GET /parcels`).
    pub q: Option<String>,
    /// How `q` matches: `auto` (default) as `GET /parcels` decides for the same
    /// filters (`prefix` plays no part), `exact` (every word) or `relaxed`
    /// (any word). Pass a list's mode (`relaxed` when it was marked so, else
    /// `exact`) to count what that list shows.
    #[param(inline)]
    pub mode: Option<TagsMode>,
    /// `exclude` (default), `include` or `only`.
    #[param(inline)]
    pub archived: Option<ArchivedFilter>,
    /// Only tags starting with this, e.g. `project:`.
    pub prefix: Option<String>,
    /// Page size (1–1000, default 200).
    pub limit: Option<i64>,
    /// Tags to skip (default 0).
    pub offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/tags",
    tag = "tags",
    description = "The tags on the parcels the filters select, each with how many of those \
        parcels carry it (facet counts): most used first, then by name. Takes the same \
        `scope`, `tag`, `q` and `archived` filters as `GET /parcels`, and `mode` for how `q` \
        matches; `relaxed` says the counts are over parcels matching any of `q`'s words.",
    params(TagsQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = TagList),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
async fn list_tags(
    State(state): State<AppState>,
    caller: AuthUser,
    axum_extra::extract::Query(q): axum_extra::extract::Query<TagsQuery>,
) -> AppResult<Json<TagList>> {
    Ok(Json(list(&state, &caller, &q).await?))
}

/// The caller's tag vocabulary (`GET /tags`, and the MCP tools).
pub(crate) async fn list(state: &AppState, caller: &AuthUser, q: &TagsQuery) -> AppResult<TagList> {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let offset = q.offset.unwrap_or(0).max(0);
    let filter = list_filter(caller, q.scope, &q.tag, q.archived, q.q.as_deref())?;
    let prefix = q
        .prefix
        .as_deref()
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty());
    if prefix
        .as_ref()
        .is_some_and(|p| p.chars().count() > MAX_TAG_CHARS)
    {
        return Err(AppError::BadRequest(format!(
            "prefix must be at most {MAX_TAG_CHARS} characters"
        )));
    }
    if prefix.as_ref().is_some_and(|p| p.contains('\0')) {
        return Err(AppError::BadRequest(
            "prefix must not contain NUL".to_string(),
        ));
    }
    let mode = match q.mode.unwrap_or_default() {
        // The list's own decision: the same filters, without `prefix`.
        TagsMode::Auto => parcels::search_mode(&state.db, &filter).await?,
        TagsMode::Exact => SearchMode::Exact,
        // Without a query there is nothing to relax.
        TagsMode::Relaxed if filter.query.is_some() => SearchMode::Relaxed,
        TagsMode::Relaxed => SearchMode::Exact,
    };
    let (rows, total) =
        parcels::tag_counts(&state.db, &filter, mode, prefix.as_deref(), limit, offset).await?;
    Ok(TagList {
        tags: rows
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect(),
        total,
        relaxed: mode == SearchMode::Relaxed,
    })
}

#[utoipa::path(
    post,
    path = "/tags/rename",
    tag = "tags",
    description = "Rename a tag on every parcel the caller owns. Renaming into a tag that is \
        already in use merges the two. Doesn't change any parcel's `updated_at`.",
    request_body = RenameTagRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = TagsUpdated),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
async fn rename_tag(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(body): Json<RenameTagRequest>,
) -> AppResult<Json<TagsUpdated>> {
    Ok(Json(rename(&state, &caller, &body).await?))
}

pub(crate) async fn rename(
    state: &AppState,
    caller: &AuthUser,
    body: &RenameTagRequest,
) -> AppResult<TagsUpdated> {
    let (from, to) = (
        normalize_tag(&body.from).map_err(tag_error)?,
        normalize_tag(&body.to).map_err(tag_error)?,
    );
    let updated = tags::rename(&state.db, caller.user.id, &from, &to).await?;
    tracing::info!(user = %caller.user.id, updated, "tag renamed");
    Ok(TagsUpdated { updated })
}

#[utoipa::path(
    post,
    path = "/tags/delete",
    tag = "tags",
    description = "Remove a tag from every parcel the caller owns (a tag can hold `/`, so it \
        goes in the body). Doesn't change any parcel's `updated_at`.",
    request_body = DeleteTagRequest,
    security(("bearer" = [])),
    responses(
        (status = 200, body = TagsUpdated),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    )
)]
async fn delete_tag(
    State(state): State<AppState>,
    caller: AuthUser,
    Json(body): Json<DeleteTagRequest>,
) -> AppResult<Json<TagsUpdated>> {
    Ok(Json(delete(&state, &caller, &body).await?))
}

pub(crate) async fn delete(
    state: &AppState,
    caller: &AuthUser,
    body: &DeleteTagRequest,
) -> AppResult<TagsUpdated> {
    let tag = normalize_tag(&body.tag).map_err(tag_error)?;
    let updated = tags::delete(&state.db, caller.user.id, &tag).await?;
    tracing::info!(user = %caller.user.id, updated, "tag deleted");
    Ok(TagsUpdated { updated })
}
