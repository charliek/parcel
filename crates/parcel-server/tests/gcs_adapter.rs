//! The GCS blob store's adapter over stridelabs-gcp, against wiremock: which
//! endpoint and credentials each configuration uses, the service-account
//! (RS256) and metadata-server token requests, the retry caps, the read cap,
//! and that no error or log line carries Google's message, an object name,
//! a bearer or key material. (`tests/gcs.rs` runs the real API shape against
//! fake-gcs-server.)

mod common;

use std::collections::HashMap;
use std::time::Duration;

use axum::response::IntoResponse;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use bytes::Bytes;
use common::Captured;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use parcel_server::error::AppError;
use parcel_server::storage::{BlobStore, GcsBlobStore, GcsEndpoints, StorageError};
use serde_json::json;
use stridelabs_auth::test_support::TestKey;
use stridelabs_gcp::auth::DEVSTORAGE_READ_WRITE_SCOPE;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const BUCKET: &str = "bucket";
const KEY: &str = "blobs/sha256/ab/abcdef";
const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
const OBJECT_PATH: &str = r"^/storage/v1/b/bucket/o/[^/]+$";
const UPLOAD_PATH: &str = "/upload/storage/v1/b/bucket/o";

/// The mocks a store can talk to: the JSON API, an emulator, the metadata
/// server and a service account's token endpoint.
struct Mocks {
    api: MockServer,
    emulator: MockServer,
    metadata: MockServer,
    token: MockServer,
}

impl Mocks {
    async fn start() -> Self {
        let mocks = Self {
            api: MockServer::start().await,
            emulator: MockServer::start().await,
            metadata: MockServer::start().await,
            token: MockServer::start().await,
        };
        for gcs in [&mocks.api, &mocks.emulator] {
            Mock::given(method("GET"))
                .and(path_regex(OBJECT_PATH))
                .and(query_param("fields", "name"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": KEY})))
                .mount(gcs)
                .await;
        }
        Mock::given(method("GET"))
            .and(path(METADATA_TOKEN_PATH))
            .and(header("Metadata-Flavor", "Google"))
            .respond_with(token_response("metadata-bearer"))
            .mount(&mocks.metadata)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(token_response("key-bearer"))
            .mount(&mocks.token)
            .await;
        mocks
    }

    fn endpoints(&self) -> GcsEndpoints {
        GcsEndpoints {
            storage_base_url: self.api.uri(),
            metadata_base_url: self.metadata.uri(),
        }
    }

    fn token_uri(&self) -> String {
        format!("{}/token", self.token.uri())
    }

    fn store(&self, emulator: bool, credentials: Option<&str>) -> anyhow::Result<GcsBlobStore> {
        GcsBlobStore::with_endpoints(
            reqwest::Client::new(),
            BUCKET.into(),
            "env".into(),
            emulator.then(|| self.emulator.uri()),
            credentials,
            1 << 20,
            &self.endpoints(),
        )
    }

    async fn counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for (n, server) in [&self.api, &self.emulator, &self.metadata, &self.token]
            .into_iter()
            .enumerate()
        {
            counts[n] = requests(server).await.len();
        }
        counts
    }
}

fn token_response(bearer: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "access_token": bearer,
        "expires_in": 3600,
        "token_type": "Bearer",
    }))
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

fn authorization(request: &Request) -> Option<String> {
    request
        .headers
        .get("authorization")
        .map(|v| v.to_str().unwrap().to_string())
}

/// A service-account key file for [`TestKey::primary`], with no `type`
/// (parcel never required one) unless `extra` adds it.
fn key_file(token_uri: &str, extra: serde_json::Value) -> String {
    let mut key = json!({
        "client_email": "parcel@example.iam.gserviceaccount.com",
        "private_key": TestKey::primary().pem(),
        "token_uri": token_uri,
    });
    for (k, v) in extra.as_object().unwrap() {
        key[k] = v.clone();
    }
    BASE64.encode(key.to_string())
}

/// Every way `canary` could leak from an error: its `Display` (plain and
/// alternate), its `Debug`, and the log line `AppError::Internal` writes
/// when it becomes a response.
fn assert_never_shows(err: anyhow::Error, canary: &str) -> String {
    for text in [
        format!("{err}"),
        format!("{err:#}"),
        format!("{err:?}"),
        format!("{err:#?}"),
    ] {
        assert!(!text.contains(canary), "{canary} leaked: {text}");
    }
    let logs = Captured::default();
    {
        let _guard = logs.install();
        let _ = AppError::Internal(err).into_response();
    }
    let text = logs.text();
    assert!(text.contains("internal error"), "logged: {text}");
    assert!(!text.contains(canary), "{canary} leaked into logs: {text}");
    text
}

// --- (1) endpoint × credentials ------------------------------------------------

/// The four configurations, as the module docs table them: the endpoint
/// follows `STORAGE_EMULATOR_HOST` alone, the credentials
/// `GOOGLE_CREDENTIALS` alone, and the emulator without a key sends no
/// `Authorization` at all.
#[tokio::test]
async fn each_configuration_uses_its_endpoint_and_credentials() {
    // [api, emulator, metadata, token] requests, and the bearer GCS saw.
    let cases: [(bool, bool, [usize; 4], Option<&str>); 4] = [
        (false, false, [1, 0, 1, 0], Some("Bearer metadata-bearer")),
        (false, true, [1, 0, 0, 1], Some("Bearer key-bearer")),
        (true, false, [0, 1, 0, 0], None),
        (true, true, [0, 1, 0, 1], Some("Bearer key-bearer")),
    ];
    for (emulator, with_key, expected, bearer) in cases {
        let mocks = Mocks::start().await;
        let key = key_file(&mocks.token_uri(), json!({}));
        let store = mocks
            .store(emulator, with_key.then_some(key.as_str()))
            .unwrap();
        assert!(store.exists(KEY).await.unwrap());
        let case = format!("emulator={emulator} key={with_key}");
        assert_eq!(mocks.counts().await, expected, "{case}");
        let gcs = if emulator {
            &mocks.emulator
        } else {
            &mocks.api
        };
        let seen = requests(gcs).await;
        assert_eq!(authorization(&seen[0]).as_deref(), bearer, "{case}");
        // The literal, prefixed name, encoded as one path segment.
        assert_eq!(
            seen[0].url.path(),
            "/storage/v1/b/bucket/o/env%2Fblobs%2Fsha256%2Fab%2Fabcdef",
            "{case}"
        );
    }
}

/// Credentials that are set but unusable stop startup, with nothing sent
/// anywhere: never a fallback to the metadata server or to anonymous
/// requests, with or without the emulator.
#[tokio::test]
async fn bad_credentials_fail_startup_without_a_request() {
    let mocks = Mocks::start().await;
    let not_a_pem = key_file(
        &mocks.token_uri(),
        json!({"private_key": "-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----\n"}),
    );
    let no_token_uri = BASE64.encode(
        json!({
            "client_email": "parcel@example.iam.gserviceaccount.com",
            "private_key": TestKey::primary().pem(),
        })
        .to_string(),
    );
    let plain_http = key_file("http://token.example/token", json!({}));
    let wrong_type = key_file(&mocks.token_uri(), json!({"type": "authorized_user"}));
    for emulator in [false, true] {
        for bad in [
            "not base64!",
            "",
            &BASE64.encode("not json"),
            &not_a_pem,
            &no_token_uri,
            &plain_http,
            &wrong_type,
        ] {
            let err = mocks.store(emulator, Some(bad)).err().expect("refused");
            assert!(format!("{err:#}").contains("GOOGLE_CREDENTIALS"), "{err:#}");
        }
    }
    assert_eq!(mocks.counts().await, [0, 0, 0, 0]);
}

// --- (2) the service-account exchange --------------------------------------

/// The key-file path end to end, in parcel's own dependency graph: the PEM
/// signs RS256 (jsonwebtoken on aws-lc-rs, the one crypto backend), the
/// assertion asks for the read-write storage scope, and its bearer reaches
/// GCS — without `x-goog-user-project`, whatever the key's quota project.
#[tokio::test]
async fn a_service_account_key_signs_rs256_and_its_bearer_reaches_gcs() {
    let mocks = Mocks::start().await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .and(query_param("uploadType", "media"))
        .and(query_param("name", format!("env/{KEY}")))
        .and(header("authorization", "Bearer key-bearer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(2)
        .mount(&mocks.api)
        .await;
    let key = key_file(
        &mocks.token_uri(),
        json!({"quota_project_id": "someone-elses-project", "private_key_id": "k1"}),
    );
    let store = mocks.store(false, Some(&key)).unwrap();
    store
        .put(KEY, Bytes::from_static(b"<p>hi</p>"), "text/html")
        .await
        .unwrap();
    // The token is cached: a second operation makes no second exchange.
    store
        .put(KEY, Bytes::from_static(b"<p>hi</p>"), "text/html")
        .await
        .unwrap();
    let exchanges = requests(&mocks.token).await;
    assert_eq!(exchanges.len(), 1);

    let form: HashMap<String, String> = url::form_urlencoded::parse(&exchanges[0].body)
        .into_owned()
        .collect();
    assert_eq!(
        form["grant_type"],
        "urn:ietf:params:oauth:grant-type:jwt-bearer"
    );
    let jwks = TestKey::primary().jwks();
    let decoding = DecodingKey::from_jwk(&jwks.keys[0]).unwrap();
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[mocks.token_uri()]);
    validation.set_issuer(&["parcel@example.iam.gserviceaccount.com"]);
    let claims =
        jsonwebtoken::decode::<serde_json::Value>(&form["assertion"], &decoding, &validation)
            .unwrap()
            .claims;
    assert_eq!(claims["scope"], DEVSTORAGE_READ_WRITE_SCOPE);

    let uploads = requests(&mocks.api).await;
    assert_eq!(uploads.len(), 2);
    for upload in &uploads {
        assert!(upload.headers.get("x-goog-user-project").is_none());
        assert_eq!(upload.body, b"<p>hi</p>");
    }
}

// --- (3) the metadata server -------------------------------------------------

/// Production's token request is the one parcel always made: no `?scopes=`
/// (the instance's scopes apply), `Metadata-Flavor: Google`, and a 10 s
/// timeout — this answer takes 4 s, past the library's 3 s default.
#[tokio::test]
async fn the_metadata_request_is_unchanged() {
    let mocks = Mocks::start().await;
    mocks.metadata.reset().await;
    Mock::given(method("GET"))
        .and(path(METADATA_TOKEN_PATH))
        .and(header("Metadata-Flavor", "Google"))
        .respond_with(token_response("metadata-bearer").set_delay(Duration::from_secs(4)))
        .mount(&mocks.metadata)
        .await;
    let store = mocks.store(false, None).unwrap();
    assert!(store.exists(KEY).await.unwrap());
    let asked = requests(&mocks.metadata).await;
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].url.query(), None, "{}", asked[0].url);
    let seen = requests(&mocks.api).await;
    assert_eq!(
        authorization(&seen[0]).as_deref(),
        Some("Bearer metadata-bearer")
    );
}

/// A 0-byte file's upload states `Content-Length: 0`. parcel's reqwest
/// speaks HTTP/1.1, where hyper sends no length for an empty body, and
/// Google's front end answers such a `POST` with a 411 before any auth: an
/// empty file in a publish was a 500 (fixed in stridelabs-gcp v0.11.1).
#[tokio::test]
async fn an_empty_file_states_its_length() {
    let mocks = Mocks::start().await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&mocks.api)
        .await;
    let store = mocks.store(false, None).unwrap();
    store.put(KEY, Bytes::new(), "text/plain").await.unwrap();
    let upload = requests(&mocks.api)
        .await
        .into_iter()
        .find(|r| r.url.path() == UPLOAD_PATH)
        .unwrap();
    assert_eq!(
        upload
            .headers
            .get("content-length")
            .map(|v| v.to_str().unwrap()),
        Some("0")
    );
}

// --- (4) retries ---------------------------------------------------------------

/// Each client's attempt cap, on a 1 ms schedule: a persistent 503 is tried
/// three times by the writer (put, exists, delete) and twice by the reader
/// (get); a 403 or a 404 is an answer, tried once.
#[tokio::test]
async fn transient_failures_retry_up_to_each_clients_cap() {
    let gcs = MockServer::start().await;
    let store = GcsBlobStore::new(
        reqwest::Client::new(),
        BUCKET.into(),
        String::new(),
        Some(gcs.uri()),
        None,
        1 << 20,
    )
    .unwrap()
    .with_retry_delay(Duration::from_millis(1));

    for (status, expected) in [
        (503, [3, 2, 3, 3]),
        (403, [1, 1, 1, 1]),
        (404, [1, 1, 1, 1]),
    ] {
        gcs.reset().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(status))
            .mount(&gcs)
            .await;
        let mut counts = [0; 4];
        let put = store.put(KEY, Bytes::from_static(b"x"), "text/plain").await;
        counts[0] = requests(&gcs).await.len();
        let get = store.get(KEY).await;
        counts[1] = requests(&gcs).await.len() - counts[0];
        let exists = store.exists(KEY).await;
        counts[2] = requests(&gcs).await.len() - counts[0] - counts[1];
        let delete = store.delete(KEY).await;
        counts[3] = requests(&gcs).await.len() - counts[0] - counts[1] - counts[2];
        assert_eq!(counts, expected, "HTTP {status}");
        assert!(put.is_err(), "HTTP {status}");
        if status == 404 {
            // Missing is an answer: no blob, not there, already deleted.
            assert!(get.unwrap().is_none());
            assert!(!exists.unwrap());
            delete.unwrap();
        } else {
            for err in [get.err(), exists.err(), delete.err()] {
                let err = err.expect("an error");
                assert!(err.to_string().contains(&status.to_string()), "{err}");
            }
        }
    }
}

/// An attempt cut off by its timeout is never retried (the library would:
/// each policy's deadline is shorter than its client's timeout), while a
/// quick transient failure still is. Production's ratios, scaled down: the
/// writer 600 ms an attempt and retries within 300 ms, the reader 300 ms
/// and 100 ms.
#[tokio::test]
async fn a_stalled_attempt_is_never_retried() {
    let gcs = MockServer::start().await;
    let store = GcsBlobStore::new(
        reqwest::Client::new(),
        BUCKET.into(),
        String::new(),
        Some(gcs.uri()),
        None,
        1 << 20,
    )
    .unwrap()
    .with_retry_delay(Duration::from_millis(1))
    .with_timeouts(Duration::from_millis(600), Duration::from_millis(300));
    let timed_out = |err: StorageError, operation: &str| {
        assert!(
            matches!(
                &err,
                StorageError::Gcs(stridelabs_gcp::StorageError::Timeout { operation: op, .. })
                    if *op == operation
            ),
            "{operation}: {err:?}"
        );
    };

    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&gcs)
        .await;
    let put = store.put(KEY, Bytes::from_static(b"x"), "text/plain").await;
    assert_eq!(requests(&gcs).await.len(), 1, "put");
    timed_out(put.unwrap_err(), "upload");
    let exists = store.exists(KEY).await;
    assert_eq!(requests(&gcs).await.len(), 2, "exists");
    timed_out(exists.unwrap_err(), "exists");
    let delete = store.delete(KEY).await;
    assert_eq!(requests(&gcs).await.len(), 3, "delete");
    timed_out(delete.unwrap_err(), "delete");
    let get = store.get(KEY).await;
    assert_eq!(requests(&gcs).await.len(), 4, "get");
    timed_out(get.unwrap_err(), "download");

    // The same timings still retry a quick 503 up to each cap.
    gcs.reset().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&gcs)
        .await;
    store
        .put(KEY, Bytes::from_static(b"x"), "text/plain")
        .await
        .unwrap_err();
    assert_eq!(requests(&gcs).await.len(), 3, "put 503");
    store.get(KEY).await.unwrap_err();
    assert_eq!(requests(&gcs).await.len(), 5, "get 503");
}

// --- (5) the read cap ---------------------------------------------------------------

/// A one-response HTTP server that answers every request with `body` in
/// one chunk and no `Content-Length`, so the cap must count as it reads.
async fn chunked_server(body: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\
                     Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(&body);
                response.extend_from_slice(b"\r\n0\r\n\r\n");
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{address}")
}

/// Exactly the cap reads; one byte more is refused, naming the cap, whether
/// or not the response says its length up front.
#[tokio::test]
async fn reads_stop_at_the_cap() {
    const CAP: usize = 1000;
    let store_at = |base: String| {
        GcsBlobStore::new(
            reqwest::Client::new(),
            BUCKET.into(),
            String::new(),
            Some(base),
            None,
            CAP,
        )
        .unwrap()
    };

    let gcs = MockServer::start().await;
    for (len, fits) in [(CAP, true), (CAP + 1, false)] {
        gcs.reset().await;
        Mock::given(method("GET"))
            .and(path_regex(OBJECT_PATH))
            .and(query_param("alt", "media"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; len]))
            .mount(&gcs)
            .await;
        let with_length = store_at(gcs.uri()).get(KEY).await;
        let chunked = store_at(chunked_server(vec![b'a'; len]).await)
            .get(KEY)
            .await;
        for (form, result) in [("Content-Length", with_length), ("chunked", chunked)] {
            if fits {
                assert_eq!(result.unwrap().unwrap().len(), CAP, "{form}");
            } else {
                let err = result.expect_err(form);
                assert!(
                    matches!(
                        err,
                        StorageError::Gcs(stridelabs_gcp::StorageError::TooLarge {
                            limit: CAP,
                            ..
                        })
                    ),
                    "{form}: {err:?}"
                );
                assert!(err.to_string().contains("1000"), "{form}: {err}");
            }
        }
    }
}

// --- (6) canaries -------------------------------------------------------------

/// A 403 whose Google message names the object (as Google's do) and an
/// object name carrying a canary: neither reaches an error or a log line;
/// the status and Google's plain code do.
#[tokio::test]
async fn a_refusal_never_shows_googles_message_or_the_object_name() {
    let canary = format!("CANARY{}", uuid::Uuid::new_v4().simple());
    let gcs = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": {
                "code": 403,
                "message": format!("svc@x has no storage.objects.create on {canary}/obj"),
                "status": "PERMISSION_DENIED",
                "errors": [{"message": canary, "reason": "forbidden"}],
            }
        })))
        .mount(&gcs)
        .await;
    let store = GcsBlobStore::new(
        reqwest::Client::new(),
        BUCKET.into(),
        format!("prefix-{canary}"),
        Some(gcs.uri()),
        None,
        1 << 20,
    )
    .unwrap();
    let key = format!("blobs/{canary}");

    let put = store
        .put(&key, Bytes::from_static(b"x"), "text/plain")
        .await
        .unwrap_err();
    let get = store.get(&key).await.unwrap_err();
    for err in [put, get] {
        assert!(err.to_string().contains("403"), "{err}");
        let logged = assert_never_shows(err.into(), &canary);
        assert!(logged.contains("PERMISSION_DENIED"), "{logged}");
    }
}

/// The metadata server's bearer never reaches an error or a log line —
/// including the token refresh's and every retry's debug lines.
#[tokio::test]
async fn a_bearer_never_shows() {
    let canary = format!("ya29.CANARY{}", uuid::Uuid::new_v4().simple());
    let mocks = Mocks::start().await;
    mocks.metadata.reset().await;
    Mock::given(method("GET"))
        .and(path(METADATA_TOKEN_PATH))
        .respond_with(token_response(&canary))
        .mount(&mocks.metadata)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&mocks.api)
        .await;
    let store = mocks
        .store(false, None)
        .unwrap()
        .with_retry_delay(Duration::from_millis(1));

    let logs = Captured::default();
    let err = {
        let _guard = logs.install();
        store
            .put(KEY, Bytes::from_static(b"x"), "text/plain")
            .await
            .unwrap_err()
    };
    // It did reach GCS, three times.
    let seen = requests(&mocks.api).await;
    assert_eq!(seen.len(), 3);
    assert_eq!(authorization(&seen[0]), Some(format!("Bearer {canary}")));
    assert!(!logs.text().contains(&canary), "{}", logs.text());
    assert!(!logs.text().contains("CANARY"), "{}", logs.text());
    assert_never_shows(err.into(), "CANARY");
}

/// A key that cannot be used fails startup with a message naming the
/// variable and the problem, never a byte of the value.
#[tokio::test]
async fn a_malformed_key_never_shows() {
    let canary = format!("CANARY{}", uuid::Uuid::new_v4().simple());
    let mocks = Mocks::start().await;
    let token_uri = mocks.token_uri();
    let pem_with_canary =
        format!("-----BEGIN PRIVATE KEY-----\n{canary}\n-----END PRIVATE KEY-----\n");
    for bad in [
        // Not base64, the canary in it.
        format!("{canary}!!"),
        // Not JSON.
        BASE64.encode(format!("{{\"{canary}\"")),
        // A private key that is not one.
        key_file(&token_uri, json!({"private_key": pem_with_canary})),
        // A field of the wrong type.
        key_file(&token_uri, json!({"client_email": [canary]})),
        // A token endpoint that is not safe, its path carrying the canary.
        key_file(&format!("http://token.example/{canary}"), json!({})),
        key_file(&format!("https://token.example/t?{canary}"), json!({})),
        // A `type` google-auth does not know.
        key_file(&token_uri, json!({"type": canary})),
    ] {
        let err = mocks.store(false, Some(&bad)).err().expect("refused");
        assert_never_shows(err, &canary);
    }
    assert_eq!(mocks.counts().await, [0, 0, 0, 0]);
}
