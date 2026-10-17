//! Sharing a parcel with named people, by email.
//!
//! A share is `(parcel, email, role)`. It is matched against the signed-in
//! user's email when they ask for the parcel, so sharing with someone who
//! has never signed in works from their first sign-in.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::ParcelSummary;

/// Longest email accepted (RFC 5321's path limit).
pub const MAX_EMAIL_CHARS: usize = 254;

/// What a person a parcel is shared with may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ShareRole {
    /// See the parcel, its comments and its data.
    #[default]
    Viewer,
    /// Also comment.
    Commenter,
}

impl ShareRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ShareRole::Viewer => "viewer",
            ShareRole::Commenter => "commenter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(ShareRole::Viewer),
            "commenter" => Some(ShareRole::Commenter),
            _ => None,
        }
    }
}

/// Normalize an email for sharing and matching: trimmed and lowercased.
/// Checks only the shape (`local@domain.tld`, no spaces or control
/// characters); whether the address exists is the provider's business.
pub fn normalize_email(raw: &str) -> Result<String, String> {
    let email = raw.trim().to_lowercase();
    let bad = || format!("{raw:?} is not an email address");
    if email.is_empty() || email.chars().count() > MAX_EMAIL_CHARS {
        return Err(bad());
    }
    if email
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '/'))
    {
        return Err(bad());
    }
    let Some((local, domain)) = email.rsplit_once('@') else {
        return Err(bad());
    };
    if local.is_empty()
        || local.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return Err(bad());
    }
    Ok(email)
}

/// The domain part of a normalized email.
pub fn email_domain(email: &str) -> &str {
    email.rsplit_once('@').map_or("", |(_, d)| d)
}

/// One person a parcel is shared with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Share {
    /// Lowercased.
    pub email: String,
    pub role: ShareRole,
    /// Their display name, once someone with this email has signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// `GET /api/v1/parcels/{id}/shares`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ShareList {
    /// By email.
    pub shares: Vec<Share>,
}

/// `PUT /api/v1/parcels/{id}/shares/{email}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PutShareRequest {
    #[serde(default)]
    pub role: ShareRole,
}

/// A parcel someone shared with the caller by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SharedParcel {
    pub parcel: ParcelSummary,
    pub owner_name: String,
    pub role: ShareRole,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub shared_at: OffsetDateTime,
}

/// `GET /api/v1/shared`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SharedParcelList {
    /// Most recently updated first.
    pub parcels: Vec<SharedParcel>,
    pub total: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_normalize() {
        assert_eq!(
            normalize_email("  Ada@Example.COM ").unwrap(),
            "ada@example.com"
        );
        assert_eq!(normalize_email("a.b+c@x.co.uk").unwrap(), "a.b+c@x.co.uk");
        for bad in [
            "",
            "ada",
            "@x.com",
            "ada@",
            "ada@localhost",
            "a b@x.com",
            "ada@x..com",
            "ada@.x.com",
            "a@b@x.com",
            "ada@x\u{7}.com",
            "<ada@x.com>",
        ] {
            assert!(normalize_email(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(normalize_email(&format!("{}@x.com", "a".repeat(300))).is_err());
        assert_eq!(email_domain("ada@x.com"), "x.com");
    }

    #[test]
    fn roles_order() {
        assert!(ShareRole::Commenter > ShareRole::Viewer);
        assert_eq!(ShareRole::parse("commenter"), Some(ShareRole::Commenter));
        assert_eq!(ShareRole::parse("editor"), None);
    }
}
