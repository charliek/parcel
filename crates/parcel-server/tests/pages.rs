//! Pages within a parcel: titles kept at publish, `pages` on the detail
//! types, view grants for one page, and Markdown pages rendered only when
//! opened as a page (plan 001 §3.11).

mod common;

use axum::http::StatusCode;
use axum::response::Response;
use common::*;
use parcel_types::{PageEntry, ParcelDetail, PublishResult, VersionDetail, ViewGrant};
use serde_json::json;

const NOTES_MD: &str = "# Build notes\n\nSee <b>the settings</b>.\n";
const SETTINGS: &str = "<!doctype html><html><head><title>Rust build settings</title></head>\
    <body><a href=\"index.html\">back</a></body></html>";
const HOME: &str = "<!doctype html><html><head><title>Home</title></head>\
    <body><a href=\"rust-build-settings.html\">settings</a> \
    <a href=\"docs/notes.md\">notes</a></body></html>";

/// The `y4pbpra3ffhb` v3 shape plus a Markdown page, an untitled page and a
/// supporting file.
async fn publish_site(app: &TestApp, token: &str, metadata: serde_json::Value) -> PublishResult {
    let form = Form::new()
        .metadata(metadata)
        .file("index.html", HOME)
        .file("rust-build-settings.html", SETTINGS)
        .file("docs/notes.md", NOTES_MD)
        .file("Zed.htm", "<p>no title here</p>")
        .file("app.js", "console.log('<title>not a page</title>')");
    json(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(token), &form)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await
}

fn page(path: &str, title: Option<&str>) -> PageEntry {
    PageEntry {
        path: path.into(),
        title: title.map(str::to_string),
    }
}

fn site_pages() -> Vec<PageEntry> {
    vec![
        page("index.html", Some("Home")),
        // Byte order: uppercase before lowercase.
        page("Zed.htm", None),
        page("docs/notes.md", Some("Build notes")),
        page("rust-build-settings.html", Some("Rust build settings")),
    ]
}

async fn grant(app: &TestApp, token: Option<&str>, id: &str, query: &str) -> Response {
    app.get(&format!("/api/v1/parcels/{id}/view{query}"), token)
        .await
}

async fn grant_url(app: &TestApp, token: Option<&str>, id: &str, query: &str) -> String {
    let g: ViewGrant =
        json(expect_status(grant(app, token, id, query).await, StatusCode::OK).await).await;
    g.content_url
}

#[tokio::test]
async fn pages_are_listed_with_their_titles() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let published = publish_site(&app, &owner.token, json!({})).await;
    let id = &published.parcel.id;

    assert_eq!(published.version.pages, site_pages());
    let title_of = |path: &str| {
        published
            .version
            .files
            .iter()
            .find(|f| f.path == path)
            .unwrap()
            .page_title
            .clone()
    };
    assert_eq!(title_of("docs/notes.md").as_deref(), Some("Build notes"));
    assert_eq!(title_of("Zed.htm"), None);
    assert_eq!(title_of("app.js"), None, "only pages have titles");

    let detail: ParcelDetail = json(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&owner.token))
            .await,
    )
    .await;
    assert_eq!(detail.pages, site_pages());
    let version: VersionDetail = json(
        app.get(
            &format!("/api/v1/parcels/{id}/versions/1"),
            Some(&owner.token),
        )
        .await,
    )
    .await;
    assert_eq!(version.pages, site_pages());

    // Whoever sees the file list sees the pages: a viewer it is shared with.
    let viewer = person("viewer");
    expect_status(
        app.put_json(
            &format!("/api/v1/parcels/{id}/shares/{}", viewer.email),
            Some(&owner.token),
            &json!({"role": "viewer"}),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let seen: ParcelDetail = json(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&viewer.token))
            .await,
    )
    .await;
    assert!(!seen.is_owner);
    assert_eq!(seen.pages, site_pages());

    // The wire form: a page without a title says so with null.
    let raw: serde_json::Value = json(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&owner.token))
            .await,
    )
    .await;
    assert_eq!(raw["pages"][1], json!({"path": "Zed.htm", "title": null}));
    assert!(raw["files"][0].get("page_title").is_none(), "{raw}");
}

#[tokio::test]
async fn titles_follow_files_through_merges_and_restores() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_site(&app, &token, json!({})).await.parcel.id;
    let versions = format!("/api/v1/parcels/{id}/versions");

    // A merge that re-sends one page: it gets its new title, the pages it
    // keeps keep theirs.
    let form = Form::new().file("docs/notes.md", "# Build notes, revised\n");
    let v2: PublishResult = json(
        expect_status(
            app.post_multipart(&versions, Some(&token), &form).await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let mut expected = site_pages();
    expected[2].title = Some("Build notes, revised".into());
    assert_eq!(v2.version.pages, expected);

    // A replace whose new entry is Markdown; the entry leads the list.
    let form = Form::new()
        .metadata(json!({"mode": "replace", "entry": "README.md"}))
        .file("README.md", "intro\n\n# Readme\n")
        .file("a.html", "<title>A</title>");
    let v3: PublishResult = json(
        expect_status(
            app.post_multipart(&versions, Some(&token), &form).await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(
        v3.version.pages,
        [page("README.md", Some("Readme")), page("a.html", Some("A"))]
    );

    // A restore copies the restored version's titles.
    let v4: PublishResult = json(
        expect_status(
            app.post_json(&format!("{versions}/2/restore"), Some(&token), &json!({}))
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(v4.version.pages, expected);

    // Rows from before titles were kept say null, and the pages still list.
    sqlx::query("UPDATE version_files SET page_title = NULL WHERE parcel_id = $1")
        .bind(&id)
        .execute(&app.state.db)
        .await
        .unwrap();
    let detail: ParcelDetail = json(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&token))
            .await,
    )
    .await;
    assert_eq!(
        detail.pages,
        site_pages()
            .into_iter()
            .map(|p| page(&p.path, None))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn hostile_page_titles_are_plain_text() {
    let app = TestApp::new().await;
    let token = user_token();
    let form = Form::new()
        .file(
            "index.html",
            "<title>\u{1b}[2J&lt;img src=x onerror=alert(1)&gt;\u{7}</title>",
        )
        .file("b.md", "# \u{9b}31m`<script>`\n");
    let published: PublishResult = json(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(&token), &form)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(
        published.version.pages,
        [
            page("index.html", Some("[2J<img src=x onerror=alert(1)>")),
            page("b.md", Some("31m<script>")),
        ]
    );
}

/// A grant for one page; relative links from it reach the version's other
/// files and pages under the same capability prefix.
#[tokio::test]
async fn view_grants_open_one_page() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_site(&app, &token, json!({})).await.parcel.id;

    let entry = grant_url(&app, Some(&token), &id, "").await;
    assert!(entry.ends_with("/index.html"), "{entry}");
    let settings = grant_url(&app, Some(&token), &id, "?path=rust-build-settings.html").await;
    assert!(
        settings.ends_with("/rust-build-settings.html"),
        "{settings}"
    );
    let page = expect_status(app.get_content(&settings, &[]).await, StatusCode::OK).await;
    assert_eq!(
        body_bytes(page).await,
        app.state.helper.inject(SETTINGS.as_bytes())
    );
    let notes = grant_url(&app, Some(&token), &id, "?version=1&path=docs%2Fnotes.md").await;
    assert!(notes.ends_with("/docs/notes.md"), "{notes}");

    // Version 2 adds a page.
    let form = Form::new().file("new.html", "<title>New</title>");
    expect_status(
        app.post_multipart(
            &format!("/api/v1/parcels/{id}/versions"),
            Some(&token),
            &form,
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    grant_url(&app, Some(&token), &id, "?path=new.html").await;

    // Not a page of that version: a 404, whatever the reason.
    for query in [
        "?path=app.js",               // a file, not a page
        "?path=nope.html",            // no such file
        "?path=docs/",                // a directory
        "?path=",                     // nothing
        "?path=%00.html",             // a NUL: not a storable path (was a 500)
        "?path=../index.html",        // traversal
        "?version=1&path=new.html",   // a page of a later version only
        "?version=9&path=index.html", // no such version
    ] {
        expect_status(
            grant(&app, Some(&token), &id, query).await,
            StatusCode::NOT_FOUND,
        )
        .await;
    }

    // Nobody without access learns whether a page exists.
    let stranger = user_token();
    for query in ["?path=index.html", "?path=nope.html"] {
        expect_status(
            grant(&app, Some(&stranger), &id, query).await,
            StatusCode::NOT_FOUND,
        )
        .await;
    }
}

/// The `y4pbpra3ffhb` v3 shape: `index.html` links to
/// `rust-build-settings.html`. Followed in the frame, the link resolves
/// against the entry's URL and is served under the same capability.
#[tokio::test]
async fn a_second_html_page_is_served_under_the_entry_grant() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_site(&app, &token, json!({})).await.parcel.id;
    let entry = grant_url(&app, Some(&token), &id, "").await;

    let linked = url::Url::parse(&entry)
        .unwrap()
        .join("rust-build-settings.html")
        .unwrap();
    let page = expect_status(
        app.get_content(linked.as_str(), &[("sec-fetch-dest", "iframe")])
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
    assert_sandboxed(&page);
    assert!(page.headers().get("vary").is_none(), "HTML has one variant");
    assert_eq!(
        body_bytes(page).await,
        app.state.helper.inject(SETTINGS.as_bytes())
    );

    // And back, and on to the Markdown page.
    let back = linked.join("index.html").unwrap();
    assert_eq!(back.as_str(), entry);
    let notes = linked.join("docs/notes.md").unwrap();
    let page = expect_status(
        app.get_content(notes.as_str(), &[("sec-fetch-dest", "iframe")])
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
}

fn assert_sandboxed(response: &Response) {
    let headers = response.headers();
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.starts_with("sandbox allow-scripts"), "{csp}");
    assert!(!csp.contains("allow-same-origin"), "{csp}");
    assert!(
        csp.contains(&format!("frame-ancestors {APP_ORIGIN}")),
        "{csp}"
    );
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["cache-control"], "private, max-age=3600");
}

fn etag(response: &Response) -> String {
    response.headers()["etag"].to_str().unwrap().to_string()
}

fn vary(response: &Response) -> &str {
    response.headers()["vary"].to_str().unwrap()
}

/// What a page request and a `fetch()` look like.
const AS_PAGE: &[(&str, &str)] = &[("sec-fetch-dest", "iframe")];
const AS_FETCH: &[(&str, &str)] = &[("sec-fetch-dest", "empty"), ("accept", "*/*")];

fn with<'a>(base: &[(&'a str, &'a str)], more: (&'a str, &'a str)) -> Vec<(&'a str, &'a str)> {
    let mut all = base.to_vec();
    all.push(more);
    all
}

async fn assert_rendered(app: &TestApp, response: Response) -> String {
    let response = expect_status(response, StatusCode::OK).await;
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert_sandboxed(&response);
    assert_eq!(vary(&response), "Sec-Fetch-Dest, Accept");
    let tag = etag(&response);
    let html = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(html.contains("<title>Build notes</title>"), "{html}");
    assert!(html.contains("<h1>Build notes</h1>"), "{html}");
    assert!(
        html.contains(&format!("/_parcel/helper.{}.js", app.state.helper.hash)),
        "the rendered page gets the helper"
    );
    tag
}

async fn assert_raw(response: Response) -> String {
    let response = expect_status(response, StatusCode::OK).await;
    assert_eq!(
        response.headers()["content-type"],
        "text/markdown; charset=utf-8"
    );
    assert_sandboxed(&response);
    assert_eq!(vary(&response), "Sec-Fetch-Dest, Accept");
    let tag = etag(&response);
    assert_eq!(body_bytes(response).await, NOTES_MD.as_bytes());
    tag
}

#[tokio::test]
async fn markdown_pages_render_only_when_opened_as_pages() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_site(&app, &token, json!({})).await.parcel.id;
    let url = grant_url(&app, Some(&token), &id, "?path=docs/notes.md").await;

    // Raw first, then rendered, then raw again: neither variant leaks into
    // the other (the blob cache holds the stored bytes only).
    let raw_tag = assert_raw(app.get_content(&url, AS_FETCH).await).await;
    let page_tag = assert_rendered(&app, app.get_content(&url, AS_PAGE).await).await;
    assert_eq!(
        assert_raw(app.get_content(&url, AS_FETCH).await).await,
        raw_tag
    );
    assert_ne!(raw_tag, page_tag, "each variant has its own ETag");

    // Which requests are pages.
    for headers in [
        &[("sec-fetch-dest", "document")][..],
        &[("sec-fetch-dest", "iframe")],
        &[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")],
    ] {
        let tag = assert_rendered(&app, app.get_content(&url, headers).await).await;
        assert_eq!(tag, page_tag, "{headers:?}");
    }
    for headers in [
        &[("sec-fetch-dest", "empty")][..],
        &[("sec-fetch-dest", "script")],
        &[("sec-fetch-dest", "empty"), ("accept", "text/html")],
        &[("accept", "text/markdown, */*")],
        &[],
    ] {
        let tag = assert_raw(app.get_content(&url, headers).await).await;
        assert_eq!(tag, raw_tag, "{headers:?}");
    }

    // A conditional request matches only its own variant's ETag, and a 304
    // carries the same Vary and sandbox headers.
    for (base, own, other) in [
        (AS_PAGE, &page_tag, &raw_tag),
        (AS_FETCH, &raw_tag, &page_tag),
    ] {
        let hit = app
            .get_content(&url, &with(base, ("if-none-match", own)))
            .await;
        assert_eq!(hit.status(), StatusCode::NOT_MODIFIED, "{base:?}");
        assert_eq!(vary(&hit), "Sec-Fetch-Dest, Accept");
        assert_eq!(&etag(&hit), own);
        assert_sandboxed(&hit);

        let miss = app
            .get_content(&url, &with(base, ("if-none-match", other)))
            .await;
        assert_eq!(miss.status(), StatusCode::OK, "{base:?}");
        assert_eq!(&etag(&miss), own);
    }
}

#[tokio::test]
async fn markdown_pages_render_first_too() {
    let app = TestApp::new().await;
    let token = user_token();
    let id = publish_site(&app, &token, json!({})).await.parcel.id;
    let url = grant_url(&app, Some(&token), &id, "?path=docs/notes.md").await;
    let page_tag = assert_rendered(&app, app.get_content(&url, AS_PAGE).await).await;
    let raw_tag = assert_raw(app.get_content(&url, AS_FETCH).await).await;
    assert_ne!(raw_tag, page_tag);
}

/// The entry keeps today's behaviour: rendered for every request, one
/// variant, no `Vary`.
#[tokio::test]
async fn a_markdown_entry_renders_for_every_request() {
    let app = TestApp::new().await;
    let token = user_token();
    let form = Form::new()
        .metadata(json!({"entry": "notes.md"}))
        .file("notes.md", NOTES_MD)
        .file("other.md", "# Other\n");
    let id = json::<PublishResult>(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(&token), &form)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await
    .parcel
    .id;
    let url = grant_url(&app, Some(&token), &id, "?path=notes.md").await;
    let mut tags = Vec::new();
    for headers in [AS_PAGE, AS_FETCH, &[]] {
        let response = expect_status(app.get_content(&url, headers).await, StatusCode::OK).await;
        assert_eq!(
            response.headers()["content-type"],
            "text/html; charset=utf-8"
        );
        assert!(response.headers().get("vary").is_none());
        tags.push(etag(&response));
    }
    assert!(tags.windows(2).all(|w| w[0] == w[1]), "{tags:?}");
    // The other Markdown file isn't the entry: a fetch gets its text.
    let other = url.replace("/notes.md", "/other.md");
    let response = expect_status(app.get_content(&other, AS_FETCH).await, StatusCode::OK).await;
    assert_eq!(body_bytes(response).await, b"# Other\n");
}

/// Access is checked before the conditional response: a member whose share
/// is withdrawn gets no 304, whichever variant's ETag they hold.
#[tokio::test]
async fn revoked_access_denies_before_a_conditional_response() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let viewer = person("viewer");
    let id = publish_site(&app, &owner.token, json!({})).await.parcel.id;
    let share = format!("/api/v1/parcels/{id}/shares/{}", viewer.email);
    expect_status(
        app.put_json(&share, Some(&owner.token), &json!({"role": "viewer"}))
            .await,
        StatusCode::OK,
    )
    .await;
    let url = grant_url(&app, Some(&viewer.token), &id, "?path=docs/notes.md").await;
    let page_tag = assert_rendered(&app, app.get_content(&url, AS_PAGE).await).await;
    let raw_tag = assert_raw(app.get_content(&url, AS_FETCH).await).await;

    expect_status(
        app.delete(&share, Some(&owner.token)).await,
        StatusCode::NO_CONTENT,
    )
    .await;
    for (base, tag) in [(AS_PAGE, &page_tag), (AS_FETCH, &raw_tag)] {
        let denied = app
            .get_content(&url, &with(base, ("if-none-match", tag)))
            .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN, "{base:?}");
        assert_sandboxed_error(&denied);
    }
}

fn assert_sandboxed_error(response: &Response) {
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.starts_with("sandbox ") && !csp.contains("allow-same-origin"));
    assert!(response.headers().get("etag").is_none());
}

/// The SPA opens `/@handle/slug?page=…` by resolving the slug, then asking
/// for a grant by id: the two compose with nothing slug-specific in `view`.
#[tokio::test]
async fn a_page_opens_through_a_slug() {
    let app = TestApp::new().await;
    let owner = person("owner");
    publish_site(&app, &owner.token, json!({"slug": "parcel/pages-demo"})).await;
    let found: ParcelDetail = json(
        expect_status(
            app.get(
                "/api/v1/parcels/by-slug?slug=parcel/pages-demo",
                Some(&owner.token),
            )
            .await,
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(found.pages, site_pages());
    let url = grant_url(
        &app,
        Some(&owner.token),
        &found.parcel.id,
        "?path=rust-build-settings.html",
    )
    .await;
    expect_status(app.get_content(&url, &[]).await, StatusCode::OK).await;
}
