//! Parcels, their versions, and each version's files.

use anyhow::{Context, Result};
use parcel_types::{FileEntry, Usage, Visibility};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// A parcel joined with its current version's entry and totals.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ParcelRow {
    pub id: String,
    pub owner_id: Uuid,
    pub title: String,
    pub description: String,
    pub visibility: String,
    pub current_version: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub entry: String,
    pub file_count: i32,
    pub total_bytes: i64,
    pub open_comments: i64,
}

impl ParcelRow {
    pub fn visibility(&self) -> Visibility {
        Visibility::parse(&self.visibility).unwrap_or_default()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VersionRow {
    pub version: i32,
    pub entry: String,
    pub label: Option<String>,
    pub file_count: i32,
    pub total_bytes: i64,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileRow {
    pub path: String,
    pub blob_sha256: String,
    pub size_bytes: i64,
    pub content_type: String,
}

impl From<FileRow> for FileEntry {
    fn from(f: FileRow) -> Self {
        FileEntry {
            path: f.path,
            size_bytes: f.size_bytes,
            content_type: f.content_type,
            sha256: f.blob_sha256,
        }
    }
}

macro_rules! parcel_columns {
    () => {
        "p.id, p.owner_id, p.title, p.description, p.visibility,
            p.current_version, p.created_at, p.updated_at,
            v.entry, v.file_count, v.total_bytes,
            (SELECT count(*) FROM comments c
              WHERE c.parcel_id = p.id AND c.parent_number IS NULL
                AND c.resolved_at IS NULL) AS open_comments"
    };
}
macro_rules! parcel_from {
    () => {
        "FROM parcels p
     JOIN parcel_versions v ON v.parcel_id = p.id AND v.version = p.current_version"
    };
}

/// The columns of a [`ParcelRow`] (over `p` and `v`), for queries that
/// select more alongside them.
pub const PARCEL_COLUMNS: &str = parcel_columns!();
/// The `FROM` that [`PARCEL_COLUMNS`] reads, ready for more joins.
pub const PARCEL_FROM: &str = parcel_from!();
const PARCEL_SELECT: &str = concat!("SELECT ", parcel_columns!(), "\n     ", parcel_from!());

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<ParcelRow>> {
    sqlx::query_as::<_, ParcelRow>(&format!("{PARCEL_SELECT} WHERE p.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("loading parcel")
}

/// Owned parcels, most recently updated first, optionally filtered by a
/// case-insensitive substring of title, description or id.
pub async fn list_for_owner(
    pool: &PgPool,
    owner_id: Uuid,
    query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ParcelRow>, i64)> {
    let pattern = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{}%", escape_like(q)));
    let filter = "p.owner_id = $1 AND ($2::text IS NULL
        OR p.title ILIKE $2 ESCAPE '\\' OR p.description ILIKE $2 ESCAPE '\\' OR p.id ILIKE $2 ESCAPE '\\')";
    let rows = sqlx::query_as::<_, ParcelRow>(&format!(
        "{PARCEL_SELECT} WHERE {filter} ORDER BY p.updated_at DESC, p.id LIMIT $3 OFFSET $4"
    ))
    .bind(owner_id)
    .bind(pattern.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .context("listing parcels")?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM parcels p WHERE {filter}"))
        .bind(owner_id)
        .bind(pattern.as_deref())
        .fetch_one(pool)
        .await
        .context("counting parcels")?;
    Ok((rows, total))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn list_versions(pool: &PgPool, parcel_id: &str) -> Result<Vec<VersionRow>> {
    sqlx::query_as::<_, VersionRow>(
        "SELECT version, entry, label, file_count, total_bytes, created_at
         FROM parcel_versions WHERE parcel_id = $1 ORDER BY version DESC",
    )
    .bind(parcel_id)
    .fetch_all(pool)
    .await
    .context("listing versions")
}

/// Generic over the executor so a publish can read inside its own
/// transaction instead of taking a second pool connection.
pub async fn get_version<'e>(
    db: impl PgExecutor<'e>,
    parcel_id: &str,
    version: i32,
) -> Result<Option<VersionRow>> {
    sqlx::query_as::<_, VersionRow>(
        "SELECT version, entry, label, file_count, total_bytes, created_at
         FROM parcel_versions WHERE parcel_id = $1 AND version = $2",
    )
    .bind(parcel_id)
    .bind(version)
    .fetch_optional(db)
    .await
    .context("loading version")
}

pub async fn version_files<'e>(
    db: impl PgExecutor<'e>,
    parcel_id: &str,
    version: i32,
) -> Result<Vec<FileRow>> {
    sqlx::query_as::<_, FileRow>(
        "SELECT path, blob_sha256, size_bytes, content_type
         FROM version_files WHERE parcel_id = $1 AND version = $2 ORDER BY path",
    )
    .bind(parcel_id)
    .bind(version)
    .fetch_all(db)
    .await
    .context("listing version files")
}

pub async fn find_file(
    pool: &PgPool,
    parcel_id: &str,
    version: i32,
    path: &str,
) -> Result<Option<FileRow>> {
    sqlx::query_as::<_, FileRow>(
        "SELECT path, blob_sha256, size_bytes, content_type
         FROM version_files WHERE parcel_id = $1 AND version = $2 AND path = $3",
    )
    .bind(parcel_id)
    .bind(version)
    .bind(path)
    .fetch_optional(pool)
    .await
    .context("finding file")
}

/// Everything the content server needs for one request, in one round trip.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContentRow {
    pub visibility: String,
    /// The owner's account is disabled: their own grants stop working.
    pub owner_disabled: bool,
    pub entry: String,
    pub path: String,
    pub blob_sha256: String,
    pub content_type: String,
}

pub async fn content_lookup(
    pool: &PgPool,
    parcel_id: &str,
    version: i32,
    path: &str,
) -> Result<Option<ContentRow>> {
    sqlx::query_as::<_, ContentRow>(
        "SELECT p.visibility, o.disabled_at IS NOT NULL AS owner_disabled,
                v.entry, f.path, f.blob_sha256, f.content_type
         FROM parcels p
         JOIN users o ON o.id = p.owner_id
         JOIN parcel_versions v ON v.parcel_id = p.id AND v.version = $2
         JOIN version_files f ON f.parcel_id = p.id AND f.version = v.version AND f.path = $3
         WHERE p.id = $1",
    )
    .bind(parcel_id)
    .bind(version)
    .bind(path)
    .fetch_optional(pool)
    .await
    .context("content lookup")
}

/// What a content grant needs re-checked: the parcel's visibility and
/// whether its owner's account is disabled. `None` if the parcel is gone.
pub async fn grant_state(pool: &PgPool, id: &str) -> Result<Option<(Visibility, bool)>> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT p.visibility, o.disabled_at IS NOT NULL
         FROM parcels p JOIN users o ON o.id = p.owner_id WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("loading parcel grant state")?;
    Ok(row.map(|(v, disabled)| (Visibility::parse(&v).unwrap_or_default(), disabled)))
}

/// The entry path of a version, when the parcel still exists.
pub async fn version_entry(pool: &PgPool, parcel_id: &str, version: i32) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT v.entry FROM parcel_versions v WHERE v.parcel_id = $1 AND v.version = $2",
    )
    .bind(parcel_id)
    .bind(version)
    .fetch_optional(pool)
    .await
    .context("loading version entry")
}

pub async fn update_metadata(
    pool: &PgPool,
    id: &str,
    title: Option<&str>,
    description: Option<&str>,
    visibility: Option<Visibility>,
) -> Result<()> {
    sqlx::query(
        "UPDATE parcels SET
             title = COALESCE($2, title),
             description = COALESCE($3, description),
             visibility = COALESCE($4, visibility),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .bind(visibility.map(Visibility::as_str))
    .execute(pool)
    .await
    .context("updating parcel")?;
    Ok(())
}

/// Delete a parcel (versions and file rows cascade). Blobs it referenced are
/// left for GC, which only collects what nothing references anymore.
pub async fn delete(pool: &PgPool, id: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM parcels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("deleting parcel")?;
    Ok(result.rows_affected() > 0)
}

pub async fn usage(pool: &PgPool, owner_id: Uuid) -> Result<Usage> {
    let (parcel_count, version_count, stored_bytes): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM parcels WHERE owner_id = $1),
             count(v.*),
             COALESCE(sum(v.total_bytes), 0)::bigint
         FROM parcel_versions v JOIN parcels p ON p.id = v.parcel_id
         WHERE p.owner_id = $1",
    )
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .context("computing usage")?;
    Ok(Usage {
        parcel_count,
        version_count,
        stored_bytes,
    })
}

// ---- publish transaction pieces -------------------------------------------

/// Insert a brand-new parcel row (its first version is inserted separately,
/// in the same transaction).
pub async fn insert_parcel(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    owner_id: Uuid,
    title: &str,
    description: &str,
    visibility: Visibility,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO parcels (id, owner_id, title, description, visibility, current_version)
         VALUES ($1, $2, $3, $4, $5, 0)",
    )
    .bind(id)
    .bind(owner_id)
    .bind(title)
    .bind(description)
    .bind(visibility.as_str())
    .execute(&mut **tx)
    .await
    .context("inserting parcel")?;
    Ok(())
}

/// Lock a parcel for a new version; returns `(owner_id, current_version)`.
pub async fn lock_for_update(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<Option<(Uuid, i32)>> {
    sqlx::query_as("SELECT owner_id, current_version FROM parcels WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .context("locking parcel")
}

pub struct NewVersion<'a> {
    pub parcel_id: &'a str,
    pub version: i32,
    pub entry: &'a str,
    pub label: Option<&'a str>,
    pub created_via: &'a str,
    pub files: &'a [FileRow],
}

/// Insert a version and its files, and make it current.
pub async fn insert_version(tx: &mut Transaction<'_, Postgres>, v: &NewVersion<'_>) -> Result<()> {
    let total: i64 = v.files.iter().map(|f| f.size_bytes).sum();
    let count = i32::try_from(v.files.len()).context("file count overflow")?;
    sqlx::query(
        "INSERT INTO parcel_versions (parcel_id, version, entry, label, file_count, total_bytes, created_via)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(v.parcel_id)
    .bind(v.version)
    .bind(v.entry)
    .bind(v.label)
    .bind(count)
    .bind(total)
    .bind(v.created_via)
    .execute(&mut **tx)
    .await
    .context("inserting version")?;

    let paths: Vec<&str> = v.files.iter().map(|f| f.path.as_str()).collect();
    let shas: Vec<&str> = v.files.iter().map(|f| f.blob_sha256.as_str()).collect();
    let sizes: Vec<i64> = v.files.iter().map(|f| f.size_bytes).collect();
    let types: Vec<&str> = v.files.iter().map(|f| f.content_type.as_str()).collect();
    sqlx::query(
        "INSERT INTO version_files (parcel_id, version, path, blob_sha256, size_bytes, content_type)
         SELECT $1, $2, t.path, t.sha, t.size, t.ctype
         FROM UNNEST($3::text[], $4::text[], $5::bigint[], $6::text[]) AS t(path, sha, size, ctype)",
    )
    .bind(v.parcel_id)
    .bind(v.version)
    .bind(&paths)
    .bind(&shas)
    .bind(&sizes)
    .bind(&types)
    .execute(&mut **tx)
    .await
    .context("inserting version files")?;

    sqlx::query("UPDATE parcels SET current_version = $2, updated_at = now() WHERE id = $1")
        .bind(v.parcel_id)
        .bind(v.version)
        .execute(&mut **tx)
        .await
        .context("advancing current version")?;
    Ok(())
}

/// Apply optional metadata changes inside a publish transaction.
pub async fn update_metadata_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    title: Option<&str>,
    description: Option<&str>,
    visibility: Option<Visibility>,
) -> Result<()> {
    sqlx::query(
        "UPDATE parcels SET
             title = COALESCE($2, title),
             description = COALESCE($3, description),
             visibility = COALESCE($4, visibility)
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .bind(visibility.map(Visibility::as_str))
    .execute(&mut **tx)
    .await
    .context("updating parcel metadata")?;
    Ok(())
}

/// Sum of every version's bytes for a user, read under the publish
/// transaction so the quota check and the insert see the same state.
pub async fn stored_bytes_tx(tx: &mut Transaction<'_, Postgres>, owner_id: Uuid) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(v.total_bytes), 0)::bigint
         FROM parcel_versions v JOIN parcels p ON p.id = v.parcel_id
         WHERE p.owner_id = $1",
    )
    .bind(owner_id)
    .fetch_one(&mut **tx)
    .await
    .context("computing stored bytes")
}

/// Serialize a user's publishes so two concurrent uploads can't both pass
/// the quota check. Transaction-scoped: released at commit/rollback.
pub async fn lock_owner_quota(tx: &mut Transaction<'_, Postgres>, owner_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 7101))")
        .bind(owner_id)
        .execute(&mut **tx)
        .await
        .context("taking quota lock")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::escape_like;

    #[test]
    fn like_patterns_are_escaped() {
        assert_eq!(escape_like("50%_off\\"), "50\\%\\_off\\\\");
    }
}
