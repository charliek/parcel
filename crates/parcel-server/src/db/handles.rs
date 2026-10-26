//! Handles: the `@<handle>` in a user's clean URLs.
//!
//! `user_handles` keeps every handle a user has had; one is current. All
//! allocation goes through the `parcel_allocate_handle` SQL function
//! (migration 0007), which returns the user's current handle or allocates
//! one from their email: the same rule for the migration's backfill,
//! sign-in ([`super::users::sign_in`]) and `/me` (via [`allocate`]), which
//! covers a user an older revision created during a rollover.

use anyhow::Result;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::error::DbContext;

/// The user's current handle, allocated first if they have none yet.
pub async fn allocate<'e>(db: impl PgExecutor<'e>, user_id: Uuid, email: &str) -> Result<String> {
    sqlx::query_scalar("SELECT parcel_allocate_handle($1, $2)")
        .bind(user_id)
        .bind(email)
        .fetch_one(db)
        .await
        .db_context("allocating handle")
}

/// [`allocate`] for a user known by id only.
pub async fn ensure<'e>(db: impl PgExecutor<'e>, user_id: Uuid) -> Result<String> {
    sqlx::query_scalar("SELECT parcel_allocate_handle(id, email) FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
        .db_context("allocating handle")
}

/// The handle a rename asked for is someone else's (current or retired).
#[derive(Debug, PartialEq, Eq)]
pub struct HandleTaken;

/// Make `new` (already normalized) the user's handle. Their old one stays
/// theirs and keeps resolving; re-taking one of their own old handles just
/// makes it current again. Returns the previous handle, or `None` when
/// `new` already was the current one.
pub async fn rename(
    pool: &PgPool,
    user_id: Uuid,
    email: &str,
    new: &str,
) -> Result<Result<Option<String>, HandleTaken>> {
    // A user an older revision created has no handle yet: allocate it in a
    // step of its own, committed, so the rename below inserts only the one
    // handle it asks for. Allocating inside it would hold two uncommitted
    // handles, and two crossed first renames (one asking for the other's
    // would-be handle) would each wait on the other's: a deadlock. It also
    // keeps the allocation when it turns out to be the handle asked for.
    allocate(pool, user_id, email).await?;
    let mut tx = pool.begin().await.db_context("beginning handle rename")?;
    // Serialize a user's renames on their row (`NO KEY`: inserts that
    // reference the user, like a publish, aren't held up).
    sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .db_context("locking user for handle rename")?;
    // Allocated above, so this only reads it.
    let current = allocate(&mut *tx, user_id, email).await?;
    if current == new {
        return Ok(Ok(None));
    }
    let holder: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM user_handles WHERE handle = $1")
            .bind(new)
            .fetch_optional(&mut *tx)
            .await
            .db_context("checking handle")?;
    if holder.is_some_and(|holder| holder != user_id) {
        return Ok(Err(HandleTaken));
    }
    // Retire the current one first: at most one may be current at a time.
    sqlx::query("UPDATE user_handles SET is_current = false WHERE user_id = $1 AND is_current")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .db_context("retiring handle")?;
    let claimed = if holder.is_some() {
        sqlx::query("UPDATE user_handles SET is_current = true WHERE handle = $1")
            .bind(new)
            .execute(&mut *tx)
            .await
            .db_context("reclaiming handle")?
            .rows_affected()
    } else {
        // Someone may have claimed it since the check: then nothing is
        // inserted, and dropping the transaction undoes the retirement.
        sqlx::query(
            "INSERT INTO user_handles (handle, user_id, is_current) VALUES ($1, $2, true)
             ON CONFLICT DO NOTHING",
        )
        .bind(new)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .db_context("claiming handle")?
        .rows_affected()
    };
    if claimed == 0 {
        return Ok(Err(HandleTaken));
    }
    tx.commit().await.db_context("committing handle rename")?;
    Ok(Ok(Some(current)))
}
