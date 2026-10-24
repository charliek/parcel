//! Full-text search over the real router and Postgres: what is indexed,
//! how results rank, snippets, re-indexing on new versions and restores,
//! the guarded write, the backfill, and value-free failure logs.

mod common;

use axum::http::StatusCode;
use common::*;
use parcel_server::db::parcels::HEADLINE_OPTIONS;
use parcel_server::search::{self, MAX_ATTEMPTS};
use parcel_server::storage::{blob_key, BlobStore};
use parcel_types::{ParcelList, ParcelListItem, PublishResult};
use serde_json::{json, Value};

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
