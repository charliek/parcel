//! `/api/v1/parcels/{id}/comments` — comment threads, and `/activity`, the
//! cheap poll the viewer uses to notice new versions and comment changes.
//!
//! Who may do what:
//! * read and write comments: the owner, plus any signed-in user when the
//!   parcel is public (anonymous viewers see none);
//! * edit a comment: its author;
//! * resolve or reopen a thread: the owner or the thread's author;
//! * delete a comment: its author or the owner.

use axum::extract::{Path, Query, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use parcel_types::{
    comments::validate_body, Comment, CommentAuthor, CommentList, CommentReply, CommentStatus,
    CommentVia, ErrorBody, NewComment, ParcelActivity, UpdateComment,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::auth::{AuthUser, AuthVia, MaybeAuthUser};
use crate::db::comments::{self, CommentRow, InsertRefused, NewCommentRow};
use crate::db::parcels::{self, ParcelRow};
use crate::error::{not_found, too_large, AppError, AppResult};
use crate::http::views::readable;
use crate::state::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_comments, create_comment))
        .routes(routes!(update_comment, delete_comment))
        .routes(routes!(get_activity))
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// `open` (default), `resolved` or `all`.
    #[param(inline)]
    pub status: Option<CommentStatus>,
}

/// What a comment request needs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    Read,
    Write,
}

/// A parcel whose comments the caller may read (or write). Comments are for
/// signed-in people the parcel is open to; anonymous viewers of a public
/// parcel see none.
async fn commentable(
    state: &AppState,
    id: &str,
    caller: Option<&AuthUser>,
    need: Need,
) -> Result<ParcelRow, AppError> {
    let (row, access) = readable(state, id, caller).await?;
    if !access.can_read_comments() {
        return Err(AppError::Unauthorized(
            "sign in to see and add comments".into(),
        ));
    }
    if need == Need::Write && !access.can_comment() {
        return Err(AppError::Forbidden(
            "this parcel is shared with you to view, not to comment on".into(),
        ));
    }
    Ok(row)
}

fn comment_not_found(number: i32) -> AppError {
    AppError::NotFound(format!("comment #{number} not found"))
}

fn author(row: &CommentRow, owner_id: Uuid, caller_id: Uuid) -> CommentAuthor {
    CommentAuthor {
        name: row.author_display(),
        is_owner: row.author_id == owner_id,
        is_you: row.author_id == caller_id,
    }
}

/// Group rows (ordered by number) into threads with their replies.
fn threads(rows: Vec<CommentRow>, owner_id: Uuid, caller_id: Uuid) -> Vec<Comment> {
    let mut out: Vec<Comment> = Vec::new();
    for row in rows {
        let who = author(&row, owner_id, caller_id);
        match row.parent_number {
            None => out.push(Comment {
                number: row.number,
                version: row.version,
                author: who,
                via: row.via(),
                body: row.body,
                anchor: row.anchor.map(|a| a.0),
                resolved: row.resolved_at.is_some(),
                resolved_at: row.resolved_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
                replies: Vec::new(),
            }),
            Some(parent) => {
                // Replies sort after their thread, so it is already present.
                if let Some(thread) = out.iter_mut().rev().find(|t| t.number == parent) {
                    thread.replies.push(CommentReply {
                        number: row.number,
                        author: who,
                        via: row.via(),
                        body: row.body,
                        created_at: row.created_at,
                        updated_at: row.updated_at,
                    });
                }
            }
        }
    }
    out
}

async fn load_thread(
    state: &AppState,
    parcel: &ParcelRow,
    number: i32,
    caller: &AuthUser,
) -> AppResult<Comment> {
    let rows = comments::thread(&state.db, &parcel.id, number).await?;
    threads(rows, parcel.owner_id, caller.user.id)
        .into_iter()
        .next()
        .ok_or_else(|| comment_not_found(number))
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/comments",
    tag = "comments",
    description = "A parcel's comment threads with their replies, oldest first. Visible to \
        the owner, and to any signed-in user when the parcel is public.",
    params(("id" = String, Path, description = "Parcel id"), ListQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, body = CommentList),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn list_comments(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<CommentList>> {
    Ok(Json(
        list(&state, caller.as_ref(), &id, q.status.unwrap_or_default()).await?,
    ))
}

/// A parcel's threads (`GET …/comments`, and the MCP `list_comments` tool).
pub(crate) async fn list(
    state: &AppState,
    caller: Option<&AuthUser>,
    id: &str,
    status: CommentStatus,
) -> AppResult<CommentList> {
    let parcel = commentable(state, id, caller, Need::Read).await?;
    let caller = caller.expect("commentable requires a caller");
    let rows = comments::list(&state.db, id, status).await?;
    let open = comments::count_open(&state.db, id).await?;
    Ok(CommentList {
        comments: threads(rows, parcel.owner_id, caller.user.id),
        open,
    })
}

#[utoipa::path(
    post,
    path = "/parcels/{id}/comments",
    tag = "comments",
    description = "Start a thread (optionally pinned to a spot on the page with `anchor`) or \
        reply to one with `reply_to`. Returns the whole thread.",
    params(("id" = String, Path, description = "Parcel id")),
    request_body = NewComment,
    security(("bearer" = [])),
    responses(
        (status = 201, body = Comment),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 413, body = ErrorBody, description = "The parcel has too many comments"),
    )
)]
async fn create_comment(
    State(state): State<AppState>,
    caller: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<NewComment>,
) -> AppResult<(StatusCode, Json<Comment>)> {
    Ok((
        StatusCode::CREATED,
        Json(create(&state, &caller, &id, &body).await?),
    ))
}

/// Start a thread or reply to one; returns the whole thread.
pub(crate) async fn create(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    body: &NewComment,
) -> AppResult<Comment> {
    let parcel = commentable(state, id, Some(caller), Need::Write).await?;
    let text = validate_body(&body.body).map_err(AppError::BadRequest)?;
    if body.reply_to.is_some() && body.anchor.is_some() {
        return Err(AppError::BadRequest(
            "replies can't have an anchor; anchor the thread instead".into(),
        ));
    }
    if let Some(anchor) = &body.anchor {
        anchor.validate().map_err(AppError::BadRequest)?;
    }
    let version = body.version.unwrap_or(parcel.current_version);
    if body.reply_to.is_none()
        && parcels::get_version(&state.db, id, version)
            .await?
            .is_none()
    {
        return Err(AppError::BadRequest(format!(
            "version {version} does not exist"
        )));
    }

    let via = match caller.via {
        AuthVia::Token(_) => CommentVia::Cli,
        AuthVia::Mcp => CommentVia::Mcp,
        AuthVia::Session | AuthVia::Dev => CommentVia::Web,
    };
    let inserted = comments::insert(
        &state.db,
        &NewCommentRow {
            parcel_id: id,
            version,
            reply_to: body.reply_to,
            author_id: caller.user.id,
            via,
            body: text,
            anchor: body.anchor.as_ref(),
        },
    )
    .await?;
    let thread = match inserted {
        Ok(thread) => thread,
        Err(InsertRefused::ParcelGone) => return Err(not_found()),
        Err(InsertRefused::ThreadNotFound) => {
            return Err(comment_not_found(body.reply_to.unwrap_or_default()))
        }
        Err(InsertRefused::ReplyToReply) => {
            return Err(AppError::BadRequest(
                "reply to the thread's first comment, not to a reply".into(),
            ))
        }
        Err(InsertRefused::TooMany) => {
            return Err(too_large(format!(
                "a parcel can hold at most {} comments",
                comments::MAX_COMMENTS_PER_PARCEL
            )))
        }
    };
    load_thread(state, &parcel, thread, caller).await
}

#[utoipa::path(
    patch,
    path = "/parcels/{id}/comments/{number}",
    tag = "comments",
    description = "Edit a comment's text (its author) or resolve/reopen a thread (the owner \
        or the thread's author). Returns the whole thread.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("number" = i32, Path, description = "Comment number"),
    ),
    request_body = UpdateComment,
    security(("bearer" = [])),
    responses(
        (status = 200, body = Comment),
        (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn update_comment(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, number)): Path<(String, i32)>,
    Json(body): Json<UpdateComment>,
) -> AppResult<Json<Comment>> {
    Ok(Json(update(&state, &caller, &id, number, &body).await?))
}

/// Edit a comment or resolve/reopen a thread; returns the whole thread.
pub(crate) async fn update(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    number: i32,
    body: &UpdateComment,
) -> AppResult<Comment> {
    let parcel = commentable(state, id, Some(caller), Need::Write).await?;
    let comment = comments::get(&state.db, id, number)
        .await?
        .ok_or_else(|| comment_not_found(number))?;
    let is_author = comment.author_id == caller.user.id;
    let is_owner = parcel.owner_id == caller.user.id;

    if body.body.is_none() && body.resolved.is_none() {
        return Err(AppError::BadRequest(
            "send `body`, `resolved`, or both".into(),
        ));
    }
    let text = match &body.body {
        Some(text) => {
            if !is_author {
                return Err(AppError::Forbidden(
                    "only its author can edit a comment".into(),
                ));
            }
            Some(validate_body(text).map_err(AppError::BadRequest)?)
        }
        None => None,
    };
    if body.resolved.is_some() {
        if comment.parent_number.is_some() {
            return Err(AppError::BadRequest(
                "resolve the thread, not a reply".into(),
            ));
        }
        if !is_author && !is_owner {
            return Err(AppError::Forbidden(
                "only the parcel's owner or the thread's author can resolve it".into(),
            ));
        }
    }

    if let Some(text) = text {
        comments::update_body(&state.db, id, number, text).await?;
    }
    if let Some(resolved) = body.resolved {
        comments::set_resolved(&state.db, id, number, resolved, caller.user.id).await?;
    }
    let thread = comment.parent_number.unwrap_or(number);
    load_thread(state, &parcel, thread, caller).await
}

#[utoipa::path(
    delete,
    path = "/parcels/{id}/comments/{number}",
    tag = "comments",
    description = "Delete a comment (its author or the parcel's owner). Deleting a thread's \
        first comment deletes its replies too.",
    params(
        ("id" = String, Path, description = "Parcel id"),
        ("number" = i32, Path, description = "Comment number"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
    )
)]
async fn delete_comment(
    State(state): State<AppState>,
    caller: AuthUser,
    Path((id, number)): Path<(String, i32)>,
) -> AppResult<StatusCode> {
    delete(&state, &caller, &id, number).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a comment (and, for a thread's first, its replies).
pub(crate) async fn delete(
    state: &AppState,
    caller: &AuthUser,
    id: &str,
    number: i32,
) -> AppResult<()> {
    // Taking back one's own words needs only read access.
    let parcel = commentable(state, id, Some(caller), Need::Read).await?;
    let comment = comments::get(&state.db, id, number)
        .await?
        .ok_or_else(|| comment_not_found(number))?;
    if comment.author_id != caller.user.id && parcel.owner_id != caller.user.id {
        return Err(AppError::Forbidden(
            "only its author or the parcel's owner can delete a comment".into(),
        ));
    }
    comments::delete(&state.db, id, number).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/parcels/{id}/activity",
    tag = "parcels",
    description = "The parcel's current version and comment counters: a cheap poll for \
        noticing new versions and comment changes. Readable wherever the parcel is.",
    params(("id" = String, Path, description = "Parcel id")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, body = ParcelActivity),
        (status = 404, body = ErrorBody),
    )
)]
async fn get_activity(
    State(state): State<AppState>,
    MaybeAuthUser(caller): MaybeAuthUser,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    readable(&state, &id, caller.as_ref()).await?;
    let (current_version, updated_at, comments_revision, open_comments) =
        comments::activity(&state.db, &id)
            .await?
            .ok_or_else(not_found)?;
    Ok((
        [(CACHE_CONTROL, "no-store")],
        Json(ParcelActivity {
            current_version,
            updated_at,
            open_comments,
            comments_revision,
        }),
    ))
}
