//! Google OAuth access tokens for the GCS JSON API.
//!
//! Two sources, chosen at startup:
//!
//! * **Service-account key** (`GOOGLE_CREDENTIALS`, base64 JSON — the tapper
//!   convention): sign a JWT-bearer assertion and exchange it at the key's
//!   `token_uri`.
//! * **Metadata server**: on Cloud Run the runtime service account's token is
//!   one GET away, so production needs no key file at all.
//!
//! Tokens are cached and refreshed a minute before expiry; a single mutex
//! makes concurrent misses share one refresh.

use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::StorageError;

const SCOPE: &str = "https://www.googleapis.com/auth/devstorage.read_write";
const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
const REFRESH_MARGIN: Duration = Duration::from_secs(60);

struct ServiceAccount {
    client_email: String,
    token_uri: String,
    key: EncodingKey,
}

#[derive(Deserialize)]
struct ServiceAccountJson {
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

enum Source {
    ServiceAccount(ServiceAccount),
    Metadata,
}

pub struct TokenProvider {
    http: reqwest::Client,
    source: Source,
    cached: Mutex<Option<(String, Instant)>>,
}

impl TokenProvider {
    /// From a base64 service-account JSON key.
    pub fn from_service_account(http: reqwest::Client, b64: &str) -> anyhow::Result<Self> {
        let decoded = BASE64
            .decode(b64.trim().as_bytes())
            .map_err(|e| anyhow::anyhow!("GOOGLE_CREDENTIALS is not valid base64: {e}"))?;
        let parsed: ServiceAccountJson = serde_json::from_slice(&decoded)
            .map_err(|e| anyhow::anyhow!("GOOGLE_CREDENTIALS is not service-account JSON: {e}"))?;
        let key = EncodingKey::from_rsa_pem(parsed.private_key.as_bytes()).map_err(|e| {
            anyhow::anyhow!("GOOGLE_CREDENTIALS private_key is not an RSA PEM: {e}")
        })?;
        Ok(Self {
            http,
            source: Source::ServiceAccount(ServiceAccount {
                client_email: parsed.client_email,
                token_uri: parsed.token_uri,
                key,
            }),
            cached: Mutex::new(None),
        })
    }

    /// From the GCE/Cloud Run metadata server.
    pub fn metadata(http: reqwest::Client) -> Self {
        Self {
            http,
            source: Source::Metadata,
            cached: Mutex::new(None),
        }
    }

    pub async fn access_token(&self) -> Result<String, StorageError> {
        let mut cached = self.cached.lock().await;
        if let Some((token, expires)) = cached.as_ref() {
            if Instant::now() + REFRESH_MARGIN < *expires {
                return Ok(token.clone());
            }
        }
        let response = self.fetch().await?;
        let lifetime = Duration::from_secs(response.expires_in.unwrap_or(300));
        *cached = Some((response.access_token.clone(), Instant::now() + lifetime));
        Ok(response.access_token)
    }

    async fn fetch(&self) -> Result<TokenResponse, StorageError> {
        let request = match &self.source {
            Source::Metadata => self
                .http
                .get(METADATA_TOKEN_URL)
                .header("Metadata-Flavor", "Google"),
            Source::ServiceAccount(sa) => {
                let now = time::OffsetDateTime::now_utc().unix_timestamp();
                let claims = AssertionClaims {
                    iss: &sa.client_email,
                    scope: SCOPE,
                    aud: &sa.token_uri,
                    iat: now,
                    exp: now + 3600,
                };
                let assertion =
                    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &sa.key)
                        .map_err(|e| StorageError::Auth(format!("signing assertion: {e}")))?;
                self.http
                    .post(&sa.token_uri)
                    .form(&[("grant_type", JWT_BEARER_GRANT), ("assertion", &assertion)])
            }
        };
        let response = request
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| StorageError::Auth(format!("token request failed: {}", redact(&e))))?;
        if !response.status().is_success() {
            return Err(StorageError::Auth(format!(
                "token endpoint returned HTTP {}",
                response.status().as_u16()
            )));
        }
        response
            .json::<TokenResponse>()
            .await
            .map_err(|e| StorageError::Auth(format!("token response unreadable: {}", redact(&e))))
    }
}

/// `reqwest::Error`'s Display can include the request URL; the kind is enough.
pub(super) fn redact(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_decode() {
        "decode error"
    } else if e.is_body() {
        "body error"
    } else {
        "request error"
    }
}
