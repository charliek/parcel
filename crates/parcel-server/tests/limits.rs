//! Upload validation and configurable limits.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

async fn error_message(response: axum::response::Response) -> String {
    let body: serde_json::Value = json(response).await;
    body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn rejects_bad_paths_types_and_shapes() {
    let app = TestApp::new().await;
    let token = user_token();
    let cases: Vec<(Form, &str)> = vec![
        (Form::new().file("../escape.html", "x"), "'.' or '..'"),
        (Form::new().file("/abs.html", "x"), "relative"),
        (Form::new().file("tool.exe", "x"), "unsupported file type"),
        (
            Form::new().file("a.html", "x").file("a.html", "y"),
            "uploaded twice",
        ),
        (Form::new().file("style.css", "x"), "entry page"),
        (
            Form::new().file("a.html", "x").file("b.html", "y"),
            "pass `entry`",
        ),
        (
            Form::new()
                .metadata(json!({"titel": "typo"}))
                .file("a.html", "x"),
            "unknown field",
        ),
        (
            Form::new()
                .metadata(json!({"entry": "style.css"}))
                .file("style.css", "x")
                .file("a.html", "y"),
            "must be an .html",
        ),
        (Form::new(), "at least one file"),
    ];
    for (form, needle) in cases {
        let response = app
            .post_multipart("/api/v1/parcels", Some(&token), &form)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "expected 400 for {needle}"
        );
        let message = error_message(response).await;
        assert!(
            message.contains(needle),
            "{message:?} should mention {needle:?}"
        );
    }
}

#[tokio::test]
async fn size_and_count_limits_are_configurable() {
    let app = TestApp::with_config(|c| {
        c.limits.max_entry_bytes = 100;
        c.limits.max_file_bytes = 10;
        c.limits.max_files_per_upload = 3;
        c.limits.max_upload_bytes = 1000;
    })
    .await;
    let token = user_token();

    let big_entry = Form::new().file("index.html", vec![b'a'; 101]);
    let r = app
        .post_multipart("/api/v1/parcels", Some(&token), &big_entry)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let big_asset = Form::new()
        .file("index.html", "ok")
        .file("app.js", vec![b'a'; 11]);
    let r = app
        .post_multipart("/api/v1/parcels", Some(&token), &big_asset)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error_message(r).await.contains("file limit is 10"));

    let many = Form::new()
        .file("index.html", "ok")
        .file("a.css", "a")
        .file("b.css", "b")
        .file("c.css", "c");
    let r = app
        .post_multipart("/api/v1/parcels", Some(&token), &many)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let fine = Form::new()
        .file("index.html", vec![b'a'; 100])
        .file("app.js", vec![b'a'; 10]);
    expect_status(
        app.post_multipart("/api/v1/parcels", Some(&token), &fine)
            .await,
        StatusCode::CREATED,
    )
    .await;
}

#[tokio::test]
async fn whole_upload_limit_is_enforced_while_streaming() {
    let app = TestApp::with_config(|c| {
        c.limits.max_upload_bytes = 50;
        c.limits.max_file_bytes = 40;
        c.limits.max_entry_bytes = 40;
    })
    .await;
    let token = user_token();
    let form = Form::new()
        .file("index.html", vec![b'a'; 30])
        .file("a.js", vec![b'a'; 30]);
    let r = app
        .post_multipart("/api/v1/parcels", Some(&token), &form)
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error_message(r).await.contains("upload is larger"));
}

#[tokio::test]
async fn quota_counts_every_version() {
    let app = TestApp::with_config(|c| c.limits.user_quota_bytes = 25).await;
    let token = user_token();
    let first = Form::new().file("index.html", vec![b'a'; 20]);
    let created: parcel_types::PublishResult = json(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(&token), &first)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    // A second version of the same size would bring the total to 40 > 25.
    let again = Form::new().file("index.html", vec![b'b'; 20]);
    let r = app
        .post_multipart(
            &format!("/api/v1/parcels/{}/versions", created.parcel.id),
            Some(&token),
            &again,
        )
        .await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(error_message(r).await.contains("quota"));
}

#[tokio::test]
async fn metadata_is_validated() {
    let app = TestApp::new().await;
    let token = user_token();
    let form = Form::new()
        .metadata(json!({"title": "t".repeat(201)}))
        .file("index.html", "x");
    let r = app
        .post_multipart("/api/v1/parcels", Some(&token), &form)
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    let form = Form::new()
        .metadata(json!({"visibility": "public", "description": "  trimmed  "}))
        .file("index.html", "x");
    let created: parcel_types::PublishResult = json(
        expect_status(
            app.post_multipart("/api/v1/parcels", Some(&token), &form)
                .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(created.parcel.description, "trimmed");
    assert_eq!(created.parcel.visibility, parcel_types::Visibility::Public);
    assert_eq!(
        created.parcel.title, "index.html",
        "no <title>, so the file name"
    );
}
