//! Google Cloud Storage blob store: [`BlobStore`] over stridelabs-gcp's
//! [`GcsClient`], which owns the JSON API, the access tokens, retries, the
//! download cap and value-free errors.
//!
//! Two clients share one token provider, because the two kinds of call have
//! different callers waiting on them:
//!
//! - **writer** — `put`, `exists` (the publish path) and `delete` (GC): 60 s
//!   per attempt, at most 3 attempts, retries only within 30 s of the first
//!   attempt's start.
//! - **reader** — `get` (viewing and indexing): 30 s per attempt, at most 2
//!   attempts, retries only within 10 s. It also carries the read cap
//!   ([`super::read_cap`]).
//!
//! The library retries 408, 429, 500, 502, 503, 504, transport or token
//! failures that are transient, *and an attempt cut off by its timeout*;
//! never 401, 403 or 404. A retry starts only if its backoff ends within the
//! policy's deadline, measured from the first attempt's start. Each deadline
//! is shorter than its client's per-attempt timeout, so a timed-out attempt
//! is never retried, while quick transient failures still are. Worst cases:
//! a reader waits about 10 s of quick retries plus one 30 s attempt; a
//! writer about 30 s plus one 60 s attempt. GC deletes a batch's objects
//! one after another while it holds that batch's blob-row locks (`gc.rs`),
//! so a lock lasts for the whole batch: each delete within the writer's
//! bound, the batch their sum. Accepted: ordering is unchanged, and only a
//! publish touching one of those blobs waits.
//!
//! **Object names** go to GCS as given ([`ObjectNamePolicy::Literal`]), with
//! `GCS_PREFIX` (outer slashes trimmed, nothing else) and a `/` in front of
//! the key — the names this store has always written, so every existing
//! object stays reachable whatever the prefix looks like.
//!
//! **Endpoint and credentials** are chosen independently, as before:
//!
//! | `STORAGE_EMULATOR_HOST` | `GOOGLE_CREDENTIALS` | |
//! |---|---|---|
//! | unset | unset | the JSON API, the metadata server's token (Cloud Run) |
//! | unset | set | the JSON API, the service-account key |
//! | set | unset | the emulator, no `Authorization` |
//! | set | set | the emulator, the service-account key |
//!
//! A blank `GOOGLE_CREDENTIALS` is unset, like every variable
//! ([`crate::config`]). Credentials that are set but unusable fail startup:
//! there is never a
//! fallback to the metadata server or to anonymous requests. Nothing here
//! reads the process environment (stridelabs-gcp's `from_environment` would:
//! `GOOGLE_APPLICATION_CREDENTIALS`, gcloud's file) — configuration comes only
//! from [`crate::config`].

use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use bytes::Bytes;
use stridelabs_gcp::auth::{
    ServiceAccountKey, TokenOptions, TokenProvider, DEFAULT_METADATA_BASE_URL,
    DEVSTORAGE_READ_WRITE_SCOPE,
};
use stridelabs_gcp::storage::DEFAULT_BASE_URL;
use stridelabs_gcp::{validate_bucket_name, GcsClient, ObjectNamePolicy, RetryPolicy};

use super::{BlobStore, StorageError};

/// The writer's bound on one attempt (the store's one timeout before the
/// library).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// The reader's bound on one attempt.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Google's documented backoff (1 s doubling to 60 s), at most three
/// attempts, retrying only within 30 s: less than [`WRITE_TIMEOUT`], so an
/// attempt that timed out is never retried (the library would retry it).
pub const WRITE_RETRY: RetryPolicy = RetryPolicy::google_default()
    .with_max_attempts(3)
    .with_deadline(Duration::from_secs(30));
/// The same backoff, at most two attempts within 10 s (less than
/// [`READ_TIMEOUT`]): a viewer is waiting.
pub const READ_RETRY: RetryPolicy = RetryPolicy::google_default()
    .with_max_attempts(2)
    .with_deadline(Duration::from_secs(10));
/// The metadata server's timeout: the store's own before the library (whose
/// default is 3 s), so production's token request is unchanged.
pub const METADATA_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the store sends what is not the emulator: the JSON API and the
/// metadata server. Production uses [`GcsEndpoints::default`]; tests point
/// them at mocks.
#[derive(Debug, Clone)]
pub struct GcsEndpoints {
    pub storage_base_url: String,
    pub metadata_base_url: String,
}

impl Default for GcsEndpoints {
    fn default() -> Self {
        Self {
            storage_base_url: DEFAULT_BASE_URL.to_string(),
            metadata_base_url: DEFAULT_METADATA_BASE_URL.to_string(),
        }
    }
}

pub struct GcsBlobStore {
    writer: GcsClient,
    reader: GcsClient,
    bucket: String,
    prefix: String,
}

impl GcsBlobStore {
    /// The store for `bucket` under `prefix`, against the emulator when
    /// `emulator_host` is set and the JSON API otherwise (see the module
    /// docs for the credentials), reading at most `max_read_bytes` a blob.
    pub fn new(
        http: reqwest::Client,
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
        credentials_b64: Option<&str>,
        max_read_bytes: usize,
    ) -> anyhow::Result<Self> {
        Self::with_endpoints(
            http,
            bucket,
            prefix,
            emulator_host,
            credentials_b64,
            max_read_bytes,
            &GcsEndpoints::default(),
        )
    }

    /// [`GcsBlobStore::new`] with other endpoints for the JSON API and the
    /// metadata server.
    pub fn with_endpoints(
        http: reqwest::Client,
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
        credentials_b64: Option<&str>,
        max_read_bytes: usize,
        endpoints: &GcsEndpoints,
    ) -> anyhow::Result<Self> {
        // The library's reason names the rule, never the bucket.
        validate_bucket_name(&bucket).map_err(|e| anyhow!("GCS_BUCKET: {e}"))?;
        // The module docs' table, row by row.
        let client = match (emulator_host.as_deref(), credentials_b64) {
            (None, None) => {
                // No scopes: no `?scopes=` query, the request this store has
                // always made (the instance's own scopes apply).
                let tokens = TokenProvider::metadata_server(http.clone(), &[]).with_options(
                    TokenOptions::default()
                        .metadata_base_url(endpoints.metadata_base_url.clone())
                        .metadata_timeout(METADATA_TIMEOUT),
                );
                GcsClient::new(http, tokens).with_base_url(&endpoints.storage_base_url)
            }
            (None, Some(b64)) => GcsClient::new(http.clone(), service_account(http, b64)?)
                .with_base_url(&endpoints.storage_base_url),
            (Some(host), None) => GcsClient::emulator(http, &emulator_base_url(host), None),
            (Some(host), Some(b64)) => {
                let tokens = service_account(http.clone(), b64)?;
                GcsClient::emulator(http, &emulator_base_url(host), Some(tokens))
            }
        }
        .with_object_name_policy(ObjectNamePolicy::Literal);
        Ok(Self {
            writer: client
                .clone()
                .with_timeout(WRITE_TIMEOUT)
                .with_retry(WRITE_RETRY),
            reader: client
                .with_timeout(READ_TIMEOUT)
                .with_retry(READ_RETRY)
                .with_max_download_bytes(max_read_bytes),
            bucket,
            prefix: prefix.trim_matches('/').to_string(),
        })
    }

    /// Both clients' retries on a faster schedule (the same attempt caps and
    /// budget), for tests that count attempts without waiting seconds.
    pub fn with_retry_delay(self, delay: Duration) -> Self {
        let faster = |policy: RetryPolicy| policy.with_initial_delay(delay).with_max_delay(delay);
        let (writer, reader) = (self.writer.retry_policy(), self.reader.retry_policy());
        Self {
            writer: self.writer.with_retry(faster(writer)),
            reader: self.reader.with_retry(faster(reader)),
            ..self
        }
    }

    /// Other per-attempt timeouts, each client's retry deadline scaled with
    /// it (the production ratio, so a timed-out attempt is still never
    /// retried), for tests that stall a response without waiting a minute.
    pub fn with_timeouts(self, write: Duration, read: Duration) -> Self {
        let scaled = |client: GcsClient, timeout: Duration, production: (Duration, Duration)| {
            // Whole seconds in production, so the ratio is exact.
            let secs = |d: Duration| u32::try_from(d.as_secs()).unwrap_or(u32::MAX);
            let (base_timeout, base_deadline) = production;
            let deadline = timeout.saturating_mul(secs(base_deadline)) / secs(base_timeout);
            let policy = client.retry_policy();
            client
                .with_timeout(timeout)
                .with_retry(policy.with_deadline(deadline))
        };
        Self {
            writer: scaled(self.writer, write, (WRITE_TIMEOUT, WRITE_RETRY.deadline)),
            reader: scaled(self.reader, read, (READ_TIMEOUT, READ_RETRY.deadline)),
            ..self
        }
    }

    /// The client for `put`, `exists` and `delete`.
    pub fn writer(&self) -> &GcsClient {
        &self.writer
    }

    /// The client for `get`.
    pub fn reader(&self) -> &GcsClient {
        &self.reader
    }

    /// The object name for `key`: `<prefix>/<key>`, or `key` alone.
    pub fn object_name(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}/{key}", self.prefix)
        }
    }
}

/// `STORAGE_EMULATOR_HOST` as a base URL: `http://` unless it names a
/// scheme, trailing slashes dropped (what this store has always done).
fn emulator_base_url(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", host.trim_end_matches('/'))
    }
}

/// The token provider for a `GOOGLE_CREDENTIALS` key, checked offline first.
/// Every error names the variable and the problem, never a byte of the
/// value: the decoder's and the parser's own messages can quote it.
fn service_account(http: reqwest::Client, b64: &str) -> anyhow::Result<TokenProvider> {
    let decoded = BASE64
        .decode(b64.trim().as_bytes())
        .map_err(|_| anyhow!("GOOGLE_CREDENTIALS is not valid base64"))?;
    let json = with_service_account_type(&decoded)?;
    ServiceAccountKey::from_json(&json)
        .map_err(|e| anyhow!("GOOGLE_CREDENTIALS: {e}"))?
        .validate()
        .map_err(|e| anyhow!("GOOGLE_CREDENTIALS: {e}"))?;
    let tokens =
        TokenProvider::from_service_account_json(http, &[DEVSTORAGE_READ_WRITE_SCOPE], &json)
            .map_err(|e| anyhow!("GOOGLE_CREDENTIALS: {e}"))?;
    // No `x-goog-user-project`, whatever the key's `quota_project_id`: the
    // requests this store has always sent.
    Ok(tokens.with_quota_project(None))
}

/// The key JSON with `"type": "service_account"` added when it has no
/// `type`: parcel never required the field, the library does. A key with
/// another `type` is left for the library to refuse.
fn with_service_account_type(decoded: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(decoded)
        .map_err(|_| anyhow!("GOOGLE_CREDENTIALS is not service-account JSON"))?;
    if let Some(object) = value.as_object_mut() {
        object
            .entry("type")
            .or_insert_with(|| "service_account".into());
    }
    Ok(serde_json::to_vec(&value)?)
}

#[async_trait]
impl BlobStore for GcsBlobStore {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> Result<(), StorageError> {
        let name = self.object_name(key);
        Ok(self
            .writer
            .upload(&self.bucket, &name, bytes, content_type)
            .await?)
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, StorageError> {
        let name = self.object_name(key);
        let object = self.reader.download(&self.bucket, &name).await?;
        Ok(object.map(|(bytes, _content_type)| bytes))
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let name = self.object_name(key);
        Ok(self.writer.exists(&self.bucket, &name).await?)
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        // `false` is a 404: already gone, which is what was asked.
        let name = self.object_name(key);
        self.writer.delete(&self.bucket, &name).await?;
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "gcs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(prefix: &str, emulator: Option<&str>) -> GcsBlobStore {
        GcsBlobStore::new(
            reqwest::Client::new(),
            "bucket".into(),
            prefix.into(),
            emulator.map(str::to_string),
            None,
            1024,
        )
        .unwrap()
    }

    #[test]
    fn names_join_the_trimmed_prefix_and_go_literally() {
        let s = store("/env/", Some("localhost:4447"));
        assert_eq!(
            s.object_name("blobs/sha256/ab/abc"),
            "env/blobs/sha256/ab/abc"
        );
        // Only the outer slashes go: an unusual prefix keeps its inner ones.
        let s = store("//a//b/", Some("localhost:4447"));
        assert_eq!(s.object_name("k"), "a//b/k");
        let s = store("", Some("localhost:4447"));
        assert_eq!(s.object_name("k"), "k");
        for client in [s.writer(), s.reader()] {
            assert_eq!(client.object_name_policy(), ObjectNamePolicy::Literal);
        }
    }

    #[test]
    fn the_emulator_host_keeps_its_normalization() {
        assert_eq!(emulator_base_url("localhost:4447"), "http://localhost:4447");
        assert_eq!(
            emulator_base_url("localhost:4447//"),
            "http://localhost:4447"
        );
        assert_eq!(emulator_base_url("https://gcs.test/"), "https://gcs.test");
        assert_eq!(emulator_base_url("http://gcs.test:1"), "http://gcs.test:1");
        let s = store("", Some("localhost:4447/"));
        assert_eq!(s.writer().base_url(), "http://localhost:4447");
        assert_eq!(s.reader().base_url(), "http://localhost:4447");
        // No emulator: the JSON API.
        let s = store("", None);
        assert_eq!(s.writer().base_url(), DEFAULT_BASE_URL);
    }

    /// The retries, pinned: the publish path and GC get three attempts
    /// within 30 s, a viewer two within 10 s, on Google's backoff — each
    /// deadline under its client's per-attempt timeout, so a timed-out
    /// attempt is never retried.
    #[test]
    fn production_retry_policies_are_pinned() {
        let s = store("", None);
        assert_eq!(
            s.writer().retry_policy(),
            RetryPolicy::google_default()
                .with_max_attempts(3)
                .with_deadline(Duration::from_secs(30))
        );
        assert_eq!(
            s.reader().retry_policy(),
            RetryPolicy::google_default()
                .with_max_attempts(2)
                .with_deadline(Duration::from_secs(10))
        );
        assert_eq!(
            (WRITE_TIMEOUT, READ_TIMEOUT),
            (Duration::from_secs(60), Duration::from_secs(30))
        );
        assert!(s.writer().retry_policy().deadline < WRITE_TIMEOUT);
        assert!(s.reader().retry_policy().deadline < READ_TIMEOUT);
        // The test schedules keep the caps, and the deadlines' ratio.
        let fast = s
            .with_retry_delay(Duration::from_millis(1))
            .with_timeouts(Duration::from_millis(600), Duration::from_millis(300));
        assert_eq!(fast.writer().retry_policy().max_attempts, Some(3));
        assert_eq!(fast.reader().retry_policy().max_attempts, Some(2));
        assert_eq!(
            fast.writer().retry_policy().deadline,
            Duration::from_millis(300)
        );
        assert_eq!(
            fast.reader().retry_policy().deadline,
            Duration::from_millis(100)
        );
    }

    #[test]
    fn a_bad_bucket_name_fails_startup() {
        let err = GcsBlobStore::new(
            reqwest::Client::new(),
            "Not_A_Bucket!".into(),
            String::new(),
            Some("localhost:4447".into()),
            None,
            1024,
        )
        .err()
        .expect("refused");
        let text = format!("{err:#}");
        assert!(
            text.starts_with("GCS_BUCKET: invalid GCS bucket name"),
            "{text}"
        );
        assert!(!text.contains("Not_A_Bucket"), "{text}");
    }

    #[test]
    fn a_missing_type_becomes_service_account() {
        let json = with_service_account_type(br#"{"client_email":"a@b"}"#).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["type"], "service_account");
        assert_eq!(value["client_email"], "a@b");
        // A `type` that is there is kept (and refused by the library later).
        let json = with_service_account_type(br#"{"type":"authorized_user"}"#).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["type"], "authorized_user");
    }
}
