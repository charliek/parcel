//! Shared application state, handed to handlers via axum's `State`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use sqlx::PgPool;
use stridelabs_auth::{SlauthConfig, Verifier};
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::db;
use crate::storage::{self, BlobStore};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub http: reqwest::Client,
    pub blobs: Arc<dyn BlobStore>,
    /// Immutable blob bytes keyed by SHA-256, weighed by size. Content
    /// addressing means an entry can never go stale.
    pub blob_cache: moka::future::Cache<String, Bytes>,
    /// slauth access-token verifier. Built once: it owns the JWKS cache.
    pub verifier: Verifier,
    /// Bounds concurrent publishes, since each holds its upload in memory.
    pub upload_permits: Arc<Semaphore>,
}

impl AppState {
    /// Build state from config with a lazily connected pool.
    pub fn new(config: Config) -> Result<Self> {
        let db = db::connect_lazy(&config.database_url)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("building HTTP client")?;
        let blobs = storage::from_config(&config.blob_backend, http.clone())?;
        Ok(Self::with_parts(config, db, http, blobs))
    }

    /// Assemble state from already-built parts (tests inject a pool and a
    /// memory blob store this way).
    pub fn with_parts(
        config: Config,
        db: PgPool,
        http: reqwest::Client,
        blobs: Arc<dyn BlobStore>,
    ) -> Self {
        let verifier = Verifier::new(
            SlauthConfig {
                issuer: config.slauth.issuer.clone(),
                jwks_url: config.slauth.jwks_url.clone(),
                audience: config.slauth.audience.clone(),
                pat_validate_url: None,
            },
            http.clone(),
        );
        let blob_cache = moka::future::Cache::builder()
            .max_capacity(config.blob_cache_max_bytes)
            .weigher(|_key: &String, value: &Bytes| u32::try_from(value.len()).unwrap_or(u32::MAX))
            .build();
        let upload_permits = Arc::new(Semaphore::new(config.max_concurrent_uploads.max(1)));
        Self {
            config: Arc::new(config),
            db,
            http,
            blobs,
            blob_cache,
            verifier,
            upload_permits,
        }
    }
}
