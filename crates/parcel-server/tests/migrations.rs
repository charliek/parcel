//! Migrations that move data, against the state they meet in production.

mod common;

use std::sync::Arc;

use parcel_server::db::parcels::{self, ListFilter};
use parcel_server::state::AppState;
use parcel_server::storage::{blob_key, BlobStore, MemoryBlobStore};
use parcel_types::{ArchivedFilter, ListScope};
use sqlx::{AssertSqlSafe, PgPool};

const BEFORE_0005: [&str; 4] = [
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_comments_and_data.sql"),
    include_str!("../migrations/0003_identities_and_sharing.sql"),
    include_str!("../migrations/0004_comment_via_mcp.sql"),
];
const DROP_USERS_SUB: &str = include_str!("../migrations/0005_drop_users_sub.sql");
const TAGS_ARCHIVE_SEARCH: &str = include_str!("../migrations/0006_tags_archive_search.sql");

async fn apply(pool: &PgPool, sql: &str) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .map(|_| ())
}

/// Each test pool lives in a schema of its own, whether or not
/// `DATABASE_URL` is set (`require_postgres` would let a set one replace the
/// URL carrying the `search_path`, putting every test in `public`).
#[tokio::test]
async fn test_pools_keep_their_own_search_path() {
    let pool = common::schema_pool().await;
    let schema: String = sqlx::query_scalar("SELECT current_schema()::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(schema.starts_with("t_"), "{schema}");
    let other: String = sqlx::query_scalar("SELECT current_schema()::text")
        .fetch_one(&common::schema_pool().await)
        .await
        .unwrap();
    assert!(
        other.starts_with("t_") && other != schema,
        "{other} vs {schema}"
    );
}

#[tokio::test]
async fn dropping_users_sub_refuses_to_strand_a_user() {
    let pool = common::schema_pool().await;
    for sql in BEFORE_0005 {
        apply(&pool, sql).await.expect("earlier migration");
    }
    // A legacy row with no identity (0003 backfills one for every user, so
    // only a hand-edited database looks like this).
    let stranded: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (sub, email, name) VALUES ('legacy-sub', 'old@example.com', 'Old')
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let refused = apply(&pool, DROP_USERS_SUB).await.unwrap_err();
    assert!(
        refused.to_string().contains("not dropping users.sub"),
        "{refused}"
    );

    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email)
         VALUES ('slauth', 'legacy-sub', $1, 'old@example.com')",
    )
    .bind(stranded)
    .execute(&pool)
    .await
    .unwrap();
    apply(&pool, DROP_USERS_SUB)
        .await
        .expect("drops once every user can sign in");
    let column: Option<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'users' AND column_name = 'sub'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(column, None);
}

/// A parcel as a revision from before 0006 left it: a user, a parcel with
/// one version whose entry page is in the blob store. Returns the owner and
/// the parcel id.
async fn legacy_parcel(pool: &PgPool, blobs: &MemoryBlobStore) -> (uuid::Uuid, String) {
    let owner: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, name) VALUES ('legacy@example.com', 'Legacy') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let page = b"<html><head><title>Ocelot migration plan</title></head>\
        <body><p>The capybara section explains the rollout.</p></body></html>";
    let sha = "a".repeat(64);
    blobs
        .put(
            &blob_key(&sha),
            bytes::Bytes::from_static(page),
            "text/html",
        )
        .await
        .unwrap();
    let id = "legacyparcel";
    let statements = [
        format!("INSERT INTO blobs (sha256, size_bytes) VALUES ('{sha}', {})", page.len()),
        format!(
            "INSERT INTO parcels (id, owner_id, title, description, current_version)
             VALUES ('{id}', '{owner}', 'Ocelot migration plan', 'quarterly numbers', 0)"
        ),
        format!(
            "INSERT INTO parcel_versions (parcel_id, version, entry, file_count, total_bytes)
             VALUES ('{id}', 1, 'index.html', 1, {})",
            page.len()
        ),
        format!(
            "INSERT INTO version_files (parcel_id, version, path, blob_sha256, size_bytes, content_type)
             VALUES ('{id}', 1, 'index.html', '{sha}', {}, 'text/html; charset=utf-8')",
            page.len()
        ),
        format!("UPDATE parcels SET current_version = 1 WHERE id = '{id}'"),
    ];
    for sql in statements {
        apply(pool, &sql).await.expect("legacy fixture");
    }
    (owner, id.to_string())
}

async fn apply_through_0005(pool: &PgPool) {
    for sql in BEFORE_0005 {
        apply(pool, sql).await.expect("earlier migration");
    }
    apply(pool, DROP_USERS_SUB).await.expect("0005");
}

fn mine(owner: uuid::Uuid, query: Option<&str>) -> ListFilter {
    ListFilter {
        scope: ListScope::Mine,
        user_id: owner,
        email: "legacy@example.com".into(),
        tags: vec![],
        archived: ArchivedFilter::Exclude,
        query: query.map(str::to_string),
    }
}

/// 0006 meets parcels that already exist: they get empty tags, aren't
/// archived, and their metadata is searchable at once — before any page
/// text is extracted. The backfill then makes the page text searchable.
#[tokio::test]
async fn tags_and_search_reach_existing_parcels() {
    let pool = common::schema_pool().await;
    apply_through_0005(&pool).await;
    let blobs = Arc::new(MemoryBlobStore::default());
    let (owner, id) = legacy_parcel(&pool, &blobs).await;
    apply(&pool, TAGS_ARCHIVE_SEARCH).await.expect("0006");

    let (tags, archived, text, attempts): (
        Vec<String>,
        Option<time::OffsetDateTime>,
        Option<String>,
        i16,
    ) = sqlx::query_as(
        "SELECT tags, archived_at, search_text, search_attempts FROM parcels WHERE id = $1",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(tags.is_empty() && archived.is_none() && text.is_none() && attempts == 0);

    for (query, found) in [
        (Some("ocelot"), true),    // title, full text
        (Some("quarterly"), true), // description
        (Some("migrat"), true),    // title substring
        (Some("capybara"), false), // page text: not extracted yet
        (None, true),
    ] {
        let (rows, total) = parcels::search(&pool, &mine(owner, query), 50, 0)
            .await
            .unwrap();
        assert_eq!(total, i64::from(found), "{query:?}");
        assert_eq!(rows.len(), usize::from(found), "{query:?}");
    }

    let state = AppState::with_parts(
        common::test_config("http://127.0.0.1:9"),
        pool.clone(),
        reqwest::Client::new(),
        blobs,
    );
    let report = parcel_server::search::drain(&state, None, parcel_server::search::MAX_ATTEMPTS)
        .await
        .unwrap();
    assert_eq!(report.indexed, 1);
    let (rows, total) = parcels::search(&pool, &mine(owner, Some("capybara")), 50, 0)
        .await
        .unwrap();
    assert_eq!(total, 1);
    let snippet = parcel_server::search::segments(rows[0].headline.as_deref().unwrap()).unwrap();
    assert!(
        snippet
            .segments
            .iter()
            .any(|s| s.hit && s.text == "capybara"),
        "{snippet:?}"
    );
}

/// A version published by a revision that knows nothing of search (a plain
/// `UPDATE … SET current_version`) still clears the stale page text, and
/// the old page's words leave `search_vec` with it: the invalidating
/// trigger runs before the one that rebuilds the vector, and the rebuild
/// fires on a version change although the statement never names
/// `search_text`.
#[tokio::test]
async fn a_version_bump_from_an_old_revision_clears_the_page_text() {
    let pool = common::schema_pool().await;
    apply_through_0005(&pool).await;
    let blobs = Arc::new(MemoryBlobStore::default());
    let (_, id) = legacy_parcel(&pool, &blobs).await;
    apply(&pool, TAGS_ARCHIVE_SEARCH).await.expect("0006");
    apply(
        &pool,
        "UPDATE parcels SET search_text = 'the capybara section', search_attempts = 3",
    )
    .await
    .unwrap();
    let matches = |word: &'static str| {
        let pool = pool.clone();
        let id = id.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT search_vec @@ to_tsquery('english', $2) FROM parcels WHERE id = $1",
            )
            .bind(id)
            .bind(word)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert!(matches("capybara").await);

    // Touching other columns (a comment counter, the same version) keeps it.
    apply(
        &pool,
        "UPDATE parcels SET comments_rev = comments_rev + 1, current_version = current_version",
    )
    .await
    .unwrap();
    assert!(matches("capybara").await);

    // What an older revision's publish does.
    for sql in [
        format!(
            "INSERT INTO parcel_versions (parcel_id, version, entry, file_count, total_bytes)
             SELECT parcel_id, 2, entry, file_count, total_bytes FROM parcel_versions
             WHERE parcel_id = '{id}' AND version = 1"
        ),
        format!("UPDATE parcels SET current_version = 2, updated_at = now() WHERE id = '{id}'"),
    ] {
        apply(&pool, &sql).await.unwrap();
    }
    let (text, attempts): (Option<String>, i16) =
        sqlx::query_as("SELECT search_text, search_attempts FROM parcels WHERE id = $1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((text, attempts), (None, 0));
    assert!(!matches("capybara").await, "the old page's words are gone");
    assert!(matches("ocelot").await, "the title is still there");
}

/// Archive state lives in the same trigger: an older revision's publish or
/// restore knows nothing of `archived_at`, yet the new version must come back
/// live. Bumps that keep the version leave the archive alone.
#[tokio::test]
async fn a_version_bump_from_an_old_revision_unarchives() {
    let pool = common::schema_pool().await;
    apply_through_0005(&pool).await;
    let blobs = Arc::new(MemoryBlobStore::default());
    let (_, id) = legacy_parcel(&pool, &blobs).await;
    apply(&pool, TAGS_ARCHIVE_SEARCH).await.expect("0006");
    apply(&pool, "UPDATE parcels SET archived_at = now()")
        .await
        .unwrap();
    let archived = |pool: sqlx::PgPool, id: String| async move {
        sqlx::query_scalar::<_, bool>("SELECT archived_at IS NOT NULL FROM parcels WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    assert!(archived(pool.clone(), id.clone()).await);

    apply(
        &pool,
        "UPDATE parcels SET comments_rev = comments_rev + 1, current_version = current_version",
    )
    .await
    .unwrap();
    assert!(
        archived(pool.clone(), id.clone()).await,
        "the same version keeps it archived"
    );

    for sql in [
        format!(
            "INSERT INTO parcel_versions (parcel_id, version, entry, file_count, total_bytes)
             SELECT parcel_id, 2, entry, file_count, total_bytes FROM parcel_versions
             WHERE parcel_id = '{id}' AND version = 1"
        ),
        format!("UPDATE parcels SET current_version = 2, updated_at = now() WHERE id = '{id}'"),
    ] {
        apply(&pool, &sql).await.unwrap();
    }
    assert!(
        !archived(pool.clone(), id.clone()).await,
        "the new version is live"
    );
}
