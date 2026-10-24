//! JSON request and response bodies for `/api/v1`.
//!
//! Timestamps are RFC 3339 strings on the wire. Sizes are bytes (`i64`) and
//! version numbers are `i32`, matching the Postgres columns they come from.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::source::SourceInfo;

/// Who can open a parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// The owner and the people it is shared with (the default).
    #[default]
    Private,
    /// Anyone signed in to this Parcel.
    Internal,
    /// Anyone with the link, signed in or not.
    Public,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Internal => "internal",
            Visibility::Public => "public",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "private" => Some(Visibility::Private),
            "internal" => Some(Visibility::Internal),
            "public" => Some(Visibility::Public),
            _ => None,
        }
    }
}

/// What the caller may do with a parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ParcelAccess {
    /// Signed out, on a public parcel: view only, no comments.
    Public,
    /// Shared as a viewer: view, and read comments.
    Viewer,
    /// View, read and write comments.
    Commenter,
    /// Everything.
    Owner,
}

impl ParcelAccess {
    /// Comments are for signed-in people the parcel is open to.
    pub fn can_read_comments(self) -> bool {
        self >= ParcelAccess::Viewer
    }

    pub fn can_comment(self) -> bool {
        self >= ParcelAccess::Commenter
    }
}

/// How a new version's file set is derived from the previous one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
    /// The viewer URL in the web app (`https://…/p/<id>`). Stays this
    /// shape for good (clients append `/v/<n>` to it); show and share
    /// `canonical_url`.
    pub url: String,
    /// Unresolved comment threads.
    #[serde(default)]
    pub open_comments: i64,
    /// Sorted by bytes. Everyone who can view the parcel sees them.
    #[serde(default)]
    pub tags: Vec<String>,
    /// When the owner archived it. Archived parcels stay viewable by URL but
    /// are left out of lists, tag counts and search unless asked for.
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub archived_at: Option<OffsetDateTime>,
    /// The parcel's name under its owner's handle (`parcel/plan-001`), if
    /// the owner gave it one.
    #[serde(default)]
    pub slug: Option<String>,
    /// The owner's handle. Left out for anonymous callers of a parcel
    /// without a slug (they aren't told who published it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_handle: Option<String>,
    /// The URL to show and share: `https://…/@<handle>/<slug>` when the
    /// parcel has a slug, else the same as `url`.
    #[serde(default)]
    pub canonical_url: String,
}

/// Which parcels a list covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ListScope {
    /// Parcels the caller owns (the default).
    #[default]
    Mine,
    /// Parcels other people shared with the caller by email.
    Shared,
    /// Both.
    All,
}

/// How a list treats archived parcels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ArchivedFilter {
    /// Leave archived parcels out (the default).
    #[default]
    Exclude,
    /// Archived and live parcels alike.
    Include,
    /// Archived parcels only.
    Only,
}

/// One run of a search snippet: text, and whether it is a matched word.
/// Render it as text (never as HTML): it is the parcel's own content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MatchSegment {
    pub text: String,
    pub hit: bool,
}

/// Where a full-text query matched the parcel's page text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ListMatch {
    /// The snippet, in order; concatenated they read as a passage of the page.
    pub segments: Vec<MatchSegment>,
}

/// How a parcel in a list came to be shared with the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SharedInfo {
    pub owner_name: String,
    pub role: crate::sharing::ShareRole,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub shared_at: OffsetDateTime,
}

/// A parcel in `GET /api/v1/parcels`: the summary's fields, plus what the
/// list knows about this caller and query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelListItem {
    #[serde(flatten)]
    pub parcel: ParcelSummary,
    /// Set when `q` matched the parcel's page text.
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub matched: Option<ListMatch>,
    /// Set when the caller isn't the owner (someone shared it with them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<SharedInfo>,
}

impl std::ops::Deref for ParcelListItem {
    type Target = ParcelSummary;

    fn deref(&self) -> &ParcelSummary {
        &self.parcel
    }
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
    /// Where it was published from. The owner's only: absent for everyone else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
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
    /// A page's (`.html`, `.htm`, `.md`, `.markdown`) `<title>` or first
    /// `# heading`, read when it was published. Absent for other files, for
    /// a page without one, and for pages published before titles were kept.
    /// Plain text: show it as text, never as markup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
}

/// One page of a version: a file the viewer can open (HTML or Markdown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PageEntry {
    pub path: String,
    /// The page's title (plain text), or null when it has none on record
    /// (show the path instead).
    pub title: Option<String>,
}

/// A version with its file list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VersionDetail {
    #[serde(flatten)]
    pub summary: VersionSummary,
    pub files: Vec<FileEntry>,
    /// The version's pages: the entry first, then the others by path.
    #[serde(default)]
    pub pages: Vec<PageEntry>,
}

/// A parcel with its history and current files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParcelDetail {
    pub parcel: ParcelSummary,
    /// Whether the caller owns this parcel (and so may change it).
    pub is_owner: bool,
    /// What the caller may do.
    pub access: ParcelAccess,
    /// The owner's display name, for signed-in callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// Newest first.
    pub versions: Vec<VersionSummary>,
    /// Files of the current version.
    pub files: Vec<FileEntry>,
    /// Pages of the current version: the entry first, then the others by path.
    #[serde(default)]
    pub pages: Vec<PageEntry>,
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
    pub parcels: Vec<ParcelListItem>,
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
    /// Tags to start with (normalized; at most 20 with `auto_tags`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Tags a client derives from its environment (the CLI's
    /// `project:<repo>`). Applied only when this call creates the parcel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_tags: Vec<String>,
    /// Publish under this slug: when the caller already has a parcel with
    /// it (or had, and renamed it), this publish is a new version of that
    /// parcel (all files replaced; only the fields sent here change; `tags`
    /// are added; `auto_tags` ignored) and answers 200 with `created:
    /// false`. Otherwise it creates the parcel with this slug (201).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// With `slug` only: fail with 409 unless the parcel with this slug is
    /// still at this version (or, when none has the slug yet, always).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<i32>,
    /// Where this is published from (shown to the owner only).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::source::deserialize_opt"
    )]
    pub source: Option<SourceInfo>,
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
    /// Tags to add to the parcel (existing ones are kept).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_tags: Vec<String>,
    /// Where this is published from (shown to the owner only).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::source::deserialize_opt"
    )]
    pub source: Option<SourceInfo>,
}

/// Returned by both create and new-version publishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PublishResult {
    pub parcel: ParcelSummary,
    pub version: VersionDetail,
    /// Whether this publish created the parcel (a create with a `slug` the
    /// caller already uses adds a version instead).
    #[serde(default)]
    pub created: bool,
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
    /// Replace the tag set (`[]` clears it). Not with `add_tags`/`remove_tags`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Tags to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_tags: Vec<String>,
    /// Tags to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_tags: Vec<String>,
    /// Archive (`true`) or unarchive (`false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// Give the parcel this slug (`""` clears it). The old one keeps
    /// redirecting here until another of your parcels takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// One tag and how many of the listed parcels carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

/// `GET /api/v1/tags`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagList {
    /// Most used first, then by name.
    pub tags: Vec<TagCount>,
    /// How many distinct tags match (for paging).
    #[serde(default)]
    pub total: i64,
}

/// `POST /api/v1/tags/rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct RenameTagRequest {
    pub from: String,
    /// May be a tag already in use: the two merge.
    pub to: String,
}

/// `POST /api/v1/tags/delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct DeleteTagRequest {
    pub tag: String,
}

/// How many of the caller's parcels a tag rename or delete changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagsUpdated {
    pub updated: i64,
}

/// `GET /api/v1/parcels/{id}/view` — what the viewer iframe loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ViewGrant {
    pub parcel: ParcelSummary,
    pub version: VersionSummary,
    pub is_owner: bool,
    /// What the caller may do.
    pub access: ParcelAccess,
    /// The owner's display name, for signed-in callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
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
    /// Data a parcel may keep (each item's JSON plus a per-item overhead).
    #[serde(default)]
    pub max_data_bytes: i64,
    /// Largest single data value, as compact JSON.
    #[serde(default)]
    pub max_data_value_bytes: i64,
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
    /// Label of the provider this web session signed in with (absent for
    /// API tokens and dev mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in_with: Option<String>,
    /// The user's handle: the `@<handle>` in their parcels' clean URLs.
    #[serde(default)]
    pub handle: String,
}

/// `PATCH /api/v1/me`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct UpdateMeRequest {
    /// A new handle. The old one keeps resolving to you (and no one else
    /// can take it), so links with it keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

/// A sign-in provider the web app can offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthProvider {
    /// Pass as `/auth/login?provider=<id>`.
    pub id: String,
    /// Button text, e.g. `Acme Okta`.
    pub label: String,
}

/// How the web app signs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuthInfo {
    /// `oidc` (sign in through a provider at `/auth/login`) or `dev`
    /// (every request is a fixed local user).
    pub mode: String,
    /// Empty in `dev` mode.
    pub providers: Vec<AuthProvider>,
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
    /// Whether parcels may be made public; when false, `internal` is the
    /// widest visibility.
    pub allow_public: bool,
    pub limits: Limits,
    pub allowed_extensions: Vec<String>,
    /// Optional API features this server has (`tags`, `search`, `archive`,
    /// …), so a newer client can refuse a parameter an older server would
    /// silently ignore.
    #[serde(default)]
    pub features: Vec<String>,
}

/// The optional features this version of the API has, as listed in
/// [`ServerInfo::features`]: what the server reports and what a client looks for.
pub const FEATURES: &[&str] = &["tags", "search", "archive", "slugs", "pages"];

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
                source: None,
            },
            files: vec![],
            pages: vec![PageEntry {
                path: "index.html".into(),
                title: None,
            }],
        };
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["pages"][0]["title"], serde_json::Value::Null);
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

    fn summary() -> ParcelSummary {
        ParcelSummary {
            id: "abcdefghijkm".into(),
            title: "T".into(),
            description: String::new(),
            visibility: Visibility::Private,
            current_version: 1,
            entry: "index.html".into(),
            file_count: 1,
            total_bytes: 1,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            url: "https://x/p/abcdefghijkm".into(),
            open_comments: 0,
            tags: vec!["plan".into()],
            archived_at: None,
            slug: None,
            owner_handle: None,
            canonical_url: "https://x/p/abcdefghijkm".into(),
        }
    }

    #[test]
    fn list_items_flatten_and_omit_what_is_unset() {
        let item = ParcelListItem {
            parcel: summary(),
            matched: None,
            shared: None,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "abcdefghijkm");
        assert_eq!(json["tags"], serde_json::json!(["plan"]));
        assert!(json.get("match").is_none() && json.get("shared").is_none());
        // What an older client reads: every summary field, where it was.
        let old: ParcelSummary = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old, item.parcel);
        let back: ParcelListItem = serde_json::from_value(json).unwrap();
        assert_eq!(back, item);
        assert_eq!(back.title, "T", "derefs to the summary");
    }

    #[test]
    fn older_server_responses_still_parse() {
        let mut json = serde_json::to_value(summary()).unwrap();
        let object = json.as_object_mut().unwrap();
        for key in ["tags", "archived_at", "slug", "canonical_url"] {
            object.remove(key);
        }
        let parsed: ParcelListItem = serde_json::from_value(json).unwrap();
        assert!(parsed.tags.is_empty() && parsed.archived_at.is_none());
        assert!(parsed.slug.is_none() && parsed.canonical_url.is_empty());
        let result: PublishResult = serde_json::from_value(serde_json::json!({
            "parcel": serde_json::to_value(summary()).unwrap(),
            "version": {"version": 1, "entry": "index.html", "label": null, "file_count": 1,
                        "total_bytes": 1, "created_at": "1970-01-01T00:00:00Z", "files": []},
        }))
        .unwrap();
        assert!(!result.created && result.version.summary.source.is_none());
    }

    #[test]
    fn update_request_sends_an_empty_tag_set() {
        let clear = UpdateParcelRequest {
            tags: Some(vec![]),
            ..UpdateParcelRequest::default()
        };
        assert_eq!(serde_json::to_string(&clear).unwrap(), r#"{"tags":[]}"#);
        assert_eq!(
            serde_json::to_string(&UpdateParcelRequest::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn error_body_shape() {
        let body: ErrorBody =
            serde_json::from_str(r#"{"error":{"message":"nope","type":"Not Found"}}"#).unwrap();
        assert_eq!(body.error.kind, "Not Found");
    }
}
