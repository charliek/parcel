//! Comment threads and the viewer's activity poll, over the real router +
//! Postgres: who may read, write, edit, resolve and delete.

mod common;

use axum::http::StatusCode;
use common::*;
use parcel_types::{
    Comment, CommentList, CommentVia, CreatedToken, ParcelActivity, ParcelDetail, PublishResult,
};
use serde_json::{json, Value};

async fn publish(app: &TestApp, token: &str, visibility: &str) -> String {
    let form = Form::new()
        .metadata(json!({"visibility": visibility}))
        .file("index.html", "<h1>Quarterly results</h1><p>Numbers.</p>");
    let response = app
        .post_multipart("/api/v1/parcels", Some(token), &form)
        .await;
    let published: PublishResult = json(expect_status(response, StatusCode::CREATED).await).await;
    published.parcel.id
}

async fn comment(app: &TestApp, token: &str, id: &str, body: Value) -> Comment {
    let response = app
        .post_json(
            &format!("/api/v1/parcels/{id}/comments"),
            Some(token),
            &body,
        )
        .await;
    json(expect_status(response, StatusCode::CREATED).await).await
}

async fn list(app: &TestApp, token: Option<&str>, id: &str, status: &str) -> CommentList {
    let response = app
        .get(
            &format!("/api/v1/parcels/{id}/comments?status={status}"),
            token,
        )
        .await;
    json(expect_status(response, StatusCode::OK).await).await
}

async fn activity(app: &TestApp, token: Option<&str>, id: &str) -> ParcelActivity {
    let response = app
        .get(&format!("/api/v1/parcels/{id}/activity"), token)
        .await;
    let response = expect_status(response, StatusCode::OK).await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    json(response).await
}

fn anchor() -> Value {
    json!({
        "path": "index.html",
        "selector": "html > body > h1",
        "text": "Quarterly results",
        "x": 0.25,
        "y": 0.5
    })
}

#[tokio::test]
async fn threads_replies_and_resolution() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish(&app, &owner, "private").await;
    let before = activity(&app, Some(&owner), &id).await;
    assert_eq!(before.open_comments, 0);

    let thread = comment(
        &app,
        &owner,
        &id,
        json!({"body": "  Make this bigger  ", "anchor": anchor()}),
    )
    .await;
    assert_eq!(thread.number, 1);
    assert_eq!(thread.version, 1);
    assert_eq!(thread.body, "Make this bigger");
    assert_eq!(thread.via, CommentVia::Web);
    assert!(thread.author.is_owner && thread.author.is_you);
    assert_eq!(thread.author.name, "Test User");
    let stored_anchor = thread.anchor.as_ref().unwrap();
    assert_eq!(stored_anchor.selector.as_deref(), Some("html > body > h1"));
    assert_eq!(stored_anchor.x, Some(0.25));
    assert!(!thread.resolved);

    let general = comment(&app, &owner, &id, json!({"body": "Overall: nice"})).await;
    assert_eq!(general.number, 2);
    assert!(general.anchor.is_none());

    // Replies take the next number and come back inside their thread.
    let with_reply = comment(
        &app,
        &owner,
        &id,
        json!({"body": "Done in v2", "reply_to": 1}),
    )
    .await;
    assert_eq!(with_reply.number, 1);
    assert_eq!(with_reply.replies.len(), 1);
    assert_eq!(with_reply.replies[0].number, 3);

    let open = list(&app, Some(&owner), &id, "open").await;
    assert_eq!(open.open, 2);
    assert_eq!(
        open.comments.iter().map(|c| c.number).collect::<Vec<_>>(),
        [1, 2]
    );
    let after = activity(&app, Some(&owner), &id).await;
    assert_eq!(after.open_comments, 2);
    assert!(after.comments_revision > before.comments_revision);
    let detail: ParcelDetail = json(
        app.get(&format!("/api/v1/parcels/{id}"), Some(&owner))
            .await,
    )
    .await;
    assert_eq!(detail.parcel.open_comments, 2);

    // Resolving moves the thread (with its reply) to the resolved list.
    let resolved: Comment = json(
        expect_status(
            app.patch_json(
                &format!("/api/v1/parcels/{id}/comments/1"),
                Some(&owner),
                &json!({"resolved": true}),
            )
            .await,
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert!(resolved.resolved && resolved.resolved_at.is_some());
    let open = list(&app, Some(&owner), &id, "open").await;
    assert_eq!(
        open.comments.iter().map(|c| c.number).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(open.open, 1);
    let done = list(&app, Some(&owner), &id, "resolved").await;
    assert_eq!(done.comments.len(), 1);
    assert_eq!(done.comments[0].replies.len(), 1);
    assert_eq!(list(&app, Some(&owner), &id, "all").await.comments.len(), 2);

    // Edit, then delete the thread: its reply goes with it.
    let edited: Comment = json(
        app.patch_json(
            &format!("/api/v1/parcels/{id}/comments/3"),
            Some(&owner),
            &json!({"body": "Done in v3"}),
        )
        .await,
    )
    .await;
    assert_eq!(edited.number, 1, "edits return the whole thread");
    assert_eq!(edited.replies[0].body, "Done in v3");
    let gone = app
        .delete(&format!("/api/v1/parcels/{id}/comments/1"), Some(&owner))
        .await;
    assert_eq!(gone.status(), StatusCode::NO_CONTENT);
    let all = list(&app, Some(&owner), &id, "all").await;
    assert_eq!(
        all.comments.iter().map(|c| c.number).collect::<Vec<_>>(),
        [2]
    );
    // Numbers are never reused.
    let next = comment(&app, &owner, &id, json!({"body": "again"})).await;
    assert_eq!(next.number, 4);
}

#[tokio::test]
async fn bad_comments_are_rejected() {
    let app = TestApp::new().await;
    let owner = user_token();
    let id = publish(&app, &owner, "private").await;
    comment(&app, &owner, &id, json!({"body": "thread"})).await;
    comment(&app, &owner, &id, json!({"body": "reply", "reply_to": 1})).await;
    let uri = format!("/api/v1/parcels/{id}/comments");

    let cases = [
        (json!({"body": "   "}), StatusCode::BAD_REQUEST),
        (json!({"body": "x".repeat(4001)}), StatusCode::BAD_REQUEST),
        (json!({"body": "x", "version": 9}), StatusCode::BAD_REQUEST),
        (
            json!({"body": "x", "anchor": {"path": "../etc"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"body": "x", "anchor": {"path": "a.html", "x": 2}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"body": "x", "anchor": {"path": "a.html", "bogus": 1}}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            json!({"body": "x", "reply_to": 1, "anchor": anchor()}),
            StatusCode::BAD_REQUEST,
        ),
        (json!({"body": "x", "reply_to": 2}), StatusCode::BAD_REQUEST),
        (json!({"body": "x", "reply_to": 77}), StatusCode::NOT_FOUND),
    ];
    for (body, status) in cases {
        let response = app.post_json(&uri, Some(&owner), &body).await;
        assert_eq!(response.status(), status, "{body}");
    }

    // Replies can't be resolved on their own.
    let response = app
        .patch_json(
            &format!("{uri}/2"),
            Some(&owner),
            &json!({"resolved": true}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .patch_json(&format!("{uri}/1"), Some(&owner), &json!({}))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .patch_json(&format!("{uri}/50"), Some(&owner), &json!({"body": "x"}))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_parcels_hide_their_comments() {
    let app = TestApp::new().await;
    let owner = user_token();
    let stranger = user_token();
    let id = publish(&app, &owner, "private").await;
    comment(&app, &owner, &id, json!({"body": "note to self"})).await;
    let uri = format!("/api/v1/parcels/{id}/comments");

    assert_eq!(
        app.get(&uri, Some(&stranger)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get(&uri, None).await.status(), StatusCode::NOT_FOUND);
    let post = app
        .post_json(&uri, Some(&stranger), &json!({"body": "hi"}))
        .await;
    assert_eq!(post.status(), StatusCode::NOT_FOUND);
    let resolve = app
        .patch_json(
            &format!("{uri}/1"),
            Some(&stranger),
            &json!({"resolved": true}),
        )
        .await;
    assert_eq!(resolve.status(), StatusCode::NOT_FOUND);
    let activity = app
        .get(&format!("/api/v1/parcels/{id}/activity"), None)
        .await;
    assert_eq!(activity.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn public_parcels_take_comments_from_signed_in_users() {
    let app = TestApp::new().await;
    let owner = user_token();
    let guest = user_token();
    let id = publish(&app, &owner, "public").await;
    let uri = format!("/api/v1/parcels/{id}/comments");
    comment(&app, &owner, &id, json!({"body": "owner thread"})).await;

    // Anonymous viewers see the parcel but not its comments.
    assert_eq!(app.get(&uri, None).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.post_json(&uri, None, &json!({"body": "drive-by"}))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let polled = activity(&app, None, &id).await;
    assert_eq!(polled.open_comments, 1);

    let mine = comment(&app, &guest, &id, json!({"body": "guest thread"})).await;
    assert_eq!(mine.number, 2);
    assert!(mine.author.is_you && !mine.author.is_owner);
    let seen = list(&app, Some(&guest), &id, "open").await;
    let owner_view = &seen.comments[0];
    assert!(owner_view.author.is_owner && !owner_view.author.is_you);

    let status = |r: axum::response::Response| r.status();
    // The guest can't edit, resolve or delete the owner's thread…
    for (method, body) in [
        ("edit", json!({"body": "x"})),
        ("resolve", json!({"resolved": true})),
    ] {
        let r = app
            .patch_json(&format!("{uri}/1"), Some(&guest), &body)
            .await;
        assert_eq!(status(r), StatusCode::FORBIDDEN, "{method}");
    }
    assert_eq!(
        status(app.delete(&format!("{uri}/1"), Some(&guest)).await),
        StatusCode::FORBIDDEN
    );
    // …but can resolve their own, and the owner can't rewrite it.
    assert_eq!(
        status(
            app.patch_json(
                &format!("{uri}/2"),
                Some(&guest),
                &json!({"resolved": true})
            )
            .await
        ),
        StatusCode::OK
    );
    assert_eq!(
        status(
            app.patch_json(
                &format!("{uri}/2"),
                Some(&owner),
                &json!({"body": "edited"})
            )
            .await
        ),
        StatusCode::FORBIDDEN
    );
    // The owner moderates: reopen and delete anyone's thread.
    assert_eq!(
        status(
            app.patch_json(
                &format!("{uri}/2"),
                Some(&owner),
                &json!({"resolved": false})
            )
            .await
        ),
        StatusCode::OK
    );
    assert_eq!(
        status(app.delete(&format!("{uri}/2"), Some(&owner)).await),
        StatusCode::NO_CONTENT
    );

    // Once private again, the guest loses access entirely.
    app.patch_json(
        &format!("/api/v1/parcels/{id}"),
        Some(&owner),
        &json!({"visibility": "private"}),
    )
    .await;
    assert_eq!(
        app.get(&uri, Some(&guest)).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn cli_tokens_comment_as_cli_and_activity_tracks_versions() {
    let app = TestApp::new().await;
    let jwt = user_token();
    let id = publish(&app, &jwt, "private").await;
    let created: CreatedToken = json(
        app.post_json("/api/v1/tokens", Some(&jwt), &json!({"name": "agent"}))
            .await,
    )
    .await;
    let reply = comment(
        &app,
        &created.secret,
        &id,
        json!({"body": "from the agent"}),
    )
    .await;
    assert_eq!(reply.via, CommentVia::Cli);
    assert!(reply.author.is_you);

    let form = Form::new().file("index.html", "<h1>v2</h1>");
    let response = app
        .post_multipart(&format!("/api/v1/parcels/{id}/versions"), Some(&jwt), &form)
        .await;
    expect_status(response, StatusCode::CREATED).await;
    let polled = activity(&app, Some(&created.secret), &id).await;
    assert_eq!(polled.current_version, 2);

    // New threads default to the current version; older ones keep theirs.
    let on_v2 = comment(&app, &jwt, &id, json!({"body": "v2 looks good"})).await;
    assert_eq!(on_v2.version, 2);
    let on_v1 = comment(&app, &jwt, &id, json!({"body": "v1 note", "version": 1})).await;
    assert_eq!(on_v1.version, 1);
}
