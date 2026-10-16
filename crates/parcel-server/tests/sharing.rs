//! Sharing with named people, `internal` visibility, and turning public
//! links off: who sees what, and that open pages lose access at once.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use parcel_types::{
    ParcelAccess, ParcelDetail, PublishResult, Share, ShareList, SharedParcelList, ViewGrant,
};
use serde_json::{json, Value};

/// A person: their stand-in credential and email.
struct Person {
    token: String,
    email: String,
}

fn person(name: &str) -> Person {
    let sub = uuid::Uuid::new_v4().simple().to_string();
    let email = format!("{name}-{sub}@example.com");
    Person {
        token: token_for(&sub, &email),
        email,
    }
}

async fn publish(app: &TestApp, owner: &Person, visibility: &str) -> String {
    let form = Form::new()
        .metadata(json!({"visibility": visibility}))
        .file("index.html", "<h1>Plan</h1>");
    let r = app
        .post_multipart("/api/v1/parcels", Some(&owner.token), &form)
        .await;
    let published: PublishResult = json(expect_status(r, StatusCode::CREATED).await).await;
    published.parcel.id
}

async fn share(
    app: &TestApp,
    owner: &Person,
    id: &str,
    email: &str,
    role: &str,
) -> axum::response::Response {
    app.put_json(
        &format!("/api/v1/parcels/{id}/shares/{email}"),
        Some(&owner.token),
        &json!({"role": role}),
    )
    .await
}

async fn detail(app: &TestApp, who: Option<&Person>, id: &str) -> axum::response::Response {
    app.get(
        &format!("/api/v1/parcels/{id}"),
        who.map(|p| p.token.as_str()),
    )
    .await
}

async fn view(app: &TestApp, who: Option<&Person>, id: &str) -> ViewGrant {
    let r = app
        .get(
            &format!("/api/v1/parcels/{id}/view"),
            who.map(|p| p.token.as_str()),
        )
        .await;
    json(expect_status(r, StatusCode::OK).await).await
}

async fn comment(app: &TestApp, who: &Person, id: &str) -> StatusCode {
    app.post_json(
        &format!("/api/v1/parcels/{id}/comments"),
        Some(&who.token),
        &json!({"body": "Looks good"}),
    )
    .await
    .status()
}

#[tokio::test]
async fn owners_manage_who_a_parcel_is_shared_with() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let bob = person("bob");
    let id = publish(&app, &owner, "private").await;

    // Emails are normalized; the person hasn't signed in yet, so no name.
    let r = share(&app, &owner, &id, &bob.email.to_uppercase(), "viewer").await;
    let s: Share = json(expect_status(r, StatusCode::OK).await).await;
    assert_eq!(
        (s.email.as_str(), s.role),
        (bob.email.as_str(), parcel_types::ShareRole::Viewer)
    );
    assert_eq!(s.name, None);

    // Once they have signed in, their name shows.
    detail(&app, Some(&bob), &id).await;
    let list: ShareList = json(
        app.get(&format!("/api/v1/parcels/{id}/shares"), Some(&owner.token))
            .await,
    )
    .await;
    assert_eq!(list.shares.len(), 1);
    assert_eq!(list.shares[0].name.as_deref(), Some("Test User"));

    // Changing a role is the same PUT.
    let r = share(&app, &owner, &id, &bob.email, "commenter").await;
    let s: Share = json(expect_status(r, StatusCode::OK).await).await;
    assert_eq!(s.role, parcel_types::ShareRole::Commenter);

    // Only the owner sees or changes the list: bob can see the parcel (403),
    // a stranger can't (404).
    let shares_url = format!("/api/v1/parcels/{id}/shares");
    expect_status(
        app.get(&shares_url, Some(&bob.token)).await,
        StatusCode::FORBIDDEN,
    )
    .await;
    let stranger = person("eve");
    expect_status(
        app.get(&shares_url, Some(&stranger.token)).await,
        StatusCode::NOT_FOUND,
    )
    .await;
    expect_status(
        share(&app, &bob, &id, &stranger.email, "viewer").await,
        StatusCode::FORBIDDEN,
    )
    .await;

    let r = app
        .delete(&format!("{shares_url}/{}", bob.email), Some(&owner.token))
        .await;
    expect_status(r, StatusCode::NO_CONTENT).await;
    let r = app
        .delete(&format!("{shares_url}/{}", bob.email), Some(&owner.token))
        .await;
    expect_status(r, StatusCode::NOT_FOUND).await;
    expect_status(detail(&app, Some(&bob), &id).await, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn shares_are_validated() {
    let app =
        TestApp::with_config(|c| c.providers[0].allowed_domains = vec!["example.com".into()]).await;
    let owner = person("owner");
    let id = publish(&app, &owner, "private").await;
    for (email, needle) in [
        ("not-an-email", "not an email address"),
        (owner.email.as_str(), "that's you"),
        ("ada@gmail.com", "nobody at @gmail.com can sign in"),
    ] {
        let r = expect_status(
            share(&app, &owner, &id, email, "viewer").await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        let body: Value = json(r).await;
        assert!(
            body["error"]["message"].as_str().unwrap().contains(needle),
            "{email}: {body}"
        );
    }
    let r = share(&app, &owner, &id, "ada@example.com", "editor").await;
    assert_eq!(
        r.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown roles are refused"
    );

    // At most 200 people.
    for i in 0..200 {
        expect_status(
            share(&app, &owner, &id, &format!("p{i}@example.com"), "viewer").await,
            StatusCode::OK,
        )
        .await;
    }
    let r = expect_status(
        share(&app, &owner, &id, "one-more@example.com", "viewer").await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    let body: Value = json(r).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("at most 200"),
        "{body}"
    );
    // Changing an existing person's role still works at the cap.
    expect_status(
        share(&app, &owner, &id, "p0@example.com", "commenter").await,
        StatusCode::OK,
    )
    .await;
}

#[tokio::test]
async fn viewers_see_and_commenters_comment() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let bob = person("bob");
    let id = publish(&app, &owner, "private").await;
    // Shared before bob ever signed in: it works from his first visit.
    expect_status(
        share(&app, &owner, &id, &bob.email, "viewer").await,
        StatusCode::OK,
    )
    .await;

    let d: ParcelDetail =
        json(expect_status(detail(&app, Some(&bob), &id).await, StatusCode::OK).await).await;
    assert_eq!(d.access, ParcelAccess::Viewer);
    assert!(!d.is_owner);
    assert_eq!(d.owner_name.as_deref(), Some("Test User"));
    expect_status(
        app.get(&format!("/api/v1/parcels/{id}/comments"), Some(&bob.token))
            .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(comment(&app, &bob, &id).await, StatusCode::FORBIDDEN);

    expect_status(
        share(&app, &owner, &id, &bob.email, "commenter").await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(comment(&app, &bob, &id).await, StatusCode::CREATED);
    // Page data stays the owner's to write.
    let r = app
        .put_json(
            &format!("/api/v1/parcels/{id}/data/c/k"),
            Some(&bob.token),
            &json!(1),
        )
        .await;
    expect_status(r, StatusCode::FORBIDDEN).await;

    let shared: SharedParcelList = json(app.get("/api/v1/shared", Some(&bob.token)).await).await;
    assert_eq!(shared.total, 1);
    assert_eq!(shared.parcels[0].parcel.id, id);
    assert_eq!(shared.parcels[0].role, parcel_types::ShareRole::Commenter);
    assert_eq!(shared.parcels[0].owner_name, "Test User");
    let mine: SharedParcelList = json(app.get("/api/v1/shared", Some(&owner.token)).await).await;
    assert_eq!(mine.total, 0, "your own parcels aren't shared with you");

    // Strangers and anonymous callers still can't tell it exists.
    expect_status(
        detail(&app, Some(&person("eve")), &id).await,
        StatusCode::NOT_FOUND,
    )
    .await;
    expect_status(detail(&app, None, &id).await, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn unsharing_cuts_off_open_pages() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let bob = person("bob");
    let id = publish(&app, &owner, "private").await;
    expect_status(
        share(&app, &owner, &id, &bob.email, "viewer").await,
        StatusCode::OK,
    )
    .await;

    let grant = view(&app, Some(&bob), &id).await;
    assert_eq!(grant.access, ParcelAccess::Viewer);
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::OK,
    )
    .await;
    // The page's data reads with the member grant, but never writes.
    let token = grant
        .content_url
        .split("/v/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    expect_status(
        app.content_request(Method::GET, &format!("/d/{token}"), None)
            .await,
        StatusCode::OK,
    )
    .await;
    expect_status(
        app.content_request(Method::PUT, &format!("/d/{token}/c/k"), Some("1"))
            .await,
        StatusCode::FORBIDDEN,
    )
    .await;

    let r = app
        .delete(
            &format!("/api/v1/parcels/{id}/shares/{}", bob.email),
            Some(&owner.token),
        )
        .await;
    expect_status(r, StatusCode::NO_CONTENT).await;
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::FORBIDDEN,
    )
    .await;
    expect_status(
        app.content_request(Method::GET, &format!("/d/{token}"), None)
            .await,
        StatusCode::FORBIDDEN,
    )
    .await;
    expect_status(detail(&app, Some(&bob), &id).await, StatusCode::NOT_FOUND).await;

    // A disabled account loses access the same way.
    expect_status(
        share(&app, &owner, &id, &bob.email, "viewer").await,
        StatusCode::OK,
    )
    .await;
    let grant = view(&app, Some(&bob), &id).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = $1")
        .bind(&bob.email)
        .execute(&app.pool)
        .await
        .unwrap();
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn internal_parcels_are_for_everyone_signed_in() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let eve = person("eve");
    let id = publish(&app, &owner, "internal").await;

    let d: ParcelDetail =
        json(expect_status(detail(&app, Some(&eve), &id).await, StatusCode::OK).await).await;
    assert_eq!(d.access, ParcelAccess::Commenter);
    assert_eq!(d.parcel.visibility, parcel_types::Visibility::Internal);
    assert_eq!(comment(&app, &eve, &id).await, StatusCode::CREATED);
    expect_status(detail(&app, None, &id).await, StatusCode::NOT_FOUND).await;
    expect_status(
        app.get(&format!("/api/v1/parcels/{id}/view"), None).await,
        StatusCode::NOT_FOUND,
    )
    .await;

    // Back to private: eve's open page stops loading.
    let grant = view(&app, Some(&eve), &id).await;
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::OK,
    )
    .await;
    let r = app
        .patch_json(
            &format!("/api/v1/parcels/{id}"),
            Some(&owner.token),
            &json!({"visibility": "private"}),
        )
        .await;
    expect_status(r, StatusCode::OK).await;
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::FORBIDDEN,
    )
    .await;

    // Signed-in viewers of a public parcel get member grants too; anonymous
    // ones get public grants and no comments.
    let r = app
        .patch_json(
            &format!("/api/v1/parcels/{id}"),
            Some(&owner.token),
            &json!({"visibility": "public"}),
        )
        .await;
    expect_status(r, StatusCode::OK).await;
    assert_eq!(
        view(&app, Some(&eve), &id).await.access,
        ParcelAccess::Commenter
    );
    let anon = view(&app, None, &id).await;
    assert_eq!(anon.access, ParcelAccess::Public);
    assert_eq!(
        anon.owner_name, None,
        "anonymous viewers aren't told who published it"
    );
}

#[tokio::test]
async fn turning_public_links_off_makes_public_parcels_internal() {
    let app = TestApp::with_config(|c| c.allow_public = false).await;
    let owner = person("owner");
    let form = Form::new()
        .metadata(json!({"visibility": "public"}))
        .file("index.html", "<h1>x</h1>");
    let r = app
        .post_multipart("/api/v1/parcels", Some(&owner.token), &form)
        .await;
    let body: Value = json(expect_status(r, StatusCode::BAD_REQUEST).await).await;
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("public links are turned off"));

    let id = publish(&app, &owner, "private").await;
    let r = app
        .patch_json(
            &format!("/api/v1/parcels/{id}"),
            Some(&owner.token),
            &json!({"visibility": "public"}),
        )
        .await;
    expect_status(r, StatusCode::BAD_REQUEST).await;

    // A parcel made public while it was allowed now reports and behaves as
    // internal, and public grants minted before stop working.
    sqlx::query("UPDATE parcels SET visibility = 'public' WHERE id = $1")
        .bind(&id)
        .execute(&app.pool)
        .await
        .unwrap();
    let d: ParcelDetail = json(detail(&app, Some(&person("eve")), &id).await).await;
    assert_eq!(d.parcel.visibility, parcel_types::Visibility::Internal);
    expect_status(detail(&app, None, &id).await, StatusCode::NOT_FOUND).await;
    let (url, _) = parcel_server::content::grant_url(
        &app.state,
        &id,
        1,
        "index.html",
        parcel_server::content::token::Scope::Public,
    );
    expect_status(app.get_content(&url, &[]).await, StatusCode::FORBIDDEN).await;

    let info: parcel_types::ServerInfo = json(app.get("/api/v1/info", None).await).await;
    assert!(!info.allow_public);
}

#[tokio::test]
async fn a_disabled_owner_loses_their_open_pages() {
    let app = TestApp::new().await;
    let owner = person("owner");
    let id = publish(&app, &owner, "private").await;
    let grant = view(&app, Some(&owner), &id).await;
    assert_eq!(grant.access, ParcelAccess::Owner);
    let token = grant
        .content_url
        .split("/v/")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    expect_status(
        app.content_request(Method::PUT, &format!("/d/{token}/c/k"), Some("1"))
            .await,
        StatusCode::OK,
    )
    .await;

    sqlx::query("UPDATE users SET disabled_at = now() WHERE email = $1")
        .bind(&owner.email)
        .execute(&app.pool)
        .await
        .unwrap();
    expect_status(
        app.get_content(&grant.content_url, &[]).await,
        StatusCode::FORBIDDEN,
    )
    .await;
    expect_status(
        app.content_request(Method::PUT, &format!("/d/{token}/c/k"), Some("2"))
            .await,
        StatusCode::FORBIDDEN,
    )
    .await;
    expect_status(
        app.content_request(Method::DELETE, &format!("/d/{token}/c"), None)
            .await,
        StatusCode::FORBIDDEN,
    )
    .await;
}
