//! Local-directory blob store (development default).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;

use super::{BlobStore, StorageError};

pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn new(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)
            .map_err(|e| anyhow::anyhow!("creating blob directory {}: {e}", root.display()))?;
        Ok(Self { root })
    }

    /// Keys are produced by `blob_key` (hex + fixed segments), never from
    /// user input, so joining them under the root cannot escape it.
    fn path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, key: &str, bytes: Bytes, _content_type: &str) -> Result<(), StorageError> {
        let path = self.path(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Write-then-rename so a reader never observes a half-written blob.
        let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, StorageError> {
        match tokio::fs::read(self.path(key)).await {
            Ok(data) => Ok(Some(Bytes::from(data))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        Ok(tokio::fs::try_exists(self.path(key)).await?)
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        match tokio::fs::remove_file(self.path(key)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn backend(&self) -> &'static str {
        "fs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::new(dir.path()).unwrap();
        assert!(store.get("blobs/x/y").await.unwrap().is_none());
        store
            .put("blobs/x/y", Bytes::from_static(b"hi"), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.get("blobs/x/y").await.unwrap().unwrap(), "hi");
        store.delete("blobs/x/y").await.unwrap();
        store.delete("blobs/x/y").await.unwrap();
        assert!(store.get("blobs/x/y").await.unwrap().is_none());
    }
}
