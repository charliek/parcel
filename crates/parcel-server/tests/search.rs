//! Full-text search over the real router and Postgres: what is indexed,
//! how results rank, snippets, re-indexing on new versions and restores,
//! the guarded write, the backfill, value-free failure logs, when the
//! search vector is rebuilt, and the relaxed (any-word) fallback.

mod common;

use axum::http::StatusCode;
use common::*;
use parcel_server::db::parcels::HEADLINE_OPTIONS;
use parcel_server::search::{self, MAX_ATTEMPTS};
use parcel_server::storage::{blob_key, BlobStore};
use parcel_types::{ParcelList, ParcelListItem, PublishResult, TagList, Visibility};
use serde_json::{json, Value};
use sqlx::AssertSqlSafe;

async fn post_files(
    app: &TestApp,
    token: &str,
    uri: &str,
    metadata: Value,
    files: &[(&str, &str)],
) -> PublishResult {
    let mut form = Form::new().metadata(metadata);
    for (path, body) in files {
        form = form.file(path, body.to_string());
    }
    expect_json(
        app.post_multipart(uri, Some(token), &form).await,
        StatusCode::CREATED,
    )
    .await
}

async fn publish(
    app: &TestApp,
    token: &str,
    metadata: Value,
    files: &[(&str, &str)],
) -> PublishResult {
    post_files(app, token, "/api/v1/parcels", metadata, files).await
}

async fn new_version(
    app: &TestApp,
    token: &str,
    id: &str,
    metadata: Value,
    files: &[(&str, &str)],
) -> PublishResult {
    let uri = format!("/api/v1/parcels/{id}/versions");
    post_files(app, token, &uri, metadata, files).await
}

/// Wait for every post-commit index task to finish, then sweep up anything
/// left, as the backfill would. Waiting matters: a task still in flight can
/// store its text after a test has cleared it (a yield doesn't run it to
/// completion, as its extraction runs on the blocking pool).
async fn settle(app: &TestApp) {
    search::wait_for_index_tasks(&app.state).await;
    search::drain(&app.state, None, MAX_ATTEMPTS).await.unwrap();
}

async fn find(app: &TestApp, token: &str, q: &str) -> ParcelList {
    let uri = format!("/api/v1/parcels?q={}", urlencode(q));
    expect_json(app.get(&uri, Some(token)).await, StatusCode::OK).await
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn titles(list: &ParcelList) -> Vec<&str> {
    list.parcels.iter().map(|p| p.title.as_str()).collect()
}

fn hits(item: &ParcelListItem) -> Vec<String> {
    item.matched
        .as_ref()
        .map(|m| {
            m.segments
                .iter()
                .filter(|s| s.hit)
                .map(|s| s.text.clone())
                .collect()
        })
        .unwrap_or_default()
}

async fn search_text(app: &TestApp, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT search_text FROM parcels WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn words_in_the_page_body_find_html_and_markdown_parcels() {
    let app = TestApp::new().await;
    let me = user_token();
    let html = publish(
        &app,
        &me,
        json!({"title": "Quarterly report"}),
        &[("index.html", "<html><head><title>x</title><script>const hidden = 'okapi';</script></head><body><p>Our zanzibar office grew.</p></body></html>")],
    )
    .await;
    let md = publish(
        &app,
        &me,
        json!({}),
        &[("notes.md", "# Field notes\n\nSaw a **quokka** today.\n")],
    )
    .await;
    settle(&app).await;

    assert_eq!(
        titles(&find(&app, &me, "zanzibar").await),
        ["Quarterly report"]
    );
    assert_eq!(titles(&find(&app, &me, "quokka").await), ["Field notes"]);
    assert_eq!(
        find(&app, &me, "okapi").await.total,
        0,
        "script contents aren't text"
    );
    // Substring matches still work on ids and titles.
    assert_eq!(
        find(&app, &me, &html.parcel.id[2..9]).await.parcels[0].id,
        html.parcel.id
    );
    assert_eq!(
        find(&app, &me, "uarterl").await.parcels[0].id,
        html.parcel.id
    );
    assert_eq!(find(&app, &me, "notes").await.parcels[0].id, md.parcel.id);
    // A query of only stop words or punctuation falls back to substrings.
    assert_eq!(find(&app, &me, "the").await.total, 0);
    assert_eq!(find(&app, &me, "\"&|!").await.total, 0);
    // Someone else's search sees none of it.
    assert_eq!(find(&app, &user_token(), "zanzibar").await.total, 0);
}

#[tokio::test]
async fn titles_outrank_descriptions_and_tags_which_outrank_page_text() {
    let app = TestApp::new().await;
    let me = user_token();
    let page = |body: &str| format!("<p>{body}</p>");
    // Published oldest to newest, so recency alone would reverse the order.
    publish(
        &app,
        &me,
        json!({"title": "Heliotrope garden"}),
        &[("index.html", &page("flowers"))],
    )
    .await;
    publish(
        &app,
        &me,
        json!({"title": "Described", "description": "about heliotrope"}),
        &[("index.html", &page("plants"))],
    )
    .await;
    publish(
        &app,
        &me,
        json!({"title": "Tagged", "tags": ["heliotrope"]}),
        &[("index.html", &page("seeds"))],
    )
    .await;
    publish(
        &app,
        &me,
        json!({"title": "Body only"}),
        &[("index.html", &page("a heliotrope in the text"))],
    )
    .await;
    publish(
        &app,
        &me,
        json!({"title": "Substring xheliotropex"}),
        &[("index.html", &page("nothing"))],
    )
    .await;
    settle(&app).await;

    let found = find(&app, &me, "heliotrope").await;
    let order = titles(&found);
    assert_eq!(found.total, 5);
    assert_eq!(order[0], "Heliotrope garden");
    let mut middle = order[1..3].to_vec();
    middle.sort_unstable();
    assert_eq!(middle, ["Described", "Tagged"]);
    assert_eq!(order[3], "Body only");
    assert_eq!(
        order[4], "Substring xheliotropex",
        "a substring-only match ranks last"
    );

    // Only a page-text match carries a snippet.
    let body = found
        .parcels
        .iter()
        .find(|p| p.title == "Body only")
        .unwrap();
    assert_eq!(hits(body), ["heliotrope"]);
    let text: String = body
        .matched
        .as_ref()
        .unwrap()
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(text.contains("heliotrope in the text"), "{text:?}");
    for p in found.parcels.iter().filter(|p| p.title != "Body only") {
        assert!(p.matched.is_none(), "{}", p.title);
    }
    // Without a query, the newest is first again.
    let all: ParcelList = json(app.get("/api/v1/parcels", Some(&me)).await).await;
    assert_eq!(all.parcels[0].title, "Substring xheliotropex");
}

#[tokio::test]
async fn snippets_are_plain_text_segments() {
    let app = TestApp::new().await;
    let me = user_token();
    publish(
        &app,
        &me,
        json!({"title": "Hostile"}),
        &[("index.html", "<p>&lt;img src=x onerror=alert(1)&gt; the needle &#2;fake&#3; \u{1b}[31m 👩‍💻 cafe\u{301}</p>")],
    )
    .await;
    settle(&app).await;
    let found = find(&app, &me, "needle").await;
    let item = &found.parcels[0];
    assert_eq!(hits(item), ["needle"]);
    let m = item.matched.as_ref().unwrap();
    let text: String = m.segments.iter().map(|s| s.text.as_str()).collect();
    // ts_headline drops what its parser takes for markup; the rest is the
    // page's text, as text (emoji and combining marks intact).
    assert!(
        text.ends_with("onerror=alert(1)> the needle fake [31m 👩‍💻 cafe\u{301}"),
        "{text:?}"
    );
    assert!(m
        .segments
        .iter()
        .all(|s| !s.text.chars().any(char::is_control)));
    // An encoded marker in the page can't fake a hit.
    let found = find(&app, &me, "alert").await;
    assert_eq!(hits(&found.parcels[0]), ["alert"]);
}

/// `ts_headline` receives the control-character markers intact (they are
/// what makes a snippet's hits unambiguous).
#[tokio::test]
async fn headline_markers_reach_ts_headline_intact() {
    let app = TestApp::new().await;
    let out: String = sqlx::query_scalar(
        "SELECT ts_headline('english', 'alpha beta gamma', websearch_to_tsquery('english', 'beta'), $1)",
    )
    .bind(HEADLINE_OPTIONS)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(out, "alpha \u{2}beta\u{3} gamma");
}

#[tokio::test]
async fn new_versions_merges_and_restores_index_the_right_page() {
    let app = TestApp::new().await;
    let me = user_token();
    let p = publish(
        &app,
        &me,
        json!({"title": "Evolving"}),
        &[("index.html", "<p>alpha words</p>")],
    )
    .await;
    let id = p.parcel.id.as_str();
    settle(&app).await;
    assert_eq!(find(&app, &me, "alpha").await.total, 1);

    new_version(
        &app,
        &me,
        id,
        json!({"mode": "replace"}),
        &[("index.html", "<p>beta words</p>")],
    )
    .await;
    settle(&app).await;
    assert_eq!(find(&app, &me, "beta").await.total, 1);
    assert_eq!(find(&app, &me, "alpha").await.total, 0);

    // A merge that doesn't re-send the entry: the text comes from the store.
    new_version(
        &app,
        &me,
        id,
        json!({}),
        &[("style.css", "p { color: red }")],
    )
    .await;
    settle(&app).await;
    assert_eq!(search_text(&app, id).await.as_deref(), Some("beta words"));
    assert_eq!(find(&app, &me, "beta").await.total, 1);

    // A restore indexes the restored page.
    expect_status(
        app.post_json(
            &format!("/api/v1/parcels/{id}/versions/1/restore"),
            Some(&me),
            &json!({}),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    settle(&app).await;
    assert_eq!(search_text(&app, id).await.as_deref(), Some("alpha words"));
    assert_eq!(find(&app, &me, "alpha").await.total, 1);
    assert_eq!(find(&app, &me, "beta").await.total, 0);

    // A new entry page in a merge is the one indexed.
    new_version(
        &app,
        &me,
        id,
        json!({"entry": "other.md"}),
        &[("other.md", "gamma words")],
    )
    .await;
    settle(&app).await;
    assert_eq!(search_text(&app, id).await.as_deref(), Some("gamma words"));
}

/// The write is guarded by the version: text extracted for version 1 that
/// arrives after version 2 exists is dropped, whether version 2's own text
/// is already stored or still pending.
#[tokio::test]
async fn a_slow_extraction_never_overwrites_a_newer_version() {
    let app = TestApp::new().await;
    let me = user_token();
    let p = publish(&app, &me, json!({}), &[("index.html", "<p>first</p>")]).await;
    let id = p.parcel.id.as_str();
    settle(&app).await;
    let stale = search::extract_text("index.html", b"<p>first</p>");

    new_version(&app, &me, id, json!({}), &[("index.html", "<p>second</p>")]).await;
    settle(&app).await;
    assert!(
        !parcel_server::db::search::store_text(&app.pool, id, 1, &stale)
            .await
            .unwrap()
    );
    assert_eq!(search_text(&app, id).await.as_deref(), Some("second"));

    // Version 2's text still pending: the stale write is refused, the right
    // one lands, and a second right one is a no-op.
    sqlx::query("UPDATE parcels SET search_text = NULL WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(
        !parcel_server::db::search::store_text(&app.pool, id, 1, &stale)
            .await
            .unwrap()
    );
    assert_eq!(search_text(&app, id).await, None);
    assert!(search::index_version(&app.state, id, 2).await.unwrap());
    assert!(!search::index_version(&app.state, id, 2).await.unwrap());
    assert!(!search::index_version(&app.state, id, 1).await.unwrap());
    assert_eq!(search_text(&app, id).await.as_deref(), Some("second"));
}

#[tokio::test]
async fn the_backfill_retries_a_bounded_number_of_times() {
    let app = TestApp::new().await;
    let me = user_token();
    let ok = publish(&app, &me, json!({}), &[("index.html", "<p>present</p>")]).await;
    let broken = publish(&app, &me, json!({}), &[("index.html", "<p>vanished</p>")]).await;
    settle(&app).await;
    // Forget both texts; lose the second page's blob.
    sqlx::query("UPDATE parcels SET search_text = NULL")
        .execute(&app.pool)
        .await
        .unwrap();
    let sha = &broken.version.files[0].sha256;
    app.blobs.delete(&blob_key(sha)).await.unwrap();
    app.state.blob_cache.invalidate(sha).await;

    let first = search::drain(&app.state, None, MAX_ATTEMPTS).await.unwrap();
    assert_eq!((first.indexed, first.failed), (1, 1));
    assert_eq!(
        search_text(&app, &ok.parcel.id).await.as_deref(),
        Some("present")
    );
    for _ in 1..MAX_ATTEMPTS {
        assert_eq!(
            search::drain(&app.state, None, MAX_ATTEMPTS)
                .await
                .unwrap()
                .failed,
            1
        );
    }
    let attempts: i16 = sqlx::query_scalar("SELECT search_attempts FROM parcels WHERE id = $1")
        .bind(&broken.parcel.id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(attempts, MAX_ATTEMPTS);
    let gave_up = search::drain(&app.state, None, MAX_ATTEMPTS).await.unwrap();
    assert_eq!((gave_up.indexed, gave_up.failed), (0, 0), "no more tries");
    // `reindex` tries everything again; a new version starts over.
    assert_eq!(
        search::drain(&app.state, None, i16::MAX)
            .await
            .unwrap()
            .failed,
        1
    );
    new_version(
        &app,
        &me,
        &broken.parcel.id,
        json!({}),
        &[("index.html", "<p>back</p>")],
    )
    .await;
    settle(&app).await;
    assert_eq!(
        search_text(&app, &broken.parcel.id).await.as_deref(),
        Some("back")
    );
}

/// When storing page text fails with a database error whose message quotes
/// the row (as a check or trigger can), the logs say only that it failed.
#[tokio::test]
async fn indexing_failures_log_no_page_text() {
    let canary = "canarytext7731";
    let app = TestApp::new().await;
    let me = user_token();
    sqlx::raw_sql(
        "CREATE FUNCTION refuse_text() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'refusing %', NEW.search_text; END $$;
         CREATE TRIGGER refuse_text BEFORE UPDATE OF search_text ON parcels
         FOR EACH ROW WHEN (NEW.search_text IS NOT NULL) EXECUTE FUNCTION refuse_text();",
    )
    .execute(&app.pool)
    .await
    .unwrap();

    let logs = Captured::default();
    let _guard = logs.install();

    let p = publish(
        &app,
        &me,
        json!({"title": "Plain title"}),
        &[("index.html", &format!("<p>secret {canary} words</p>"))],
    )
    .await;
    settle(&app).await;
    assert!(search::index_version(&app.state, &p.parcel.id, 1)
        .await
        .is_err());

    let logged = logs.text();
    assert!(
        logged.contains("indexing a parcel's page text failed"),
        "{logged}"
    );
    assert!(logged.contains("SQLSTATE P0001"), "{logged}");
    assert!(!logged.contains(canary), "{logged}");
    assert!(!logged.contains("refusing"), "{logged}");
    let attempts: i16 = sqlx::query_scalar("SELECT search_attempts FROM parcels WHERE id = $1")
        .bind(&p.parcel.id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(attempts >= 1, "the failure was counted");
}

/// Mark `id`'s search vector with a sentinel no rebuild produces. An
/// UPDATE naming only `search_vec` doesn't fire the trigger.
async fn set_sentinel(app: &TestApp, id: &str) {
    sqlx::query("UPDATE parcels SET search_vec = 'sentinel'::tsvector WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
}

/// Whether `id`'s vector is still the sentinel, and whether it is the one
/// its row's document builds.
async fn vector_state(app: &TestApp, id: &str) -> (bool, bool) {
    sqlx::query_as(
        "SELECT search_vec = 'sentinel'::tsvector,
                search_vec = parcel_search_document(title, description, tags, search_text)
         FROM parcels WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn assert_kept(app: &TestApp, id: &str, what: &str) {
    assert_eq!(vector_state(app, id).await, (true, false), "{what}: kept");
}

async fn assert_rebuilt(app: &TestApp, id: &str, what: &str) {
    assert_eq!(
        vector_state(app, id).await,
        (false, true),
        "{what}: rebuilt"
    );
}

/// What an older revision's publish does: a version row and a plain bump.
async fn bump_version(app: &TestApp, id: &str) {
    sqlx::raw_sql(AssertSqlSafe(format!(
        "INSERT INTO parcel_versions (parcel_id, version, entry, file_count, total_bytes)
         SELECT parcel_id, version + 1, entry, file_count, total_bytes FROM parcel_versions
         WHERE parcel_id = '{id}' ORDER BY version DESC LIMIT 1;
         UPDATE parcels SET current_version = current_version + 1 WHERE id = '{id}';"
    )))
    .execute(&app.pool)
    .await
    .unwrap();
}

/// The search trigger rebuilds the vector only when the document it is
/// built from changed (migration 0009): writes that name a searched column
/// without changing it, and writes that name none, leave a sentinel in
/// place; every real change to the title, description, tags or page text
/// replaces it with the right vector.
#[tokio::test]
async fn the_search_vector_is_rebuilt_only_when_its_document_changes() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let p = publish(
        &app,
        &ada.token,
        json!({"title": "Guarded", "tags": ["kind:plan"]}),
        &[("index.html", "<p>page words</p>")],
    )
    .await;
    let id = p.parcel.id.as_str();
    settle(&app).await;
    assert_eq!(search_text(&app, id).await.as_deref(), Some("page words"));

    set_sentinel(&app, id).await;
    patched(&app, &ada, id, json!({"visibility": "internal"})).await;
    assert_kept(&app, id, "a visibility-only PATCH").await;

    // What a new version without metadata asks for: no statement at all
    // (0009 alone would keep the vector, so count UPDATEs on parcels; a
    // statement-level trigger fires even for one that matches no row).
    sqlx::raw_sql(
        "CREATE SEQUENCE parcel_updates;
         CREATE FUNCTION count_update() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN PERFORM nextval('parcel_updates'); RETURN NULL; END $$;
         CREATE TRIGGER count_update BEFORE UPDATE ON parcels
             FOR EACH STATEMENT EXECUTE FUNCTION count_update();",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    parcel_server::db::parcels::update_metadata_tx(&mut tx, id, None, None, None, false)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let updates: bool = sqlx::query_scalar("SELECT is_called FROM parcel_updates")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!updates, "an all-None metadata update runs no UPDATE");
    sqlx::raw_sql("DROP TRIGGER count_update ON parcels")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_kept(&app, id, "an all-None metadata update").await;

    patched(&app, &ada, id, json!({"tags": ["kind:plan"]})).await;
    assert_kept(&app, id, "the same tags again").await;
    sqlx::query(
        "UPDATE parcels SET title = title, description = description, tags = tags,
             search_text = search_text WHERE id = $1",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_kept(&app, id, "every searched column named, none changed").await;

    patched(&app, &ada, id, json!({"title": "Renamed"})).await;
    assert_rebuilt(&app, id, "a title PATCH").await;
    set_sentinel(&app, id).await;
    patched(&app, &ada, id, json!({"description": "described"})).await;
    assert_rebuilt(&app, id, "a description PATCH").await;
    set_sentinel(&app, id).await;
    patched(&app, &ada, id, json!({"tags": ["kind:report"]})).await;
    assert_rebuilt(&app, id, "a tag change").await;
    set_sentinel(&app, id).await;
    sqlx::query("UPDATE parcels SET search_text = 'other words' WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_rebuilt(&app, id, "storing page text").await;

    // A version bump clears the stored text, which changes the document.
    set_sentinel(&app, id).await;
    bump_version(&app, id).await;
    assert_eq!(search_text(&app, id).await, None);
    assert_rebuilt(&app, id, "a version bump over page text").await;
    // With the text already gone, a bump changes nothing searched.
    set_sentinel(&app, id).await;
    bump_version(&app, id).await;
    assert_kept(&app, id, "a version bump with no page text").await;
}

/// A new version and a slug upsert still apply the title, description and
/// visibility they send (the publish path's metadata update is only
/// skipped when it has nothing to apply), and the vector follows.
#[tokio::test]
async fn new_versions_and_upserts_apply_the_metadata_they_send() {
    let app = TestApp::new().await;
    let me = user_token();
    let p = publish(
        &app,
        &me,
        json!({"title": "One", "slug": "notes"}),
        &[("index.html", "<p>first</p>")],
    )
    .await;
    let id = p.parcel.id.as_str();

    let v2 = new_version(
        &app,
        &me,
        id,
        json!({"title": "Second title", "description": "aardvark", "visibility": "internal"}),
        &[("index.html", "<p>second</p>")],
    )
    .await;
    assert_eq!(
        (
            v2.parcel.title.as_str(),
            v2.parcel.description.as_str(),
            v2.parcel.visibility
        ),
        ("Second title", "aardvark", Visibility::Internal)
    );

    let form = Form::new()
        .metadata(json!({
            "slug": "notes", "title": "Third title", "description": "bandicoot",
            "visibility": "private",
        }))
        .file("index.html", "<p>third</p>");
    let v3: PublishResult = expect_json(
        app.post_multipart("/api/v1/parcels", Some(&me), &form)
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(v3.parcel.id, id);
    assert_eq!(
        (
            v3.parcel.title.as_str(),
            v3.parcel.description.as_str(),
            v3.parcel.visibility
        ),
        ("Third title", "bandicoot", Visibility::Private)
    );
    settle(&app).await;
    assert_eq!(find(&app, &me, "bandicoot").await.total, 1);
    assert_eq!(find(&app, &me, "aardvark").await.total, 0);
}

// ---- Relaxed search: when nothing matches every word, any word will do.

/// `GET /parcels?<query>` as `who`.
async fn list_as(app: &TestApp, token: &str, query: &str) -> ParcelList {
    let uri = format!("/api/v1/parcels?{query}");
    expect_json(app.get(&uri, Some(token)).await, StatusCode::OK).await
}

/// `GET /tags?<query>` as `who`.
async fn tags_as(app: &TestApp, token: &str, query: &str) -> TagList {
    let uri = format!("/api/v1/tags?{query}");
    expect_json(app.get(&uri, Some(token)).await, StatusCode::OK).await
}

fn counts(list: &TagList) -> Vec<(&str, i64)> {
    list.tags
        .iter()
        .map(|t| (t.tag.as_str(), t.count))
        .collect()
}

/// Tag counts computed from a list's own items, in `/tags` order.
fn counts_of(list: &ParcelList) -> Vec<(String, i64)> {
    let mut counts = std::collections::BTreeMap::<String, i64>::new();
    for item in &list.parcels {
        for tag in &item.tags {
            *counts.entry(tag.clone()).or_default() += 1;
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

fn owned(counts: Vec<(&str, i64)>) -> Vec<(String, i64)> {
    counts
        .into_iter()
        .map(|(t, n)| (t.to_string(), n))
        .collect()
}

/// The walkthrough phrases of plan 001: each target parcel lacks one of the
/// phrase's words (the reason the AND query found nothing), and distractors
/// share a single word with it.
async fn seed_walkthrough(app: &TestApp, me: &str) {
    let page = |body: &str| format!("<html><body><p>{body}</p></body></html>");
    for (title, tags, body) in [
        (
            "Invoice retry strategy",
            vec!["project:billing", "kind:plan"],
            "What to do when a payment fails: retry the invoice with backoff, then email the customer.",
        ),
        (
            "Rust build settings review",
            vec!["project:tooling", "kind:review"],
            "Switching to the mold linker gave us quicker incremental compiles.",
        ),
        (
            "Query latency dashboard",
            vec!["project:search", "kind:dashboard"],
            "Cutting p95 search latency for the listing page.",
        ),
        // Distractors: one word each.
        ("Payment provider notes", vec!["project:billing"], "Which payment provider we use."),
        ("Error handling guide", vec!["kind:notes"], "Handling errors in services."),
        ("Rust newsletter", vec!["kind:notes"], "Monthly rust news."),
        ("Search tips", vec!["kind:notes"], "Tips for the search box."),
    ] {
        publish(
            app,
            me,
            json!({"title": title, "tags": tags}),
            &[("index.html", &page(body))],
        )
        .await;
    }
    settle(app).await;
}

/// A4: natural-language phrases that match nothing exactly return their
/// obvious parcel first, marked relaxed; a phrase that matches exactly
/// isn't relaxed.
#[tokio::test]
async fn natural_phrases_fall_back_to_any_word_best_first() {
    let app = TestApp::new().await;
    let me = user_token();
    seed_walkthrough(&app, &me).await;

    for (phrase, first) in [
        ("how do we handle failed payments", "Invoice retry strategy"),
        ("faster rust builds", "Rust build settings review"),
        ("reduce latency in search", "Query latency dashboard"),
    ] {
        let found = find(&app, &me, phrase).await;
        assert!(found.relaxed, "{phrase}: {found:?}");
        assert_eq!(titles(&found)[0], first, "{phrase}");
        assert!(found.total >= 2, "{phrase}: the distractors match too");
        assert_eq!(found.total, found.parcels.len() as i64, "{phrase}");
    }
    let exact = find(&app, &me, "rust build").await;
    assert!(!exact.relaxed);
    assert_eq!(titles(&exact), ["Rust build settings review"]);
    // On the wire an exact list has no `relaxed` at all; a relaxed one says so.
    let raw = body_bytes(app.get("/api/v1/parcels?q=rust+build", Some(&me)).await).await;
    let raw: Value = serde_json::from_slice(&raw).unwrap();
    assert!(raw.get("relaxed").is_none(), "{raw}");
    let raw = body_bytes(
        app.get("/api/v1/parcels?q=faster+rust+builds", Some(&me))
            .await,
    )
    .await;
    let raw: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(raw["relaxed"], true, "{raw}");
}

/// Relaxed results rank by how many of the query's words they have, before
/// `ts_rank_cd` (which alone would put a one-word title match first); the
/// snippet marks every matched word; pages are consistent with the total.
#[tokio::test]
async fn relaxed_results_rank_by_matched_words_first() {
    let app = TestApp::new().await;
    let me = user_token();
    let page = |body: &str| format!("<p>{body}</p>");
    // Oldest first, so recency alone would reverse the order.
    for (title, body) in [
        (
            "Three of four",
            "Notes on quartz, then basalt, and finally granite samples.",
        ),
        ("Two of four", "Notes on quartz and basalt samples."),
        ("Marble", "Nothing else from the query here."),
        ("None of them", "Sandstone only."),
    ] {
        publish(
            &app,
            &me,
            json!({"title": title}),
            &[("index.html", &page(body))],
        )
        .await;
    }
    settle(&app).await;

    let query = "quartz basalt granite marble";
    let found = find(&app, &me, query).await;
    assert!(found.relaxed);
    assert_eq!(titles(&found), ["Three of four", "Two of four", "Marble"]);
    assert_eq!(found.total, 3);
    assert_eq!(hits(&found.parcels[0]), ["quartz", "basalt", "granite"]);
    assert_eq!(hits(&found.parcels[1]), ["quartz", "basalt"]);
    assert!(found.parcels[2].matched.is_none(), "a title-only match");

    // Page by page, in the same order, each page saying it is relaxed.
    let q = urlencode(query);
    let mut paged = Vec::new();
    for offset in 0..3 {
        let one = list_as(&app, &me, &format!("q={q}&limit=1&offset={offset}")).await;
        assert!(one.relaxed);
        assert_eq!(one.total, 3);
        paged.push(one.parcels[0].title.clone());
    }
    assert_eq!(paged, titles(&found));
    let past = list_as(&app, &me, &format!("q={q}&limit=1&offset=3")).await;
    assert!(past.relaxed && past.parcels.is_empty());
    assert_eq!(past.total, 3);

    // Repeated and stem-equivalent words count once: "quartzes" is "quartz".
    let found = find(&app, &me, "quartz quartzes granite").await;
    assert!(!found.relaxed, "Three of four has both, exactly");
    let found = find(&app, &me, "granite granites marble").await;
    assert!(found.relaxed);
    assert_eq!(titles(&found), ["Marble", "Three of four"]);
}

/// Only plain multi-word queries relax: an exact hit (full text or
/// substring), an operator, a single (hyphenated or dotted) token, one
/// distinct word, or only stop words keep the exact meaning.
#[tokio::test]
async fn operators_single_tokens_and_exact_hits_never_relax() {
    let app = TestApp::new().await;
    let me = user_token();
    let page = |body: &str| format!("<p>{body}</p>");
    for (title, slug, body) in [
        ("Wombat care", None, "wombat"),
        ("Zone map", None, "zone"),
        ("Plan", Some("ops/zebrafish-migration"), "plan mail"),
        ("Xxkoala bearyy", None, "nothing"),
    ] {
        publish(
            &app,
            &me,
            json!({"title": title, "slug": slug}),
            &[("index.html", &page(body))],
        )
        .await;
    }
    settle(&app).await;

    // Words that appear only in different parcels relax...
    assert!(find(&app, &me, "wombat zone").await.relaxed);
    // ...but not as a substring of a title (the exact clause's ILIKE).
    let substring = find(&app, &me, "xkoala beary").await;
    assert!(!substring.relaxed);
    assert_eq!(titles(&substring), ["Xxkoala bearyy"]);
    // A slug fragment is an exact (substring) hit.
    let slug = find(&app, &me, "zebrafish-migr").await;
    assert!(!slug.relaxed);
    assert_eq!(titles(&slug), ["Plan"]);

    for q in [
        "\"wombat zone\"",
        "wombat -zone",
        "-zone wombat",
        "wombat or zone",
        "wombat OR zone",
        "plan-002",
        "e-mail",
        "example.com",
        "zebra zebras",
        "the and of it",
        "nothing",
    ] {
        let found = find(&app, &me, q).await;
        assert!(!found.relaxed, "{q:?}: {found:?}");
    }
    // The operators still mean what they did.
    assert_eq!(find(&app, &me, "wombat or zone").await.total, 2);
    assert_eq!(
        titles(&find(&app, &me, "wombat -zone").await),
        ["Wombat care"]
    );
    assert_eq!(find(&app, &me, "\"wombat zone\"").await.total, 0);
    assert_eq!(find(&app, &me, "plan-002").await.total, 0);
}

/// Hostile queries (quotes, URLs, tsquery syntax, backslashes, a 500
/// character query) never error, relaxed or not, in lists and tag counts.
#[tokio::test]
async fn hostile_queries_never_error() {
    let app = TestApp::new().await;
    let me = user_token();
    publish(
        &app,
        &me,
        json!({"title": "O'Brien report", "tags": ["people"]}),
        &[(
            "index.html",
            "<p>See http://example.com/it's/x and C:\\foo\\bar.</p>",
        )],
    )
    .await;
    settle(&app).await;

    let mut long = String::new();
    let mut i = 0u32;
    while long.chars().count() < 490 {
        // Distinct words of letters only, so each is its own lexeme.
        let word: String = format!("{i:05}")
            .bytes()
            .map(|d| char::from(b'a' + (d - b'0')))
            .collect();
        long.push_str(&word);
        long.push(' ');
        i += 1;
    }
    let long = long.trim_end().to_string();
    assert!(long.chars().count() <= 500);
    let inputs = [
        "O'Brien report",
        "O'Brien quarterly",
        "http://example.com/it's/x docs",
        "foo:bar baz*qux",
        "C:\\foo\\bar baz",
        "a|b & !c <-> d",
        "foo:* bar:A",
        "'' \\' \\\\",
        "() ) (",
        "例え テスト",
        long.as_str(),
    ];
    for q in inputs {
        let enc = urlencode(q);
        let list = list_as(&app, &me, &format!("q={enc}")).await;
        for mode in ["auto", "exact", "relaxed"] {
            tags_as(&app, &me, &format!("q={enc}&mode={mode}")).await;
        }
        if q.starts_with("O'Brien") {
            assert_eq!(titles(&list), ["O'Brien report"], "{q}");
        }
    }
    assert!(!find(&app, &me, "O'Brien report").await.relaxed);
    assert!(find(&app, &me, "O'Brien quarterly").await.relaxed);
    assert!(
        find(&app, &me, "http://example.com/it's/x docs")
            .await
            .relaxed
    );
    assert_eq!(find(&app, &me, &long).await.total, 0);

    // PostgreSQL refuses NUL in text: a 400 here, never a 500.
    for uri in [
        "/api/v1/parcels?q=quartz%00%20basalt",
        "/api/v1/tags?q=quartz%00%20basalt",
        "/api/v1/tags?prefix=a%00b",
    ] {
        let res = app.get(uri, Some(&me)).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}

/// `/tags` counts what the list shows: `auto` decides as `/parcels` does
/// (ignoring `prefix`), an explicit mode is followed, and a facet's swap
/// request with the list's mode counts over the relaxed list.
#[tokio::test]
async fn tag_counts_follow_the_lists_mode() {
    let app = TestApp::new().await;
    let me = user_token();
    seed_walkthrough(&app, &me).await;
    let phrase = urlencode("faster rust builds");

    let list = list_as(&app, &me, &format!("q={phrase}")).await;
    assert!(list.relaxed);
    for query in [
        format!("q={phrase}"),
        format!("q={phrase}&mode=auto"),
        format!("q={phrase}&mode=relaxed"),
    ] {
        let tags = tags_as(&app, &me, &query).await;
        assert!(tags.relaxed, "{query}");
        assert_eq!(owned(counts(&tags)), counts_of(&list), "{query}");
    }
    // `prefix` narrows the counts, never the decision.
    let kinds = tags_as(&app, &me, &format!("q={phrase}&prefix=kind:")).await;
    assert!(kinds.relaxed);
    assert_eq!(counts(&kinds), [("kind:notes", 1), ("kind:review", 1)]);
    // Exact: what every word matches (nothing), and no flag.
    let exact = tags_as(&app, &me, &format!("q={phrase}&mode=exact")).await;
    assert!(!exact.relaxed && exact.tags.is_empty());
    // An exact list's counts are exact under auto; relaxed counts are wider.
    let exact_list = find(&app, &me, "rust build").await;
    let auto = tags_as(&app, &me, "q=rust+build").await;
    assert!(!auto.relaxed);
    assert_eq!(owned(counts(&auto)), counts_of(&exact_list));
    assert!(
        tags_as(&app, &me, "q=rust+build&mode=relaxed")
            .await
            .relaxed
    );
    // No query: nothing to relax, whatever is asked.
    assert!(!tags_as(&app, &me, "mode=relaxed").await.relaxed);

    // A swap request (the dashboard's facet for a selected project) drops
    // that project and keeps the list's mode. Within project:a nothing has
    // both words, so the list is relaxed; across projects one parcel has
    // both, so `auto` would decide exact and count only that one.
    let page = |body: &str| format!("<p>{body}</p>");
    for (project, body) in [("a", "kestrel"), ("a", "falcon"), ("b", "kestrel falcon")] {
        publish(
            &app,
            &me,
            json!({"title": format!("Birds {project}"), "tags": [format!("project:{project}"), "topic:birds"]}),
            &[("index.html", &page(body))],
        )
        .await;
    }
    settle(&app).await;
    let birds = urlencode("kestrel falcon");
    let selected = list_as(
        &app,
        &me,
        &format!("q={birds}&tag=topic:birds&tag=project:a"),
    )
    .await;
    assert!(selected.relaxed);
    assert_eq!(selected.total, 2);
    let swap = tags_as(
        &app,
        &me,
        &format!("q={birds}&tag=topic:birds&prefix=project:&mode=relaxed"),
    )
    .await;
    assert!(swap.relaxed);
    assert_eq!(counts(&swap), [("project:a", 2), ("project:b", 1)]);
    let unswapped = list_as(
        &app,
        &me,
        &format!("q={birds}&tag=topic:birds&tag=project:a"),
    )
    .await;
    assert_eq!(
        unswapped.total,
        counts(&swap)[0].1,
        "the selected facet's count is the list's"
    );
    let auto = tags_as(
        &app,
        &me,
        &format!("q={birds}&tag=topic:birds&prefix=project:"),
    )
    .await;
    assert!(!auto.relaxed);
    assert_eq!(counts(&auto), [("project:b", 1)]);
}

/// A4 / plan 001's sentinel rule, relaxed: someone else's private parcel
/// holding every word of the query (and a unique word) never shows up in
/// relaxed results, totals, snippets or facets, and doesn't even make the
/// query exact for anyone else (the probe is scoped). Scopes and the
/// archive filter hold.
#[tokio::test]
async fn relaxed_search_keeps_scopes_and_leaks_nothing() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let bob = person("bob");
    let sentinel = "xylophonesentinel";
    let page = |body: &str| format!("<p>{body}</p>");
    // Ada's private parcel matches the whole query exactly.
    let private = publish(
        &app,
        &ada.token,
        json!({"title": format!("Birds {sentinel}"), "tags": [sentinel]}),
        &[(
            "index.html",
            &page(&format!("kestrel falcon osprey {sentinel}")),
        )],
    )
    .await;
    // Shared with Bob: one word.
    let shared = publish(
        &app,
        &ada.token,
        json!({"title": "Falcon notes", "tags": ["birds"]}),
        &[("index.html", &page("falcon"))],
    )
    .await;
    expect_status(
        app.put_json(
            &format!("/api/v1/parcels/{}/shares/{}", shared.parcel.id, bob.email),
            Some(&ada.token),
            &json!({"role": "viewer"}),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    // Bob's own: one word, and an archived one with another.
    let own = publish(
        &app,
        &bob.token,
        json!({"title": "Kestrel watch", "tags": ["birds"]}),
        &[("index.html", &page("kestrel"))],
    )
    .await;
    let archived = publish(
        &app,
        &bob.token,
        json!({"title": "Osprey log", "tags": ["old"]}),
        &[("index.html", &page("osprey"))],
    )
    .await;
    patched(&app, &bob, &archived.parcel.id, json!({"archived": true})).await;
    settle(&app).await;

    let ids = |l: &ParcelList| -> Vec<String> {
        let mut ids: Vec<String> = l.parcels.iter().map(|p| p.id.clone()).collect();
        ids.sort();
        ids
    };
    let sorted = |mut v: Vec<&str>| -> Vec<String> {
        v.sort_unstable();
        v.into_iter().map(str::to_string).collect()
    };
    let (own_id, shared_id, archived_id) = (
        own.parcel.id.as_str(),
        shared.parcel.id.as_str(),
        archived.parcel.id.as_str(),
    );
    for words in [
        "kestrel falcon osprey",
        &format!("{sentinel} kestrel falcon"),
    ] {
        let q = urlencode(words);
        for (scope, archived, expect) in [
            ("mine", "exclude", vec![own_id]),
            ("mine", "include", vec![own_id, archived_id]),
            ("mine", "only", vec![archived_id]),
            ("shared", "exclude", vec![shared_id]),
            ("all", "exclude", vec![own_id, shared_id]),
            ("all", "include", vec![own_id, shared_id, archived_id]),
        ] {
            let filters = format!("q={q}&scope={scope}&archived={archived}");
            let list = list_as(&app, &bob.token, &filters).await;
            let mut expect = expect;
            if words.starts_with(sentinel) {
                // Bob's archived parcel has no word of this query.
                expect.retain(|id| *id != archived_id);
                if expect.is_empty() {
                    assert_eq!(list.total, 0, "{filters}");
                    continue;
                }
            }
            assert!(list.relaxed, "{filters}: Ada's exact match is out of scope");
            assert_eq!(ids(&list), sorted(expect.clone()), "{filters}");
            assert_eq!(list.total, expect.len() as i64, "{filters}");
            assert!(!ids(&list).contains(&private.parcel.id), "{filters}");
            let raw = serde_json::to_string(&list).unwrap();
            assert!(!raw.contains(sentinel), "{filters}: {raw}");
            for mode in ["auto", "relaxed"] {
                let tags = tags_as(&app, &bob.token, &format!("{filters}&mode={mode}")).await;
                assert!(tags.relaxed, "{filters} {mode}");
                assert_eq!(owned(counts(&tags)), counts_of(&list), "{filters} {mode}");
                let raw = serde_json::to_string(&tags).unwrap();
                assert!(!raw.contains(sentinel), "{filters} {mode}: {raw}");
            }
        }
    }
    // Ada, whose parcel it is, gets it as an exact match.
    let mine = find(&app, &ada.token, "kestrel falcon osprey").await;
    assert!(!mine.relaxed);
    assert_eq!(titles(&mine), [format!("Birds {sentinel}").as_str()]);
}
