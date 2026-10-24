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
    /// People sign in through the configured OIDC providers.
    Oidc,
    Dev,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::Oidc => "oidc",
            AuthMode::Dev => "dev",
        }
    }
}

/// A value that must never reach logs (`Debug` prints a placeholder).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// One OIDC provider people can sign in with (`AUTH_<ID>_*`).
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// `[a-z][a-z0-9]{0,31}`; stored with each identity, so renaming one
    /// relinks its users by email on their next sign-in.
    pub id: String,
    pub label: String,
    /// Exactly as the provider's discovery document states it.
    pub issuer: String,
    pub client_id: String,
    /// Absent: a public client (PKCE only), as slauth's `parcel` client is.
    pub client_secret: Option<Secret>,
    /// Lowercase email domains this provider may sign in; empty = any.
    pub allowed_domains: Vec<String>,
    /// Accept emails the provider doesn't mark `email_verified`.
    pub trust_email: bool,
    pub scopes: String,
    /// Signing out of Parcel also signs out of the provider (its
    /// `end_session_endpoint`). Off by default: leaving Parcel shouldn't end
    /// someone's company-wide SSO session. On for `slauth`.
    pub provider_logout: bool,
}

impl ProviderConfig {
    pub fn allows_domain(&self, domain: &str) -> bool {
        self.allowed_domains.is_empty() || self.allowed_domains.iter().any(|d| d == domain)
    }
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
    /// In `AUTH_PROVIDERS` order (the sign-in page's order).
    pub providers: Vec<ProviderConfig>,
    /// How long a web session lasts before the provider is asked again.
    pub session_ttl: Duration,
    /// API tokens stop working when their owner's last web sign-in is older
    /// than this; `None` = never.
    pub token_login_max_age: Option<Duration>,
    /// Whether parcels may be public. When false, stored `public` parcels
    /// behave and report as `internal`.
    pub allow_public: bool,

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

    /// Apply migrations at boot. Production turns this off and runs
    /// `parcel-server migrate` as a job on Neon's direct (non-pooled) URL.
    pub run_migrations: bool,

    /// The MCP server at `<app origin>/mcp`; `None` when off.
    pub mcp: Option<McpConfig>,
}

/// The MCP server: which provider's access tokens it accepts. Those tokens
/// are minted for the MCP resource (`aud` = `<app origin>/mcp`), never the
/// web client, so they work nowhere else in the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    /// A configured provider's id (`PARCEL_MCP_PROVIDER`, default `slauth`).
    pub provider: String,
    /// That provider's issuer.
    pub issuer: String,
    /// Where its signing keys are (`PARCEL_MCP_JWKS_URL`, default
    /// `<issuer>/.well-known/jwks.json`, which is slauth's).
    pub jwks_url: String,
}

pub const DEFAULT_SCOPES: &str = "openid email profile";

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
            auth_mode: AuthMode::Oidc,
            dev_auth_email: "dev@example.com".into(),
            dev_auth_name: "Dev User".into(),
            providers: vec![ProviderConfig {
                id: "slauth".into(),
                label: "slauth".into(),
                issuer: "https://auth.local.stridelabs.ai".into(),
                client_id: "parcel".into(),
                client_secret: None,
                allowed_domains: Vec::new(),
                trust_email: true,
                scopes: DEFAULT_SCOPES.into(),
                provider_logout: true,
            }],
            session_ttl: Duration::from_secs(7 * 24 * 3600),
            token_login_max_age: Some(Duration::from_secs(30 * 24 * 3600)),
            allow_public: true,
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
                max_data_bytes: 10 * MIB,
                max_data_value_bytes: 256 * 1024,
            },
            max_concurrent_uploads: 8,
            static_dir: None,
            gc_interval: Duration::from_secs(3600),
            gc_grace: Duration::from_secs(3600),
            run_migrations: true,
            mcp: None,
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

        // Text locally, JSON wherever a log aggregator is listening. `cloud`
        // (Google Cloud Logging lines: `severity`, a top-level `message`,
        // `requestId`) is opt-in for now: it writes no span fields, so until
        // request logging can carry the redacted path on its own line
        // (RequestLogLayer logs the raw one), `json` keeps more per line.
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
            "cloud" => LogFormat::Cloud,
            other => {
                errs.push(
                    "LOG_FORMAT",
                    format!("expected text|json|cloud, got {other:?}"),
                );
                LogFormat::Text
            }
        };

        // `slauth` was the only mode before providers became configurable.
        let auth_mode = match env.or("AUTH_MODE", "oidc").to_ascii_lowercase().as_str() {
            "oidc" | "slauth" => AuthMode::Oidc,
            "dev" => AuthMode::Dev,
            other => {
                errs.push("AUTH_MODE", format!("expected oidc|dev, got {other:?}"));
                AuthMode::Oidc
            }
        };
        if auth_mode == AuthMode::Dev && app_env == AppEnv::Production {
            errs.push(
                "AUTH_MODE",
                "AUTH_MODE=dev is forbidden when APP_ENV=production; use AUTH_MODE=oidc",
            );
        }
        let providers = parse_providers(&env, app_env, auth_mode, &mut errs);
        let token_login_max_age_days: u64 = env.parse(
            "PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS",
            d.token_login_max_age.map_or(0, |a| a.as_secs() / 86400),
            &mut errs,
        );

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
            max_data_bytes: env.parse("PARCEL_MAX_DATA_BYTES", l.max_data_bytes, &mut errs),
            max_data_value_bytes: env.parse(
                "PARCEL_MAX_DATA_VALUE_BYTES",
                l.max_data_value_bytes,
                &mut errs,
            ),
        };
        validate_limits(&limits, &mut errs);
        let mcp = parse_mcp(&env, auth_mode, &providers, &app_origin, &mut errs);

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
            providers,
            session_ttl: Duration::from_secs(env.parse(
                "PARCEL_SESSION_TTL_SECS",
                d.session_ttl.as_secs(),
                &mut errs,
            )),
            token_login_max_age: (token_login_max_age_days > 0)
                .then(|| Duration::from_secs(token_login_max_age_days.saturating_mul(86400))),
            allow_public: env.parse("PARCEL_ALLOW_PUBLIC", d.allow_public, &mut errs),
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
            run_migrations: env.parse("RUN_MIGRATIONS", d.run_migrations, &mut errs),
            mcp,
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

    /// The URL to show for a parcel: `<app>/@<handle>/<slug>` when it has a
    /// slug (and its owner a handle), else the viewer URL. Handles and
    /// slugs are URL-safe by construction (`parcel_types::slugs`).
    pub fn canonical_url(
        &self,
        parcel_id: &str,
        handle: Option<&str>,
        slug: Option<&str>,
    ) -> String {
        match (handle, slug) {
            (Some(handle), Some(slug)) => format!("{}/@{handle}/{slug}", self.app_origin_str()),
            _ => self.viewer_url(parcel_id),
        }
    }

    /// Where providers send people back after sign-in.
    pub fn auth_callback_url(&self) -> String {
        format!("{}/auth/callback", self.app_origin_str())
    }

    /// The MCP server's canonical resource URL: its address, and the
    /// audience its tokens must carry.
    pub fn mcp_resource_url(&self) -> String {
        format!("{}/mcp", self.app_origin_str())
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Whether some configured provider could sign in an address at
    /// `domain` (always true in dev mode).
    pub fn domain_can_sign_in(&self, domain: &str) -> bool {
        self.auth_mode == AuthMode::Dev || self.providers.iter().any(|p| p.allows_domain(domain))
    }

    /// The key for one kind of signed cookie, derived from the content
    /// signing key with a per-purpose label, so there is one secret to
    /// manage and a value signed for one purpose never verifies as another.
    pub fn cookie_key(&self, purpose: &str) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.content_signing_key)
            .expect("HMAC accepts keys of any length");
        mac.update(b"parcel-cookie-v1:");
        mac.update(purpose.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    /// Cookies are `Secure` (and `__Host-`/`__Secure-` prefixed) unless the
    /// app is served over plain http (local e2e).
    pub fn secure_cookies(&self) -> bool {
        self.app_origin.scheme() == "https"
    }
}

/// `PARCEL_MCP_ENABLED` and the provider whose tokens `/mcp` accepts.
fn parse_mcp<F: Fn(&str) -> Option<String>>(
    env: &Lookup<F>,
    auth_mode: AuthMode,
    providers: &[ProviderConfig],
    app_origin: &Url,
    errs: &mut Errors,
) -> Option<McpConfig> {
    if !env.parse("PARCEL_MCP_ENABLED", false, errs) {
        return None;
    }
    if auth_mode != AuthMode::Oidc {
        errs.push("PARCEL_MCP_ENABLED", "the MCP server needs AUTH_MODE=oidc");
        return None;
    }
    // The resource URL is the token audience, which the provider registers
    // as an exact canonical string: https and no port.
    if app_origin.scheme() != "https" || app_origin.port().is_some() {
        errs.push(
            "PARCEL_MCP_ENABLED",
            format!(
                "the MCP server needs an https PARCEL_APP_ORIGIN without a port, got {}",
                origin_string(app_origin)
            ),
        );
        return None;
    }
    let id = env.or("PARCEL_MCP_PROVIDER", "slauth").to_ascii_lowercase();
    let Some(provider) = providers.iter().find(|p| p.id == id) else {
        errs.push(
            "PARCEL_MCP_PROVIDER",
            format!("{id:?} is not one of the configured AUTH_PROVIDERS"),
        );
        return None;
    };
    if let Err(e) = stridelabs_mcp::ProtectedResource::new(
        &format!("{}/mcp", origin_string(app_origin)),
        provider.issuer.as_str(),
        ["parcel:read"],
    ) {
        errs.push(
            "PARCEL_APP_ORIGIN",
            format!("not usable as an MCP resource: {e}"),
        );
        return None;
    }
    let jwks_url = env.get("PARCEL_MCP_JWKS_URL").unwrap_or_else(|| {
        format!(
            "{}/.well-known/jwks.json",
            provider.issuer.trim_end_matches('/')
        )
    });
    // The keys decide which tokens are genuine; fetch them as securely as
    // the issuer itself is reached.
    if provider.issuer.starts_with("https://") && !jwks_url.starts_with("https://") {
        errs.push(
            "PARCEL_MCP_JWKS_URL",
            "must be an https URL, like the issuer",
        );
        return None;
    }
    Some(McpConfig {
        provider: provider.id.clone(),
        issuer: provider.issuer.clone(),
        jwks_url,
    })
}

/// `AUTH_PROVIDERS` and each provider's `AUTH_<ID>_*`. Unset means one
/// provider, `slauth`, configured from the older `SLAUTH_*` variables.
fn parse_providers<F: Fn(&str) -> Option<String>>(
    env: &Lookup<F>,
    app_env: AppEnv,
    auth_mode: AuthMode,
    errs: &mut Errors,
) -> Vec<ProviderConfig> {
    let ids: Vec<String> = match env.get("AUTH_PROVIDERS") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        None => vec!["slauth".into()],
    };
    let mut providers: Vec<ProviderConfig> = Vec::new();
    for id in ids {
        let valid_id = id.len() <= 32
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !valid_id || id == "dev" {
            errs.push(
                "AUTH_PROVIDERS",
                format!("{id:?} is not a provider id: use [a-z][a-z0-9]{{0,31}}, not \"dev\""),
            );
            continue;
        }
        if providers.iter().any(|p| p.id == id) {
            errs.push("AUTH_PROVIDERS", format!("{id:?} is listed twice"));
            continue;
        }
        let var = |suffix: &str| format!("AUTH_{}_{suffix}", id.to_ascii_uppercase());
        let legacy = id == "slauth";
        let get = |suffix: &str, legacy_key: &str| {
            env.get(&var(suffix))
                .or_else(|| legacy.then(|| env.get(legacy_key)).flatten())
        };
        let issuer = get("ISSUER", "SLAUTH_ISSUER")
            .or_else(|| legacy.then(|| "https://auth.local.stridelabs.ai".to_string()));
        let client_id =
            get("CLIENT_ID", "SLAUTH_CLIENT_ID").or_else(|| legacy.then(|| "parcel".to_string()));
        let (Some(issuer), Some(client_id)) = (issuer, client_id) else {
            if auth_mode == AuthMode::Oidc {
                errs.push(
                    var("ISSUER"),
                    format!("{} and {} are required", var("ISSUER"), var("CLIENT_ID")),
                );
            }
            continue;
        };
        let issuer = issuer.trim().to_string();
        match Url::parse(&issuer) {
            Ok(url)
                if url.scheme() == "https"
                    || (url.scheme() == "http" && app_env == AppEnv::Development) => {}
            _ => errs.push(
                var("ISSUER"),
                "must be an https URL (http is allowed only in development)",
            ),
        }
        let mut allowed_domains = Vec::new();
        for domain in env
            .get(&var("ALLOWED_DOMAINS"))
            .unwrap_or_default()
            .split(',')
        {
            let domain = domain.trim().trim_start_matches('@').to_ascii_lowercase();
            if domain.is_empty() {
                continue;
            }
            if !domain.contains('.')
                || domain
                    .chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            {
                errs.push(
                    var("ALLOWED_DOMAINS"),
                    format!("{domain:?} is not a domain"),
                );
            }
            allowed_domains.push(domain);
        }
        let trust_email = env.parse(&var("TRUST_EMAIL"), legacy, errs);
        let provider_logout = env.parse(&var("PROVIDER_LOGOUT"), legacy, errs);
        providers.push(ProviderConfig {
            label: env.get(&var("LABEL")).unwrap_or_else(|| id.clone()),
            issuer,
            client_id: client_id.trim().to_string(),
            client_secret: env
                .get(&var("CLIENT_SECRET"))
                .map(|s| Secret(s.trim().to_string())),
            allowed_domains,
            trust_email,
            scopes: env
                .get(&var("SCOPES"))
                .unwrap_or_else(|| DEFAULT_SCOPES.to_string()),
            provider_logout,
            id,
        });
    }
    if auth_mode == AuthMode::Oidc && providers.is_empty() {
        errs.push(
            "AUTH_PROVIDERS",
            "AUTH_MODE=oidc needs at least one provider",
        );
    }
    providers
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
    let positive: [(&str, i64); 8] = [
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
        ("PARCEL_MAX_DATA_BYTES", l.max_data_bytes),
        ("PARCEL_MAX_DATA_VALUE_BYTES", l.max_data_value_bytes),
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
        assert_eq!(c.auth_mode, AuthMode::Oidc);
        assert_eq!(c.providers.len(), 1);
        assert!(c.allow_public);
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
    fn log_format_accepts_cloud() {
        for (value, format) in [
            ("text", LogFormat::Text),
            ("json", LogFormat::Json),
            ("cloud", LogFormat::Cloud),
            ("Cloud", LogFormat::Cloud),
        ] {
            let c = load(&[("LOG_FORMAT", value)]).unwrap();
            assert_eq!(c.log_format, format, "{value}");
        }
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

    #[test]
    fn legacy_slauth_vars_configure_one_provider() {
        let c = load(&[
            ("AUTH_MODE", "slauth"),
            ("SLAUTH_ISSUER", "https://auth.example"),
            ("SLAUTH_CLIENT_ID", "parcel-x"),
        ])
        .unwrap();
        assert_eq!(c.auth_mode, AuthMode::Oidc);
        let [p] = c.providers.as_slice() else {
            panic!("one provider")
        };
        assert_eq!(
            (p.id.as_str(), p.issuer.as_str(), p.client_id.as_str()),
            ("slauth", "https://auth.example", "parcel-x")
        );
        assert!(p.trust_email, "slauth sends no email_verified");
        assert!(p.client_secret.is_none());
    }

    #[test]
    fn several_providers_load_in_order() {
        let c = load(&[
            ("AUTH_PROVIDERS", "okta, keycloak"),
            ("AUTH_OKTA_ISSUER", "https://acme.okta.com"),
            ("AUTH_OKTA_CLIENT_ID", "0oa1"),
            ("AUTH_OKTA_CLIENT_SECRET", "s3cret"),
            ("AUTH_OKTA_LABEL", "Acme (US)"),
            ("AUTH_OKTA_ALLOWED_DOMAINS", "Acme.com, @us.acme.com"),
            ("AUTH_KEYCLOAK_ISSUER", "https://kc.acme.eu/realms/acme"),
            ("AUTH_KEYCLOAK_CLIENT_ID", "parcel"),
            ("PARCEL_ALLOW_PUBLIC", "false"),
            ("PARCEL_TOKEN_LOGIN_MAX_AGE_DAYS", "0"),
        ])
        .unwrap();
        let ids: Vec<&str> = c.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["okta", "keycloak"]);
        let okta = c.provider("okta").unwrap();
        assert_eq!(okta.label, "Acme (US)");
        assert_eq!(okta.allowed_domains, ["acme.com", "us.acme.com"]);
        assert!(!okta.trust_email);
        assert!(okta.allows_domain("us.acme.com") && !okta.allows_domain("gmail.com"));
        assert!(
            !format!("{okta:?}").contains("s3cret"),
            "secrets stay out of Debug"
        );
        assert_eq!(c.provider("keycloak").unwrap().label, "keycloak");
        assert!(
            c.domain_can_sign_in("anything.eu"),
            "keycloak takes any domain"
        );
        assert!(!c.allow_public);
        assert_eq!(c.token_login_max_age, None);
    }

    #[test]
    fn bad_providers_are_reported() {
        let err = load(&[
            ("AUTH_PROVIDERS", "okta,Bad-Id,okta,dev"),
            ("AUTH_OKTA_ISSUER", "ftp://acme"),
            ("AUTH_OKTA_CLIENT_ID", "x"),
            ("AUTH_OKTA_ALLOWED_DOMAINS", "nodot"),
        ])
        .unwrap_err()
        .to_string();
        for needle in [
            "\"bad-id\" is not a provider id",
            "\"okta\" is listed twice",
            "\"dev\" is not a provider id",
            "AUTH_OKTA_ISSUER",
            "\"nodot\" is not a domain",
        ] {
            assert!(err.contains(needle), "{needle} missing from: {err}");
        }
        let err = load(&[("AUTH_PROVIDERS", "okta")]).unwrap_err().to_string();
        assert!(err.contains("AUTH_OKTA_CLIENT_ID are required"), "{err}");
        let err = load(&[
            ("APP_ENV", "production"),
            ("PARCEL_CONTENT_SIGNING_KEY", &"k".repeat(40)),
            ("AUTH_PROVIDERS", "kc"),
            ("AUTH_KC_ISSUER", "http://kc.internal/realms/a"),
            ("AUTH_KC_CLIENT_ID", "parcel"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("must be an https URL"), "{err}");
    }

    #[test]
    fn mcp_is_off_unless_enabled_and_names_a_provider() {
        assert_eq!(load(&[]).unwrap().mcp, None);

        let c = load(&[("PARCEL_MCP_ENABLED", "true")]).unwrap();
        let mcp = c.mcp.as_ref().expect("enabled");
        assert_eq!(mcp.provider, "slauth");
        assert_eq!(mcp.issuer, "https://auth.local.stridelabs.ai");
        assert_eq!(
            mcp.jwks_url,
            "https://auth.local.stridelabs.ai/.well-known/jwks.json"
        );
        assert_eq!(
            c.mcp_resource_url(),
            "https://parcel.local.stridelabs.ai/mcp"
        );

        let c = load(&[
            ("PARCEL_MCP_ENABLED", "true"),
            ("PARCEL_MCP_JWKS_URL", "https://keys.example/jwks"),
        ])
        .unwrap();
        assert_eq!(c.mcp.unwrap().jwks_url, "https://keys.example/jwks");
        let err = load(&[
            ("PARCEL_MCP_ENABLED", "true"),
            ("PARCEL_MCP_JWKS_URL", "http://keys.example/jwks"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("must be an https URL"), "{err}");

        let err = load(&[
            ("PARCEL_MCP_ENABLED", "true"),
            ("PARCEL_MCP_PROVIDER", "okta"),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("not one of the configured"), "{err}");
    }

    #[test]
    fn mcp_needs_oidc_and_a_canonical_https_origin() {
        let err = load(&[("PARCEL_MCP_ENABLED", "true"), ("AUTH_MODE", "dev")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("AUTH_MODE=oidc"), "{err}");
        for origin in ["http://localhost:18110", "https://parcel.example:8443"] {
            let err = load(&[
                ("PARCEL_MCP_ENABLED", "true"),
                ("PARCEL_APP_ORIGIN", origin),
            ])
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("https PARCEL_APP_ORIGIN without a port"),
                "{err}"
            );
        }
    }

    #[test]
    fn cookie_keys_differ_by_purpose() {
        let c = Config::default();
        assert_ne!(c.cookie_key("session"), c.cookie_key("login"));
        assert_ne!(c.cookie_key("session"), c.content_signing_key);
    }
}
