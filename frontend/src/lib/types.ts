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
