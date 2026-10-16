//! Application configuration, loaded from the environment (12-factor).
//!
//! Env var names and defaults are an operator contract: renaming one is a
//! deploy-breaking change. Every value is read through [`Config::from_lookup`],
//! an injectable lookup, so tests exercise parsing without mutating the
//! process environment. `.env` is loaded by the binary (dotenvy) before this
//! runs.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use parcel_types::Limits;
use stridelabs_config::validate::Errors;
pub use stridelabs_observability::LogFormat;
use url::Url;

/// Deployment environment. Gates the dev-auth guard, Swagger UI, log format
/// defaults and the content-signing-key requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Production,
}

impl AppEnv {
    pub fn as_str(self) -> &'static str {
        match self {
            AppEnv::Development => "development",
            AppEnv::Production => "production",
        }
    }
}

/// How API requests are authenticated.
///
/// `Dev` is a local-only bypass that treats every request without a parcel
/// token as a fixed user. It is OFF by default and refused when
/// `APP_ENV=production`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Slauth,
    Dev,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::Slauth => "slauth",
            AuthMode::Dev => "dev",
        }
    }
}

/// slauth (Ory Hydra) settings. `issuer`/`jwks_url`/`audience` are the
/// resource-server half; `client_id` is handed to the SPA for its PKCE flow.
#[derive(Debug, Clone)]
pub struct SlauthSettings {
    pub issuer: String,
    pub jwks_url: String,
    pub audience: String,
    pub client_id: String,
}

/// Where blob bytes live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobBackend {
    /// A local directory (development default).
    Fs { root: String },
    /// A Google Cloud Storage bucket. `emulator_host` points at
    /// fake-gcs-server locally (`STORAGE_EMULATOR_HOST`).
    Gcs {
        bucket: String,
        prefix: String,
        emulator_host: Option<String>,
        /// Base64 service-account JSON. Absent on Cloud Run, where the
        /// runtime service account's token comes from the metadata server.
        credentials_b64: Option<String>,
    },
    /// Process memory. Tests only — nothing survives a restart.
    Memory,
}

/// Fully resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub app_env: AppEnv,
    pub bind_addr: SocketAddr,
    pub log_format: LogFormat,
    pub database_url: String,

    /// The SPA + API origin, e.g. `https://parcel.stridelabs.ai`.
    pub app_origin: Url,
    /// The sandboxed user-content origin, e.g. `https://parcelcontent.stridelabs.ai`.
    pub content_origin: Url,
    /// HMAC key for content capability tokens.
    pub content_signing_key: Vec<u8>,
    pub content_token_ttl: Duration,

    pub auth_mode: AuthMode,
    pub dev_auth_email: String,
    pub dev_auth_name: String,
    pub slauth: SlauthSettings,

    pub blob_backend: BlobBackend,
    /// In-memory cache of blob bytes for the content server (immutable,
    /// content-addressed, so never stale).
    pub blob_cache_max_bytes: u64,

    pub limits: Limits,
    pub max_concurrent_uploads: usize,

    /// Built SPA to serve (with index.html fallback); `None` in dev, where
    /// Vite serves the app.
    pub static_dir: Option<String>,

    pub gc_interval: Duration,
    pub gc_grace: Duration,
}

const DEV_SIGNING_KEY: &str = "parcel-dev-content-signing-key-not-for-production";

const MIB: i64 = 1024 * 1024;

impl Default for Config {
    /// Development defaults — what a fresh checkout runs with under prox.
    /// Tests build on this directly so they never depend on process env.
    fn default() -> Self {
        Self {
            app_env: AppEnv::Development,
            bind_addr: "0.0.0.0:8106".parse().expect("valid default bind addr"),
            log_format: LogFormat::Text,
            database_url: "postgres://postgres:localdev@localhost:5439/parcel".into(),
            app_origin: Url::parse("https://parcel.local.stridelabs.ai").expect("valid url"),
            content_origin: Url::parse("https://parcelcontent.local.stridelabs.ai")
                .expect("valid url"),
            content_signing_key: DEV_SIGNING_KEY.as_bytes().to_vec(),
            content_token_ttl: Duration::from_secs(12 * 3600),
            auth_mode: AuthMode::Slauth,
            dev_auth_email: "dev@example.com".into(),
            dev_auth_name: "Dev User".into(),
            slauth: SlauthSettings {
                issuer: "https://auth.local.stridelabs.ai".into(),
                jwks_url: "https://auth.local.stridelabs.ai/.well-known/jwks.json".into(),
                audience: "parcel".into(),
                client_id: "parcel".into(),
            },
            blob_backend: BlobBackend::Fs {
                root: "./data/blobs".into(),
            },
            blob_cache_max_bytes: 128 * 1024 * 1024,
            limits: Limits {
                max_entry_bytes: 16 * MIB,
                max_file_bytes: 15 * MIB,
                max_upload_bytes: 64 * MIB,
                max_files_per_upload: 256,
                max_files_per_version: 512,
                max_version_bytes: 256 * MIB,
                user_quota_bytes: 2048 * MIB,
            },
            max_concurrent_uploads: 8,
            static_dir: None,
            gc_interval: Duration::from_secs(3600),
            gc_grace: Duration::from_secs(3600),
        }
    }
}

/// Reads one variable, treating empty as unset (the `stridelabs_config::env_or`
/// rule: an unfilled compose/k8s/.env entry is "not set", not "set to empty").
struct Lookup<F: Fn(&str) -> Option<String>>(F);

impl<F: Fn(&str) -> Option<String>> Lookup<F> {
    fn get(&self, key: &str) -> Option<String> {
        (self.0)(key).filter(|v| !v.trim().is_empty())
    }

    fn or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or_else(|| default.to_string())
    }

    /// Parse `key` when set, recording a validation error (and keeping the
    /// default) when it doesn't parse — so a bad config reports every problem
    /// at once instead of the first.
    fn parse<T: std::str::FromStr>(&self, key: &str, default: T, errs: &mut Errors) -> T
    where
        T::Err: std::fmt::Display,
    {
        match self.get(key) {
            Some(raw) => match raw.trim().parse::<T>() {
                Ok(v) => v,
                Err(e) => {
                    errs.push(key, e.to_string());
                    default
                }
            },
            None => default,
        }
    }
}

impl Config {
    /// Load from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Load from an arbitrary lookup (tests pass a map).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let env = Lookup(get);
        let d = Config::default();
        let mut errs = Errors::new();

        let app_env = match env
            .or("APP_ENV", "development")
            .to_ascii_lowercase()
            .as_str()
        {
            "production" | "prod" => AppEnv::Production,
            "development" | "dev" => AppEnv::Development,
            other => {
                errs.push(
                    "APP_ENV",
                    format!("expected development|production, got {other:?}"),
                );
                AppEnv::Development
            }
        };

        // Text locally, JSON wherever a log aggregator is listening.
        let default_format = match app_env {
            AppEnv::Development => "text",
            AppEnv::Production => "json",
        };
        let log_format = match env
            .or("LOG_FORMAT", default_format)
            .to_ascii_lowercase()
            .as_str()
        {
            "json" => LogFormat::Json,
            "text" => LogFormat::Text,
            other => {
                errs.push("LOG_FORMAT", format!("expected text|json, got {other:?}"));
                LogFormat::Text
            }
        };

        let auth_mode = match env.or("AUTH_MODE", "slauth").to_ascii_lowercase().as_str() {
            "slauth" => AuthMode::Slauth,
            "dev" => AuthMode::Dev,
            other => {
                errs.push("AUTH_MODE", format!("expected slauth|dev, got {other:?}"));
                AuthMode::Slauth
            }
        };
        if auth_mode == AuthMode::Dev && app_env == AppEnv::Production {
            errs.push(
                "AUTH_MODE",
                "AUTH_MODE=dev is forbidden when APP_ENV=production; use AUTH_MODE=slauth",
            );
        }

        // Cloud Run injects PORT; an explicit BIND_ADDR wins over it.
        let bind_addr = match (env.get("BIND_ADDR"), env.get("PORT")) {
            (Some(addr), _) => addr.parse().unwrap_or_else(|e| {
                errs.push("BIND_ADDR", format!("{e}"));
                d.bind_addr
            }),
            (None, Some(port)) => match port.parse::<u16>() {
                Ok(p) => SocketAddr::from(([0, 0, 0, 0], p)),
                Err(e) => {
                    errs.push("PORT", e.to_string());
                    d.bind_addr
                }
            },
            (None, None) => d.bind_addr,
        };

        let app_origin = parse_origin(&env, "PARCEL_APP_ORIGIN", &d.app_origin, &mut errs);
        let content_origin =
            parse_origin(&env, "PARCEL_CONTENT_ORIGIN", &d.content_origin, &mut errs);
        if app_origin.host_str() == content_origin.host_str()
            && app_origin.port_or_known_default() == content_origin.port_or_known_default()
        {
            errs.push(
                "PARCEL_CONTENT_ORIGIN",
                "must differ from PARCEL_APP_ORIGIN: user content is never served from the app origin",
            );
        }

        let content_signing_key = match env.get("PARCEL_CONTENT_SIGNING_KEY") {
            Some(key) if key.len() >= 32 => key.into_bytes(),
            Some(_) => {
                errs.push(
                    "PARCEL_CONTENT_SIGNING_KEY",
                    "must be at least 32 characters",
                );
                d.content_signing_key.clone()
            }
            None if app_env == AppEnv::Production => {
                errs.push(
                    "PARCEL_CONTENT_SIGNING_KEY",
                    "is required when APP_ENV=production",
                );
                d.content_signing_key.clone()
            }
            None => d.content_signing_key.clone(),
        };

        let content_token_ttl = Duration::from_secs(env.parse(
            "PARCEL_CONTENT_TOKEN_TTL_SECS",
            d.content_token_ttl.as_secs(),
            &mut errs,
        ));

        let blob_backend = match env.or("BLOB_BACKEND", "fs").to_ascii_lowercase().as_str() {
            "fs" => BlobBackend::Fs {
                root: env.or("BLOB_FS_ROOT", "./data/blobs"),
            },
            "gcs" => {
                let bucket = env.get("GCS_BUCKET").unwrap_or_else(|| {
                    errs.push("GCS_BUCKET", "is required when BLOB_BACKEND=gcs");
                    String::new()
                });
                BlobBackend::Gcs {
                    bucket,
                    prefix: env.or("GCS_PREFIX", ""),
                    emulator_host: env.get("STORAGE_EMULATOR_HOST"),
                    credentials_b64: env.get("GOOGLE_CREDENTIALS"),
                }
            }
            "memory" => BlobBackend::Memory,
            other => {
                errs.push(
                    "BLOB_BACKEND",
                    format!("expected fs|gcs|memory, got {other:?}"),
                );
                d.blob_backend.clone()
            }
        };

        let l = &d.limits;
        let limits = Limits {
            max_entry_bytes: env.parse("PARCEL_MAX_ENTRY_BYTES", l.max_entry_bytes, &mut errs),
            max_file_bytes: env.parse("PARCEL_MAX_FILE_BYTES", l.max_file_bytes, &mut errs),
            max_upload_bytes: env.parse("PARCEL_MAX_UPLOAD_BYTES", l.max_upload_bytes, &mut errs),
            max_files_per_upload: env.parse(
                "PARCEL_MAX_FILES_PER_UPLOAD",
                l.max_files_per_upload,
                &mut errs,
            ),
            max_files_per_version: env.parse(
                "PARCEL_MAX_FILES_PER_VERSION",
                l.max_files_per_version,
                &mut errs,
            ),
            max_version_bytes: env.parse(
                "PARCEL_MAX_VERSION_BYTES",
                l.max_version_bytes,
                &mut errs,
            ),
            user_quota_bytes: env.parse("PARCEL_USER_QUOTA_BYTES", l.user_quota_bytes, &mut errs),
        };
        validate_limits(&limits, &mut errs);

        let config = Config {
            app_env,
            bind_addr,
            log_format,
            database_url: env.or("DATABASE_URL", &d.database_url),
            app_origin,
            content_origin,
            content_signing_key,
            content_token_ttl,
            auth_mode,
            dev_auth_email: env.or("DEV_AUTH_EMAIL", &d.dev_auth_email),
            dev_auth_name: env.or("DEV_AUTH_NAME", &d.dev_auth_name),
            slauth: SlauthSettings {
                issuer: env.or("SLAUTH_ISSUER", &d.slauth.issuer),
                jwks_url: env.or("SLAUTH_JWKS_URL", &d.slauth.jwks_url),
                audience: env.or("SLAUTH_AUDIENCE", &d.slauth.audience),
                client_id: env.or("SLAUTH_CLIENT_ID", &d.slauth.client_id),
            },
            blob_backend,
            blob_cache_max_bytes: env.parse(
                "BLOB_CACHE_MAX_BYTES",
                d.blob_cache_max_bytes,
                &mut errs,
            ),
            limits,
            max_concurrent_uploads: env.parse(
                "PARCEL_MAX_CONCURRENT_UPLOADS",
                d.max_concurrent_uploads,
                &mut errs,
            ),
            static_dir: env.get("PARCEL_STATIC_DIR"),
            gc_interval: Duration::from_secs(env.parse(
                "PARCEL_GC_INTERVAL_SECS",
                d.gc_interval.as_secs(),
                &mut errs,
            )),
            gc_grace: Duration::from_secs(env.parse(
                "PARCEL_GC_GRACE_SECS",
                d.gc_grace.as_secs(),
                &mut errs,
            )),
        };

        errs.into_result().map_err(|list| {
            let lines: Vec<String> = list
                .iter()
                .map(|e| format!("  {}: {}", e.location, e.message))
                .collect();
            anyhow!("invalid configuration:\n{}", lines.join("\n"))
        })?;
        Ok(config)
    }

    /// The origin string the SPA and CORS-free checks compare against
    /// (`https://host[:port]`, no trailing slash).
    pub fn app_origin_str(&self) -> String {
        origin_string(&self.app_origin)
    }

    pub fn content_origin_str(&self) -> String {
        origin_string(&self.content_origin)
    }

    /// The viewer URL for a parcel in the web app.
    pub fn viewer_url(&self, parcel_id: &str) -> String {
        format!("{}/p/{parcel_id}", self.app_origin_str())
    }
}

/// `scheme://host[:port]` without a trailing slash.
pub fn origin_string(url: &Url) -> String {
    url.origin().ascii_serialization()
}

fn parse_origin<F: Fn(&str) -> Option<String>>(
    env: &Lookup<F>,
    key: &str,
    default: &Url,
    errs: &mut Errors,
) -> Url {
    let Some(raw) = env.get(key) else {
        return default.clone();
    };
    match Url::parse(raw.trim()) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {
            if url.path() != "/" || url.query().is_some() {
                errs.push(key, "must be an origin (scheme://host[:port]) with no path");
            }
            url
        }
        Ok(_) => {
            errs.push(key, "must be an http(s) origin");
            default.clone()
        }
        Err(e) => {
            errs.push(key, e.to_string());
            default.clone()
        }
    }
}

fn validate_limits(l: &Limits, errs: &mut Errors) {
    let positive: [(&str, i64); 6] = [
        ("PARCEL_MAX_ENTRY_BYTES", l.max_entry_bytes),
        ("PARCEL_MAX_FILE_BYTES", l.max_file_bytes),
        ("PARCEL_MAX_UPLOAD_BYTES", l.max_upload_bytes),
        (
            "PARCEL_MAX_FILES_PER_UPLOAD",
            i64::from(l.max_files_per_upload),
        ),
        (
            "PARCEL_MAX_FILES_PER_VERSION",
            i64::from(l.max_files_per_version),
        ),
        ("PARCEL_MAX_VERSION_BYTES", l.max_version_bytes),
    ];
    for (key, value) in positive {
        if value <= 0 {
            errs.push(key, "must be greater than zero");
        }
    }
    if l.user_quota_bytes < 0 {
        errs.push(
            "PARCEL_USER_QUOTA_BYTES",
            "must be zero (unlimited) or positive",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn load(pairs: &[(&str, &str)]) -> Result<Config> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(move |k| map.get(k).cloned())
    }

    #[test]
    fn defaults_load() {
        let c = load(&[]).unwrap();
        assert_eq!(c.app_env, AppEnv::Development);
        assert_eq!(c.auth_mode, AuthMode::Slauth);
        assert_eq!(c.limits.max_file_bytes, 15 * MIB);
        assert_eq!(c.app_origin_str(), "https://parcel.local.stridelabs.ai");
    }

    #[test]
    fn dev_auth_is_refused_in_production() {
        let err = load(&[
            ("APP_ENV", "production"),
            ("AUTH_MODE", "dev"),
            ("PARCEL_CONTENT_SIGNING_KEY", &"k".repeat(40)),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("AUTH_MODE=dev is forbidden"),
            "{err}"
        );
    }

    #[test]
    fn production_requires_a_signing_key() {
        let err = load(&[("APP_ENV", "production")]).unwrap_err();
        assert!(
            err.to_string().contains("PARCEL_CONTENT_SIGNING_KEY"),
            "{err}"
        );
        let ok = load(&[
            ("APP_ENV", "production"),
            ("PARCEL_CONTENT_SIGNING_KEY", &"k".repeat(40)),
        ])
        .unwrap();
        assert_eq!(ok.log_format, LogFormat::Json);
    }

    #[test]
    fn content_origin_must_differ() {
        let err = load(&[
            ("PARCEL_APP_ORIGIN", "https://a.example"),
            ("PARCEL_CONTENT_ORIGIN", "https://a.example"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("must differ"), "{err}");
    }

    #[test]
    fn port_env_is_honored() {
        let c = load(&[("PORT", "9000")]).unwrap();
        assert_eq!(c.bind_addr.port(), 9000);
        let c = load(&[("PORT", "9000"), ("BIND_ADDR", "127.0.0.1:7000")]).unwrap();
        assert_eq!(c.bind_addr.port(), 7000);
    }

    #[test]
    fn reports_every_bad_value() {
        let err = load(&[
            ("PARCEL_MAX_FILE_BYTES", "lots"),
            ("BLOB_BACKEND", "gcs"),
            ("LOG_FORMAT", "xml"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("PARCEL_MAX_FILE_BYTES"), "{err}");
        assert!(err.contains("GCS_BUCKET"), "{err}");
        assert!(err.contains("LOG_FORMAT"), "{err}");
    }

    #[test]
    fn empty_values_count_as_unset() {
        let c = load(&[("APP_ENV", ""), ("BLOB_BACKEND", " ")]).unwrap();
        assert_eq!(c.app_env, AppEnv::Development);
        assert!(matches!(c.blob_backend, BlobBackend::Fs { .. }));
    }
}
