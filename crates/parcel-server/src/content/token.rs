//! Content capability tokens.
//!
//! A token authorizes reading one version of one parcel on the content
//! origin until it expires. It is the path prefix of every content URL
//! (`/v/{token}/…`), so relative URLs inside a multi-file parcel inherit it
//! and no cookie is ever needed on the content origin.
//!
//! Format: `base64url(payload) "." base64url(HMAC-SHA256(key, payload))`,
//! where payload is `{parcel_id}.{version}.{expires_unix}.{scope}` and, for
//! a member grant, `.{user_id}` after it.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Why a viewer was granted access. Checked again on every content request,
/// so making a public parcel private, unsharing someone, or turning public
/// links off revokes outstanding grants immediately instead of at expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Owner,
    /// Anyone, because the parcel is public.
    Public,
    /// This signed-in user, because the parcel is internal or shared with them.
    Member(Uuid),
}

impl Scope {
    fn code(self) -> String {
        match self {
            Scope::Owner => "o".into(),
            Scope::Public => "p".into(),
            Scope::Member(user) => format!("m.{}", user.simple()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentClaims {
    pub parcel_id: String,
    pub version: i32,
    pub expires_at: i64,
    pub scope: Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("malformed content token")]
    Malformed,
    #[error("content token signature mismatch")]
    BadSignature,
    #[error("content token expired")]
    Expired,
}

fn mac(key: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length")
}

pub fn sign(key: &[u8], claims: &ContentClaims) -> String {
    let payload = format!(
        "{}.{}.{}.{}",
        claims.parcel_id,
        claims.version,
        claims.expires_at,
        claims.scope.code()
    );
    let mut m = mac(key);
    m.update(payload.as_bytes());
    let sig = m.finalize().into_bytes();
    format!("{}.{}", B64.encode(payload), B64.encode(sig))
}

/// Verify signature first (constant time), then parse, then check expiry.
pub fn verify(key: &[u8], token: &str, now_unix: i64) -> Result<ContentClaims, TokenError> {
    let (payload_b64, sig_b64) = token.split_once('.').ok_or(TokenError::Malformed)?;
    let payload = B64.decode(payload_b64).map_err(|_| TokenError::Malformed)?;
    let sig = B64.decode(sig_b64).map_err(|_| TokenError::Malformed)?;
    let mut m = mac(key);
    m.update(&payload);
    m.verify_slice(&sig).map_err(|_| TokenError::BadSignature)?;

    let payload = String::from_utf8(payload).map_err(|_| TokenError::Malformed)?;
    let parts: Vec<&str> = payload.split('.').collect();
    let (id, version, exp, scope) = match parts.as_slice() {
        [id, version, exp, "o"] => (id, version, exp, Scope::Owner),
        [id, version, exp, "p"] => (id, version, exp, Scope::Public),
        [id, version, exp, "m", user] => (
            id,
            version,
            exp,
            Scope::Member(Uuid::try_parse(user).map_err(|_| TokenError::Malformed)?),
        ),
        _ => return Err(TokenError::Malformed),
    };
    let claims = ContentClaims {
        parcel_id: id.to_string(),
        version: version.parse().map_err(|_| TokenError::Malformed)?,
        expires_at: exp.parse().map_err(|_| TokenError::Malformed)?,
        scope,
    };
    if claims.expires_at <= now_unix {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> ContentClaims {
        ContentClaims {
            parcel_id: "abcdefghijkm".into(),
            version: 3,
            expires_at: 2_000,
            scope: Scope::Public,
        }
    }

    #[test]
    fn round_trips() {
        let token = sign(b"key", &claims());
        assert!(!token.contains('/'));
        assert_eq!(verify(b"key", &token, 1_000), Ok(claims()));
    }

    #[test]
    fn rejects_tampering_and_expiry() {
        let token = sign(b"key", &claims());
        assert_eq!(
            verify(b"other", &token, 1_000),
            Err(TokenError::BadSignature)
        );
        assert_eq!(verify(b"key", &token, 2_000), Err(TokenError::Expired));
        let forged = format!(
            "{}.{}",
            B64.encode("abcdefghijkm.4.2000.p"),
            token.split_once('.').unwrap().1
        );
        assert_eq!(
            verify(b"key", &forged, 1_000),
            Err(TokenError::BadSignature)
        );
        assert_eq!(verify(b"key", "garbage", 1_000), Err(TokenError::Malformed));
    }

    #[test]
    fn member_grants_carry_the_user() {
        let user = Uuid::from_u128(0xabc);
        let member = ContentClaims {
            scope: Scope::Member(user),
            ..claims()
        };
        let token = sign(b"key", &member);
        assert_eq!(verify(b"key", &token, 1_000), Ok(member));
        // A member scope without its user, or with junk, is malformed.
        for payload in [
            "abcdefghijkm.3.2000.m",
            "abcdefghijkm.3.2000.m.nope",
            "abcdefghijkm.3.2000.o.x",
        ] {
            let mut m = mac(b"key");
            m.update(payload.as_bytes());
            let forged = format!(
                "{}.{}",
                B64.encode(payload),
                B64.encode(m.finalize().into_bytes())
            );
            assert_eq!(
                verify(b"key", &forged, 1_000),
                Err(TokenError::Malformed),
                "{payload}"
            );
        }
    }
}
