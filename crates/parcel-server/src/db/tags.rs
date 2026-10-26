//! Renaming and deleting a tag across all of an owner's parcels.
//!
//! Each is one `UPDATE` under the owner's lock (taken before any row lock,
//! the order every multi-lock write uses). The array is rebuilt with
//! `array_agg(DISTINCT … COLLATE "C" ORDER BY … COLLATE "C")`, the byte
//! order Rust's `normalize_tags` produces, so a renamed or merged set is
//! stored exactly as the app would have stored it. Neither touches
//! `updated_at`: organizing isn't editing. A deadlock or serialization
//! failure reruns the transaction once ([`retry_once`]).

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use super::error::DbContext;
use super::parcels::lock_owner_quota;
use super::retry::{retry_once, retryable};

/// Rewrite `from` → `to` (both normalized) on every parcel `owner_id` owns,
/// merging where `to` is already there. A rename never adds a tag, so it
/// can't break the per-parcel cap. Returns how many parcels changed.
pub async fn rename(pool: &PgPool, owner_id: Uuid, from: &str, to: &str) -> Result<i64> {
    if from == to {
        return Ok(0);
    }
    retry_once(
        "tag rename",
        || rename_once(pool, owner_id, from, to),
        retryable,
    )
    .await
}

async fn rename_once(pool: &PgPool, owner_id: Uuid, from: &str, to: &str) -> Result<i64> {
    let mut tx = pool.begin().await.db_context("beginning tag rename")?;
    lock_owner_quota(&mut tx, owner_id).await?;
    let updated = sqlx::query(
        "UPDATE parcels p SET tags = (
             SELECT array_agg(DISTINCT (r.tag COLLATE \"C\") ORDER BY (r.tag COLLATE \"C\"))
             FROM (SELECT CASE WHEN t = $2 THEN $3 ELSE t END AS tag
                   FROM unnest(p.tags) AS u(t)) r)
         WHERE p.owner_id = $1 AND p.tags @> ARRAY[$2]::text[]",
    )
    .bind(owner_id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx)
    .await
    .db_context("renaming tag")?
    .rows_affected();
    tx.commit().await.db_context("committing tag rename")?;
    Ok(count(updated))
}

/// Remove `tag` (normalized) from every parcel `owner_id` owns. Returns how
/// many parcels changed.
pub async fn delete(pool: &PgPool, owner_id: Uuid, tag: &str) -> Result<i64> {
    retry_once("tag delete", || delete_once(pool, owner_id, tag), retryable).await
}

async fn delete_once(pool: &PgPool, owner_id: Uuid, tag: &str) -> Result<i64> {
    let mut tx = pool.begin().await.db_context("beginning tag delete")?;
    lock_owner_quota(&mut tx, owner_id).await?;
    let updated = sqlx::query(
        // Tags are stored sorted and distinct; removing one keeps them so.
        "UPDATE parcels p SET tags = array_remove(p.tags, $2)
         WHERE p.owner_id = $1 AND p.tags @> ARRAY[$2]::text[]",
    )
    .bind(owner_id)
    .bind(tag)
    .execute(&mut *tx)
    .await
    .db_context("deleting tag")?
    .rows_affected();
    tx.commit().await.db_context("committing tag delete")?;
    Ok(count(updated))
}

fn count(rows_affected: u64) -> i64 {
    i64::try_from(rows_affected).unwrap_or(i64::MAX)
}
