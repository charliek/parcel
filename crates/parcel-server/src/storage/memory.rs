//! In-memory blob store for tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;

use super::{BlobStore, StorageError};

#[derive(Default)]
pub struct MemoryBlobStore {
    objects: Mutex<HashMap<String, Bytes>>,
}

impl MemoryBlobStore {
    /// Number of stored objects (tests assert GC behavior with this).
    pub fn len(&self) -> usize {
        self.objects.lock().expect("lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, key: &str) -> bool {
        self.objects.lock().expect("lock").contains_key(key)
    }
}

#[async_trait]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, bytes: Bytes, _content_type: &str) -> Result<(), StorageError> {
        self.objects
            .lock()
            .expect("lock")
            .insert(key.to_string(), bytes);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>, StorageError> {
        Ok(self.objects.lock().expect("lock").get(key).cloned())
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.objects.lock().expect("lock").remove(key);
        Ok(())
    }

    fn backend(&self) -> &'static str {
        "memory"
    }
}
