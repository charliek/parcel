//! Content-addressed blob rows and garbage collection.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::time::Duration;

/// The outcome of registering a blob a publish is about to reference.
#[derive(Debug, sqlx::FromRow)]
pub struct Touched {
    pub sha256: String,
    /// True when some committed version already references this blob, which
    /// guarantees its object is in the store (a publish writes objects
    /// before committing the rows that reference them). Otherwise the caller
    /// writes the object itself — content addressing makes that idempotent.
    pub referenced: bool,
}

/// Upsert blob rows and bump `touched_at`, which shields them from GC for the
/// grace period while the publish finishes.
pub async fn touch(pool: &PgPool, shas: &[String], sizes: &[i64]) -> Result<Vec<Touched>> {
    sqlx::query_as::<_, Touched>(
        "INSERT INTO blobs (sha256, size_bytes)
         SELECT * FROM UNNEST($1::text[], $2::bigint[])
         ON CONFLICT (sha256) DO UPDATE SET touched_at = now()
         RETURNING sha256,
                   EXISTS (SELECT 1 FROM version_files f WHERE f.blob_sha256 = blobs.sha256) AS referenced",
    )
    .bind(shas)
    .bind(sizes)
    .fetch_all(pool)
    .await
    .context("registering blobs")
}

/// Lock and return up to `limit` collectable blobs: unreferenced and
/// untouched for `grace`. Rows stay locked until `tx` ends, so a concurrent
/// publish that wants one of them blocks until the GC transaction commits (the
/// row is then gone, and the publish re-inserts it and rewrites the object).
pub async fn lock_collectable(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    grace: Duration,
    limit: i64,
) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT b.sha256 FROM blobs b
         WHERE b.touched_at < now() - make_interval(secs => $1)
           AND NOT EXISTS (SELECT 1 FROM version_files f WHERE f.blob_sha256 = b.sha256)
         ORDER BY b.touched_at
         LIMIT $2
         FOR UPDATE SKIP LOCKED",
    )
    .bind(grace.as_secs_f64())
    .bind(limit)
    .fetch_all(&mut **tx)
    .await
    .context("selecting collectable blobs")
}

/// Of `shas` (already locked by [`lock_collectable`]), the ones still
/// unreferenced. This is a fresh statement, so under READ COMMITTED it sees
/// every version committed before it ran — including one that committed
/// after `lock_collectable`'s snapshot was taken. A reference that is still
/// uncommitted holds a key-share lock on the blob row, which is why
/// `lock_collectable` (FOR UPDATE SKIP LOCKED) never returned it.
pub async fn still_unreferenced(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    shas: &[String],
) -> Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT s FROM UNNEST($1::text[]) AS s
         WHERE NOT EXISTS (SELECT 1 FROM version_files f WHERE f.blob_sha256 = s)",
    )
    .bind(shas)
    .fetch_all(&mut **tx)
    .await
    .context("rechecking blob references")
}

pub async fn delete_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    shas: &[String],
) -> Result<u64> {
    let result = sqlx::query("DELETE FROM blobs WHERE sha256 = ANY($1)")
        .bind(shas)
        .execute(&mut **tx)
        .await
        .context("deleting blob rows")?;
    Ok(result.rows_affected())
}
