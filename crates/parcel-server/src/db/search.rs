//! The page-text side of search: which parcels still need their text, and
//! storing it (`crate::search` does the extraction).
//!
//! Every write is guarded by `current_version` and `search_text IS NULL`, so
//! a slow extraction can never store an older version's text over a newer
//! one. Errors stay `sqlx::Error` so callers can log them value-free
//! (`error_summary`).

use sqlx::PgPool;

/// A parcel whose current version has no page text yet, with its entry
/// page's path and blob.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Pending {
    pub id: String,
    pub version: i32,
    pub entry: String,
    pub blob_sha256: String,
}

const PENDING_SELECT: &str = "SELECT p.id, p.current_version AS version, v.entry, f.blob_sha256
     FROM parcels p
     JOIN parcel_versions v ON v.parcel_id = p.id AND v.version = p.current_version
     JOIN version_files f ON f.parcel_id = p.id AND f.version = v.version AND f.path = v.entry
     WHERE p.search_text IS NULL";

/// `version` of a parcel, if it is still current and still lacks its text.
pub async fn pending(db: &PgPool, id: &str, version: i32) -> Result<Option<Pending>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{PENDING_SELECT} AND p.id = $1 AND p.current_version = $2"
    )))
    .bind(id)
    .bind(version)
    .fetch_optional(db)
    .await
}

/// Up to `limit` parcels lacking their text and tried fewer than
/// `max_attempts` times, in id order after `after`.
pub async fn pending_batch(
    db: &PgPool,
    max_attempts: i16,
    after: &str,
    limit: i64,
) -> Result<Vec<Pending>, sqlx::Error> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{PENDING_SELECT} AND p.search_attempts < $1 AND p.id > $2 ORDER BY p.id LIMIT $3"
    )))
    .bind(max_attempts)
    .bind(after)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Store `text` as `version`'s page text, unless the parcel has moved on to
/// another version or already has its text. Returns whether it was stored.
pub async fn store_text(
    db: &PgPool,
    id: &str,
    version: i32,
    text: &str,
) -> Result<bool, sqlx::Error> {
    let stored = sqlx::query(
        "UPDATE parcels SET search_text = $3
         WHERE id = $1 AND current_version = $2 AND search_text IS NULL",
    )
    .bind(id)
    .bind(version)
    .bind(text)
    .execute(db)
    .await?
    .rows_affected();
    Ok(stored > 0)
}

/// Count a failed extraction against `version` (a new version resets the
/// count).
pub async fn record_failure(db: &PgPool, id: &str, version: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE parcels SET search_attempts = LEAST(search_attempts, 32766) + 1
         WHERE id = $1 AND current_version = $2 AND search_text IS NULL",
    )
    .bind(id)
    .bind(version)
    .execute(db)
    .await?;
    Ok(())
}
