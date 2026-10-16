//! Parcel data over the real router + Postgres: the owner's API, the page's
//! content-origin endpoints, limits, and who may read or write.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use parcel_types::{DataItem, DataItemList, DataSummary, PublishResult, ViewGrant};
use serde_json::{json, Value};

async fn publish(app: &TestApp, token: &str, visibility: &str) -> String {
    let form = Form::new()
        .metadata(json!({"visibility": visibility}))
        .file("index.html", "<h1>Poll</h1>");
    let response = app
        .post_multipart("/api/v1/parcels", Some(token), &form)
        .await;
    let published: PublishResult = json(expect_status(response, StatusCode::CREATED).await).await;
    published.parcel.id
}

/// The `/d/{token}` base for a viewer's grant.
async fn data_base(app: &TestApp, token: Option<&str>, id: &str) -> String {
    let grant: ViewGrant = json(
        expect_status(
            app.get(&format!("/api/v1/parcels/{id}/view"), token).await,
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let path = url::Url::parse(&grant.content_url)
        .unwrap()
        .path()
        .to_string();
    let capability = path.split('/').nth(2).unwrap().to_string();
    format!("/d/{capability}")
}

async fn put(app: &TestApp, token: &str, id: &str, item: &str, value: Value) -> DataItem {
    let response = app
        .put_json(
            &format!("/api/v1/parcels/{id}/data/{item}"),
            Some(token),
            &value,
        )
        .await;
    json(expect_status(response, StatusCode::OK).await).await
}

#[tokio::test]
async fn owner_reads_and_writes_through_the_api() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish(&app, &owner, "private").await;
    let base = format!("/api/v1/parcels/{id}/data");

    let empty: DataSummary = json(app.get(&base, Some(&owner)).await).await;
    assert!(empty.writable);
    assert_eq!(empty.usage_bytes, 0);
    assert!(empty.collections.is_empty());

    let item = put(
        &app,
        &owner,
        &id,
        "votes/alice",
        json!({"choice": "tea", "n": 1}),
    )
    .await;
    assert_eq!(item.collection, "votes");
    assert_eq!(item.key, "alice");
    assert_eq!(item.size_bytes, 22);
    put(&app, &owner, &id, "votes/bob", json!("coffee")).await;
    put(&app, &owner, &id, "votes/carol", json!(3)).await;
    put(&app, &owner, &id, "settings/theme", json!(null)).await;

    // RFC 7386 merge: fields merge, null removes.
    let merged: DataItem = json(
        expect_status(
            app.patch_json(
                &format!("{base}/votes/alice"),
                Some(&owner),
                &json!({"n": null, "note": "hot"}),
            )
            .await,
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(merged.value, json!({"choice": "tea", "note": "hot"}));
    // Merging into a missing key creates it.
    let created: DataItem = json(
        app.patch_json(
            &format!("{base}/votes/dave"),
            Some(&owner),
            &json!({"a": 1}),
        )
        .await,
    )
    .await;
    assert_eq!(created.value, json!({"a": 1}));

    let got: DataItem = json(app.get(&format!("{base}/votes/bob"), Some(&owner)).await).await;
    assert_eq!(got.value, json!("coffee"));

    // Pages of the collection, by key.
    let page: DataItemList = json(
        app.get(&format!("{base}/votes?limit=2"), Some(&owner))
            .await,
    )
    .await;
    let keys: Vec<_> = page.items.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["alice", "bob"]);
    assert_eq!(page.next_after.as_deref(), Some("bob"));
    let rest: DataItemList = json(
        app.get(&format!("{base}/votes?limit=2&after=bob"), Some(&owner))
            .await,
    )
    .await;
    let keys: Vec<_> = rest.items.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["carol", "dave"]);
    assert!(rest.next_after.is_none());

    let summary: DataSummary = json(app.get(&base, Some(&owner)).await).await;
    let names: Vec<_> = summary
        .collections
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["settings", "votes"]);
    assert_eq!(summary.collections[1].count, 4);
    assert!(summary.revision > empty.revision);
    assert_eq!(
        summary.usage_bytes,
        summary
            .collections
            .iter()
            .map(|c| c.usage_bytes)
            .sum::<i64>()
    );

    // Deletes: one item, then a whole collection.
    let r = app.delete(&format!("{base}/votes/bob"), Some(&owner)).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = app.get(&format!("{base}/votes/bob"), Some(&owner)).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = app.delete(&format!("{base}/votes"), Some(&owner)).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let after: DataSummary = json(app.get(&base, Some(&owner)).await).await;
    assert_eq!(after.collections.len(), 1);

    // Data is per parcel, not per version: a new version keeps it.
    let form = Form::new().file("index.html", "<h1>v2</h1>");
    let r = app
        .post_multipart(
            &format!("/api/v1/parcels/{id}/versions"),
            Some(&owner),
            &form,
        )
        .await;
    expect_status(r, StatusCode::CREATED).await;
    let r = app
        .get(&format!("{base}/settings/theme"), Some(&owner))
        .await;
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn bad_names_bodies_and_limits_are_refused() {
    let app = TestApp::with_config(|c| {
        c.limits.max_data_value_bytes = 64;
        c.limits.max_data_bytes = 600;
    })
    .await;
    let owner = user_token();
    let id = publish(&app, &owner, "private").await;
    let base = format!("/api/v1/parcels/{id}/data");

    for bad in ["has%20space/k", "c/-k", "%E2%9C%93/k"] {
        let r = app
            .put_json(&format!("{base}/{bad}"), Some(&owner), &json!(1))
            .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    let r = app
        .send(
            axum::http::Request::builder()
                .method(Method::PUT)
                .uri(format!("{base}/c/k"))
                .header("host", "parcel.test")
                .header("authorization", format!("Bearer {owner}"))
                .body(axum::body::Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = app
        .patch_json(&format!("{base}/c/k"), Some(&owner), &json!([1, 2]))
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // One value over 64 bytes.
    let big = json!("x".repeat(80));
    let r = app
        .put_json(&format!("{base}/c/big"), Some(&owner), &big)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // The parcel total: each item is its JSON plus 128 bytes.
    let fifty = json!("y".repeat(48)); // 50 bytes as JSON
    put(&app, &owner, &id, "c/a", fifty.clone()).await;
    put(&app, &owner, &id, "c/b", fifty.clone()).await;
    put(&app, &owner, &id, "c/c", fifty.clone()).await; // 534 in use
    let r = app
        .put_json(&format!("{base}/c/d"), Some(&owner), &fifty)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // Replacing an existing item only counts the difference.
    put(&app, &owner, &id, "c/c", json!("z".repeat(60))).await;
}

#[tokio::test]
async fn only_the_owner_writes_and_private_data_stays_private() {
    let app = TestApp::new().await;
    let owner = user_token();
    let guest = user_token();
    let private = publish(&app, &owner, "private").await;
    let public = publish(&app, &owner, "public").await;
    put(&app, &owner, &public, "poll/result", json!({"yes": 3})).await;

    let r = app
        .get(&format!("/api/v1/parcels/{private}/data"), Some(&guest))
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = app
        .get(&format!("/api/v1/parcels/{private}/data"), None)
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    let read: DataItem = json(
        app.get(&format!("/api/v1/parcels/{public}/data/poll/result"), None)
            .await,
    )
    .await;
    assert_eq!(read.value, json!({"yes": 3}));
    let summary: DataSummary = json(
        app.get(&format!("/api/v1/parcels/{public}/data"), Some(&guest))
            .await,
    )
    .await;
    assert!(!summary.writable);

    let item = format!("/api/v1/parcels/{public}/data/poll/result");
    let r = app.put_json(&item, Some(&guest), &json!(1)).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = app.patch_json(&item, Some(&guest), &json!({"x": 1})).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = app.delete(&item, Some(&guest)).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = app.put_json(&item, None, &json!(1)).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pages_use_their_capability_on_the_content_origin() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish(&app, &owner, "public").await;
    let owner_base = data_base(&app, Some(&owner), &id).await;
    let public_base = data_base(&app, None, &id).await;

    // CORS: the sandbox's `null` origin preflights writes.
    let pre = app
        .content_request(Method::OPTIONS, &format!("{owner_base}/votes/a"), None)
        .await;
    assert_eq!(pre.status(), StatusCode::NO_CONTENT);
    assert_eq!(pre.headers()["access-control-allow-origin"], "*");
    assert!(pre.headers()["access-control-allow-methods"]
        .to_str()
        .unwrap()
        .contains("PATCH"));

    let summary = expect_status(
        app.content_request(Method::GET, &owner_base, None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(summary.headers()["access-control-allow-origin"], "*");
    assert_eq!(summary.headers()["cache-control"], "no-store");
    let summary: DataSummary = json(summary).await;
    assert!(summary.writable);

    // The owner's page writes; a visitor's page reads.
    let written = app
        .content_request(
            Method::PUT,
            &format!("{owner_base}/votes/a"),
            Some(r#"{"n":1}"#),
        )
        .await;
    let written: DataItem = json(expect_status(written, StatusCode::OK).await).await;
    assert_eq!(written.value, json!({"n": 1}));
    let merged = app
        .content_request(
            Method::PATCH,
            &format!("{owner_base}/votes/a"),
            Some(r#"{"m":2}"#),
        )
        .await;
    let merged: DataItem = json(expect_status(merged, StatusCode::OK).await).await;
    assert_eq!(merged.value, json!({"n": 1, "m": 2}));

    let visitor: DataSummary =
        json(app.content_request(Method::GET, &public_base, None).await).await;
    assert!(!visitor.writable);
    assert_eq!(visitor.collections[0].count, 1);
    let read: DataItem = json(
        app.content_request(Method::GET, &format!("{public_base}/votes/a"), None)
            .await,
    )
    .await;
    assert_eq!(read.value, json!({"n": 1, "m": 2}));
    let list: DataItemList = json(
        app.content_request(Method::GET, &format!("{public_base}/votes?limit=5"), None)
            .await,
    )
    .await;
    assert_eq!(list.items.len(), 1);
    let denied = app
        .content_request(Method::PUT, &format!("{public_base}/votes/b"), Some("1"))
        .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(denied.headers()["access-control-allow-origin"], "*");
    let denied = app
        .content_request(Method::DELETE, &format!("{public_base}/votes"), None)
        .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // Going private cuts off visitors' grants, not the owner's.
    app.patch_json(
        &format!("/api/v1/parcels/{id}"),
        Some(&owner),
        &json!({"visibility": "private"}),
    )
    .await;
    let r = app.content_request(Method::GET, &public_base, None).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = app.content_request(Method::GET, &owner_base, None).await;
    assert_eq!(r.status(), StatusCode::OK);

    // A forged capability is a 404, and so is a deleted parcel's data.
    let r = app
        .content_request(Method::GET, "/d/bm9wZQ.bm9wZQ/votes", None)
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    app.delete(&format!("/api/v1/parcels/{id}"), Some(&owner))
        .await;
    let r = app
        .content_request(Method::GET, &format!("{owner_base}/votes/a"), None)
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Data routes exist only on the content host.
    let r = app.get(&format!("{owner_base}/votes/a"), None).await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn html_pages_get_the_helper_and_nothing_else_does() {
    let app = TestApp::new().await;
    let owner = user_token();
    let form = Form::new()
        .file(
            "index.html",
            "<!doctype html><html><head><title>x</title></head><body>hi",
        )
        .file("notes.md", "# Notes")
        .file("app.js", "console.log(1)");
    let response = app
        .post_multipart("/api/v1/parcels", Some(&owner), &form)
        .await;
    let published: PublishResult = json(expect_status(response, StatusCode::CREATED).await).await;
    let id = published.parcel.id;
    let grant: ViewGrant = json(
        app.get(&format!("/api/v1/parcels/{id}/view"), Some(&owner))
            .await,
    )
    .await;
    let base = grant.content_url.trim_end_matches("index.html").to_string();
    let hash = app.state.helper.hash.clone();
    let tag = format!("<script src=\"/_parcel/helper.{hash}.js\"></script>");

    let page = expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::OK,
    )
    .await;
    let etag = page.headers()["etag"].to_str().unwrap().to_string();
    assert!(etag.ends_with(&format!("-h{hash}\"")), "{etag}");
    let html = String::from_utf8(body_bytes(page).await).unwrap();
    assert!(
        html.starts_with(&format!("<!doctype html><html><head>{tag}<title>")),
        "{html}"
    );
    let cached = app
        .get_content(&grant.content_url, &[("if-none-match", &etag)])
        .await;
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);

    // Non-entry Markdown is served raw; JS untouched.
    let md =
        String::from_utf8(body_bytes(app.get_content(&format!("{base}notes.md"), &[]).await).await)
            .unwrap();
    assert_eq!(md, "# Notes");
    let js =
        String::from_utf8(body_bytes(app.get_content(&format!("{base}app.js"), &[]).await).await)
            .unwrap();
    assert_eq!(js, "console.log(1)");

    // The raw-file API on the app origin returns the stored bytes.
    let raw = app
        .get(
            &format!("/api/v1/parcels/{id}/versions/1/files/index.html"),
            Some(&owner),
        )
        .await;
    let raw = String::from_utf8(body_bytes(raw).await).unwrap();
    assert!(!raw.contains("/_parcel/"), "{raw}");

    // The script itself: immutable at its hash, revalidated at any other.
    let script = app
        .content_request(Method::GET, &format!("/_parcel/helper.{hash}.js"), None)
        .await;
    let script = expect_status(script, StatusCode::OK).await;
    assert_eq!(
        script.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(script.headers()["cache-control"]
        .to_str()
        .unwrap()
        .contains("immutable"));
    let body = String::from_utf8(body_bytes(script).await).unwrap();
    assert!(body.contains(&format!("const APP_ORIGIN = \"{APP_ORIGIN}\";")));
    let old = app
        .content_request(Method::GET, "/_parcel/helper.000000000000.js", None)
        .await;
    assert_eq!(old.status(), StatusCode::OK);
    assert_eq!(old.headers()["cache-control"], "no-cache");
    let other = app
        .content_request(Method::GET, "/_parcel/other.js", None)
        .await;
    assert_eq!(other.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn markdown_entries_get_the_helper_too() {
    let app = TestApp::new().await;
    let owner = user_token();
    let form = Form::new().file("README.md", "# Hello");
    let response = app
        .post_multipart("/api/v1/parcels", Some(&owner), &form)
        .await;
    let published: PublishResult = json(expect_status(response, StatusCode::CREATED).await).await;
    let grant: ViewGrant = json(
        app.get(
            &format!("/api/v1/parcels/{}/view", published.parcel.id),
            Some(&owner),
        )
        .await,
    )
    .await;
    let page = app.get_content(&grant.content_url, &[]).await;
    let html = String::from_utf8(body_bytes(page).await).unwrap();
    assert!(html.contains("/_parcel/helper."), "{html}");
    assert!(html.contains(">Hello</h1>"), "{html}");
}
