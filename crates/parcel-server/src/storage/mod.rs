//! Blob storage: the bytes behind every published file.
//!
//! Blobs are content-addressed (`blobs/sha256/ab/<hash>`), so a key's bytes
//! never change once written — which is what makes the content server's
//! in-memory cache safe and lets identical files share one object across
//! versions. The database decides what is referenced; this layer only moves
//! bytes. Three backends implement [`BlobStore`]: a local directory (dev), a
//! GCS bucket (prod; fake-gcs-server locally), and process memory (tests).

mod fs;
mod gcs;
mod google_auth;
mod memory;

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

pub use fs::FsBlobStore;
pub use gcs::GcsBlobStore;
pub use memory::MemoryBlobStore;

use crate::config::BlobBackend;

/// A failure talking to the blob store. Messages never include credentials.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("blob store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob store request failed: {0}")]
    Request(String),
    #[error("blob store returned HTTP {status} for {operation}")]
    Status {
        status: u16,
        operation: &'static str,
    },
    #[error("blob store authentication failed: {0}")]
    Auth(String),
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Write `bytes` at `key`, replacing anything there (idempotent: keys are
    /// content addresses, so a rewrite stores identical bytes).
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> Result<(), StorageError>;

    /// Read `key`, or `None` if it does not exist.
    async fn get(&self, key: &str) -> Result<Option<Bytes>, StorageError>;

    /// Whether `key` exists, without fetching its bytes.
    async fn exists(&self, key: &str) -> Result<bool, StorageError>;

    /// Delete `key`. Deleting a missing key is not an error.
    async fn delete(&self, key: &str) -> Result<(), StorageError>;

    /// Short backend name for logs and readiness output.
    fn backend(&self) -> &'static str;
}

/// The storage key for a blob, sharded by the first byte of its hash so no
/// single directory (or GCS listing prefix) grows unbounded.
pub fn blob_key(sha256: &str) -> String {
    let shard = sha256.get(..2).unwrap_or("xx");
    format!("blobs/sha256/{shard}/{sha256}")
}

/// Build the configured backend.
pub fn from_config(
    backend: &BlobBackend,
    http: reqwest::Client,
) -> anyhow::Result<Arc<dyn BlobStore>> {
    Ok(match backend {
        BlobBackend::Fs { root } => Arc::new(FsBlobStore::new(root)?),
        BlobBackend::Gcs {
            bucket,
            prefix,
            emulator_host,
            credentials_b64,
        } => Arc::new(GcsBlobStore::new(
            http,
            bucket.clone(),
            prefix.clone(),
            emulator_host.clone(),
            credentials_b64.as_deref(),
        )?),
        BlobBackend::Memory => Arc::new(MemoryBlobStore::default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sharded() {
        assert_eq!(blob_key("abcdef"), "blobs/sha256/ab/abcdef");
    }
}
