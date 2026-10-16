//! Blob deduplication and garbage collection.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use parcel_server::storage::blob_key;
use parcel_types::PublishResult;

#[tokio::test]
async fn identical_files_share_a_blob_and_gc_spares_referenced_ones() {
    let app = TestApp::new().await;
    let token = user_token();
    let unique = format!("<p>{}</p>", uuid::Uuid::new_v4());
    let shared = format!("/* shared {} */", uuid::Uuid::new_v4());

    let a: PublishResult = json(
        expect_status(
            app.post_multipart(
                "/api/v1/parcels",
                Some(&token),
                &Form::new()
                    .file("index.html", unique.clone())
                    .file("s.css", shared.clone()),
            )
            .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let b: PublishResult = json(
        expect_status(
            app.post_multipart(
                "/api/v1/parcels",
                Some(&token),
                &Form::new()
                    .file("index.html", "<p>b</p>")
                    .file("s.css", shared.clone()),
            )
            .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let sha_of = |r: &PublishResult, path: &str| {
        r.version
            .files
            .iter()
            .find(|f| f.path == path)
            .unwrap()
            .sha256
            .clone()
    };
    let shared_sha = sha_of(&a, "s.css");
    assert_eq!(shared_sha, sha_of(&b, "s.css"));
    let unique_sha = sha_of(&a, "index.html");
    assert!(app.blobs.contains(&blob_key(&shared_sha)));
    assert!(app.blobs.contains(&blob_key(&unique_sha)));

    expect_status(
        app.delete(&format!("/api/v1/parcels/{}", a.parcel.id), Some(&token))
            .await,
        StatusCode::NO_CONTENT,
    )
    .await;

    // Within the grace period nothing is collected.
    parcel_server::gc::run_once(&app.state, Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(app.blobs.contains(&blob_key(&unique_sha)));

    // With no grace, the orphan goes and the still-referenced blob stays.
    parcel_server::gc::run_once(&app.state, Duration::ZERO)
        .await
        .unwrap();
    assert!(!app.blobs.contains(&blob_key(&unique_sha)));
    assert!(app.blobs.contains(&blob_key(&shared_sha)));

    // Re-publishing the collected content writes the object again.
    let again: PublishResult = json(
        expect_status(
            app.post_multipart(
                "/api/v1/parcels",
                Some(&token),
                &Form::new().file("index.html", unique.clone()),
            )
            .await,
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    assert_eq!(sha_of(&again, "index.html"), unique_sha);
    assert!(app.blobs.contains(&blob_key(&unique_sha)));
}
