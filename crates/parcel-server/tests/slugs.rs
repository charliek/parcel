//! Handles, slugs and their redirects, by-slug resolution, publish upsert,
//! and version sources: what is stored, who sees what, and that the one
//! lock order keeps concurrent writers finishing.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::Response;
use common::*;
use parcel_server::db::{handles, parcels};
use parcel_server::publish::{self, VersionSource};
use parcel_types::{
    CreateParcelMetadata, ErrorBody, Me, NewVersionMetadata, ParcelDetail, ParcelList,
    ParcelSummary, PublishResult, SourceInfo, VersionDetail, VersionList, ViewGrant,
};
use serde_json::{json, Value};

const BY_SLUG_NOT_FOUND: &str = "no parcel at this address";

fn page(title: &str) -> String {
    format!("<!doctype html><title>{title}</title><p>{title} body</p>")
}

async fn publish(app: &TestApp, who: &Person, metadata: Value, html: &str) -> Response {
    let form = Form::new()
        .metadata(metadata)
        .file("index.html", html.to_string());
    app.post_multipart("/api/v1/parcels", Some(&who.token), &form)
        .await
}

async fn created(app: &TestApp, who: &Person, metadata: Value) -> PublishResult {
    expect_json(
        publish(app, who, metadata, &page("A page")).await,
        StatusCode::CREATED,
    )
    .await
}

async fn me(app: &TestApp, who: &Person) -> Me {
    expect_json(
        app.get("/api/v1/me", Some(&who.token)).await,
        StatusCode::OK,
    )
    .await
}

async fn patch(app: &TestApp, who: &Person, id: &str, body: Value) -> Response {
    app.patch_json(&format!("/api/v1/parcels/{id}"), Some(&who.token), &body)
        .await
}

async fn patched(app: &TestApp, who: &Person, id: &str, body: Value) -> ParcelSummary {
    expect_json(patch(app, who, id, body).await, StatusCode::OK).await
}

async fn rename_handle(app: &TestApp, who: &Person, handle: &str) -> Response {
    app.patch_json("/api/v1/me", Some(&who.token), &json!({ "handle": handle }))
        .await
}

async fn by_slug(app: &TestApp, token: Option<&str>, handle: &str, slug: &str) -> Response {
    let uri = format!("/api/v1/parcels/by-slug?handle={handle}&slug={slug}");
    app.get(&uri, token).await
}

async fn found(app: &TestApp, token: Option<&str>, handle: &str, slug: &str) -> ParcelDetail {
    expect_json(by_slug(app, token, handle, slug).await, StatusCode::OK).await
}

async fn detail(app: &TestApp, token: Option<&str>, id: &str) -> ParcelDetail {
    expect_json(
        app.get(&format!("/api/v1/parcels/{id}"), token).await,
        StatusCode::OK,
    )
    .await
}

async fn share(app: &TestApp, owner: &Person, id: &str, email: &str, role: &str) {
    expect_status(
        app.put_json(
            &format!("/api/v1/parcels/{id}/shares/{email}"),
            Some(&owner.token),
            &json!({ "role": role }),
        )
        .await,
        StatusCode::OK,
    )
    .await;
}

async fn message(response: Response, status: StatusCode) -> String {
    expect_json::<ErrorBody>(response, status)
        .await
        .error
        .message
}

async fn owner_id(app: &TestApp, parcel: &str) -> uuid::Uuid {
    sqlx::query_scalar("SELECT owner_id FROM parcels WHERE id = $1")
        .bind(parcel)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

// ---- handles ----------------------------------------------------------------

#[tokio::test]
async fn sign_in_gives_a_handle_from_the_email() {
    let app = TestApp::new().await;
    let grace = Person {
        token: token_for("sub-grace", "Grace.Hopper+navy@example.com"),
        email: "grace.hopper+navy@example.com".into(),
    };
    assert_eq!(me(&app, &grace).await.handle, "grace-hopper");
    // The same person again keeps it; a second Grace gets the next one.
    assert_eq!(me(&app, &grace).await.handle, "grace-hopper");
    let other = Person {
        token: token_for("sub-grace-2", "grace_hopper@other.example"),
        email: "grace_hopper@other.example".into(),
    };
    assert_eq!(me(&app, &other).await.handle, "grace-hopper-2");
}

/// A user an older revision created (mid-rollout) has no handle: the first
/// thing that needs one allocates it, and until then their parcels still
/// list (the handle join is an outer join) with `canonical_url` = `url`.
#[tokio::test]
async fn a_user_without_a_handle_gets_one_lazily() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let p = created(&app, &ada, json!({})).await.parcel;
    let owner = owner_id(&app, &p.id).await;
    sqlx::query("DELETE FROM user_handles WHERE user_id = $1")
        .bind(owner)
        .execute(&app.pool)
        .await
        .unwrap();
    // (Read straight from the database: every request through the test
    // router signs the user in again, which would allocate the handle.)
    let filter = parcels::ListFilter {
        scope: parcel_types::ListScope::Mine,
        user_id: owner,
        email: ada.email.clone(),
        tags: vec![],
        archived: parcel_types::ArchivedFilter::Exclude,
        query: None,
    };
    let (rows, total) = parcels::search(&app.pool, &filter, 50, 0).await.unwrap();
    assert_eq!(total, 1, "the handle join is an outer join");
    assert_eq!(rows[0].parcel.owner_handle, None);
    let summary = parcel_server::http::views::summary(
        &app.state.config,
        &rows[0].parcel,
        parcel_server::http::views::Audience::SignedIn,
    );
    assert_eq!(summary.canonical_url, summary.url);
    assert_eq!(summary.owner_handle, None);

    let handle = handles::ensure(&app.pool, owner).await.unwrap();
    assert!(handle.starts_with("ada-"), "{handle}");
    // `/me` (signing the same person in again) finds the same handle.
    assert_eq!(me(&app, &ada).await.handle, handle);
}

/// Concurrent allocations for one user converge on one handle; concurrent
/// allocations for users whose emails collide never share one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_allocations_converge_and_never_collide() {
    let app = TestApp::new().await;
    let mut users = Vec::new();
    for i in 0..8 {
        let id: uuid::Uuid =
            sqlx::query_scalar("INSERT INTO users (email, name) VALUES ($1, '') RETURNING id")
                .bind(format!("same.name+{i}@example.com"))
                .fetch_one(&app.pool)
                .await
                .unwrap();
        users.push(id);
    }
    let mut tasks = tokio::task::JoinSet::new();
    for &id in &users {
        for _ in 0..3 {
            let pool = app.pool.clone();
            tasks.spawn(async move { (id, handles::ensure(&pool, id).await.unwrap()) });
        }
    }
    let mut by_user = std::collections::BTreeMap::<uuid::Uuid, BTreeSet<String>>::new();
    while let Some(done) = tasks.join_next().await {
        let (id, handle) = done.unwrap();
        by_user.entry(id).or_default().insert(handle);
    }
    let all: BTreeSet<String> = by_user
        .values()
        .map(|h| {
            assert_eq!(h.len(), 1, "one user, one handle: {h:?}");
            h.first().unwrap().clone()
        })
        .collect();
    assert_eq!(all.len(), users.len(), "{all:?}");
    let expected: BTreeSet<String> = std::iter::once("same-name".to_string())
        .chain((2..=8).map(|n| format!("same-name-{n}")))
        .collect();
    assert_eq!(all, expected);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM user_handles")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 8);
}

#[tokio::test]
async fn patch_me_renames_and_old_handles_stay_reserved() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let bob = person("bob");
    let first = me(&app, &ada).await.handle;
    let bobs = me(&app, &bob).await.handle;
    let p = created(&app, &ada, json!({"slug": "plans/q3"}))
        .await
        .parcel;

    for bad in ["-ada", "ada-", "a_b", "a.b", "@ada", "", &"a".repeat(33)] {
        let msg = message(
            rename_handle(&app, &ada, bad).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert!(msg.contains("handle"), "{bad:?}: {msg}");
    }
    assert!(
        message(rename_handle(&app, &ada, &bobs).await, StatusCode::CONFLICT)
            .await
            .contains("taken")
    );
    // A rename to the current handle is a no-op.
    let same: Me = expect_json(rename_handle(&app, &ada, &first).await, StatusCode::OK).await;
    assert_eq!(same.handle, first);

    let renamed: Me = expect_json(
        rename_handle(&app, &ada, " Ada-Lovelace ").await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(renamed.handle, "ada-lovelace");
    assert_eq!(me(&app, &ada).await.handle, "ada-lovelace");
    // Old links keep working, and point at the new clean URL.
    let via_old = found(&app, Some(&ada.token), &first, "plans/q3").await;
    assert_eq!(via_old.parcel.id, p.id);
    assert_eq!(
        via_old.parcel.canonical_url,
        format!("{APP_ORIGIN}/@ada-lovelace/plans/q3")
    );
    assert_eq!(via_old.parcel.owner_handle.as_deref(), Some("ada-lovelace"));
    // Nobody else can take the old handle (or the new one).
    for taken in [first.as_str(), "ada-lovelace"] {
        assert_eq!(
            rename_handle(&app, &bob, taken).await.status(),
            StatusCode::CONFLICT,
            "{taken}"
        );
    }
    // Taking an old handle back makes it current again.
    let back: Me = expect_json(rename_handle(&app, &ada, &first).await, StatusCode::OK).await;
    assert_eq!(back.handle, first);
    let rows: BTreeSet<(String, bool)> =
        sqlx::query_as("SELECT handle, is_current FROM user_handles WHERE user_id = $1")
            .bind(owner_id(&app, &p.id).await)
            .fetch_all(&app.pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert_eq!(
        rows,
        BTreeSet::from([("ada-lovelace".to_string(), false), (first.clone(), true)])
    );
    // An empty body changes nothing.
    let unchanged: Me = expect_json(
        app.patch_json("/api/v1/me", Some(&ada.token), &json!({}))
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(unchanged.handle, first);
}

/// A user straight from the database, as an older revision created them:
/// no handle yet.
async fn handleless_user(app: &TestApp, email: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO users (email, name) VALUES ($1, '') RETURNING id")
        .bind(email)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn current_handle(app: &TestApp, user: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT handle FROM user_handles WHERE user_id = $1 AND is_current")
        .bind(user)
        .fetch_optional(&app.pool)
        .await
        .unwrap()
}

/// A handle-less user's first rename, to the very handle they'd be given:
/// the allocation is kept (it is the answer), not rolled back for someone
/// else to take before `/me` allocates again.
#[tokio::test]
async fn a_first_rename_to_the_allocated_handle_keeps_it() {
    let app = TestApp::new().await;
    let id = handleless_user(&app, "first.rename@example.com").await;
    let renamed = handles::rename(&app.pool, id, "first.rename@example.com", "first-rename")
        .await
        .unwrap();
    assert_eq!(renamed, Ok(None), "already the current handle");
    assert_eq!(
        current_handle(&app, id).await.as_deref(),
        Some("first-rename")
    );
}

/// Handle-less users asking at once for each other's would-be handle: each
/// rename must finish (taken or renamed), never deadlock on the other's
/// uncommitted allocation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn crossed_first_renames_never_deadlock() {
    let app = Arc::new(TestApp::new().await);
    for round in 0..10 {
        let mut tasks = tokio::task::JoinSet::new();
        let mut users = Vec::new();
        for pair in 0..4 {
            let names = [format!("x{round}p{pair}a"), format!("x{round}p{pair}b")];
            let ids = [
                handleless_user(&app, &format!("{}@example.com", names[0])).await,
                handleless_user(&app, &format!("{}@example.com", names[1])).await,
            ];
            for (me, other) in [(0, 1), (1, 0)] {
                let (app, id) = (app.clone(), ids[me]);
                let email = format!("{}@example.com", names[me]);
                let wanted = names[other].clone();
                tasks.spawn(async move {
                    handles::rename(&app.pool, id, &email, &wanted)
                        .await
                        .map_err(|e| format!("{e:#}"))
                });
            }
            users.extend(ids);
        }
        while let Some(done) = tasks.join_next().await {
            let outcome = done.unwrap();
            assert!(outcome.is_ok(), "round {round}: {outcome:?}");
        }
        let mut current = BTreeSet::new();
        for id in users {
            let handle = current_handle(&app, id).await;
            assert!(handle.is_some(), "every user has a current handle");
            current.insert(handle);
        }
        assert_eq!(current.len(), 8, "{current:?}");
    }
}

/// `PATCH /me` with a body of the wrong shape is refused like every other
/// typed JSON body in this API (axum's `Json` rejection: 422).
#[tokio::test]
async fn patch_me_refuses_mistyped_bodies() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let before = me(&app, &ada).await.handle;
    for body in [json!({"handle": 123}), json!({"unexpected": true})] {
        let r = app.patch_json("/api/v1/me", Some(&ada.token), &body).await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    assert_eq!(me(&app, &ada).await.handle, before);
}

// ---- slugs --------------------------------------------------------------------

#[tokio::test]
async fn slug_rules_and_clashes() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let handle = me(&app, &ada).await.handle;
    let a = created(&app, &ada, json!({"title": "A"})).await.parcel;
    let b = created(&app, &ada, json!({"title": "B"})).await.parcel;
    assert_eq!(
        (a.slug.as_deref(), a.canonical_url.as_str()),
        (None, a.url.as_str())
    );

    for bad in [
        "p",
        "v",
        "plans/v/2",
        "p/abcdefghijkm",
        "abcdefghijkm",
        "plans/abcdefghijkm",
        "a/b/c/d/e",
        "-x",
        "x_y",
        "/x",
        &"x".repeat(101),
    ] {
        let msg = message(
            patch(&app, &ada, &a.id, json!({ "slug": bad })).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert!(msg.contains("slug"), "{bad:?}: {msg}");
        assert!(
            message(
                publish(&app, &ada, json!({ "slug": bad }), &page("x")).await,
                StatusCode::BAD_REQUEST
            )
            .await
            .contains("slug"),
            "{bad:?}"
        );
    }

    let set = patched(&app, &ada, &a.id, json!({"slug": " Plans/Q3 "})).await;
    assert_eq!(set.slug.as_deref(), Some("plans/q3"));
    assert_eq!(
        set.url,
        format!("{APP_ORIGIN}/p/{}", a.id),
        "url stays /p/<id>"
    );
    assert_eq!(
        set.canonical_url,
        format!("{APP_ORIGIN}/@{handle}/plans/q3")
    );
    assert_eq!(set.updated_at, a.updated_at, "naming isn't editing");
    // `q` matches slugs by substring.
    let hits: ParcelList = expect_json(
        app.get("/api/v1/parcels?q=plans/q", Some(&ada.token)).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        hits.parcels
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        [a.id.as_str()]
    );

    // Unique per owner: 409, and nothing changed.
    let msg = message(
        patch(
            &app,
            &ada,
            &b.id,
            json!({"slug": "plans/q3", "title": "B2"}),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert!(msg.contains("already"), "{msg}");
    let b_now = detail(&app, Some(&ada.token), &b.id).await.parcel;
    assert_eq!((b_now.slug, b_now.title.as_str()), (None, "B"));
    // Setting it again on the same parcel is fine.
    patched(&app, &ada, &a.id, json!({"slug": "plans/q3"})).await;
    // Another owner may use the same slug.
    let bob = person("bob");
    let bobs = created(&app, &bob, json!({"slug": "plans/q3"})).await;
    assert_eq!(bobs.parcel.slug.as_deref(), Some("plans/q3"));

    // `""` clears it; the old name keeps redirecting.
    let cleared = patched(&app, &ada, &a.id, json!({"slug": ""})).await;
    assert_eq!(cleared.slug, None);
    assert_eq!(cleared.canonical_url, cleared.url);
    assert_eq!(
        found(&app, Some(&ada.token), &handle, "plans/q3")
            .await
            .parcel
            .id,
        a.id
    );

    // Only the owner may name it.
    share(&app, &ada, &a.id, &bob.email, "commenter").await;
    assert_eq!(
        patch(&app, &bob, &a.id, json!({"slug": "mine"}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

/// A5.4: renamed slugs and handles resolve to the current clean URL; a slug
/// another of the owner's parcels takes stops redirecting to the first.
#[tokio::test]
async fn renamed_slugs_and_handles_redirect() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let old_handle = me(&app, &ada).await.handle;
    let a = created(&app, &ada, json!({"slug": "draft"})).await.parcel;
    patched(&app, &ada, &a.id, json!({"slug": "final"})).await;
    patched(&app, &ada, &a.id, json!({"slug": "final-v2"})).await;
    for asked in ["draft", "final", "final-v2"] {
        let d = found(&app, Some(&ada.token), &old_handle, asked).await;
        assert_eq!(d.parcel.id, a.id, "{asked}");
        assert_eq!(
            d.parcel.canonical_url,
            format!("{APP_ORIGIN}/@{old_handle}/final-v2")
        );
    }
    expect_json::<Me>(rename_handle(&app, &ada, "ada-l").await, StatusCode::OK).await;
    let d = found(&app, Some(&ada.token), &old_handle, "draft").await;
    assert_eq!(
        d.parcel.canonical_url,
        format!("{APP_ORIGIN}/@ada-l/final-v2")
    );

    // Another parcel takes "draft" over: the redirect row goes.
    let b = created(&app, &ada, json!({})).await.parcel;
    patched(&app, &ada, &b.id, json!({"slug": "draft"})).await;
    assert_eq!(
        found(&app, Some(&ada.token), "ada-l", "draft")
            .await
            .parcel
            .id,
        b.id
    );
    let redirects: Vec<(String, String)> =
        sqlx::query_as("SELECT slug, parcel_id FROM parcel_slug_redirects ORDER BY slug")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(redirects, [("final".to_string(), a.id.clone())]);
    // Taking back your own old name on the same parcel removes its redirect too.
    patched(&app, &ada, &a.id, json!({"slug": "final"})).await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM parcel_slug_redirects")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 1, "only final-v2 → a");

    // Deleting a parcel deletes its redirects.
    expect_status(
        app.delete(&format!("/api/v1/parcels/{}", a.id), Some(&ada.token))
            .await,
        StatusCode::NO_CONTENT,
    )
    .await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM parcel_slug_redirects")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(
        message(
            by_slug(&app, Some(&ada.token), "ada-l", "final-v2").await,
            StatusCode::NOT_FOUND
        )
        .await,
        BY_SLUG_NOT_FOUND
    );
}

/// The headers of a response, minus the ones that differ per request.
fn stable_headers(r: &Response) -> Vec<(String, String)> {
    let skip = ["date", "x-request-id"];
    let headers: &HeaderMap = r.headers();
    let mut out: Vec<(String, String)> = headers
        .iter()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect();
    out.sort();
    out
}

/// P17: every by-slug miss — unknown handle, unknown slug, no access, public
/// links off, malformed input — is the same 404 (status, body, headers);
/// whoever may read the parcel finds it through current and old handles and
/// slugs alike.
#[tokio::test]
async fn by_slug_misses_are_identical_404s() {
    for allow_public in [true, false] {
        let app = TestApp::with_config(|c| c.allow_public = allow_public).await;
        let ada = person("ada");
        let viewer = person("vic");
        let commenter = person("cora");
        let stranger = person("sam");
        let old_handle = me(&app, &ada).await.handle;
        let private = created(&app, &ada, json!({"slug": "old-private"}))
            .await
            .parcel;
        patched(&app, &ada, &private.id, json!({"slug": "private"})).await;
        share(&app, &ada, &private.id, &viewer.email, "viewer").await;
        share(&app, &ada, &private.id, &commenter.email, "commenter").await;
        let internal = created(
            &app,
            &ada,
            json!({"slug": "internal", "visibility": "internal"}),
        )
        .await;
        let public = created(&app, &ada, json!({"slug": "public"})).await.parcel;
        // Stored public (set behind the API when public links are off).
        sqlx::query("UPDATE parcels SET visibility = 'public' WHERE id = $1")
            .bind(&public.id)
            .execute(&app.pool)
            .await
            .unwrap();
        expect_json::<Me>(rename_handle(&app, &ada, "ada-new").await, StatusCode::OK).await;

        let anon: Option<&str> = None;
        let (ada_t, viewer_t, commenter_t, stranger_t) = (
            Some(ada.token.as_str()),
            Some(viewer.token.as_str()),
            Some(commenter.token.as_str()),
            Some(stranger.token.as_str()),
        );
        let old = old_handle.as_str();
        let cases: Vec<(&str, Option<&str>, &str, &str, bool)> = vec![
            // (who, token, handle, slug, finds it)
            ("owner", ada_t, "ada-new", "private", true),
            ("owner old handle", ada_t, old, "private", true),
            ("owner old slug", ada_t, "ada-new", "old-private", true),
            ("viewer", viewer_t, "ada-new", "private", true),
            ("viewer old names", viewer_t, old, "old-private", true),
            ("commenter", commenter_t, "ada-new", "private", true),
            ("stranger", stranger_t, "ada-new", "private", false),
            ("stranger old names", stranger_t, old, "old-private", false),
            ("anonymous", anon, "ada-new", "private", false),
            ("stranger internal", stranger_t, "ada-new", "internal", true),
            ("anonymous internal", anon, "ada-new", "internal", false),
            ("anonymous public", anon, old, "public", allow_public),
            ("stranger public", stranger_t, "ada-new", "public", true),
            ("unknown slug", ada_t, "ada-new", "nothing-here", false),
            ("unknown handle", ada_t, "nobody-at-all", "private", false),
            ("bad slug", ada_t, "ada-new", "p", false),
            ("bad handle", anon, "-x-", "public", false),
            ("id as slug", ada_t, "ada-new", private.id.as_str(), false),
        ];
        let mut misses = Vec::new();
        for (who, token, handle, slug, finds) in cases {
            let r = by_slug(&app, token, handle, slug).await;
            if finds {
                let d: ParcelDetail = expect_json(r, StatusCode::OK).await;
                assert!(
                    [&private.id, &internal.parcel.id, &public.id].contains(&&d.parcel.id),
                    "{who}"
                );
                assert!(d.parcel.canonical_url.contains("/@ada-new/"), "{who}");
            } else {
                assert_eq!(r.status(), StatusCode::NOT_FOUND, "{who} ({allow_public})");
                let headers = stable_headers(&r);
                let body = String::from_utf8(body_bytes(r).await).unwrap();
                misses.push((who, headers, body));
            }
        }
        let (_, first_headers, first_body) = &misses[0];
        assert!(first_body.contains(BY_SLUG_NOT_FOUND), "{first_body}");
        for (who, headers, body) in &misses {
            assert_eq!(body, first_body, "{who}");
            assert_eq!(headers, first_headers, "{who}");
        }

        // Signed in, the handle defaults to the caller's own; signed out it's needed.
        let mine: ParcelDetail = expect_json(
            app.get("/api/v1/parcels/by-slug?slug=private", Some(&ada.token))
                .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(mine.parcel.id, private.id);
        expect_status(
            app.get("/api/v1/parcels/by-slug?slug=public", None).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        // The viewer reading a readable parcel still can't change it.
        assert_eq!(
            patch(&app, &viewer, &private.id, json!({"title": "x"}))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
}

/// P34: `/parcels/by-slug` is its own route, never `/parcels/{id}` with id
/// "by-slug".
#[tokio::test]
async fn by_slug_is_not_routed_as_a_parcel_id() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let msg = message(
        app.get("/api/v1/parcels/by-slug", Some(&ada.token)).await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(msg.contains("slug"), "{msg}");
    for method in [Method::DELETE, Method::PATCH] {
        let r = app
            .send(
                Request::builder()
                    .method(method.clone())
                    .uri("/api/v1/parcels/by-slug")
                    .header("host", "parcel.test")
                    .header("authorization", format!("Bearer {}", ada.token))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await;
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED, "{method}");
    }
}

/// P18: anonymous callers learn the owner's handle only from a slugged
/// parcel (whose clean URL holds it anyway); signed-in callers always.
#[tokio::test]
async fn anonymous_callers_get_no_handle_for_unslugged_parcels() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let bob = person("bob");
    let handle = me(&app, &ada).await.handle;
    let plain = created(&app, &ada, json!({"visibility": "public"}))
        .await
        .parcel;
    let named = created(&app, &ada, json!({"visibility": "public", "slug": "named"}))
        .await
        .parcel;

    let raw = String::from_utf8(
        body_bytes(
            app.get(&format!("/api/v1/parcels/{}", plain.id), None)
                .await,
        )
        .await,
    )
    .unwrap();
    assert!(
        !raw.contains("owner_handle") && !raw.contains(&handle),
        "{raw}"
    );
    let raw = String::from_utf8(
        body_bytes(
            app.get(&format!("/api/v1/parcels/{}/view", plain.id), None)
                .await,
        )
        .await,
    )
    .unwrap();
    assert!(!raw.contains(&handle), "{raw}");

    let anon_named = detail(&app, None, &named.id).await.parcel;
    assert_eq!(anon_named.owner_handle.as_deref(), Some(handle.as_str()));
    assert_eq!(
        anon_named.canonical_url,
        format!("{APP_ORIGIN}/@{handle}/named")
    );
    let bob_plain = detail(&app, Some(&bob.token), &plain.id).await.parcel;
    assert_eq!(bob_plain.owner_handle.as_deref(), Some(handle.as_str()));
}

// ---- publish upsert -------------------------------------------------------------

/// A6.1 and P13: a second publish with the slug versions the same parcel
/// (200, `created: false`, the committed version), replacing its files and
/// changing only what was sent; tags are added, auto tags never come back.
#[tokio::test]
async fn publishing_twice_with_a_slug_versions_one_parcel() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let form = Form::new()
        .metadata(
            json!({"slug": "plans/q3", "visibility": "internal", "description": "d1",
                         "tags": ["kind:plan"], "auto_tags": ["project:parcel"]}),
        )
        .file("index.html", page("First"))
        .file("extra.css", "p{}");
    let first: PublishResult = expect_json(
        app.post_multipart("/api/v1/parcels", Some(&ada.token), &form)
            .await,
        StatusCode::CREATED,
    )
    .await;
    assert!(first.created);
    let id = first.parcel.id.clone();
    assert_eq!(first.parcel.title, "First");
    assert_eq!(first.parcel.tags, ["kind:plan", "project:parcel"]);
    // The owner removes the auto tag and archives it.
    patched(
        &app,
        &ada,
        &id,
        json!({"remove_tags": ["project:parcel"], "archived": true}),
    )
    .await;

    let second: PublishResult = expect_json(
        publish(
            &app,
            &ada,
            json!({"slug": "Plans/Q3", "tags": ["topic"], "auto_tags": ["project:parcel"],
                   "label": "second"}),
            &page("Second title in the page"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert!(!second.created);
    assert_eq!(second.parcel.id, id);
    assert_eq!(second.version.summary.version, 2);
    assert_eq!(second.parcel.current_version, 2);
    assert_eq!(second.version.summary.label.as_deref(), Some("second"));
    let p = &second.parcel;
    assert_eq!(p.title, "First", "the title isn't derived on an upsert");
    assert_eq!(
        p.visibility,
        parcel_types::Visibility::Internal,
        "not defaulted"
    );
    assert_eq!(p.description, "d1");
    assert_eq!(
        p.tags,
        ["kind:plan", "topic"],
        "auto tags apply only on create"
    );
    assert!(p.archived_at.is_none(), "a new version un-archives");
    let paths: Vec<&str> = second
        .version
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(paths, ["index.html"], "an upsert replaces the files");

    // Sent fields do change.
    let third: PublishResult = expect_json(
        publish(
            &app,
            &ada,
            json!({"slug": "plans/q3", "title": "Renamed", "visibility": "private",
                   "description": ""}),
            &page("x"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(third.version.summary.version, 3);
    assert_eq!(
        (
            third.parcel.title.as_str(),
            third.parcel.visibility,
            third.parcel.description.as_str()
        ),
        ("Renamed", parcel_types::Visibility::Private, "")
    );
    let mine: ParcelList = expect_json(
        app.get("/api/v1/parcels", Some(&ada.token)).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(mine.total, 1, "one parcel");
    // Entry rules of a version apply: the previous entry is kept.
    let form = Form::new()
        .metadata(json!({"slug": "plans/q3"}))
        .file("a.html", "a")
        .file("index.html", "i")
        .file("z.html", "z");
    let fourth: PublishResult = expect_json(
        app.post_multipart("/api/v1/parcels", Some(&ada.token), &form)
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fourth.version.summary.entry, "index.html");
}

#[tokio::test]
async fn upsert_base_version_rules() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let msg = message(
        publish(&app, &ada, json!({"base_version": 1}), &page("x")).await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(msg.contains("base_version"), "{msg}");
    let msg = message(
        publish(
            &app,
            &ada,
            json!({"slug": "new", "base_version": 1}),
            &page("x"),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert!(msg.contains("no parcel has the slug"), "{msg}");
    created(&app, &ada, json!({"slug": "new"})).await;
    let msg = message(
        publish(
            &app,
            &ada,
            json!({"slug": "new", "base_version": 2}),
            &page("x"),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;
    assert!(msg.contains("version 1"), "{msg}");
    let ok: PublishResult = expect_json(
        publish(
            &app,
            &ada,
            json!({"slug": "new", "base_version": 1}),
            &page("x"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(ok.version.summary.version, 2);
}

/// `auto_tags` count only when the publish creates the parcel, so they're
/// checked only then: an upsert carrying an extra or invalid auto tag
/// versions the parcel. Explicit tags are added, and the result is capped.
#[tokio::test]
async fn upserts_ignore_auto_tags_and_cap_the_merged_tags() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let twenty: Vec<String> = (1..=20).map(|i| format!("t{i:02}")).collect();
    created(&app, &ada, json!({"slug": "full", "tags": twenty})).await;

    for auto in [json!(["project:parcel"]), json!(["Not A Tag!"])] {
        let r: PublishResult = expect_json(
            publish(
                &app,
                &ada,
                json!({"slug": "full", "tags": twenty, "auto_tags": auto}),
                &page("x"),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(r.parcel.tags, twenty, "{auto}");
    }
    let msg = message(
        publish(
            &app,
            &ada,
            json!({"slug": "full", "tags": ["t21"]}),
            &page("x"),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(msg.contains("at most 20 tags"), "{msg}");

    // Creating, the auto tags count: checked and capped with the rest.
    for (slug, auto) in [
        (Some("new-a"), json!(["Not A Tag!"])),
        (Some("new-b"), json!(["project:parcel"])),
        (None, json!(["project:parcel"])),
    ] {
        let mut meta = json!({"tags": twenty, "auto_tags": auto});
        if let Some(slug) = slug {
            meta["slug"] = json!(slug);
        }
        let r = publish(&app, &ada, meta.clone(), &page("x")).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{meta}");
    }
}

/// P31: a publish to a slug the caller renamed away from versions the parcel
/// it now redirects to; PATCH is how an old name goes to a new parcel.
#[tokio::test]
async fn upsert_follows_the_callers_redirects() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let a = created(&app, &ada, json!({"slug": "old-name"}))
        .await
        .parcel;
    patched(&app, &ada, &a.id, json!({"slug": "new-name"})).await;
    let again: PublishResult = expect_json(
        publish(&app, &ada, json!({"slug": "old-name"}), &page("x")).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(again.parcel.id, a.id);
    assert_eq!(
        again.parcel.slug.as_deref(),
        Some("new-name"),
        "the slug isn't changed"
    );

    let b = created(&app, &ada, json!({})).await.parcel;
    patched(&app, &ada, &b.id, json!({"slug": "old-name"})).await;
    let to_b: PublishResult = expect_json(
        publish(&app, &ada, json!({"slug": "old-name"}), &page("x")).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(to_b.parcel.id, b.id);

    // Someone else's redirect is no business of this caller's upsert.
    let bob = person("bob");
    let bobs = created(&app, &bob, json!({"slug": "new-name"})).await;
    assert!(bobs.created && bobs.parcel.id != a.id);
}

/// Two first publishes of one slug at once: one creates, the other versions it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_upserts_create_one_parcel() {
    let app = Arc::new(TestApp::new().await);
    let ada = Arc::new(person("ada"));
    for round in 0..3 {
        let slug = format!("race-{round}");
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..2 {
            let (app, ada, slug) = (app.clone(), ada.clone(), slug.clone());
            tasks.spawn(async move {
                let r = publish(&app, &ada, json!({ "slug": slug }), &page(&format!("p{i}"))).await;
                let status = r.status();
                let result: PublishResult = json(r).await;
                (status, result)
            });
        }
        let mut statuses = Vec::new();
        let mut ids = BTreeSet::new();
        while let Some(done) = tasks.join_next().await {
            let (status, result) = done.unwrap();
            statuses.push(status);
            ids.insert(result.parcel.id);
        }
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::CREATED], "{slug}");
        assert_eq!(ids.len(), 1, "{slug}");
        let d = detail(&app, Some(&ada.token), ids.first().unwrap()).await;
        assert_eq!(d.versions.len(), 2);
    }
}

/// The id-retry loop retries only on the parcel primary key, named; the
/// slug index is the other unique constraint an insert can hit (a 409).
#[tokio::test]
async fn constraint_names_decide_retry_or_conflict() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let a = created(&app, &ada, json!({"slug": "taken"})).await.parcel;
    let owner = owner_id(&app, &a.id).await;
    let attempt = |id: String, slug: Option<&'static str>| {
        let pool = app.pool.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            let e = parcels::insert_parcel(
                &mut tx,
                &parcels::NewParcel {
                    id: &id,
                    owner_id: owner,
                    title: "t",
                    description: "",
                    visibility: parcel_types::Visibility::Private,
                    tags: &[],
                    slug,
                },
            )
            .await
            .unwrap_err();
            parcels::violated_constraint(&e).map(str::to_owned)
        }
    };
    assert_eq!(
        attempt(a.id.clone(), None).await.as_deref(),
        Some(parcels::PARCEL_PKEY)
    );
    assert_eq!(
        attempt("zzzzzzzzzzzz".into(), Some("taken"))
            .await
            .as_deref(),
        Some(parcels::PARCEL_SLUG_INDEX)
    );
}

fn upload(text: &str) -> Vec<publish::Upload> {
    publish::collect_uploads(
        vec![("index.html".into(), bytes::Bytes::from(text.to_string()))],
        &parcels_limits(),
    )
    .unwrap()
}

fn parcels_limits() -> parcel_types::Limits {
    test_config("http://127.0.0.1:9").limits
}

/// One round of every writer that takes both locks, on one parcel: an
/// upsert, a plain new version, a restore and a slug rename.
async fn every_writer(app: &Arc<TestApp>, owner: uuid::Uuid, id: &str, slug: &str, i: usize) {
    let upsert = CreateParcelMetadata {
        slug: Some(slug.to_string()),
        ..CreateParcelMetadata::default()
    };
    let rename = format!("{slug}-{i}");
    let mut tasks = tokio::task::JoinSet::new();
    {
        let app = app.clone();
        tasks.spawn(async move {
            publish::create(&app.state, owner, upsert, upload("<p>upsert</p>"))
                .await
                .map(|_| ())
        });
    }
    {
        let (app, id) = (app.clone(), id.to_string());
        tasks.spawn(async move {
            publish::add_version(
                &app.state,
                &id,
                owner,
                VersionSource::Upload {
                    meta: NewVersionMetadata::default(),
                    uploads: upload("<p>plain</p>"),
                },
            )
            .await
            .map(|_| ())
        });
    }
    {
        let (app, id) = (app.clone(), id.to_string());
        tasks.spawn(async move {
            publish::add_version(
                &app.state,
                &id,
                owner,
                VersionSource::Restore { from_version: 1 },
            )
            .await
            .map(|_| ())
        });
    }
    {
        let (app, id) = (app.clone(), id.to_string());
        tasks.spawn(async move {
            let patch = parcels::Patch {
                slug: Some(Some(&rename)),
                ..parcels::Patch::default()
            };
            parcels::apply_patch(&app.state.db, owner, &id, &patch)
                .await
                .map_err(parcel_server::error::AppError::Internal)?
                .map_err(|e| parcel_server::error::AppError::BadRequest(format!("{e:?}")))?;
            // The next round's upsert of `slug` follows the redirect this leaves.
            Ok(())
        });
    }
    while let Some(done) = tasks.join_next().await {
        done.expect("writer panicked").expect("writer failed");
    }
}

async fn within_a_minute<F: std::future::Future>(what: &str, fut: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .unwrap_or_else(|_| panic!("{what}: the writers didn't finish (deadlock?)"))
}

/// P1: with one lock order everywhere (owner, then row), the writers that
/// take both always finish — while another transaction holds the row lock,
/// while one holds the owner's lock, and all at once — and never deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writers_finish_under_forced_interleavings() {
    let app = Arc::new(TestApp::new().await);
    let ada = person("ada");
    let p = created(&app, &ada, json!({"slug": "busy"})).await.parcel;
    let owner = owner_id(&app, &p.id).await;

    // Someone holds the parcel's row lock (as a comment write does).
    let mut holder = app.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM parcels WHERE id = $1 FOR UPDATE")
        .bind(&p.id)
        .execute(&mut *holder)
        .await
        .unwrap();
    let writers = {
        let (app, id) = (app.clone(), p.id.clone());
        tokio::spawn(async move { every_writer(&app, owner, &id, "busy", 0).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!writers.is_finished(), "the writers wait for the held lock");
    holder.commit().await.unwrap();
    within_a_minute("row lock held", writers).await.unwrap();

    // Someone holds the owner's lock (as a tag rename does).
    let mut holder = app.pool.begin().await.unwrap();
    parcels::lock_owner_quota(&mut holder, owner).await.unwrap();
    let writers = {
        let (app, id) = (app.clone(), p.id.clone());
        tokio::spawn(async move { every_writer(&app, owner, &id, "busy", 1).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!writers.is_finished(), "the writers wait for the held lock");
    holder.commit().await.unwrap();
    within_a_minute("owner lock held", writers).await.unwrap();

    // Everything at once, several times over.
    let mut rounds = tokio::task::JoinSet::new();
    for i in 2..8 {
        let (app, id) = (app.clone(), p.id.clone());
        rounds.spawn(async move { every_writer(&app, owner, &id, "busy", i).await });
    }
    within_a_minute("all at once", async move {
        while let Some(done) = rounds.join_next().await {
            done.unwrap();
        }
    })
    .await;

    // Every upsert found the slug (current or a redirect): still one parcel.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM parcels WHERE owner_id = $1")
        .bind(owner)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // 8 rounds × (upsert + version + restore) on top of version 1.
    assert_eq!(
        detail(&app, Some(&ada.token), &p.id).await.versions.len(),
        1 + 8 * 3
    );
}

// ---- sources ------------------------------------------------------------------

const CANARY: &str = "canarycred7731";

/// A6.2 / P15: a version's source is stored sanitized, and only the owner
/// sees it — in every response that carries versions.
#[tokio::test]
async fn source_is_stored_sanitized_and_shown_to_the_owner_only() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let vic = person("vic");
    let source = json!({
        "repo": format!("https://ada:{CANARY}@GitHub.com/charliek/parcel.git"),
        "branch": "main", "commit": "abc123def456", "host": "laptop",
        "client": "parcel-cli/0.2.0",
    });
    let published = created(
        &app,
        &ada,
        json!({"slug": "src", "visibility": "public", "source": source}),
    )
    .await;
    let id = published.parcel.id.clone();
    let expected = SourceInfo {
        repo: Some("github.com/charliek/parcel".into()),
        branch: Some("main".into()),
        commit: Some("abc123def456".into()),
        host: Some("laptop".into()),
        client: Some("parcel-cli/0.2.0".into()),
    };
    assert_eq!(published.version.summary.source.as_ref(), Some(&expected));
    let stored: Value =
        sqlx::query_scalar("SELECT source FROM parcel_versions WHERE parcel_id = $1")
            .bind(&id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(!stored.to_string().contains(CANARY), "{stored}");
    share(&app, &ada, &id, &vic.email, "commenter").await;

    let handle = me(&app, &ada).await.handle;
    let paths = [
        format!("/api/v1/parcels/{id}"),
        format!("/api/v1/parcels/{id}/versions"),
        format!("/api/v1/parcels/{id}/versions/1"),
        format!("/api/v1/parcels/{id}/versions/latest"),
        format!("/api/v1/parcels/{id}/view"),
        format!("/api/v1/parcels/by-slug?handle={handle}&slug=src"),
    ];
    for path in &paths {
        let owner_raw =
            String::from_utf8(body_bytes(app.get(path, Some(&ada.token)).await).await).unwrap();
        assert!(
            owner_raw.contains("github.com/charliek/parcel"),
            "{path}: {owner_raw}"
        );
        for (who, token) in [("viewer", Some(vic.token.as_str())), ("anonymous", None)] {
            let r = app.get(path, token).await;
            assert_eq!(r.status(), StatusCode::OK, "{who} {path}");
            let raw = String::from_utf8(body_bytes(r).await).unwrap();
            assert!(!raw.contains("\"source\""), "{who} {path}: {raw}");
            assert!(!raw.contains("charliek/parcel"), "{who} {path}: {raw}");
        }
    }
    // Typed, for the owner: the version list and a version keep it.
    let versions: VersionList = expect_json(
        app.get(&format!("/api/v1/parcels/{id}/versions"), Some(&ada.token))
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(versions.versions[0].source.as_ref(), Some(&expected));
    let grant: ViewGrant = expect_json(
        app.get(&format!("/api/v1/parcels/{id}/view"), Some(&vic.token))
            .await,
        StatusCode::OK,
    )
    .await;
    assert!(grant.version.source.is_none());

    // A new version records its own source; one without has none.
    let form = Form::new()
        .metadata(json!({"source": {"client": "web"}}))
        .file("index.html", "<p>2</p>");
    let v2: PublishResult = expect_json(
        app.post_multipart(
            &format!("/api/v1/parcels/{id}/versions"),
            Some(&ada.token),
            &form,
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(
        v2.version.summary.source.unwrap().client.as_deref(),
        Some("web")
    );
    let form = Form::new().file("index.html", "<p>3</p>");
    let v3: PublishResult = expect_json(
        app.post_multipart(
            &format!("/api/v1/parcels/{id}/versions"),
            Some(&ada.token),
            &form,
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert!(v3.version.summary.source.is_none());

    // A restore is the restored version again, source included.
    let restored: PublishResult = expect_json(
        app.post_json(
            &format!("/api/v1/parcels/{id}/versions/1/restore"),
            Some(&ada.token),
            &json!({}),
        )
        .await,
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(restored.version.summary.source.as_ref(), Some(&expected));
    let v4: VersionDetail = expect_json(
        app.get(
            &format!("/api/v1/parcels/{id}/versions/4"),
            Some(&ada.token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(v4.summary.source, Some(expected));
}

/// A credential in a repo URL never reaches a response, an error or the
/// logs: stripped when it is userinfo, refused (value-free) otherwise.
#[tokio::test]
async fn source_credentials_never_reach_responses_or_logs() {
    let app = TestApp::new().await;
    let ada = person("ada");
    let logs = Captured::default();
    let _guard = logs.install();
    let mut seen = String::new();

    let mut first = None;
    for repo in [
        format!("https://x-token:{CANARY}@github.com/o/r"),
        format!("git@github.com:o/r.git?token={CANARY}#{CANARY}"),
        format!("git:{CANARY}@github.com:o/r#x"),
    ] {
        let stripped = publish(&app, &ada, json!({"source": {"repo": repo}}), &page("x")).await;
        assert_eq!(stripped.status(), StatusCode::CREATED);
        let body = String::from_utf8(body_bytes(stripped).await).unwrap();
        let result: PublishResult = serde_json::from_str(&body).unwrap();
        assert_eq!(
            result
                .version
                .summary
                .source
                .as_ref()
                .unwrap()
                .repo
                .as_deref(),
            Some("github.com/o/r")
        );
        seen.push_str(&body);
        first.get_or_insert(result);
    }
    let result = first.unwrap();

    for (metadata, field) in [
        (
            json!({"source": {"repo": format!("https://github.com:{CANARY}/o/r")}}),
            "source.repo",
        ),
        (
            json!({"source": {"repo": format!("git://{CANARY}@host/o/r")}}),
            "source.repo",
        ),
        (
            json!({"source": {"repo": format!("https://host/o/{CANARY}@r")}}),
            "source.repo",
        ),
        // Percent-encoded `@` and `:` are credential separators too.
        (
            json!({"source": {"repo": format!("https://host/o/x%40{CANARY}/r")}}),
            "source.repo",
        ),
        (
            json!({"source": {"repo": format!("git@host:o/x%3a{CANARY}")}}),
            "source.repo",
        ),
        (
            json!({"source": {"repo": format!("ssh://host/o%3A{CANARY}%40h/r")}}),
            "source.repo",
        ),
        (
            json!({"source": {"branch": format!("{CANARY}\u{1b}[0m")}}),
            "source.branch",
        ),
        (
            json!({"source": format!("https://u:{CANARY}@h/r")}),
            "source",
        ),
        (json!({"source": {"repo": [CANARY]}}), "source.repo"),
    ] {
        let r = publish(&app, &ada, metadata.clone(), &page("x")).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{metadata}");
        let body = String::from_utf8(body_bytes(r).await).unwrap();
        assert!(body.contains(field), "{body}");
        seen.push_str(&body);
        // The same through a new version.
        let form = Form::new().metadata(metadata).file("index.html", "x");
        let r = app
            .post_multipart(
                &format!("/api/v1/parcels/{}/versions", result.parcel.id),
                Some(&ada.token),
                &form,
            )
            .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        seen.push_str(&String::from_utf8(body_bytes(r).await).unwrap());
    }
    let d = String::from_utf8(
        body_bytes(
            app.get(
                &format!("/api/v1/parcels/{}", result.parcel.id),
                Some(&ada.token),
            )
            .await,
        )
        .await,
    )
    .unwrap();
    seen.push_str(&d);
    let versions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM parcel_versions WHERE parcel_id = $1")
            .bind(&result.parcel.id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(versions, 1, "nothing refused was stored");

    assert!(!seen.contains(CANARY), "{seen}");
    let logged = logs.text();
    assert!(
        logged.contains("parcel created"),
        "logs were captured: {logged}"
    );
    assert!(!logged.contains(CANARY), "{logged}");
}
