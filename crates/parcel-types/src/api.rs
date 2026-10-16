//! JSON request and response bodies for `/api/v1`.
//!
//! Timestamps are RFC 3339 strings on the wire. Sizes are bytes (`i64`) and
//! version numbers are `i32`, matching the Postgres columns they come from.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Who can open a parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Only the owner (the default).
    #[default]
    Private,
    /// Anyone with the link, signed in or not.
    Public,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Public => "public",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "private" => Some(Visibility::Private),
            "public" => Some(Visibility::Public),
            _ => None,
        }
    }
}

/// How a new version's file set is derived from the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum PublishMode {
    /// Start from the previous version: files sent are added or replaced,
    /// files left out are kept, and `remove` deletes paths.
    #[default]
    Merge,
    /// The new version is exactly the files sent.
    Replace,
}

/// A parcel as it appears in lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub visibility: Visibility,
    pub current_version: i32,
    /// Path of the current version's entry page.
    pub entry: String,
    pub file_count: i32,
    pub total_bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    /// The viewer URL in the web app (`https://…/p/<id>`).
    pub url: String,
}

/// One immutable version of a parcel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VersionSummary {
    pub version: i32,
    pub entry: String,
    /// Optional short note recorded with the publish ("Draft to legal").
    pub label: Option<String>,
    pub file_count: i32,
    pub total_bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
}

/// One file inside a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: i64,
    pub content_type: String,
    /// Lowercase hex SHA-256 of the file's bytes.
    pub sha256: String,
}

/// A version with its file list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VersionDetail {
    #[serde(flatten)]
    pub summary: VersionSummary,
    pub files: Vec<FileEntry>,
}

/// A parcel with its history and current files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelDetail {
    pub parcel: ParcelSummary,
    /// Whether the caller owns this parcel (and so may change it).
    pub is_owner: bool,
    /// Newest first.
    pub versions: Vec<VersionSummary>,
    /// Files of the current version.
    pub files: Vec<FileEntry>,
}

/// `GET /api/v1/parcels/{id}/versions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VersionList {
    /// Newest first.
    pub versions: Vec<VersionSummary>,
}

/// `GET /api/v1/parcels`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelList {
    pub parcels: Vec<ParcelSummary>,
    /// Total number of matching parcels (for pagination).
    pub total: i64,
}

/// The `metadata` part of a create (`POST /api/v1/parcels`) multipart upload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct CreateParcelMetadata {
    /// Defaults to the entry page's `<title>` / first heading, else its file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// Entry page path. Defaults to the only HTML/Markdown file, else `index.html`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The `metadata` part of a new-version (`POST /api/v1/parcels/{id}/versions`) upload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct NewVersionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<PublishMode>,
    /// Paths to drop from the previous version (merge mode only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// Optimistic concurrency: fail with 409 unless this is still the current version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<i32>,
}

/// Returned by both create and new-version publishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PublishResult {
    pub parcel: ParcelSummary,
    pub version: VersionDetail,
}

/// `PATCH /api/v1/parcels/{id}` — metadata only; files change through versions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct UpdateParcelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

/// `GET /api/v1/parcels/{id}/view` — what the viewer iframe loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ViewGrant {
    pub parcel: ParcelSummary,
    pub version: VersionSummary,
    pub is_owner: bool,
    /// Signed, expiring URL on the sandboxed content origin.
    pub content_url: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub expires_at: OffsetDateTime,
}

/// Storage and size limits enforced on publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Limits {
    pub max_entry_bytes: i64,
    pub max_file_bytes: i64,
    pub max_upload_bytes: i64,
    pub max_files_per_upload: i32,
    pub max_files_per_version: i32,
    pub max_version_bytes: i64,
    /// Total bytes across all of a user's versions; 0 means unlimited.
    pub user_quota_bytes: i64,
}

/// What a user is currently storing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Usage {
    pub parcel_count: i64,
    pub version_count: i64,
    /// Sum of every version's `total_bytes` (what the quota counts).
    pub stored_bytes: i64,
}

/// `GET /api/v1/me`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Me {
    pub id: String,
    pub email: String,
    pub name: String,
    pub usage: Usage,
    pub limits: Limits,
}

/// slauth settings the SPA needs to start an OAuth flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthInfo {
    /// `slauth` or `dev`.
    pub mode: String,
    pub issuer: String,
    pub client_id: String,
    pub audience: String,
}

/// `GET /api/v1/info` — unauthenticated server description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub app_origin: String,
    pub content_origin: String,
    pub auth: AuthInfo,
    pub limits: Limits,
    pub allowed_extensions: Vec<String>,
}

/// An API token as listed (never includes the secret).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    /// Non-secret display prefix, e.g. `pcl_a1b2c3d4`.
    pub prefix: String,
    /// `web` (minted on the Tokens page) or `device` (CLI login).
    pub created_via: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub expires_at: Option<OffsetDateTime>,
}

/// `POST /api/v1/tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct CreateTokenRequest {
    pub name: String,
    /// Days until expiry; omit for a token that never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<u32>,
}

/// A freshly minted token. `secret` is shown exactly once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreatedToken {
    pub token: ApiToken,
    pub secret: String,
}

/// `GET /api/v1/tokens`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenList {
    pub tokens: Vec<ApiToken>,
}

/// `POST /api/v1/cli/device` — start a CLI device login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct DeviceStartRequest {
    /// Shown on the approval page, e.g. `parcel CLI on laptop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceStartResponse {
    /// Secret the CLI polls with. Never shown to the user.
    pub device_code: String,
    /// Short code the user confirms in the browser, e.g. `BCDF-GHJK`.
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    /// Seconds until the codes expire.
    pub expires_in: i64,
    /// Minimum seconds between polls.
    pub interval: i64,
}

/// `POST /api/v1/cli/device/token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct DevicePollRequest {
    pub device_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    /// Waiting for the user to approve.
    Pending,
    /// Polled faster than `interval`; back off.
    SlowDown,
    /// Approved — the response carries the token (exactly once).
    Approved,
    Denied,
    /// Expired, already redeemed, or unknown.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DevicePollResponse {
    pub status: DeviceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// `GET /api/v1/cli/device/{user_code}` — what the approval page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceInfo {
    pub user_code: String,
    pub client_name: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub expires_at: OffsetDateTime,
}

/// The error envelope every non-2xx JSON response uses
/// (`stridelabs_http::AppError`'s wire contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ErrorDetail {
    /// Human-readable, safe to show.
    pub message: String,
    /// The HTTP status reason phrase, e.g. `Bad Request`.
    #[serde(rename = "type")]
    pub kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_detail_flattens() {
        let v = VersionDetail {
            summary: VersionSummary {
                version: 2,
                entry: "index.html".into(),
                label: None,
                file_count: 1,
                total_bytes: 10,
                created_at: OffsetDateTime::UNIX_EPOCH,
            },
            files: vec![],
        };
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["version"], 2);
        assert_eq!(json["created_at"], "1970-01-01T00:00:00Z");
        let back: VersionDetail = serde_json::from_value(json).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn metadata_rejects_unknown_fields() {
        let err = serde_json::from_str::<CreateParcelMetadata>(r#"{"titel":"x"}"#);
        assert!(err.is_err());
        let ok: CreateParcelMetadata = serde_json::from_str(r#"{"visibility":"public"}"#).unwrap();
        assert_eq!(ok.visibility, Some(Visibility::Public));
    }

    #[test]
    fn error_body_shape() {
        let body: ErrorBody =
            serde_json::from_str(r#"{"error":{"message":"nope","type":"Not Found"}}"#).unwrap();
        assert_eq!(body.error.kind, "Not Found");
    }
}
