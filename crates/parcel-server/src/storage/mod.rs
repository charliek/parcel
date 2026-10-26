//! Blob storage: the bytes behind every published file.
//!
//! Blobs are content-addressed (`blobs/sha256/ab/<hash>`), so a key's bytes
//! never change once written — which is what makes the content server's
//! in-memory cache safe and lets identical files share one object across
//! versions. The database decides what is referenced; this layer only moves
//! bytes. Three backends implement [`BlobStore`]: a local directory (dev), a
//! GCS bucket (prod; fake-gcs-server locally), and process memory (tests).

mod fs;
pub mod gcs;
mod memory;

use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use bytes::Bytes;
use parcel_types::Limits;

pub use fs::FsBlobStore;
pub use gcs::{GcsBlobStore, GcsEndpoints};
pub use memory::MemoryBlobStore;

use crate::config::BlobBackend;

/// A failure talking to the blob store. Messages never include credentials.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("blob store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// stridelabs-gcp's error, whose `Display` and `Debug` are safe to log:
    /// the operation, the status and Google's plain `error.status` code,
    /// never an object name or Google's free-text message (only the
    /// library's `sensitive_message()` reaches that, and nothing here calls
    /// it), and no URL beyond a token endpoint's origin.
    #[error(transparent)]
    Gcs(#[from] stridelabs_gcp::StorageError),
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

/// The floor under [`read_cap`]: the default upload limit.
const READ_CAP_FLOOR: i64 = 64 * 1024 * 1024;

/// The largest blob a read accepts (the GCS reader's download cap):
/// `max(per-file cap, max_upload_bytes, 64 MiB)`, the per-file cap being
/// `max(max_entry_bytes, max_file_bytes)` as a publish applies it. No blob
/// is larger than the limits in force when it was stored, so a stored blob
/// becomes unreadable only if its size exceeds 64 MiB *and* every current
/// one of `max_entry_bytes`, `max_file_bytes` and `max_upload_bytes` (e.g.
/// 100 MiB stored under 128 MiB limits, then all three lowered to 96 MiB).
pub fn read_cap(limits: &Limits) -> anyhow::Result<usize> {
    let per_file_cap = limits.max_entry_bytes.max(limits.max_file_bytes);
    let cap = per_file_cap
        .max(limits.max_upload_bytes)
        .max(READ_CAP_FLOOR);
    usize::try_from(cap).context("the blob read cap does not fit in usize")
}

/// Build the configured backend.
pub fn from_config(
    backend: &BlobBackend,
    http: reqwest::Client,
    limits: &Limits,
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
            credentials_b64.as_ref().map(|key| key.0.as_str()),
            read_cap(limits)?,
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

    #[test]
    fn the_read_cap_covers_every_limit_and_has_a_floor() {
        const MIB: i64 = 1024 * 1024;
        let cap = |entry: i64, file: i64, upload: i64| {
            let limits = Limits {
                max_entry_bytes: entry,
                max_file_bytes: file,
                max_upload_bytes: upload,
                max_files_per_upload: 1,
                max_files_per_version: 1,
                max_version_bytes: 1,
                user_quota_bytes: 0,
                max_data_bytes: 1,
                max_data_value_bytes: 1,
            };
            i64::try_from(read_cap(&limits).unwrap()).unwrap()
        };
        // The defaults (16 / 15 / 64 MiB), and small limits: the floor.
        assert_eq!(cap(16 * MIB, 15 * MIB, 64 * MIB), 64 * MIB);
        assert_eq!(cap(MIB, MIB, 2 * MIB), 64 * MIB);
        // Otherwise whichever limit is largest.
        assert_eq!(cap(100 * MIB, MIB, 70 * MIB), 100 * MIB);
        assert_eq!(cap(MIB, 90 * MIB, 70 * MIB), 90 * MIB);
        assert_eq!(cap(MIB, MIB, 80 * MIB), 80 * MIB);
    }
}
