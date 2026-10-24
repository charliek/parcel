//! Tags, archive and the unified list: what is stored, who sees what, and
//! that organizing never reorders anything.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use parcel_server::db::parcels::{self, Patch, TagsChange};
use parcel_server::publish::{self, VersionSource};
use parcel_types::{
    ErrorBody, NewVersionMetadata, ParcelDetail, ParcelList, ParcelSummary, PublishResult,
    ServerInfo, ShareRole, SharedParcelList, TagList, TagsUpdated,
};
use serde_json::{json, Value};

async fn publish_with(app: &TestApp, who: &Person, metadata: Value, page: &str) -> ParcelSummary {
    let form = Form::new()
        .metadata(metadata)
        .file("index.html", page.to_string());
    let r = app
        .post_multipart("/api/v1/parcels", Some(&who.token), &form)
        .await;
    let published: PublishResult = expect_json(r, StatusCode::CREATED).await;
    published.parcel
}

async fn tagged(app: &TestApp, who: &Person, title: &str, tags: &[&str]) -> ParcelSummary {
    publish_with(
        app,
        who,
        json!({"title": title, "tags": tags}),
        "<p>page</p>",
    )
    .await
}

async fn list(app: &TestApp, who: &Person, query: &str) -> ParcelList {
    let uri = format!("/api/v1/parcels?{query}");
    expect_json(app.get(&uri, Some(&who.token)).await, StatusCode::OK).await
}

async fn tag_list(app: &TestApp, who: &Person, query: &str) -> TagList {
    let uri = format!("/api/v1/tags?{query}");
    expect_json(app.get(&uri, Some(&who.token)).await, StatusCode::OK).await
}

async fn patch(app: &TestApp, who: &Person, id: &str, body: Value) -> axum::response::Response {
    app.patch_json(&format!("/api/v1/parcels/{id}"), Some(&who.token), &body)
        .await
}

async fn patched(app: &TestApp, who: &Person, id: &str, body: Value) -> ParcelSummary {
    expect_json(patch(app, who, id, body).await, StatusCode::OK).await
}

async fn bad_request(response: axum::response::Response) -> String {
    let body: ErrorBody = expect_json(response, StatusCode::BAD_REQUEST).await;
    body.error.message
}

async fn detail(app: &TestApp, who: &Person, id: &str) -> ParcelDetail {
    let uri = format!("/api/v1/parcels/{id}");
    expect_json(app.get(&uri, Some(&who.token)).await, StatusCode::OK).await
}

/// `POST /tags/{op}` (`rename` or `delete`): how many parcels it changed.
async fn tags_op(app: &TestApp, who: &Person, op: &str, body: Value) -> i64 {
    let uri = format!("/api/v1/tags/{op}");
    let r = app.post_json(&uri, Some(&who.token), &body).await;
    expect_json::<TagsUpdated>(r, StatusCode::OK).await.updated
}

fn ids(list: &ParcelList) -> Vec<&str> {
    list.parcels.iter().map(|p| p.id.as_str()).collect()
}

fn counts(list: &TagList) -> Vec<(&str, i64)> {
    list.tags
        .iter()
        .map(|t| (t.tag.as_str(), t.count))
        .collect()
}

#[tokio::test]
async fn the_server_says_it_has_these_features() {
    let app = TestApp::new().await;
    let info: ServerInfo = json(app.get("/api/v1/info", None).await).await;
    for feature in ["tags", "search", "archive", "slugs", "pages"] {
        assert!(info.features.iter().any(|f| f == feature), "{info:?}");
    }
}

#[tokio::test]
async fn publishing_stores_tags_normalized() {
    let app = TestApp::new().await;
    let me = person("ada");
    let p = publish_with(
        &app,
        &me,
        json!({"tags": ["Plan", " project:Parcel ", "plan", "rust build"], "auto_tags": ["project:parcel", "kind:Plan"]}),
        "<p>x</p>",
    )
    .await;
    assert_eq!(
        p.tags,
        ["kind:plan", "plan", "project:parcel", "rust-build"]
    );
    assert_eq!(detail(&app, &me, &p.id).await.parcel.tags, p.tags);

    let refused = |metadata: Value| {
        let form = Form::new()
            .metadata(metadata)
            .file("index.html", "<p>x</p>");
        let app = &app;
        let token = me.token.clone();
        async move {
            bad_request(
                app.post_multipart("/api/v1/parcels", Some(&token), &form)
                    .await,
            )
            .await
        }
    };
    let message = refused(json!({"tags": ["fine", "not fine!"]})).await;
    assert!(message.contains(r#""not fine!""#), "{message}");
    let many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    let message = refused(json!({"tags": many})).await;
    assert!(message.contains("at most 20"), "{message}");
    // The cap is on the result: tags and auto tags together.
    let (half, rest) = many.split_at(11);
    assert!(refused(json!({"tags": half, "auto_tags": rest}))
        .await
        .contains("at most 20"));
    // Normalized before counting: 21 spellings of 20 tags are fine.
    let mut spellings: Vec<String> = many[..20].to_vec();
    spellings.push("T0".into());
    let ok = publish_with(&app, &me, json!({"tags": spellings}), "<p>x</p>").await;
    assert_eq!(ok.tags.len(), 20);
    assert_eq!(
        list(&app, &me, "").await.total,
        2,
        "refused publishes stored nothing"
    );
}

#[tokio::test]
async fn tag_filters_match_all_and_counts_follow_the_filters() {
    let app = TestApp::new().await;
    let me = person("ada");
    let other = person("bob");
    let ab = tagged(&app, &me, "ab", &["a", "b", "project:x"]).await;
    let a = tagged(&app, &me, "a", &["a", "project:y"]).await;
    let abc = tagged(&app, &me, "abc", &["a", "b", "c", "project:x"]).await;
    tagged(&app, &me, "c", &["c"]).await;
    tagged(&app, &other, "theirs", &["a", "b"]).await;

    let both = list(&app, &me, "tag=a&tag=b").await;
    assert_eq!(both.total, 2);
    assert_eq!(ids(&both), [abc.id.as_str(), ab.id.as_str()]);
    assert_eq!(
        list(&app, &me, "tag=A").await.total,
        3,
        "filters are normalized"
    );
    assert_eq!(
        list(&app, &me, "tag=a&tag=project:y").await.parcels[0].id,
        a.id
    );
    assert_eq!(list(&app, &me, "tag=nothing").await.total, 0);
    assert!(
        bad_request(app.get("/api/v1/parcels?tag=bad!", Some(&me.token)).await)
            .await
            .contains(r#""bad!""#)
    );

    // Facet counts are counts over the parcels the same filters select.
    let all = tag_list(&app, &me, "").await;
    assert_eq!(
        counts(&all),
        [
            ("a", 3),
            ("b", 2),
            ("c", 2),
            ("project:x", 2),
            ("project:y", 1)
        ]
    );
    assert_eq!(all.total, 5);
    let within_a = tag_list(&app, &me, "tag=a").await;
    for t in &within_a.tags {
        let matching = list(&app, &me, &format!("tag=a&tag={}", t.tag)).await;
        assert_eq!(t.count, matching.total, "{}", t.tag);
    }
    assert_eq!(
        counts(&tag_list(&app, &me, "prefix=Project:").await),
        [("project:x", 2), ("project:y", 1)]
    );
    // Paging, with the distinct total for knowing when to stop.
    let page = tag_list(&app, &me, "limit=2&offset=2").await;
    assert_eq!(
        (counts(&page), page.total),
        (vec![("c", 2), ("project:x", 2)], 5)
    );
    assert!(tag_list(&app, &me, "limit=5000").await.tags.len() <= 1000);
}

#[tokio::test]
async fn auto_tags_apply_only_when_a_parcel_is_created() {
    let app = TestApp::new().await;
    let me = person("ada");
    let p = publish_with(
        &app,
        &me,
        json!({"auto_tags": ["project:parcel"]}),
        "<p>1</p>",
    )
    .await;
    assert_eq!(p.tags, ["project:parcel"]);
    patched(&app, &me, &p.id, json!({"remove_tags": ["project:parcel"]})).await;

    // A new version can add tags, never auto ones (it has no such field).
    let form = Form::new()
        .metadata(json!({"add_tags": ["kind:plan"]}))
        .file("index.html", "<p>2</p>");
    let v2: PublishResult = expect_json(
        app.post_multipart(
            &format!("/api/v1/parcels/{}/versions", p.id),
            Some(&me.token),
            &form,
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(v2.parcel.tags, ["kind:plan"]);
    let form = Form::new()
        .metadata(json!({"auto_tags": ["project:parcel"]}))
        .file("index.html", "<p>3</p>");
    let refused = app
        .post_multipart(
            &format!("/api/v1/parcels/{}/versions", p.id),
            Some(&me.token),
            &form,
        )
        .await;
    bad_request(refused).await;
    assert_eq!(detail(&app, &me, &p.id).await.parcel.tags, ["kind:plan"]);
}

#[tokio::test]
async fn patch_replaces_or_edits_the_tag_set() {
    let app = TestApp::new().await;
    let me = person("ada");
    let p = tagged(&app, &me, "p", &["a", "b"]).await;
    let id = p.id.as_str();

    assert_eq!(
        patched(&app, &me, id, json!({"tags": ["Z", "y"]}))
            .await
            .tags,
        ["y", "z"]
    );
    assert_eq!(
        patched(
            &app,
            &me,
            id,
            json!({"add_tags": ["x", "y"], "remove_tags": ["z"]})
        )
        .await
        .tags,
        ["x", "y"]
    );
    assert_eq!(
        patched(&app, &me, id, json!({"tags": []})).await.tags,
        Vec::<String>::new()
    );

    let m =
        bad_request(patch(&app, &me, id, json!({"tags": ["a"], "add_tags": ["b"]})).await).await;
    assert!(m.contains("not both"), "{m}");
    bad_request(patch(&app, &me, id, json!({"tags": ["a"], "remove_tags": ["b"]})).await).await;
    let m = bad_request(
        patch(
            &app,
            &me,
            id,
            json!({"add_tags": ["Same"], "remove_tags": ["same"]}),
        )
        .await,
    )
    .await;
    assert!(m.contains(r#""same""#), "{m}");
    bad_request(
        patch(
            &app,
            &me,
            id,
            json!({"add_tags": ["no good"], "remove_tags": ["!"]}),
        )
        .await,
    )
    .await;
    let twenty: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
    patched(&app, &me, id, json!({"tags": twenty})).await;
    let m = bad_request(patch(&app, &me, id, json!({"add_tags": ["one-more"]})).await).await;
    assert!(m.contains("at most 20"), "{m}");
    // Removing in the same edit makes room.
    let swapped = patched(
        &app,
        &me,
        id,
        json!({"add_tags": ["one-more"], "remove_tags": ["t0"]}),
    )
    .await;
    assert!(swapped.tags.contains(&"one-more".to_string()) && swapped.tags.len() == 20);

    // Only the owner may change tags; a stranger can't even see the parcel.
    let stranger = person("eve");
    expect_status(
        patch(&app, &stranger, id, json!({"tags": []})).await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn rename_merges_and_delete_touches_only_the_callers_parcels() {
    let app = TestApp::new().await;
    let me = person("ada");
    let other = person("bob");
    let p1 = tagged(&app, &me, "p1", &["a", "b"]).await;
    let p2 = tagged(&app, &me, "p2", &["a"]).await;
    let p3 = tagged(&app, &me, "p3", &["b", "c", "topic:x/y"]).await;
    let theirs = tagged(&app, &other, "theirs", &["a", "topic:x/y"]).await;
    let order = ids(&list(&app, &me, "").await).join(",");
    let stamps = |l: &ParcelList| {
        l.parcels
            .iter()
            .map(|p| (p.id.clone(), p.updated_at))
            .collect::<Vec<_>>()
    };
    let before = stamps(&list(&app, &me, "").await);

    let renamed = tags_op(&app, &me, "rename", json!({"from": "A", "to": "b"})).await;
    assert_eq!(renamed, 2);
    assert_eq!(
        detail(&app, &me, &p1.id).await.parcel.tags,
        ["b"],
        "merged without a duplicate"
    );
    assert_eq!(detail(&app, &me, &p2.id).await.parcel.tags, ["b"]);
    assert_eq!(
        detail(&app, &other, &theirs.id).await.parcel.tags,
        ["a", "topic:x/y"]
    );

    // A tag with a slash goes in the body.
    let deleted = tags_op(&app, &me, "delete", json!({"tag": "topic:x/y"})).await;
    assert_eq!(deleted, 1);
    assert_eq!(detail(&app, &me, &p3.id).await.parcel.tags, ["b", "c"]);
    assert_eq!(
        detail(&app, &other, &theirs.id).await.parcel.tags,
        ["a", "topic:x/y"]
    );
    let deleted = tags_op(&app, &me, "delete", json!({"tag": "b"})).await;
    assert_eq!(deleted, 3);
    assert!(detail(&app, &me, &p1.id).await.parcel.tags.is_empty());

    // Organizing reorders nothing and touches no updated_at.
    let after = list(&app, &me, "").await;
    assert_eq!(ids(&after).join(","), order);
    assert_eq!(stamps(&after), before);

    let none = tags_op(&app, &me, "rename", json!({"from": "c", "to": "c"})).await;
    assert_eq!(none, 0);
    bad_request(
        app.post_json(
            "/api/v1/tags/rename",
            Some(&me.token),
            &json!({"from": "c", "to": "no good!"}),
        )
        .await,
    )
    .await;
    bad_request(
        app.post_json("/api/v1/tags/delete", Some(&me.token), &json!({"tag": ""}))
            .await,
    )
    .await;
    expect_status(
        app.post_json("/api/v1/tags/delete", None, &json!({"tag": "c"}))
            .await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

/// Rust (publish, PATCH) and SQL (rename, delete) write the same arrays:
/// sorted by bytes, not by the database's collation.
#[tokio::test]
async fn sql_and_rust_store_tags_in_the_same_order() {
    let app = TestApp::new().await;
    let me = person("ada");
    let raw = ["b", "a-b", "a.b", "a+b", "a:b", "a0", "ab", "zz", "Z9"];
    let p = tagged(&app, &me, "p", &raw).await;
    let mut rust_sorted: Vec<String> = parcel_types::tags::normalize_tags(&raw).unwrap();
    assert_eq!(p.tags, rust_sorted);

    tags_op(&app, &me, "rename", json!({"from": "zz", "to": "a"})).await;
    rust_sorted.retain(|t| t != "zz");
    rust_sorted.push("a".into());
    let expected = parcel_types::tags::normalize_tags(&rust_sorted).unwrap();
    assert_eq!(detail(&app, &me, &p.id).await.parcel.tags, expected);
    tags_op(&app, &me, "delete", json!({"tag": "a.b"})).await;
    let expected: Vec<String> = expected.into_iter().filter(|t| t != "a.b").collect();
    assert_eq!(detail(&app, &me, &p.id).await.parcel.tags, expected);
}

#[tokio::test]
async fn only_edits_bump_updated_at() {
    let app = TestApp::new().await;
    let me = person("ada");
    let older = tagged(&app, &me, "older", &[]).await;
    let newer = tagged(&app, &me, "newer", &[]).await;
    let stamp = older.updated_at;
    for body in [
        json!({"tags": ["x"]}),
        json!({"add_tags": ["y"]}),
        json!({"archived": true}),
        json!({"archived": false}),
        json!({}),
    ] {
        let p = patched(&app, &me, &older.id, body.clone()).await;
        assert_eq!(p.updated_at, stamp, "{body}");
    }
    assert_eq!(
        ids(&list(&app, &me, "").await),
        [newer.id.as_str(), older.id.as_str()]
    );
    let edited = patched(
        &app,
        &me,
        &older.id,
        json!({"title": "Edited", "add_tags": ["z"]}),
    )
    .await;
    assert!(edited.updated_at > stamp);
    assert_eq!(edited.tags, ["x", "y", "z"]);
    assert_eq!(ids(&list(&app, &me, "").await)[0], older.id);
}

#[tokio::test]
async fn archived_parcels_are_left_out_unless_asked_for() {
    let app = TestApp::new().await;
    let me = person("ada");
    let live = publish_with(
        &app,
        &me,
        json!({"title": "Live wombat", "tags": ["t"]}),
        "<p>x</p>",
    )
    .await;
    let old = publish_with(
        &app,
        &me,
        json!({"title": "Old wombat", "tags": ["t", "old"]}),
        "<p>x</p>",
    )
    .await;

    let archived = patched(&app, &me, &old.id, json!({"archived": true})).await;
    let at = archived.archived_at.expect("archived");
    assert_eq!(ids(&list(&app, &me, "").await), [live.id.as_str()]);
    assert_eq!(list(&app, &me, "").await.total, 1);
    assert_eq!(list(&app, &me, "archived=include").await.total, 2);
    assert_eq!(
        ids(&list(&app, &me, "archived=only").await),
        [old.id.as_str()]
    );
    assert_eq!(list(&app, &me, "q=wombat").await.total, 1);
    assert_eq!(list(&app, &me, "q=wombat&archived=include").await.total, 2);
    assert_eq!(counts(&tag_list(&app, &me, "").await), [("t", 1)]);
    assert_eq!(
        counts(&tag_list(&app, &me, "archived=only").await),
        [("old", 1), ("t", 1)]
    );
    expect_status(
        app.get("/api/v1/parcels?archived=maybe", Some(&me.token))
            .await,
        StatusCode::BAD_REQUEST,
    )
    .await;

    // Still there by URL, for its owner and anyone it's shared with.
    let d = detail(&app, &me, &old.id).await;
    assert_eq!(d.parcel.archived_at, Some(at));
    expect_status(
        app.get(&format!("/api/v1/parcels/{}/view", old.id), Some(&me.token))
            .await,
        StatusCode::OK,
    )
    .await;
    // Archiving again keeps the first time; unarchiving brings it back.
    assert_eq!(
        patched(&app, &me, &old.id, json!({"archived": true}))
            .await
            .archived_at,
        Some(at)
    );
    assert_eq!(
        patched(&app, &me, &old.id, json!({"archived": false}))
            .await
            .archived_at,
        None
    );
    assert_eq!(list(&app, &me, "").await.total, 2);
}

#[tokio::test]
async fn a_new_version_unarchives() {
    let app = TestApp::new().await;
    let me = person("ada");
    let p = tagged(&app, &me, "p", &[]).await;
    patched(&app, &me, &p.id, json!({"archived": true})).await;
    let form = Form::new().file("index.html", "<p>v2</p>");
    let v2: PublishResult = expect_json(
        app.post_multipart(
            &format!("/api/v1/parcels/{}/versions", p.id),
            Some(&me.token),
            &form,
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(v2.parcel.archived_at, None);

    patched(&app, &me, &p.id, json!({"archived": true})).await;
    let restored: PublishResult = expect_json(
        app.post_json(
            &format!("/api/v1/parcels/{}/versions/1/restore", p.id),
            Some(&me.token),
            &json!({}),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(restored.parcel.archived_at, None);
    assert_eq!(list(&app, &me, "").await.total, 1);
}

#[tokio::test]
async fn shared_and_all_scopes_honour_every_filter() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let bob = person("bob");
    let shared_viewer = tagged(&app, &ada, "Shared kestrel", &["x"]).await;
    let shared_archived = tagged(&app, &ada, "Archived kestrel", &["y"]).await;
    let internal = publish_with(
        &app,
        &ada,
        json!({"title": "Internal kestrel", "visibility": "internal", "tags": ["x"]}),
        "<p>x</p>",
    )
    .await;
    let public = publish_with(
        &app,
        &ada,
        json!({"title": "Public kestrel", "visibility": "public", "tags": ["x"]}),
        "<p>x</p>",
    )
    .await;
    let bobs = tagged(&app, &bob, "Bob's kestrel", &["x"]).await;
    for (id, role) in [
        (&shared_viewer.id, "viewer"),
        (&shared_archived.id, "commenter"),
    ] {
        expect_status(
            app.put_json(
                &format!("/api/v1/parcels/{id}/shares/{}", bob.email.to_uppercase()),
                Some(&ada.token),
                &json!({"role": role}),
            )
            .await,
            StatusCode::OK,
        )
        .await;
    }
    patched(&app, &ada, &shared_archived.id, json!({"archived": true})).await;

    let shared = list(&app, &bob, "scope=shared").await;
    assert_eq!(
        (ids(&shared), shared.total),
        (vec![shared_viewer.id.as_str()], 1)
    );
    let info = shared.parcels[0].shared.as_ref().expect("shared info");
    assert_eq!(info.role, ShareRole::Viewer);
    assert_eq!(info.owner_name, "Test User");
    let with_archived = list(&app, &bob, "scope=shared&archived=include").await;
    assert_eq!(with_archived.total, 2);
    assert_eq!(
        list(&app, &bob, "scope=shared&tag=y&archived=include")
            .await
            .total,
        1
    );

    let all = list(&app, &bob, "scope=all").await;
    assert_eq!(all.total, 2);
    let mut got = ids(&all);
    got.sort_unstable();
    let mut want = vec![shared_viewer.id.as_str(), bobs.id.as_str()];
    want.sort_unstable();
    assert_eq!(got, want);
    let own = all.parcels.iter().find(|p| p.id == bobs.id).unwrap();
    assert!(own.shared.is_none(), "his own parcel isn't 'shared'");
    assert_eq!(list(&app, &bob, "scope=all&tag=x").await.total, 2);
    assert_eq!(
        ids(&list(&app, &bob, "scope=all&q=shared").await),
        [shared_viewer.id.as_str()]
    );
    assert_eq!(list(&app, &bob, "scope=all&archived=only").await.total, 1);
    assert_eq!(counts(&tag_list(&app, &bob, "scope=all").await), [("x", 2)]);
    assert_eq!(
        counts(&tag_list(&app, &bob, "scope=shared&archived=include").await),
        [("x", 1), ("y", 1)]
    );
    // Internal and public parcels nobody shared with him are never listed.
    for scope in ["mine", "shared", "all"] {
        let l = list(
            &app,
            &bob,
            &format!("scope={scope}&archived=include&q=kestrel"),
        )
        .await;
        assert!(!ids(&l).contains(&internal.id.as_str()) && !ids(&l).contains(&public.id.as_str()));
    }
    // His own list is still just his.
    assert_eq!(ids(&list(&app, &bob, "").await), [bobs.id.as_str()]);

    // Viewers see tags in the detail.
    assert_eq!(
        detail(&app, &bob, &shared_viewer.id).await.parcel.tags,
        ["x"]
    );

    // The legacy endpoint is unchanged: archived parcels included.
    let legacy: SharedParcelList = json(app.get("/api/v1/shared", Some(&bob.token)).await).await;
    assert_eq!(legacy.total, 2);
}

/// A private parcel's title, page text, tags, slug and source reach no one
/// else: not in any list, count, snippet, tag vocabulary or by-slug lookup;
/// and a shared parcel's source stays its owner's.
#[tokio::test]
async fn a_private_parcel_leaks_nowhere() {
    let app = TestApp::new().await;
    let owner = person("ada");
    let bob = person("bob");
    let sentinel = "xylophonesentinel";
    let source = json!({"repo": format!("https://github.com/ada/{sentinel}"), "branch": sentinel});
    let private = publish_with(
        &app,
        &owner,
        json!({"title": format!("Title {sentinel}"), "tags": [sentinel, format!("{sentinel}:ns")],
               "slug": format!("plans/{sentinel}"), "source": source}),
        &format!("<p>Body {sentinel} words</p>"),
    )
    .await;
    // Bob has things of his own, and one parcel of the owner's shared with him.
    tagged(&app, &bob, "bob's", &["mine"]).await;
    let shared = publish_with(
        &app,
        &owner,
        json!({"title": "shared one", "tags": ["visible"], "source": source}),
        "<p>page</p>",
    )
    .await;
    expect_status(
        app.put_json(
            &format!("/api/v1/parcels/{}/shares/{}", shared.id, bob.email),
            Some(&owner.token),
            &json!({"role": "viewer"}),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    parcel_server::search::drain(&app.state, None, 5)
        .await
        .unwrap();
    // The owner finds it, by every route.
    assert_eq!(list(&app, &owner, &format!("q={sentinel}")).await.total, 1);
    assert!(
        list(&app, &owner, &format!("q={sentinel}")).await.parcels[0]
            .matched
            .is_some()
    );

    for scope in ["mine", "shared", "all"] {
        for query in [
            format!("q={sentinel}"),
            format!("tag={sentinel}"),
            format!("q={sentinel}&archived=include"),
            "archived=include".to_string(),
        ] {
            let l = list(&app, &bob, &format!("scope={scope}&{query}")).await;
            assert!(!ids(&l).contains(&private.id.as_str()), "{scope} {query}");
            let raw = serde_json::to_string(&l).unwrap();
            assert!(!raw.contains(sentinel), "{scope} {query}: {raw}");
        }
        for query in ["", "archived=include", "prefix=xylo"] {
            let t = tag_list(&app, &bob, &format!("scope={scope}&{query}")).await;
            let raw = serde_json::to_string(&t).unwrap();
            assert!(!raw.contains(sentinel), "{scope} {query}: {raw}");
        }
    }
    assert_eq!(
        list(&app, &bob, &format!("scope=all&q={sentinel}"))
            .await
            .total,
        0
    );
    // The owner finds it by its slug; Bob gets the one constant miss.
    let handle: parcel_types::Me = expect_json(
        app.get("/api/v1/me", Some(&owner.token)).await,
        StatusCode::OK,
    )
    .await;
    let by_slug = format!(
        "/api/v1/parcels/by-slug?handle={}&slug=plans/{sentinel}",
        handle.handle
    );
    expect_status(app.get(&by_slug, Some(&owner.token)).await, StatusCode::OK).await;
    let miss = expect_status(
        app.get(&by_slug, Some(&bob.token)).await,
        StatusCode::NOT_FOUND,
    )
    .await;
    let raw = String::from_utf8(body_bytes(miss).await).unwrap();
    assert!(!raw.contains(sentinel), "{raw}");
    // The shared parcel's source is the owner's alone.
    for path in [
        format!("/api/v1/parcels/{}", shared.id),
        format!("/api/v1/parcels/{}/versions", shared.id),
        format!("/api/v1/parcels/{}/versions/1", shared.id),
        format!("/api/v1/parcels/{}/view", shared.id),
    ] {
        let raw =
            String::from_utf8(body_bytes(app.get(&path, Some(&bob.token)).await).await).unwrap();
        assert!(!raw.contains(sentinel), "{path}: {raw}");
    }
    expect_status(
        app.get(&format!("/api/v1/parcels/{}", private.id), Some(&bob.token))
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

/// The lock order (owner, then row) holds across every writer here, so
/// they finish under load instead of deadlocking, and concurrent tag edits
/// never lose each other's tags.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_neither_deadlock_nor_lose_tags() {
    let app = Arc::new(TestApp::new().await);
    let me = person("ada");
    let first = tagged(&app, &me, "first", &["shared-tag"]).await;
    let second = tagged(&app, &me, "second", &["shared-tag"]).await;
    let owner_id: uuid::Uuid = sqlx::query_scalar("SELECT owner_id FROM parcels WHERE id = $1")
        .bind(&first.id)
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..8 {
        let app = app.clone();
        let (a, b) = (first.id.clone(), second.id.clone());
        tasks.spawn(async move {
            let state = &app.state;
            // A tag edit on each parcel.
            for id in [&a, &b] {
                let patch = Patch {
                    tags: TagsChange::Edit {
                        add: BTreeSet::from([format!("edit-{i}")]),
                        remove: BTreeSet::new(),
                    },
                    ..Patch::default()
                };
                parcels::apply_patch(&state.db, owner_id, id, &patch)
                    .await
                    .unwrap()
                    .unwrap();
            }
            // New versions (with a tag), and a restore, on both.
            let upload = publish::collect_uploads(
                vec![(
                    "index.html".into(),
                    bytes::Bytes::from(format!("<p>{i}</p>")),
                )],
                &state.config.limits,
            )
            .unwrap();
            let meta = NewVersionMetadata {
                add_tags: vec![format!("version-{i}")],
                ..NewVersionMetadata::default()
            };
            publish::add_version(
                state,
                &b,
                owner_id,
                VersionSource::Upload {
                    meta,
                    uploads: upload,
                },
            )
            .await
            .unwrap();
            publish::add_version(
                state,
                &a,
                owner_id,
                VersionSource::Restore { from_version: 1 },
            )
            .await
            .unwrap();
            // And renames that touch both parcels at once.
            parcel_server::db::tags::rename(
                &state.db,
                owner_id,
                "shared-tag",
                &format!("renamed-{i}"),
            )
            .await
            .unwrap();
            parcel_server::db::tags::rename(
                &state.db,
                owner_id,
                &format!("renamed-{i}"),
                "shared-tag",
            )
            .await
            .unwrap();
        });
    }
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(done) = tasks.join_next().await {
            done.expect("writer panicked");
        }
    })
    .await
    .expect("writers finished (no deadlock)");

    let a = detail(&app, &me, &first.id).await.parcel;
    let b = detail(&app, &me, &second.id).await.parcel;
    for i in 0..8 {
        assert!(a.tags.contains(&format!("edit-{i}")), "{:?}", a.tags);
        assert!(b.tags.contains(&format!("edit-{i}")), "{:?}", b.tags);
        assert!(b.tags.contains(&format!("version-{i}")), "{:?}", b.tags);
    }
    assert_eq!(a.current_version, 9);
    assert_eq!(b.current_version, 9);
}
