//! Parcels, their versions, and each version's files.

use anyhow::{Context, Result};
use parcel_types::{ArchivedFilter, FileEntry, ListScope, SourceInfo, Usage, Visibility};
use std::collections::BTreeSet;

use parcel_types::tags::{edit_tags, TagError};
use sqlx::types::Json;
use sqlx::{AssertSqlSafe, PgExecutor, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::error::{DbContext, DbError};

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
    pub tags: Vec<String>,
    pub archived_at: Option<OffsetDateTime>,
    pub slug: Option<String>,
    /// The owner's current handle (`None` only for a user an older revision
    /// created and nothing has allocated a handle for yet).
    pub owner_handle: Option<String>,
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
    /// Owner-only: `views::version_summary` drops it for everyone else.
    #[sqlx(json(nullable))]
    pub source: Option<SourceInfo>,
}

const VERSION_COLUMNS: &str = "version, entry, label, file_count, total_bytes, created_at, source";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FileRow {
    pub path: String,
    pub blob_sha256: String,
    pub size_bytes: i64,
    pub content_type: String,
    /// A page's title, read at publish (`publish::page_title`); carried
    /// along with the file when a merge keeps it or a restore copies it.
    pub page_title: Option<String>,
}

impl From<FileRow> for FileEntry {
    fn from(f: FileRow) -> Self {
        FileEntry {
            path: f.path,
            size_bytes: f.size_bytes,
            content_type: f.content_type,
            sha256: f.blob_sha256,
            page_title: f.page_title,
        }
    }
}

macro_rules! parcel_columns {
    () => {
        "p.id, p.owner_id, p.title, p.description, p.visibility,
            p.current_version, p.created_at, p.updated_at, p.tags, p.archived_at,
            p.slug, ou.handle AS owner_handle,
            v.entry, v.file_count, v.total_bytes,
            (SELECT count(*) FROM comments c
              WHERE c.parcel_id = p.id AND c.parent_number IS NULL
                AND c.resolved_at IS NULL) AS open_comments"
    };
}
macro_rules! parcel_from {
    () => {
        "FROM parcels p
     JOIN parcel_versions v ON v.parcel_id = p.id AND v.version = p.current_version
     LEFT JOIN user_handles ou ON ou.user_id = p.owner_id AND ou.is_current"
    };
}

/// The columns of a [`ParcelRow`] (over `p`, `v` and the owner's current
/// handle `ou`), for queries that select more alongside them.
pub const PARCEL_COLUMNS: &str = parcel_columns!();
/// The `FROM` that [`PARCEL_COLUMNS`] reads, ready for more joins.
pub const PARCEL_FROM: &str = parcel_from!();
const PARCEL_SELECT: &str = concat!("SELECT ", parcel_columns!(), "\n     ", parcel_from!());

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<ParcelRow>> {
    sqlx::query_as::<_, ParcelRow>(AssertSqlSafe(format!("{PARCEL_SELECT} WHERE p.id = $1")))
        .bind(id)
        .fetch_optional(pool)
        .await
        .db_context("loading parcel")
}

/// What a parcel list (or its tag counts) covers. Every scope, filter and
/// query goes through [`search`] and [`tag_counts`], which share one scoped
/// relation, so results, totals, snippets and facet counts always agree.
#[derive(Debug, Clone)]
pub struct ListFilter {
    pub scope: ListScope,
    /// The caller: owner for `mine`, and never "shared with" themselves.
    pub user_id: Uuid,
    /// The caller's email, which shares are matched against.
    pub email: String,
    /// Normalized; a parcel must carry all of them.
    pub tags: Vec<String>,
    pub archived: ArchivedFilter,
    /// Trimmed and non-empty, or `None`.
    pub query: Option<String>,
}

/// A row of [`search`]: the parcel, how it is shared with the caller (when
/// they aren't the owner), and the raw snippet when `query` matched page text.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchRow {
    #[sqlx(flatten)]
    pub parcel: ParcelRow,
    pub owner_name: Option<String>,
    pub owner_email: Option<String>,
    pub share_role: Option<String>,
    pub shared_at: Option<OffsetDateTime>,
    /// `ts_headline` output, with matched words between `\x02` and `\x03`.
    pub headline: Option<String>,
}

/// `ts_headline` options: one short fragment, matched words wrapped in
/// control characters that extraction strips from page text, so a marker in
/// the output can only be one `ts_headline` put there.
pub const HEADLINE_OPTIONS: &str =
    "MaxFragments=1, MaxWords=24, MinWords=10, StartSel=\u{2}, StopSel=\u{3}";

/// Longest `q` accepted (characters).
pub const MAX_QUERY_CHARS: usize = 500;

// The scoped relation. Fixed fragments only; every value is a bound
// parameter, numbered the same in every query built here:
//   $1 caller id, $2 caller email, $3 tags (text[]), $4 q, $5 q as an
//   ILIKE pattern. Queries add their own parameters from $6.
// One relation serves every scope: the share and owner joins match only
// parcels the caller doesn't own, so the share columns are set exactly when
// a share brought the parcel to them, and a scope is just its WHERE. A
// share is matched by (parcel_id, email), the primary key, so a parcel
// joins at most one share row. Without `q`, `params.tsq` is NULL and the
// rank is 0 throughout.
const PARAMS_CTE: &str = "WITH params AS (SELECT websearch_to_tsquery('english', $4) AS tsq)";
const SCOPED_FROM: &str = concat!(
    parcel_from!(),
    "
     LEFT JOIN parcel_shares s
       ON s.parcel_id = p.id AND s.email = lower($2) AND p.owner_id <> $1
     LEFT JOIN users o ON o.id = p.owner_id AND s.parcel_id IS NOT NULL
     CROSS JOIN params"
);
const SHARED_COLUMNS: &str = "o.name AS owner_name, o.email AS owner_email,
            s.role AS share_role, s.created_at AS shared_at";
const TAGS_CLAUSE: &str = "\n       AND p.tags @> $3::text[]";
// A query that parses to no lexemes (only stop words or punctuation) has
// numnode 0: then only the substring match applies.
const QUERY_CLAUSE: &str = "
       AND (p.id ILIKE $5 ESCAPE '\\' OR p.title ILIKE $5 ESCAPE '\\'
            OR p.description ILIKE $5 ESCAPE '\\' OR p.slug ILIKE $5 ESCAPE '\\'
            OR (numnode(params.tsq) > 0 AND p.search_vec @@ params.tsq))";
const RANK: &str =
    "CASE WHEN numnode(params.tsq) > 0 THEN ts_rank_cd(p.search_vec, params.tsq) ELSE 0 END";

/// The `WHERE` conditions of the scoped relation for a filter.
fn conditions(f: &ListFilter) -> String {
    let scope = match f.scope {
        ListScope::Mine => "p.owner_id = $1",
        ListScope::Shared => "s.parcel_id IS NOT NULL",
        ListScope::All => "(p.owner_id = $1 OR s.parcel_id IS NOT NULL)",
    };
    let archived = match f.archived {
        ArchivedFilter::Exclude => "\n       AND p.archived_at IS NULL",
        ArchivedFilter::Include => "",
        ArchivedFilter::Only => "\n       AND p.archived_at IS NOT NULL",
    };
    let tags = if f.tags.is_empty() { "" } else { TAGS_CLAUSE };
    let query = if f.query.is_some() { QUERY_CLAUSE } else { "" };
    format!("{scope}{archived}{tags}{query}")
}

/// Bind the parameters every scoped query shares ($1–$5).
fn bind_filter<'q, O>(
    query: sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments>,
    f: &'q ListFilter,
) -> sqlx::query::QueryAs<'q, Postgres, O, sqlx::postgres::PgArguments> {
    query
        .bind(f.user_id)
        .bind(f.email.as_str())
        .bind(f.tags.as_slice())
        .bind(f.query.as_deref())
        .bind(f.query.as_deref().map(|q| format!("%{}%", escape_like(q))))
}

/// One page of a parcel list, and the total. With a query, ordered by
/// relevance (`ts_rank_cd`, title over description and tags over page text)
/// then recency; without, by recency. Snippets are computed only for the
/// page's rows, after `LIMIT`.
pub async fn search(
    pool: &PgPool,
    f: &ListFilter,
    limit: i64,
    offset: i64,
) -> Result<(Vec<SearchRow>, i64)> {
    let conditions = conditions(f);
    let headline = if f.query.is_some() {
        "(SELECT CASE WHEN numnode(params.tsq) > 0 AND x.search_text <> ''
                      THEN ts_headline('english', x.search_text, params.tsq, $8) END
            FROM parcels x WHERE x.id = page.id)"
    } else {
        "NULL::text"
    };
    let rows = bind_filter(
        sqlx::query_as::<_, SearchRow>(AssertSqlSafe(format!(
            "{PARAMS_CTE}, page AS (
             SELECT {PARCEL_COLUMNS}, {SHARED_COLUMNS}, {RANK} AS search_rank
             {SCOPED_FROM}
             WHERE {conditions}
             ORDER BY search_rank DESC, p.updated_at DESC, p.id
             LIMIT $6 OFFSET $7)
             SELECT page.*, {headline} AS headline
             FROM page CROSS JOIN params
             ORDER BY page.search_rank DESC, page.updated_at DESC, page.id"
        ))),
        f,
    )
    .bind(limit)
    .bind(offset)
    .bind(HEADLINE_OPTIONS)
    .fetch_all(pool);
    let total = bind_filter(
        sqlx::query_as::<_, (i64,)>(AssertSqlSafe(format!(
            "{PARAMS_CTE} SELECT count(*) {SCOPED_FROM} WHERE {conditions}"
        ))),
        f,
    )
    .fetch_one(pool);
    let (rows, (total,)) =
        tokio::try_join!(async { rows.await.db_context("listing parcels") }, async {
            total.await.db_context("counting parcels")
        },)?;
    Ok((rows, total))
}

/// Tags on the parcels a filter covers, with how many of them carry each
/// (facet counts): most used first, then by name. `prefix` (normalized)
/// keeps tags that start with it. Returns a page and the distinct total.
pub async fn tag_counts(
    pool: &PgPool,
    f: &ListFilter,
    prefix: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<(String, i64)>, i64)> {
    let pattern = prefix.map(|p| format!("{}%", escape_like(p)));
    let relation = format!(
        "{SCOPED_FROM}
     CROSS JOIN LATERAL unnest(p.tags) AS t(tag)
     WHERE {}
       AND ($6::text IS NULL OR t.tag LIKE $6 ESCAPE '\\')",
        conditions(f)
    );
    let rows = bind_filter(
        sqlx::query_as::<_, (String, i64)>(AssertSqlSafe(format!(
            "{PARAMS_CTE} SELECT t.tag, count(*) AS n {relation}
             GROUP BY t.tag ORDER BY n DESC, t.tag COLLATE \"C\" LIMIT $7 OFFSET $8"
        ))),
        f,
    )
    .bind(pattern.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool);
    let total = bind_filter(
        sqlx::query_as::<_, (i64,)>(AssertSqlSafe(format!(
            "{PARAMS_CTE} SELECT count(DISTINCT t.tag) {relation}"
        ))),
        f,
    )
    .bind(pattern.as_deref())
    .fetch_one(pool);
    let (rows, (total,)) =
        tokio::try_join!(async { rows.await.db_context("counting tags") }, async {
            total.await.db_context("counting distinct tags")
        },)?;
    Ok((rows, total))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn list_versions(pool: &PgPool, parcel_id: &str) -> Result<Vec<VersionRow>> {
    sqlx::query_as::<_, VersionRow>(AssertSqlSafe(format!(
        "SELECT {VERSION_COLUMNS}
         FROM parcel_versions WHERE parcel_id = $1 ORDER BY version DESC"
    )))
    .bind(parcel_id)
    .fetch_all(pool)
    .await
    .db_context("listing versions")
}

/// Generic over the executor so a publish can read inside its own
/// transaction instead of taking a second pool connection.
pub async fn get_version<'e>(
    db: impl PgExecutor<'e>,
    parcel_id: &str,
    version: i32,
) -> Result<Option<VersionRow>> {
    sqlx::query_as::<_, VersionRow>(AssertSqlSafe(format!(
        "SELECT {VERSION_COLUMNS}
         FROM parcel_versions WHERE parcel_id = $1 AND version = $2"
    )))
    .bind(parcel_id)
    .bind(version)
    .fetch_optional(db)
    .await
    .db_context("loading version")
}

pub async fn version_files<'e>(
    db: impl PgExecutor<'e>,
    parcel_id: &str,
    version: i32,
) -> Result<Vec<FileRow>> {
    sqlx::query_as::<_, FileRow>(
        "SELECT path, blob_sha256, size_bytes, content_type, page_title
         FROM version_files WHERE parcel_id = $1 AND version = $2 ORDER BY path",
    )
    .bind(parcel_id)
    .bind(version)
    .fetch_all(db)
    .await
    .db_context("listing version files")
}

pub async fn find_file(
    pool: &PgPool,
    parcel_id: &str,
    version: i32,
    path: &str,
) -> Result<Option<FileRow>> {
    sqlx::query_as::<_, FileRow>(
        "SELECT path, blob_sha256, size_bytes, content_type, page_title
         FROM version_files WHERE parcel_id = $1 AND version = $2 AND path = $3",
    )
    .bind(parcel_id)
    .bind(version)
    .bind(path)
    .fetch_optional(pool)
    .await
    .db_context("finding file")
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
    .db_context("content lookup")
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
    .db_context("loading parcel grant state")?;
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
    .db_context("loading version entry")
}

/// How a PATCH changes the tag set.
#[derive(Debug, Clone, Default)]
pub enum TagsChange {
    #[default]
    Keep,
    /// Normalized and capped already.
    Replace(Vec<String>),
    /// Normalized; the result is capped under the row lock.
    Edit {
        add: BTreeSet<String>,
        remove: BTreeSet<String>,
    },
}

/// A PATCH of an owned parcel. Title, description and visibility are
/// edits (they bump `updated_at`); tags, archive state and the slug are
/// organization (they don't, so tidying never reorders the list).
#[derive(Debug, Clone, Default)]
pub struct Patch<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub visibility: Option<Visibility>,
    pub tags: TagsChange,
    pub archived: Option<bool>,
    /// `Some(None)` clears the slug; `Some(Some(s))` sets it (normalized).
    pub slug: Option<Option<&'a str>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PatchRefused {
    ParcelGone,
    Tags(TagError),
    /// Another of the owner's parcels has this slug.
    SlugTaken,
}

/// Apply a PATCH in one transaction: the owner's lock, then the parcel's
/// row lock (the one lock order, so a concurrent publish or tag rename
/// can't deadlock with it), then the tag edit computed against the locked
/// row, so two concurrent edits never lose each other's tags.
pub async fn apply_patch(
    pool: &PgPool,
    owner_id: Uuid,
    id: &str,
    patch: &Patch<'_>,
) -> Result<Result<(), PatchRefused>> {
    let mut tx = pool.begin().await.db_context("beginning parcel update")?;
    lock_owner_quota(&mut tx, owner_id).await?;
    let Some(locked) = lock_for_update(&mut tx, id).await? else {
        return Ok(Err(PatchRefused::ParcelGone));
    };
    if locked.owner_id != owner_id {
        return Ok(Err(PatchRefused::ParcelGone));
    }
    let tags = match &patch.tags {
        TagsChange::Keep => None,
        TagsChange::Replace(tags) => Some(tags.clone()),
        TagsChange::Edit { add, remove } => match edit_tags(&locked.tags, add, remove) {
            Ok(tags) => Some(tags),
            Err(e) => return Ok(Err(PatchRefused::Tags(e))),
        },
    };
    if let Some(new) = patch.slug {
        if let Some(name) = new {
            if super::slugs::taken_by_other(&mut tx, owner_id, name, id).await? {
                return Ok(Err(PatchRefused::SlugTaken));
            }
        }
        super::slugs::set(&mut tx, owner_id, id, locked.slug.as_deref(), new).await?;
    }
    if tags.is_some() || patch.archived.is_some() {
        update_organization(&mut tx, id, tags.as_deref(), patch.archived).await?;
    }
    if patch.title.is_some() || patch.description.is_some() || patch.visibility.is_some() {
        update_metadata_tx(
            &mut tx,
            id,
            patch.title,
            patch.description,
            patch.visibility,
            true,
        )
        .await?;
    }
    tx.commit().await.db_context("committing parcel update")?;
    Ok(Ok(()))
}

/// Set the tags and/or the archive state, leaving `updated_at` alone.
/// `archived: Some(true)` keeps an existing archive time. Only a tags change
/// names `tags` in its `SET`: the search trigger (0006) rebuilds the search
/// vector whenever the column is listed, changed or not.
pub async fn update_organization(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    tags: Option<&[String]>,
    archived: Option<bool>,
) -> Result<()> {
    if let Some(tags) = tags {
        sqlx::query("UPDATE parcels SET tags = $2 WHERE id = $1")
            .bind(id)
            .bind(tags)
            .execute(&mut **tx)
            .await
            .db_context("updating parcel tags")?;
    }
    if let Some(archived) = archived {
        sqlx::query(
            "UPDATE parcels SET
                 archived_at = CASE WHEN $2 THEN COALESCE(archived_at, now()) ELSE NULL END
             WHERE id = $1",
        )
        .bind(id)
        .bind(archived)
        .execute(&mut **tx)
        .await
        .db_context("updating parcel archive state")?;
    }
    Ok(())
}

/// Delete a parcel (versions and file rows cascade). Blobs it referenced are
/// left for GC, which only collects what nothing references anymore.
pub async fn delete(pool: &PgPool, id: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM parcels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .db_context("deleting parcel")?;
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
    .db_context("computing usage")?;
    Ok(Usage {
        parcel_count,
        version_count,
        stored_bytes,
    })
}

// ---- publish transaction pieces -------------------------------------------

/// A brand-new parcel row.
pub struct NewParcel<'a> {
    pub id: &'a str,
    pub owner_id: Uuid,
    pub title: &'a str,
    pub description: &'a str,
    pub visibility: Visibility,
    pub tags: &'a [String],
    pub slug: Option<&'a str>,
}

/// The constraint a parcel id collision violates (the only one
/// [`insert_parcel`]'s caller retries on).
pub const PARCEL_PKEY: &str = "parcels_pkey";
/// The constraint two parcels of one owner with the same slug violate.
pub const PARCEL_SLUG_INDEX: &str = "parcels_owner_slug_idx";

/// Insert a brand-new parcel row (its first version is inserted separately,
/// in the same transaction).
pub async fn insert_parcel(tx: &mut Transaction<'_, Postgres>, p: &NewParcel<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO parcels (id, owner_id, title, description, visibility, tags, slug, current_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 0)",
    )
    .bind(p.id)
    .bind(p.owner_id)
    .bind(p.title)
    .bind(p.description)
    .bind(p.visibility.as_str())
    .bind(p.tags)
    .bind(p.slug)
    .execute(&mut **tx)
    .await
    .db_context("inserting parcel")?;
    Ok(())
}

/// The name of the unique constraint `e` violated, if that is what it is.
pub fn violated_constraint(e: &anyhow::Error) -> Option<&str> {
    DbError::find(e)
        .filter(|d| d.is_unique_violation())
        .map(|d| d.constraint().unwrap_or_default())
}

/// A parcel's row, locked for the rest of the transaction.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LockedParcel {
    pub owner_id: Uuid,
    pub current_version: i32,
    pub tags: Vec<String>,
    pub slug: Option<String>,
}

/// Lock a parcel's row. Callers that also take the owner's lock take it
/// first ([`lock_owner_quota`], then this): one order everywhere.
pub async fn lock_for_update(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<Option<LockedParcel>> {
    sqlx::query_as(
        "SELECT owner_id, current_version, tags, slug FROM parcels WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .db_context("locking parcel")
}

pub struct NewVersion<'a> {
    pub parcel_id: &'a str,
    pub version: i32,
    pub entry: &'a str,
    pub label: Option<&'a str>,
    pub created_via: &'a str,
    pub files: &'a [FileRow],
    /// Already validated (`SourceInfo::validated`).
    pub source: Option<&'a SourceInfo>,
}

/// Insert a version and its files, and make it current.
pub async fn insert_version(tx: &mut Transaction<'_, Postgres>, v: &NewVersion<'_>) -> Result<()> {
    let total: i64 = v.files.iter().map(|f| f.size_bytes).sum();
    let count = i32::try_from(v.files.len()).context("file count overflow")?;
    sqlx::query(
        "INSERT INTO parcel_versions
             (parcel_id, version, entry, label, file_count, total_bytes, created_via, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(v.parcel_id)
    .bind(v.version)
    .bind(v.entry)
    .bind(v.label)
    .bind(count)
    .bind(total)
    .bind(v.created_via)
    .bind(v.source.map(Json))
    .execute(&mut **tx)
    .await
    .db_context("inserting version")?;

    let paths: Vec<&str> = v.files.iter().map(|f| f.path.as_str()).collect();
    let shas: Vec<&str> = v.files.iter().map(|f| f.blob_sha256.as_str()).collect();
    let sizes: Vec<i64> = v.files.iter().map(|f| f.size_bytes).collect();
    let types: Vec<&str> = v.files.iter().map(|f| f.content_type.as_str()).collect();
    let titles: Vec<Option<&str>> = v.files.iter().map(|f| f.page_title.as_deref()).collect();
    sqlx::query(
        "INSERT INTO version_files
             (parcel_id, version, path, blob_sha256, size_bytes, content_type, page_title)
         SELECT $1, $2, t.path, t.sha, t.size, t.ctype, t.title
         FROM UNNEST($3::text[], $4::text[], $5::bigint[], $6::text[], $7::text[])
              AS t(path, sha, size, ctype, title)",
    )
    .bind(v.parcel_id)
    .bind(v.version)
    .bind(&paths)
    .bind(&shas)
    .bind(&sizes)
    .bind(&types)
    .bind(&titles)
    .execute(&mut **tx)
    .await
    .db_context("inserting version files")?;

    sqlx::query("UPDATE parcels SET current_version = $2, updated_at = now() WHERE id = $1")
        .bind(v.parcel_id)
        .bind(v.version)
        .execute(&mut **tx)
        .await
        .db_context("advancing current version")?;
    Ok(())
}

/// Apply optional metadata changes inside a transaction. `touch` bumps
/// `updated_at` (a PATCH; a publish has already bumped it).
pub async fn update_metadata_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    title: Option<&str>,
    description: Option<&str>,
    visibility: Option<Visibility>,
    touch: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE parcels SET
             title = COALESCE($2, title),
             description = COALESCE($3, description),
             visibility = COALESCE($4, visibility),
             updated_at = CASE WHEN $5 THEN now() ELSE updated_at END
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .bind(visibility.map(Visibility::as_str))
    .bind(touch)
    .execute(&mut **tx)
    .await
    .db_context("updating parcel metadata")?;
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
    .db_context("computing stored bytes")
}

/// Serialize a user's publishes so two concurrent uploads can't both pass
/// the quota check; also held by every other write that touches several of
/// the owner's parcels or rewrites one's tags. Always taken before any
/// parcel row lock. Transaction-scoped: released at commit/rollback.
pub async fn lock_owner_quota(tx: &mut Transaction<'_, Postgres>, owner_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 7101))")
        .bind(owner_id)
        .execute(&mut **tx)
        .await
        .db_context("taking quota lock")?;
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
