//! The GCS blob store against fake-gcs-server (`docker compose up -d`).
//! Fails loudly when the emulator isn't running — it never skips.

use bytes::Bytes;
use parcel_server::storage::{BlobStore, GcsBlobStore};

const EMULATOR: &str = "http://localhost:4447";

async fn bucket() -> String {
    let http = reqwest::Client::new();
    let name = format!("parcel-test-{}", uuid::Uuid::new_v4().simple());
    let response = http
        .post(format!("{EMULATOR}/storage/v1/b?project=test"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .unwrap_or_else(|e| {
            panic!(
                "fake-gcs-server is not reachable at {EMULATOR} ({e}). \
                 Start it with `docker compose up -d fake-gcs`."
            )
        });
    assert!(
        response.status().is_success(),
        "create bucket: {}",
        response.status()
    );
    name
}

#[tokio::test]
async fn round_trips_through_the_emulator() {
    let bucket = bucket().await;
    let store = GcsBlobStore::new(
        reqwest::Client::new(),
        bucket,
        "env".into(),
        Some(EMULATOR.into()),
        None,
    )
    .unwrap();
    let key = "blobs/sha256/ab/abcdef";
    assert!(store.get(key).await.unwrap().is_none());
    assert!(!store.exists(key).await.unwrap());
    store
        .put(key, Bytes::from_static(b"<p>hi</p>"), "text/html")
        .await
        .unwrap();
    assert!(store.exists(key).await.unwrap());
    assert_eq!(store.get(key).await.unwrap().unwrap(), "<p>hi</p>");
    // Overwrite is idempotent for content-addressed keys.
    store
        .put(key, Bytes::from_static(b"<p>hi</p>"), "text/html")
        .await
        .unwrap();
    store.delete(key).await.unwrap();
    store.delete(key).await.unwrap();
    assert!(store.get(key).await.unwrap().is_none());
}
