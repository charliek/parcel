//! Comment threads on a parcel.
//!
//! Every change bumps `parcels.comments_rev` in the same statement or
//! transaction, which is what the viewer polls (`/activity`) to notice that
//! comments changed without refetching them.

use anyhow::{Context, Result};
use parcel_types::{CommentAnchor, CommentStatus, CommentVia};
use sqlx::types::Json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Most comments (threads plus replies) one parcel may hold.
pub const MAX_COMMENTS_PER_PARCEL: i64 = 2000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommentRow {
    pub number: i32,
    pub version: i32,
    pub parent_number: Option<i32>,
    pub author_id: Uuid,
    pub author_name: String,
    pub author_email: String,
    pub via: String,
    pub body: String,
    pub anchor: Option<Json<CommentAnchor>>,
    pub resolved_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl CommentRow {
    pub fn via(&self) -> CommentVia {
        CommentVia::parse(&self.via).unwrap_or(CommentVia::Web)
    }

    /// The author's name, else their email.
    pub fn author_display(&self) -> String {
        if self.author_name.trim().is_empty() {
            self.author_email.clone()
        } else {
            self.author_name.clone()
        }
    }
}

const COMMENT_SELECT: &str = "SELECT c.number, c.version, c.parent_number, c.author_id,
            u.name AS author_name, u.email AS author_email, c.via, c.body, c.anchor,
            c.resolved_at, c.created_at, c.updated_at
     FROM comments c JOIN users u ON u.id = c.author_id";

/// Threads matching `status` and all their replies, by number. Callers group
/// replies under their thread.
pub async fn list(
    pool: &PgPool,
    parcel_id: &str,
    status: CommentStatus,
) -> Result<Vec<CommentRow>> {
    let status = match status {
        CommentStatus::Open => "open",
        CommentStatus::Resolved => "resolved",
        CommentStatus::All => "all",
    };
    sqlx::query_as::<_, CommentRow>(&format!(
        "{COMMENT_SELECT}
         JOIN comments t ON t.parcel_id = c.parcel_id
                        AND t.number = COALESCE(c.parent_number, c.number)
         WHERE c.parcel_id = $1
           AND ($2 = 'all' OR ($2 = 'open') = (t.resolved_at IS NULL))
         ORDER BY c.number"
    ))
    .bind(parcel_id)
    .bind(status)
    .fetch_all(pool)
    .await
    .context("listing comments")
}

/// One thread (by its top-level number) with its replies.
pub async fn thread(pool: &PgPool, parcel_id: &str, number: i32) -> Result<Vec<CommentRow>> {
    sqlx::query_as::<_, CommentRow>(&format!(
        "{COMMENT_SELECT}
         WHERE c.parcel_id = $1 AND (c.number = $2 OR c.parent_number = $2)
         ORDER BY c.number"
    ))
    .bind(parcel_id)
    .bind(number)
    .fetch_all(pool)
    .await
    .context("loading comment thread")
}

pub async fn get(pool: &PgPool, parcel_id: &str, number: i32) -> Result<Option<CommentRow>> {
    sqlx::query_as::<_, CommentRow>(&format!(
        "{COMMENT_SELECT} WHERE c.parcel_id = $1 AND c.number = $2"
    ))
    .bind(parcel_id)
    .bind(number)
    .fetch_optional(pool)
    .await
    .context("loading comment")
}

pub async fn count_open(pool: &PgPool, parcel_id: &str) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT count(*) FROM comments
         WHERE parcel_id = $1 AND parent_number IS NULL AND resolved_at IS NULL",
    )
    .bind(parcel_id)
    .fetch_one(pool)
    .await
    .context("counting open comments")
}

pub struct NewCommentRow<'a> {
    pub parcel_id: &'a str,
    /// Ignored for replies, which take their thread's version.
    pub version: i32,
    pub reply_to: Option<i32>,
    pub author_id: Uuid,
    pub via: CommentVia,
    pub body: &'a str,
    pub anchor: Option<&'a CommentAnchor>,
}

/// Why an insert was refused (checked under the parcel lock, so a thread
/// can't vanish between the check and the insert).
#[derive(Debug, PartialEq, Eq)]
pub enum InsertRefused {
    ParcelGone,
    ThreadNotFound,
    ReplyToReply,
    TooMany,
}

/// Add a comment; returns the number of its thread.
pub async fn insert(pool: &PgPool, c: &NewCommentRow<'_>) -> Result<Result<i32, InsertRefused>> {
    let mut tx = pool.begin().await.context("beginning comment insert")?;
    // Taking the number also takes the parcel row lock, serializing inserts.
    let Some(number) = sqlx::query_scalar::<_, i32>(
        "UPDATE parcels SET comment_seq = comment_seq + 1, comments_rev = comments_rev + 1
         WHERE id = $1 RETURNING comment_seq",
    )
    .bind(c.parcel_id)
    .fetch_optional(&mut *tx)
    .await
    .context("numbering comment")?
    else {
        return Ok(Err(InsertRefused::ParcelGone));
    };

    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM comments WHERE parcel_id = $1")
        .bind(c.parcel_id)
        .fetch_one(&mut *tx)
        .await
        .context("counting comments")?;
    if existing >= MAX_COMMENTS_PER_PARCEL {
        return Ok(Err(InsertRefused::TooMany));
    }

    let version = match c.reply_to {
        None => c.version,
        Some(parent) => {
            let row: Option<(i32, Option<i32>)> = sqlx::query_as(
                "SELECT version, parent_number FROM comments WHERE parcel_id = $1 AND number = $2",
            )
            .bind(c.parcel_id)
            .bind(parent)
            .fetch_optional(&mut *tx)
            .await
            .context("loading parent comment")?;
            match row {
                None => return Ok(Err(InsertRefused::ThreadNotFound)),
                Some((_, Some(_))) => return Ok(Err(InsertRefused::ReplyToReply)),
                Some((version, None)) => version,
            }
        }
    };

    sqlx::query(
        "INSERT INTO comments (parcel_id, number, version, parent_number, author_id, via, body, anchor)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(c.parcel_id)
    .bind(number)
    .bind(version)
    .bind(c.reply_to)
    .bind(c.author_id)
    .bind(c.via.as_str())
    .bind(c.body)
    .bind(c.anchor.map(Json))
    .execute(&mut *tx)
    .await
    .context("inserting comment")?;
    tx.commit().await.context("committing comment")?;
    Ok(Ok(c.reply_to.unwrap_or(number)))
}

/// Bumping the parcel's revision rides along in a data-modifying CTE, so the
/// change and the bump are one atomic statement.
const BUMP: &str =
    "WITH bump AS (UPDATE parcels SET comments_rev = comments_rev + 1 WHERE id = $1)";

pub async fn update_body(pool: &PgPool, parcel_id: &str, number: i32, body: &str) -> Result<()> {
    sqlx::query(&format!(
        "{BUMP} UPDATE comments SET body = $3, updated_at = now()
         WHERE parcel_id = $1 AND number = $2"
    ))
    .bind(parcel_id)
    .bind(number)
    .bind(body)
    .execute(pool)
    .await
    .context("editing comment")?;
    Ok(())
}

/// Resolve or reopen a thread. Leaves `resolved_at` alone when it already
/// matches, so resolving twice keeps the first timestamp.
pub async fn set_resolved(
    pool: &PgPool,
    parcel_id: &str,
    number: i32,
    resolved: bool,
    by: Uuid,
) -> Result<()> {
    sqlx::query(&format!(
        "{BUMP} UPDATE comments
         SET resolved_at = CASE WHEN $3 THEN COALESCE(resolved_at, now()) END,
             resolved_by = CASE WHEN $3 THEN COALESCE(resolved_by, $4) END
         WHERE parcel_id = $1 AND number = $2 AND parent_number IS NULL"
    ))
    .bind(parcel_id)
    .bind(number)
    .bind(resolved)
    .bind(by)
    .execute(pool)
    .await
    .context("resolving comment")?;
    Ok(())
}

/// Delete a comment (a thread takes its replies with it).
pub async fn delete(pool: &PgPool, parcel_id: &str, number: i32) -> Result<()> {
    sqlx::query(&format!(
        "{BUMP} DELETE FROM comments WHERE parcel_id = $1 AND number = $2"
    ))
    .bind(parcel_id)
    .bind(number)
    .execute(pool)
    .await
    .context("deleting comment")?;
    Ok(())
}

/// `(current_version, updated_at, comments_rev, open threads)` for the
/// viewer's poll.
pub async fn activity(
    pool: &PgPool,
    parcel_id: &str,
) -> Result<Option<(i32, OffsetDateTime, i64, i64)>> {
    sqlx::query_as(
        "SELECT p.current_version, p.updated_at, p.comments_rev,
                (SELECT count(*) FROM comments c
                  WHERE c.parcel_id = p.id AND c.parent_number IS NULL
                    AND c.resolved_at IS NULL)
         FROM parcels p WHERE p.id = $1",
    )
    .bind(parcel_id)
    .fetch_optional(pool)
    .await
    .context("loading parcel activity")
}
