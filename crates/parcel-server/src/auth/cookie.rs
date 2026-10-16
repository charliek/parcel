//! Signed cookies: the web session, and the short-lived sign-in transaction.
//!
//! Both are `base64url(payload) "." base64url(HMAC-SHA256(key, payload))`
//! with a per-purpose key ([`Config::cookie_key`]), so the server keeps no
//! session rows: a session is valid while its signature, expiry and the
//! user's `session_epoch` say so.

use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

const SESSION_PURPOSE: &str = "session";
const LOGIN_PURPOSE: &str = "login";

/// How long a sign-in may take at the provider.
pub const LOGIN_TTL_SECS: i64 = 600;

/// Browsers drop cookies over ~4 KB; an ID token hint bigger than this is
/// simply not kept (logout then goes to the provider without one).
const MAX_HINT_BYTES: usize = 3500;

/// A signed-in browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(rename = "u")]
    pub user_id: Uuid,
    /// `users.session_epoch` when issued.
    #[serde(rename = "e")]
    pub epoch: i32,
    #[serde(rename = "i")]
    pub issued_at: i64,
    #[serde(rename = "x")]
    pub expires_at: i64,
    /// Provider id.
    #[serde(rename = "p")]
    pub provider: String,
}

/// A sign-in in progress, between `/auth/login` and `/auth/callback`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginTx {
    #[serde(rename = "p")]
    pub provider: String,
    #[serde(rename = "s")]
    pub state: String,
    #[serde(rename = "n")]
    pub nonce: String,
    /// PKCE code verifier.
    #[serde(rename = "v")]
    pub verifier: String,
    /// Same-site path to land on afterwards.
    #[serde(rename = "r")]
    pub return_to: String,
    #[serde(rename = "x")]
    pub expires_at: i64,
}

fn mac(key: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length")
}

fn seal<T: Serialize>(key: &[u8], value: &T) -> String {
    let payload = serde_json::to_vec(value).expect("cookie payloads serialize");
    let mut m = mac(key);
    m.update(&payload);
    format!(
        "{}.{}",
        B64.encode(&payload),
        B64.encode(m.finalize().into_bytes())
    )
}

/// Signature first (constant time), then parse.
fn open<T: for<'de> Deserialize<'de>>(key: &[u8], raw: &str) -> Option<T> {
    let (payload, sig) = raw.split_once('.')?;
    let payload = B64.decode(payload).ok()?;
    let sig = B64.decode(sig).ok()?;
    let mut m = mac(key);
    m.update(&payload);
    m.verify_slice(&sig).ok()?;
    serde_json::from_slice(&payload).ok()
}

pub fn seal_session(config: &Config, session: &Session) -> String {
    seal(&config.cookie_key(SESSION_PURPOSE), session)
}

/// A session whose signature checks out and which hasn't expired. Whether
/// its user still exists and its epoch is current is the caller's check.
pub fn open_session(config: &Config, raw: &str, now: i64) -> Option<Session> {
    open::<Session>(&config.cookie_key(SESSION_PURPOSE), raw).filter(|s| s.expires_at > now)
}

pub fn seal_login(config: &Config, tx: &LoginTx) -> String {
    seal(&config.cookie_key(LOGIN_PURPOSE), tx)
}

pub fn open_login(config: &Config, raw: &str, now: i64) -> Option<LoginTx> {
    open::<LoginTx>(&config.cookie_key(LOGIN_PURPOSE), raw).filter(|t| t.expires_at > now)
}

/// Cookie names. `__Host-` pins a cookie to this exact origin (no `Domain`,
/// `Path=/`, `Secure`), so a sibling subdomain can't plant one — which for
/// the login cookie would let it complete its own sign-in in a victim's
/// browser. Plain-http dev origins can't use prefixes.
pub struct Names {
    pub session: &'static str,
    pub login: &'static str,
    pub hint: &'static str,
}

pub fn names(config: &Config) -> Names {
    if config.secure_cookies() {
        Names {
            session: "__Host-parcel_session",
            login: "__Host-parcel_login",
            hint: "__Secure-parcel_hint",
        }
    } else {
        Names {
            session: "parcel_session",
            login: "parcel_login",
            hint: "parcel_hint",
        }
    }
}

/// The login cookie must be `Path=/` to be `__Host-`; it lives ten minutes.
/// The ID-token hint is read only by `/auth/logout` (and a planted one can
/// at worst send a wrong hint to the provider), so it stays out of every
/// other request.
const LOGIN_PATH: &str = "/";
const HINT_PATH: &str = "/auth/logout";

fn cookie(config: &Config, name: &str, value: &str, path: &str, max_age: i64) -> HeaderValue {
    let secure = if config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"
    ))
    .expect("cookie values are base64url, dots and ASCII names")
}

pub fn set_session(config: &Config, value: &str) -> HeaderValue {
    let max_age = i64::try_from(config.session_ttl.as_secs()).unwrap_or(i64::MAX);
    cookie(config, names(config).session, value, "/", max_age)
}

pub fn clear_session(config: &Config) -> HeaderValue {
    cookie(config, names(config).session, "", "/", 0)
}

pub fn set_login(config: &Config, value: &str) -> HeaderValue {
    cookie(
        config,
        names(config).login,
        value,
        LOGIN_PATH,
        LOGIN_TTL_SECS,
    )
}

pub fn clear_login(config: &Config) -> HeaderValue {
    cookie(config, names(config).login, "", LOGIN_PATH, 0)
}

/// The ID token, kept only to hand back to the provider as
/// `id_token_hint` on logout. `None` if it can't be a cookie.
pub fn set_hint(config: &Config, id_token: &str) -> Option<HeaderValue> {
    let fits = id_token.len() <= MAX_HINT_BYTES
        && id_token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let max_age = i64::try_from(config.session_ttl.as_secs()).unwrap_or(i64::MAX);
    fits.then(|| cookie(config, names(config).hint, id_token, HINT_PATH, max_age))
}

pub fn clear_hint(config: &Config) -> HeaderValue {
    cookie(config, names(config).hint, "", HINT_PATH, 0)
}

/// The value of cookie `name`, if the request sent it.
pub fn read<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

/// Append `Set-Cookie` headers.
pub fn append(headers: &mut HeaderMap, cookies: impl IntoIterator<Item = HeaderValue>) {
    for c in cookies {
        headers.append(SET_COOKIE, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            user_id: Uuid::nil(),
            epoch: 2,
            issued_at: 100,
            expires_at: 1_000,
            provider: "okta".into(),
        }
    }

    #[test]
    fn sessions_round_trip_and_expire() {
        let config = Config::default();
        let raw = seal_session(&config, &session());
        assert_eq!(open_session(&config, &raw, 999), Some(session()));
        assert_eq!(open_session(&config, &raw, 1_000), None);
    }

    #[test]
    fn tampering_and_cross_purpose_use_fail() {
        let config = Config::default();
        let raw = seal_session(&config, &session());
        let (payload, sig) = raw.split_once('.').unwrap();
        let mut forged = session();
        forged.user_id = Uuid::max();
        let forged_payload = B64.encode(serde_json::to_vec(&forged).unwrap());
        assert_eq!(
            open_session(&config, &format!("{forged_payload}.{sig}"), 0),
            None
        );
        assert_eq!(open_session(&config, payload, 0), None);
        // A login cookie never verifies as a session, even with the same shape.
        let login = seal(&config.cookie_key(LOGIN_PURPOSE), &session());
        assert_eq!(open_session(&config, &login, 0), None);
        let other = Config {
            content_signing_key: b"another-key-another-key-another-key".to_vec(),
            ..Config::default()
        };
        assert_eq!(open_session(&other, &raw, 0), None);
    }

    #[test]
    fn cookies_are_read_from_any_header() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("a=1; parcel_session=xyz"));
        headers.append(COOKIE, HeaderValue::from_static("b=2"));
        assert_eq!(read(&headers, "parcel_session"), Some("xyz"));
        assert_eq!(read(&headers, "b"), Some("2"));
        assert_eq!(read(&headers, "parcel"), None);
    }

    #[test]
    fn cookie_attributes() {
        let config = Config::default();
        let set = set_session(&config, "v");
        let set = set.to_str().unwrap();
        assert!(set.starts_with("__Host-parcel_session=v; Path=/;"), "{set}");
        assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Secure"));
        let login = set_login(&config, "v");
        assert!(login
            .to_str()
            .unwrap()
            .starts_with("__Host-parcel_login=v; Path=/;"));
        assert!(set_hint(&config, "a.b.c").is_some());
        assert!(set_hint(&config, "a;b").is_none());
        assert!(set_hint(&config, &"a".repeat(MAX_HINT_BYTES + 1)).is_none());

        let plain = Config {
            app_origin: url::Url::parse("http://localhost:18110").unwrap(),
            ..Config::default()
        };
        let set = set_session(&plain, "v");
        assert!(!set.to_str().unwrap().contains("Secure"));
        assert!(set.to_str().unwrap().starts_with("parcel_session="));
    }
}
