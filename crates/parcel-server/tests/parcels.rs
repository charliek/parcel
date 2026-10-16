//! Parcel lifecycle over the real router + Postgres: publish, read, view,
//! version, restore, visibility, delete.

mod common;

use axum::http::StatusCode;
use common::*;
use parcel_types::{
    ParcelDetail, ParcelList, ParcelSummary, PublishResult, VersionDetail, VersionList, ViewGrant,
};
use serde_json::json;

async fn publish_page(app: &TestApp, token: &str) -> PublishResult {
    let form = Form::new()
        .file("index.html", PAGE)
        .file("app.js", "console.log('hi')")
        .file("data/points.json", r#"{"points":[1,2,3]}"#);
    let response = app
        .post_multipart("/api/v1/parcels", Some(token), &form)
        .await;
    json(expect_status(response, StatusCode::CREATED).await).await
}

#[tokio::test]
async fn publish_derives_title_entry_and_totals() {
    let app = TestApp::new().await;
    let token = user_token();
    let result = publish_page(&app, &token).await;
    assert_eq!(result.parcel.title, "Hello Parcel");
    assert_eq!(result.parcel.entry, "index.html");
    assert_eq!(result.parcel.current_version, 1);
    assert_eq!(result.parcel.file_count, 3);
    assert_eq!(
        result.parcel.url,
        format!("{APP_ORIGIN}/p/{}", result.parcel.id)
    );
    assert!(parcel_types::ids::is_valid_parcel_id(&result.parcel.id));
    let paths: Vec<_> = result
        .version
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, ["app.js", "data/points.json", "index.html"]);
    let js = &result.version.files[0];
    assert_eq!(js.content_type, "text/javascript; charset=utf-8");
    assert_eq!(js.size_bytes, 17);
}

#[tokio::test]
async fn list_and_search_only_show_my_parcels() {
    let app = TestApp::new().await;
    let me = user_token();
    let other = user_token();
    let mine = publish_page(&app, &me).await;
    publish_page(&app, &other).await;

    let form = Form::new()
        .metadata(json!({"title": "Quarterly report", "description": "numbers"}))
        .file("report.html", "<p>q3</p>");
    expect_status(
        app.post_multipart("/api/v1/parcels", Some(&me), &form)
            .await,
        StatusCode::CREATED,
    )
    .await;

    let list: ParcelList = json(app.get("/api/v1/parcels", Some(&me)).await).await;
    assert_eq!(list.total, 2);
    assert_eq!(list.parcels[0].title, "Quarterly report", "newest first");
    assert_eq!(list.parcels[1].id, mine.parcel.id);

    let found: ParcelList = json(app.get("/api/v1/parcels?q=QUARTER", Some(&me)).await).await;
    assert_eq!(found.total, 1);
    let none: ParcelList = json(app.get("/api/v1/parcels?q=100%25", Some(&me)).await).await;
    assert_eq!(none.total, 0, "LIKE wildcards are escaped");

    expect_status(
        app.get("/api/v1/parcels", None).await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn view_grant_serves_sandboxed_content() {
    let app = TestApp::new().await;
    let token = user_token();
    let published = publish_page(&app, &token).await;
    let id = &published.parcel.id;

    let grant: ViewGrant = json(
        app.get(&format!("/api/v1/parcels/{id}/view"), Some(&token))
            .await,
    )
    .await;
    assert!(grant.is_owner);
    assert!(grant
        .content_url
        .starts_with(&format!("https://{CONTENT_HOST}/v/")));
    assert!(grant.content_url.ends_with("/index.html"));

    let page = app.get_content(&grant.content_url, &[]).await;
    let page = expect_status(page, StatusCode::OK).await;
    let headers = page.headers().clone();
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("sandbox allow-scripts"), "{csp}");
    assert!(!csp.contains("allow-same-origin"), "{csp}");
    assert!(
        csp.contains(&format!("frame-ancestors {APP_ORIGIN}")),
        "{csp}"
    );
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert_eq!(headers["access-control-allow-origin"], "*");
    assert_eq!(headers["content-type"], "text/html; charset=utf-8");
    assert!(
        headers.get("x-frame-options").is_none(),
        "content must be frameable by the app"
    );
    // The served page is the stored page plus the helper script.
    assert_eq!(
        body_bytes(page).await,
        app.state.helper.inject(PAGE.as_bytes())
    );

    // Siblings resolve under the same token prefix.
    let base = grant.content_url.trim_end_matches("index.html");
    let js = expect_status(
        app.get_content(&format!("{base}app.js"), &[]).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        js.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    let etag = js.headers()["etag"].to_str().unwrap().to_string();
    let cached = app
        .get_content(&format!("{base}app.js"), &[("if-none-match", &etag)])
        .await;
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
    let data = app
        .get_content(&format!("{base}data/points.json"), &[])
        .await;
    assert_eq!(body_bytes(data).await, br#"{"points":[1,2,3]}"#);
    let missing = app.get_content(&format!("{base}nope.css"), &[]).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // The token root redirects to the entry.
    let root = app.get_content(base, &[]).await;
    assert_eq!(root.status(), StatusCode::FOUND);
    assert!(root.headers()["location"]
        .to_str()
        .unwrap()
        .ends_with("/index.html"));

    // Content paths are not served on the app host, and vice versa.
    let path = url::Url::parse(&grant.content_url)
        .unwrap()
        .path()
        .to_string();
    assert_eq!(app.get(&path, None).await.status(), StatusCode::NOT_FOUND);
    let api_on_content = app
        .get_content(&format!("https://{CONTENT_HOST}/api/v1/info"), &[])
        .await;
    assert_eq!(api_on_content.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tampered_or_expired_content_tokens_are_refused() {
    let app =
        TestApp::with_config(|c| c.content_token_ttl = std::time::Duration::from_secs(0)).await;
    let token = user_token();
    let published = publish_page(&app, &token).await;
    let grant: ViewGrant = json(
        app.get(
            &format!("/api/v1/parcels/{}/view", published.parcel.id),
            Some(&token),
        )
        .await,
    )
    .await;
    let expired = app.get_content(&grant.content_url, &[]).await;
    assert_eq!(expired.status(), StatusCode::FORBIDDEN);
    let bad = app
        .get_content(
            &format!("https://{CONTENT_HOST}/v/not-a-token/index.html"),
            &[],
        )
        .await;
    assert_eq!(bad.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_parcels_are_invisible_to_others() {
    let app = TestApp::new().await;
    let owner = user_token();
    let stranger = user_token();
    let id = publish_page(&app, &owner).await.parcel.id;

    for uri in [
        format!("/api/v1/parcels/{id}"),
        format!("/api/v1/parcels/{id}/view"),
        format!("/api/v1/parcels/{id}/versions"),
        format!("/api/v1/parcels/{id}/versions/1/files/index.html"),
    ] {
        expect_status(app.get(&uri, Some(&stranger)).await, StatusCode::NOT_FOUND).await;
        expect_status(app.get(&uri, None).await, StatusCode::NOT_FOUND).await;
    }
    expect_status(
        app.delete(&format!("/api/v1/parcels/{id}"), Some(&stranger))
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn public_parcels_are_readable_and_revocable() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish_page(&app, &owner).await.parcel.id;

    let updated: ParcelSummary = json(
        app.patch_json(
            &format!("/api/v1/parcels/{id}"),
            Some(&owner),
            &json!({"visibility": "public", "title": "Shared page"}),
        )
        .await,
    )
    .await;
    assert_eq!(updated.title, "Shared page");

    // Anonymous visitors can read it…
    let detail: ParcelDetail = json(app.get(&format!("/api/v1/parcels/{id}"), None).await).await;
    assert!(!detail.is_owner);
    let grant: ViewGrant = json(app.get(&format!("/api/v1/parcels/{id}/view"), None).await).await;
    assert!(!grant.is_owner);
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::OK,
    )
    .await;

    // …but not change it…
    let stranger = user_token();
    expect_status(
        app.patch_json(
            &format!("/api/v1/parcels/{id}"),
            Some(&stranger),
            &json!({"title": "x"}),
        )
        .await,
        StatusCode::FORBIDDEN,
    )
    .await;

    // …and making it private revokes the public grant immediately.
    app.patch_json(
        &format!("/api/v1/parcels/{id}"),
        Some(&owner),
        &json!({"visibility": "private"}),
    )
    .await;
    let revoked = app.get_content(&grant.content_url, &[]).await;
    assert_eq!(revoked.status(), StatusCode::FORBIDDEN);
    expect_status(
        app.get(&format!("/api/v1/parcels/{id}"), None).await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn an_invalid_token_is_401_even_on_public_routes() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish_page(&app, &owner).await.parcel.id;
    app.patch_json(
        &format!("/api/v1/parcels/{id}"),
        Some(&owner),
        &json!({"visibility": "public"}),
    )
    .await;
    expect_status(
        app.get(&format!("/api/v1/parcels/{id}"), Some("garbage.jwt.value"))
            .await,
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

#[tokio::test]
async fn versions_merge_replace_conflict_and_restore() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_page(&app, &token).await.parcel.id;
    let versions_uri = format!("/api/v1/parcels/{id}/versions");

    // Merge: add a file, replace one, remove one; others carry over.
    let form = Form::new()
        .metadata(json!({"remove": ["data/points.json"], "label": "second draft"}))
        .file("style.css", "body{}")
        .file("app.js", "console.log('v2')");
    let v2: PublishResult = json(
        expect_status(
            app.post_multipart(&versions_uri, Some(&token), &form).await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(v2.version.summary.version, 2);
    assert_eq!(v2.version.summary.label.as_deref(), Some("second draft"));
    let paths: Vec<_> = v2.version.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["app.js", "index.html", "style.css"]);
    assert_eq!(
        v2.parcel.url,
        format!("{APP_ORIGIN}/p/{id}"),
        "URL is stable"
    );

    // Optimistic concurrency.
    let stale = Form::new()
        .metadata(json!({"base_version": 1}))
        .file("x.css", "a{}");
    expect_status(
        app.post_multipart(&versions_uri, Some(&token), &stale)
            .await,
        StatusCode::CONFLICT,
    )
    .await;

    // Replace: exactly the files sent.
    let form = Form::new()
        .metadata(json!({"mode": "replace", "base_version": 2}))
        .file("README.md", "# Notes\n\nhello");
    let v3: PublishResult = json(
        expect_status(
            app.post_multipart(&versions_uri, Some(&token), &form).await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(v3.parcel.entry, "README.md");
    assert_eq!(v3.version.files.len(), 1);

    // Old versions stay viewable and fetchable.
    let v1: VersionDetail = json(app.get(&format!("{versions_uri}/1"), Some(&token)).await).await;
    assert_eq!(v1.files.len(), 3);
    let latest: VersionDetail = json(
        app.get(&format!("{versions_uri}/latest"), Some(&token))
            .await,
    )
    .await;
    assert_eq!(latest.summary.version, 3);
    let raw = app
        .get(
            &format!("{versions_uri}/1/files/data/points.json"),
            Some(&token),
        )
        .await;
    let raw = expect_status(raw, StatusCode::OK).await;
    assert!(raw.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment"));
    assert!(raw.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .starts_with("sandbox"));
    assert_eq!(body_bytes(raw).await, br#"{"points":[1,2,3]}"#);

    // Restore v1 as v4.
    let v4: PublishResult = json(
        expect_status(
            app.post_json(
                &format!("{versions_uri}/1/restore"),
                Some(&token),
                &json!({}),
            )
            .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(v4.version.summary.version, 4);
    assert_eq!(
        v4.version.summary.label.as_deref(),
        Some("Restored from v1")
    );
    assert_eq!(v4.parcel.entry, "index.html");

    let list: VersionList = json(app.get(&versions_uri, Some(&token)).await).await;
    let numbers: Vec<_> = list.versions.iter().map(|v| v.version).collect();
    assert_eq!(numbers, [4, 3, 2, 1]);

    // Removing a path that isn't there is an error, not a silent no-op.
    let form = Form::new().metadata(json!({"remove": ["ghost.txt"]}));
    expect_status(
        app.post_multipart(&versions_uri, Some(&token), &form).await,
        StatusCode::BAD_REQUEST,
    )
    .await;
}

#[tokio::test]
async fn markdown_entries_render_as_html() {
    let app = TestApp::new().await;
    let token = user_token();
    let form = Form::new().file("notes.md", "# Meeting notes\n\n- [x] ship it\n");
    let published: PublishResult = json(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(&token), &form)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(published.parcel.title, "Meeting notes");
    let grant: ViewGrant = json(
        app.get(
            &format!("/api/v1/parcels/{}/view", published.parcel.id),
            Some(&token),
        )
        .await,
    )
    .await;
    let page = expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
    let html = String::from_utf8(body_bytes(page).await).unwrap();
    assert!(html.contains("<title>Meeting notes</title>"));
    assert!(html.contains("<h1>Meeting notes</h1>"));
}

#[tokio::test]
async fn delete_removes_everything() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_page(&app, &token).await.parcel.id;
    let grant: ViewGrant = json(
        app.get(&format!("/api/v1/parcels/{id}/view"), Some(&token))
            .await,
    )
    .await;

    expect_status(
        app.delete(&format!("/api/v1/parcels/{id}"), Some(&token))
            .await,
        StatusCode::NO_CONTENT,
    )
    .await;
    expect_status(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&token))
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(
        app.get_content(&grant.content_url, &[]).await.status(),
        StatusCode::NOT_FOUND
    );
    expect_status(
        app.delete(&format!("/api/v1/parcels/{id}"), Some(&token))
            .await,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn me_reports_usage() {
    let app = TestApp::new().await;
    let token = user_token();
    publish_page(&app, &token).await;
    let me: parcel_types::Me = json(app.get("/api/v1/me", Some(&token)).await).await;
    assert_eq!(me.usage.parcel_count, 1);
    assert_eq!(me.usage.version_count, 1);
    assert!(me.usage.stored_bytes > 0);
    assert_eq!(me.name, "Test User");
    expect_status(app.get("/api/v1/me", None).await, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn info_is_public() {
    let app = TestApp::new().await;
    let info: parcel_types::ServerInfo = json(app.get("/api/v1/info", None).await).await;
    assert_eq!(info.auth.mode, "oidc");
    assert_eq!(
        info.auth.providers,
        [parcel_types::AuthProvider {
            id: "test".into(),
            label: "Test IdP".into()
        }]
    );
    assert!(info.allow_public);
    assert_eq!(info.content_origin, format!("https://{CONTENT_HOST}"));
    assert!(info.allowed_extensions.iter().any(|e| e == "html"));
}

#[tokio::test]
async fn dev_mode_needs_no_credentials() {
    let app = TestApp::with_config(|c| {
        c.auth_mode = parcel_server::config::AuthMode::Dev;
        c.dev_auth_email = format!("dev-{}@example.com", uuid::Uuid::new_v4());
    })
    .await;
    let me: parcel_types::Me =
        json(expect_status(app.get("/api/v1/me", None).await, StatusCode::OK).await).await;
    assert!(me.email.starts_with("dev-"));
}
