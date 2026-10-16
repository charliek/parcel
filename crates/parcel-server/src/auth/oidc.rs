//! The server's OIDC client: discovery, the authorization redirect, the
//! code exchange, and ID-token verification.
//!
//! Only the ID token is trusted for identity. It is the part of OIDC every
//! provider issues the same way (signed by the provider's JWKS, `aud` = our
//! client id, our `nonce`), unlike access tokens, whose format and audience
//! vary by provider and plan. Provider tokens are used once and dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use stridelabs_auth::JwksCache;
use tokio::sync::Mutex;
use url::form_urlencoded;
use url::Url;

use crate::config::ProviderConfig;

/// How long a discovery document is reused before it is fetched again.
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// Clock skew tolerated on `exp`/`nbf`/`iat`.
const LEEWAY_SECS: u64 = 60;

/// Why a sign-in failed. The messages are for logs; people see a short
/// code (see `http::auth`).
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("provider discovery failed: {0}")]
    Discovery(String),
    #[error("code exchange failed: {0}")]
    Exchange(String),
    #[error("ID token rejected: {0}")]
    IdToken(String),
    #[error("userinfo failed: {0}")]
    Userinfo(String),
}

/// The parts of a discovery document we use.
#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub userinfo_endpoint: Option<String>,
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

struct Discovered {
    at: Instant,
    metadata: Arc<Metadata>,
    jwks: Arc<JwksCache>,
}

/// One provider at runtime: its config plus cached discovery and keys.
pub struct Provider {
    pub config: ProviderConfig,
    discovered: Mutex<Option<Discovered>>,
}

/// Who the provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    /// Trimmed and lowercased; may be empty when the provider sent none.
    pub email: String,
    pub email_verified: bool,
    pub name: String,
}

/// The result of a successful callback.
pub struct SignedIn {
    pub identity: Identity,
    /// Raw ID token, kept only as the logout hint.
    pub id_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdClaims {
    sub: String,
    #[serde(default)]
    aud: Value,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(flatten)]
    profile: Profile,
}

/// Identity claims, from the ID token or `userinfo`.
#[derive(Debug, Default, Deserialize)]
struct Profile {
    #[serde(default)]
    email: Option<String>,
    /// A bool, or (from some providers) the string `"true"`.
    #[serde(default)]
    email_verified: Option<Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    given_name: Option<String>,
    #[serde(default)]
    family_name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
}

impl Profile {
    fn verified(&self) -> bool {
        match &self.email_verified {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
            _ => false,
        }
    }

    fn display_name(&self) -> String {
        let non_empty = |s: &Option<String>| {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        non_empty(&self.name)
            .or_else(|| {
                let full = [&self.given_name, &self.family_name]
                    .iter()
                    .filter_map(|s| non_empty(s))
                    .collect::<Vec<_>>()
                    .join(" ");
                (!full.is_empty()).then_some(full)
            })
            .or_else(|| non_empty(&self.preferred_username))
            .unwrap_or_default()
    }
}

/// A random URL-safe string with 256 bits of entropy.
pub fn random_token() -> String {
    B64.encode(rand::random::<[u8; 32]>())
}

/// The PKCE S256 challenge for `verifier`.
pub fn challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

impl Provider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            discovered: Mutex::new(None),
        }
    }

    /// The discovery document and key cache, fetched at most hourly.
    async fn discover(
        &self,
        http: &reqwest::Client,
    ) -> Result<(Arc<Metadata>, Arc<JwksCache>), OidcError> {
        let mut slot = self.discovered.lock().await;
        if let Some(d) = slot.as_ref().filter(|d| d.at.elapsed() < DISCOVERY_TTL) {
            return Ok((d.metadata.clone(), d.jwks.clone()));
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );
        let metadata: Metadata = http
            .get(&url)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| OidcError::Discovery(format!("{url}: {}", e.without_url())))?
            .json()
            .await
            .map_err(|e| OidcError::Discovery(format!("{url}: {}", e.without_url())))?;
        // OIDC Discovery §4.3: the document must name exactly the issuer we
        // asked about, or tokens it vouches for could come from elsewhere.
        if metadata.issuer != self.config.issuer {
            return Err(OidcError::Discovery(format!(
                "issuer mismatch: configured {:?}, provider says {:?}",
                self.config.issuer, metadata.issuer
            )));
        }
        check_endpoints(&self.config.issuer, &metadata)?;
        // Keep the key cache across refreshes unless the JWKS moved.
        let jwks = match slot.as_ref() {
            Some(d) if d.jwks.jwks_url() == metadata.jwks_uri => d.jwks.clone(),
            _ => Arc::new(JwksCache::new(metadata.jwks_uri.clone(), http.clone())),
        };
        let metadata = Arc::new(metadata);
        *slot = Some(Discovered {
            at: Instant::now(),
            metadata: metadata.clone(),
            jwks: jwks.clone(),
        });
        Ok((metadata, jwks))
    }

    /// Where to send the browser to sign in.
    pub async fn authorize_url(
        &self,
        http: &reqwest::Client,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        verifier: &str,
        login_hint: Option<&str>,
    ) -> Result<String, OidcError> {
        let (metadata, _) = self.discover(http).await?;
        let mut url = Url::parse(&metadata.authorization_endpoint)
            .map_err(|e| OidcError::Discovery(e.to_string()))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("scope", &self.config.scopes)
                .append_pair("state", state)
                .append_pair("nonce", nonce)
                .append_pair("code_challenge", &challenge(verifier))
                .append_pair("code_challenge_method", "S256");
            if let Some(hint) = login_hint {
                q.append_pair("login_hint", hint);
            }
        }
        Ok(url.into())
    }

    /// Redeem `code` and verify who signed in.
    pub async fn redeem(
        &self,
        http: &reqwest::Client,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<SignedIn, OidcError> {
        let (metadata, jwks) = self.discover(http).await?;
        let tokens = self
            .exchange(http, &metadata, redirect_uri, code, verifier)
            .await?;
        let id_token = tokens
            .id_token
            .ok_or_else(|| OidcError::Exchange("no id_token in the token response".into()))?;
        let claims = verify_id_token(
            &jwks,
            &id_token,
            &self.config.issuer,
            &self.config.client_id,
            nonce,
        )
        .await?;

        // Some providers put identity claims only in `userinfo`.
        let mut profile = claims.profile;
        let missing_email = profile.email.as_deref().is_none_or(|e| e.trim().is_empty());
        if missing_email {
            if let (Some(endpoint), Some(access_token)) =
                (&metadata.userinfo_endpoint, &tokens.access_token)
            {
                profile = userinfo(http, endpoint, access_token, &claims.sub).await?;
            }
        }
        Ok(SignedIn {
            identity: Identity {
                subject: claims.sub,
                email: profile
                    .email
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase(),
                email_verified: profile.verified(),
                name: profile.display_name(),
            },
            id_token,
        })
    }

    async fn exchange(
        &self,
        http: &reqwest::Client,
        metadata: &Metadata,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OidcError> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ];
        let mut request = http.post(&metadata.token_endpoint).timeout(HTTP_TIMEOUT);
        match &self.config.client_secret {
            None => form.push(("client_id", &self.config.client_id)),
            Some(secret) => {
                let methods = metadata.token_endpoint_auth_methods_supported.as_deref();
                let basic = methods.is_none_or(|m| {
                    m.iter().any(|m| m == "client_secret_basic")
                        || !m.iter().any(|m| m == "client_secret_post")
                });
                if basic {
                    // RFC 6749 §2.3.1: form-encode each part before Basic.
                    let enc =
                        |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
                    request = request.basic_auth(enc(&self.config.client_id), Some(enc(&secret.0)));
                } else {
                    form.push(("client_id", &self.config.client_id));
                    form.push(("client_secret", &secret.0));
                }
            }
        }
        let response = request
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::Exchange(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            // The provider's error code is safe to log; the body otherwise
            // might echo what we sent.
            let error = response
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
                .unwrap_or_default();
            return Err(OidcError::Exchange(format!("{status} {error}")));
        }
        response.json().await.map_err(|e| {
            OidcError::Exchange(format!("unreadable token response: {}", e.without_url()))
        })
    }

    /// The provider's logout URL, when it has one.
    pub async fn end_session_url(
        &self,
        http: &reqwest::Client,
        id_token_hint: Option<&str>,
        post_logout_redirect_uri: &str,
    ) -> Option<String> {
        let (metadata, _) = self.discover(http).await.ok()?;
        let mut url = Url::parse(metadata.end_session_endpoint.as_deref()?).ok()?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("client_id", &self.config.client_id)
                .append_pair("post_logout_redirect_uri", post_logout_redirect_uri);
            if let Some(hint) = id_token_hint {
                q.append_pair("id_token_hint", hint);
            }
        }
        Some(url.into())
    }
}

/// Every endpoint must be a URL, and an https issuer may not send us
/// anywhere over plain http: a downgraded code exchange or userinfo call
/// could be rewritten in transit.
fn check_endpoints(issuer: &str, metadata: &Metadata) -> Result<(), OidcError> {
    let https_only = issuer.starts_with("https://");
    let endpoints = [
        (
            "authorization_endpoint",
            Some(&metadata.authorization_endpoint),
        ),
        ("token_endpoint", Some(&metadata.token_endpoint)),
        ("jwks_uri", Some(&metadata.jwks_uri)),
        ("userinfo_endpoint", metadata.userinfo_endpoint.as_ref()),
        (
            "end_session_endpoint",
            metadata.end_session_endpoint.as_ref(),
        ),
    ];
    for (name, endpoint) in endpoints {
        let Some(endpoint) = endpoint else { continue };
        match Url::parse(endpoint) {
            Ok(url) if url.scheme() == "https" || (!https_only && url.scheme() == "http") => {}
            Ok(_) => {
                return Err(OidcError::Discovery(format!(
                    "{name} must use https like the issuer"
                )))
            }
            Err(_) => return Err(OidcError::Discovery(format!("{name} is not a URL"))),
        }
    }
    Ok(())
}

/// Asymmetric algorithms only: an `HS*` token would be "signed" with
/// whatever we decoded the key material as.
const ALLOWED_ALGS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

async fn verify_id_token(
    jwks: &JwksCache,
    token: &str,
    issuer: &str,
    client_id: &str,
    nonce: &str,
) -> Result<IdClaims, OidcError> {
    let reject = |why: &str| OidcError::IdToken(why.to_string());
    let header = jsonwebtoken::decode_header(token).map_err(|_| reject("malformed"))?;
    if !ALLOWED_ALGS.contains(&header.alg) {
        return Err(reject("unsupported signing algorithm"));
    }
    let kid = header.kid.ok_or_else(|| reject("no key id"))?;
    let set = jwks
        .jwks_for_kid(&kid)
        .await
        .map_err(|e| OidcError::IdToken(e.to_string()))?;
    let jwk = set
        .find(&kid)
        .ok_or_else(|| reject("signed by an unknown key"))?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| reject("provider key unusable"))?;

    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = LEEWAY_SECS;
    let claims = jsonwebtoken::decode::<IdClaims>(token, &key, &validation)
        .map_err(|e| OidcError::IdToken(format!("{:?}", e.kind())))?
        .claims;

    // OIDC Core §3.1.3.7: with several audiences, `azp` must be us.
    let several = claims.aud.as_array().is_some_and(|a| a.len() > 1);
    if several && claims.azp.as_deref() != Some(client_id) {
        return Err(reject("azp is not this client"));
    }
    if claims.nonce.as_deref() != Some(nonce) {
        return Err(reject("nonce mismatch"));
    }
    if claims.sub.trim().is_empty() {
        return Err(reject("empty subject"));
    }
    Ok(claims)
}

async fn userinfo(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
    subject: &str,
) -> Result<Profile, OidcError> {
    #[derive(Deserialize)]
    struct Userinfo {
        sub: String,
        #[serde(flatten)]
        profile: Profile,
    }
    let info: Userinfo = http
        .get(endpoint)
        .timeout(HTTP_TIMEOUT)
        .bearer_auth(access_token)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| OidcError::Userinfo(e.without_url().to_string()))?
        .json()
        .await
        .map_err(|e| OidcError::Userinfo(e.without_url().to_string()))?;
    // OIDC Core §5.3.4: a userinfo answer for someone else is discarded.
    if info.sub != subject {
        return Err(OidcError::Userinfo("sub differs from the ID token".into()));
    }
    Ok(info.profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636() {
        // RFC 7636 Appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn https_issuers_only_use_https_endpoints() {
        let metadata = |token: &str, userinfo: Option<&str>| Metadata {
            issuer: "https://idp.example".into(),
            authorization_endpoint: "https://idp.example/auth".into(),
            token_endpoint: token.into(),
            jwks_uri: "https://idp.example/jwks".into(),
            userinfo_endpoint: userinfo.map(String::from),
            end_session_endpoint: None,
            token_endpoint_auth_methods_supported: None,
        };
        let ok = metadata("https://idp.example/token", Some("https://idp.example/me"));
        assert!(check_endpoints("https://idp.example", &ok).is_ok());
        for bad in [
            metadata("http://idp.example/token", None),
            metadata("https://idp.example/token", Some("http://idp.example/me")),
            metadata("ftp://idp.example/token", None),
            metadata("not a url", None),
        ] {
            assert!(
                check_endpoints("https://idp.example", &bad).is_err(),
                "{bad:?}"
            );
        }
        // A local http issuer (development) may use http endpoints.
        let local = metadata("http://localhost:4448/token", None);
        assert!(check_endpoints("http://localhost:4448", &local).is_ok());
    }

    #[test]
    fn profiles_read_loosely() {
        let p: Profile = serde_json::from_value(serde_json::json!({
            "email_verified": "TRUE", "given_name": "Ada", "family_name": "Lovelace"
        }))
        .unwrap();
        assert!(p.verified());
        assert_eq!(p.display_name(), "Ada Lovelace");
        let p: Profile = serde_json::from_value(serde_json::json!({
            "email_verified": false, "name": " ", "preferred_username": "ada"
        }))
        .unwrap();
        assert!(!p.verified());
        assert_eq!(p.display_name(), "ada");
        assert_eq!(Profile::default().display_name(), "");
    }
}
