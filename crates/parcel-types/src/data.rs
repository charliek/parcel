//! Parcel data: small JSON documents a parcel keeps between visits.
//!
//! Data is grouped into named collections of keyed items and is shared by
//! every version of a parcel. Anyone who can view a parcel can read its
//! data; only its owner can change it (from the CLI, an agent, or the page
//! itself while the owner is viewing it).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// Longest collection name or item key, in bytes.
pub const MAX_DATA_NAME_LEN: usize = 128;

/// Bytes each item counts against the parcel's data limit on top of its
/// value, so a flood of tiny items can't stay under the limit.
pub const DATA_ITEM_OVERHEAD: i64 = 128;

/// Validate a collection name or item key: 1–128 of `A–Z a–z 0–9 _ . : -`,
/// starting with a letter or digit. They appear in URL paths, so nothing
/// needs escaping.
pub fn validate_data_name(kind: &str, name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= MAX_DATA_NAME_LEN
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{kind} must be 1–{MAX_DATA_NAME_LEN} letters, digits, '_', '.', ':' or '-', \
             starting with a letter or digit"
        ))
    }
}

/// One stored item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataItem {
    pub collection: String,
    pub key: String,
    /// Any JSON value.
    pub value: Value,
    /// Size of the value as compact JSON.
    pub size_bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// A page of a collection's items, ordered by key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataItemList {
    pub items: Vec<DataItem>,
    /// Pass as `after` to fetch the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after: Option<String>,
}

/// A collection as listed in a [`DataSummary`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataCollection {
    pub name: String,
    pub count: i64,
    /// What the collection counts against the parcel's data limit.
    pub usage_bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
}

/// `GET /api/v1/parcels/{id}/data`: a parcel's collections and usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataSummary {
    /// Changes whenever any item is written or deleted.
    pub revision: i64,
    /// Each item's JSON size plus a fixed per-item overhead.
    pub usage_bytes: i64,
    pub limit_bytes: i64,
    /// Whether the caller may change this parcel's data.
    pub writable: bool,
    /// By name.
    pub collections: Vec<DataCollection>,
}

#[cfg(test)]
mod tests {
    use super::validate_data_name;

    #[test]
    fn names() {
        for ok in ["votes", "a", "user:42", "v1.2-x_y", &"a".repeat(128)] {
            assert!(validate_data_name("key", ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", ".x", "a/b", "a b", "ä", &"a".repeat(129)] {
            assert!(validate_data_name("key", bad).is_err(), "{bad}");
        }
    }
}
