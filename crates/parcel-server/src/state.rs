//! Shared application state, handed to handlers via axum's `State`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;

use crate::auth::oidc::Provider;
use crate::config::Config;
use crate::storage::{self, BlobStore};
use crate::{content, db};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: PgPool,
    pub http: reqwest::Client,
    pub blobs: Arc<dyn BlobStore>,
    /// Immutable blob bytes keyed by SHA-256, weighed by size. Content
    /// addressing means an entry can never go stale.
    pub blob_cache: moka::future::Cache<String, Bytes>,
    /// For OIDC providers: never follows redirects, so a provider endpoint
    /// can't bounce a code exchange or userinfo call somewhere else.
    pub oidc_http: reqwest::Client,
    /// Sign-in providers, in config order. Built once: each caches its
    /// discovery document and keys.
    pub providers: Arc<Vec<Provider>>,
    /// Bounds concurrent publishes, since each holds its upload in memory.
    pub upload_permits: Arc<Semaphore>,
    /// The script added to every HTML page on the content origin.
    pub helper: Arc<content::helper::Helper>,
    /// Post-commit index tasks (`search::spawn_index`), so a caller can wait
    /// for the ones in flight (`search::wait_for_index_tasks`).
    pub index_tasks: TaskTracker,
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
        let providers = Arc::new(
            config
                .providers
                .iter()
                .cloned()
                .map(Provider::new)
                .collect(),
        );
        let oidc_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("building the OIDC HTTP client");
        let blob_cache = moka::future::Cache::builder()
            .max_capacity(config.blob_cache_max_bytes)
            .weigher(|_key: &String, value: &Bytes| u32::try_from(value.len()).unwrap_or(u32::MAX))
            .build();
        let upload_permits = Arc::new(Semaphore::new(config.max_concurrent_uploads.max(1)));
        let helper = Arc::new(content::helper::Helper::new(&config.app_origin_str()));
        Self {
            config: Arc::new(config),
            db,
            http,
            blobs,
            blob_cache,
            oidc_http,
            providers,
            upload_permits,
            helper,
            index_tasks: TaskTracker::new(),
        }
    }

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.config.id == id)
    }
}
