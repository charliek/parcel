/**
 * JSON wire types for `/api/v1`, mirroring `crates/parcel-types/src/api.rs`.
 *
 * Timestamps are RFC 3339 strings, sizes are bytes and version numbers are
 * integers. Optional request fields are omitted (never `null`) because the
 * server rejects unknown fields and treats a missing field as "unchanged".
 */

/** Who can open a parcel. */
export type Visibility = 'private' | 'public';

/** How a new version's file set is derived from the previous one. */
export type PublishMode = 'merge' | 'replace';

/** A parcel as it appears in lists. */
export interface ParcelSummary {
  id: string;
  title: string;
  description: string;
  visibility: Visibility;
  current_version: number;
  /** Path of the current version's entry page. */
  entry: string;
  file_count: number;
  total_bytes: number;
  created_at: string;
  updated_at: string;
  /** The viewer URL in the web app (`https://…/p/<id>`). */
  url: string;
  /** Unresolved comment threads. */
  open_comments: number;
}

/** One immutable version of a parcel. */
export interface VersionSummary {
  version: number;
  entry: string;
  /** Optional short note recorded with the publish ("Draft to legal"). */
  label: string | null;
  file_count: number;
  total_bytes: number;
  created_at: string;
}

/** One file inside a version. */
export interface FileEntry {
  path: string;
  size_bytes: number;
  content_type: string;
  /** Lowercase hex SHA-256 of the file's bytes. */
  sha256: string;
}

/** A version with its file list (`VersionSummary` fields are flattened in). */
export interface VersionDetail extends VersionSummary {
  files: FileEntry[];
}

/** A parcel with its history and current files. */
export interface ParcelDetail {
  parcel: ParcelSummary;
  /** Whether the caller owns this parcel (and so may change it). */
  is_owner: boolean;
  /** Newest first. */
  versions: VersionSummary[];
  /** Files of the current version. */
  files: FileEntry[];
}

/** `GET /api/v1/parcels/{id}/versions`. */
export interface VersionList {
  /** Newest first. */
  versions: VersionSummary[];
}

/** `GET /api/v1/parcels`. */
export interface ParcelList {
  parcels: ParcelSummary[];
  /** Total number of matching parcels (for pagination). */
  total: number;
}

/** The `metadata` part of a create (`POST /api/v1/parcels`) multipart upload. */
export interface CreateParcelMetadata {
  /** Defaults to the entry page's `<title>` / first heading, else its file name. */
  title?: string;
  description?: string;
  visibility?: Visibility;
  /** Entry page path. Defaults to the only HTML/Markdown file, else `index.html`. */
  entry?: string;
  label?: string;
}

/** The `metadata` part of a new-version (`POST /api/v1/parcels/{id}/versions`) upload. */
export interface NewVersionMetadata {
  mode?: PublishMode;
  /** Paths to drop from the previous version (merge mode only). */
  remove?: string[];
  entry?: string;
  label?: string;
  title?: string;
  description?: string;
  visibility?: Visibility;
  /** Optimistic concurrency: fail with 409 unless this is still the current version. */
  base_version?: number;
}

/** Returned by both create and new-version publishes. */
export interface PublishResult {
  parcel: ParcelSummary;
  version: VersionDetail;
}

/** `PATCH /api/v1/parcels/{id}` — metadata only; files change through versions. */
export interface UpdateParcelRequest {
  title?: string;
  description?: string;
  visibility?: Visibility;
}

/** `GET /api/v1/parcels/{id}/view` — what the viewer iframe loads. */
export interface ViewGrant {
  parcel: ParcelSummary;
  version: VersionSummary;
  is_owner: boolean;
  /** Signed, expiring URL on the sandboxed content origin. */
  content_url: string;
  expires_at: string;
}

/** Storage and size limits enforced on publishes. */
export interface Limits {
  max_entry_bytes: number;
  max_file_bytes: number;
  max_upload_bytes: number;
  max_files_per_upload: number;
  max_files_per_version: number;
  max_version_bytes: number;
  /** Total bytes across all of a user's versions; 0 means unlimited. */
  user_quota_bytes: number;
  /** Data a parcel may keep (each item's JSON plus a per-item overhead). */
  max_data_bytes: number;
  /** Largest single data value, as compact JSON. */
  max_data_value_bytes: number;
}

/** What a user is currently storing. */
export interface Usage {
  parcel_count: number;
  version_count: number;
  /** Sum of every version's `total_bytes` (what the quota counts). */
  stored_bytes: number;
}

/** `GET /api/v1/me`. */
export interface Me {
  id: string;
  email: string;
  name: string;
  usage: Usage;
  limits: Limits;
}

/** The two auth modes the backend can run in. */
export type AuthMode = 'slauth' | 'dev';

/** slauth settings the SPA needs to start an OAuth flow. */
export interface AuthInfo {
  /** `slauth` or `dev`. Typed as string on the wire; see {@link AuthMode}. */
  mode: AuthMode | (string & {});
  issuer: string;
  client_id: string;
  audience: string;
}

/** `GET /api/v1/info` — unauthenticated server description. */
export interface ServerInfo {
  name: string;
  version: string;
  app_origin: string;
  content_origin: string;
  auth: AuthInfo;
  limits: Limits;
  allowed_extensions: string[];
}

/** How a token was minted. */
export type TokenCreatedVia = 'web' | 'device' | (string & {});

/** An API token as listed (never includes the secret). */
export interface ApiToken {
  id: string;
  name: string;
  /** Non-secret display prefix, e.g. `pcl_a1b2c3d4`. */
  prefix: string;
  /** `web` (minted on the Tokens page) or `device` (CLI login). */
  created_via: TokenCreatedVia;
  created_at: string;
  last_used_at: string | null;
  expires_at: string | null;
}

/** `POST /api/v1/tokens`. */
export interface CreateTokenRequest {
  name: string;
  /** Days until expiry; omit for a token that never expires. */
  expires_in_days?: number;
}

/** A freshly minted token. `secret` is shown exactly once. */
export interface CreatedToken {
  token: ApiToken;
  secret: string;
}

/** `GET /api/v1/tokens`. */
export interface TokenList {
  tokens: ApiToken[];
}

/** `POST /api/v1/cli/device` — start a CLI device login. */
export interface DeviceStartRequest {
  /** Shown on the approval page, e.g. `parcel CLI on laptop`. */
  client_name?: string;
}

export interface DeviceStartResponse {
  /** Secret the CLI polls with. Never shown to the user. */
  device_code: string;
  /** Short code the user confirms in the browser, e.g. `BCDF-GHJK`. */
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string;
  /** Seconds until the codes expire. */
  expires_in: number;
  /** Minimum seconds between polls. */
  interval: number;
}

/** `POST /api/v1/cli/device/token`. */
export interface DevicePollRequest {
  device_code: string;
}

export type DeviceStatus = 'pending' | 'slow_down' | 'approved' | 'denied' | 'expired';

export interface DevicePollResponse {
  status: DeviceStatus;
  token?: string;
  email?: string;
}

/** `GET /api/v1/cli/device/{user_code}` — what the approval page shows. */
export interface DeviceInfo {
  user_code: string;
  client_name: string;
  created_at: string;
  expires_at: string;
}

// ---- comments (crates/parcel-types/src/comments.rs) -------------------------

/** Where on the page a comment points, as recorded by the page helper. */
export interface CommentAnchor {
  /** Published path of the page the comment was made on (`index.html`). */
  path: string;
  /** CSS selector of the element that was clicked. */
  selector?: string;
  /** The element's visible text when the comment was made. */
  text?: string;
  /** Click position inside the element's box, as a fraction of its width (0–1). */
  x?: number;
  /** Click position inside the element's box, as a fraction of its height (0–1). */
  y?: number;
}

/** Where a comment was written: the web viewer, or a `pcl_` token (CLI or agent). */
export type CommentVia = 'web' | 'cli' | (string & {});

/** Who wrote a comment, from the caller's point of view. */
export interface CommentAuthor {
  /** Display name (the account's name, else its email). */
  name: string;
  /** The author owns the parcel. */
  is_owner: boolean;
  /** The author is the caller. */
  is_you: boolean;
}

/** A reply in a comment thread. */
export interface CommentReply {
  number: number;
  author: CommentAuthor;
  body: string;
  via: CommentVia;
  created_at: string;
  updated_at: string;
}

/** A comment thread: the top-level comment and its replies. */
export interface Comment {
  number: number;
  /** The version the commenter was looking at. */
  version: number;
  author: CommentAuthor;
  body: string;
  /** Absent for a general comment about the whole parcel. */
  anchor?: CommentAnchor;
  via: CommentVia;
  resolved: boolean;
  resolved_at?: string | null;
  created_at: string;
  updated_at: string;
  /** Oldest first. */
  replies: CommentReply[];
}

/** Which threads to list. */
export type CommentStatus = 'open' | 'resolved' | 'all';

/** `GET /api/v1/parcels/{id}/comments`. */
export interface CommentList {
  /** Oldest first. */
  comments: Comment[];
  /** Unresolved threads on the parcel, whatever the filter. */
  open: number;
}

/** `POST /api/v1/parcels/{id}/comments` — a new thread, or a reply. */
export interface NewComment {
  body: string;
  /** The version being commented on (threads only). */
  version?: number;
  /** Where on the page (threads only). */
  anchor?: CommentAnchor;
  /** Reply to this thread instead of starting a new one. */
  reply_to?: number;
}

/** `PATCH /api/v1/parcels/{id}/comments/{number}`. */
export interface UpdateComment {
  body?: string;
  resolved?: boolean;
}

/** `GET /api/v1/parcels/{id}/activity` — the viewer's cheap poll. */
export interface ParcelActivity {
  current_version: number;
  updated_at: string;
  /** Unresolved comment threads. */
  open_comments: number;
  /** Changes whenever any comment is added, edited, resolved or deleted. */
  comments_revision: number;
}

// ---- page data (crates/parcel-types/src/data.rs) ----------------------------

/** One stored item. */
export interface DataItem {
  collection: string;
  key: string;
  /** Any JSON value. */
  value: unknown;
  size_bytes: number;
  updated_at: string;
}

/** A page of a collection's items, ordered by key. */
export interface DataItemList {
  items: DataItem[];
  /** Pass as `after` to fetch the next page; absent on the last page. */
  next_after?: string;
}

/** A collection as listed in a {@link DataSummary}. */
export interface DataCollection {
  name: string;
  count: number;
  /** What the collection counts against the parcel's data limit. */
  usage_bytes: number;
  updated_at: string;
}

/** `GET /api/v1/parcels/{id}/data`: a parcel's collections and usage. */
export interface DataSummary {
  /** Changes whenever any item is written or deleted. */
  revision: number;
  /** Each item's JSON size plus a fixed per-item overhead. */
  usage_bytes: number;
  limit_bytes: number;
  /** Whether the caller may change this parcel's data. */
  writable: boolean;
  /** By name. */
  collections: DataCollection[];
}

/** The error envelope every non-2xx JSON response uses. */
export interface ErrorBody {
  error: ErrorDetail;
}

export interface ErrorDetail {
  /** Human-readable, safe to show. */
  message: string;
  /** The HTTP status reason phrase, e.g. `Bad Request`. */
  type: string;
}
