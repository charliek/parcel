/**
 * Typed client for the Parcel JSON API (`/api/v1`).
 *
 * The SPA talks to the backend over relative URLs, proxied to the Rust server
 * by Vite in dev and served from the same origin in production. Sign-in is a
 * server-side session: the HttpOnly cookie rides along on every same-origin
 * request, so nothing here holds or sends a token (a non-`pcl_` bearer would
 * be refused). In dev auth mode the backend treats every request as the dev
 * user. Public parcels are readable signed out.
 */

import { ApiError, errorFromBody, parseResponse } from './http';
import type {
  ArchivedFilter,
  Comment,
  CommentList,
  CommentStatus,
  CreatedToken,
  CreateTokenRequest,
  DataSummary,
  DeviceInfo,
  ListScope,
  Me,
  NewComment,
  ParcelActivity,
  ParcelDetail,
  ParcelList,
  ParcelSummary,
  PublishResult,
  Share,
  ShareList,
  ShareRole,
  TagList,
  TagsMode,
  TagsUpdated,
  TokenList,
  UpdateComment,
  UpdateParcelRequest,
  VersionDetail,
  VersionList,
  ViewGrant,
} from './types';

let onUnauthorized: (() => void) | null = null;

/**
 * Called whenever the API answers 401: the session ended (the server also
 * cleared its cookie in that response). The auth store signs the tab out.
 */
export function setUnauthorizedHandler(handler: (() => void) | null): void {
  onUnauthorized = handler;
}

interface ApiFetchOptions extends Omit<RequestInit, 'body'> {
  /** JSON body to send; sets `Content-Type` and serializes for you. */
  json?: unknown;
  body?: BodyInit;
}

/** Send an API request; the caller checks the status (see {@link apiFetch}). */
async function rawFetch(path: string, options: ApiFetchOptions = {}): Promise<Response> {
  const { json, headers, body, ...rest } = options;
  const response = await fetch(path, {
    ...rest,
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json',
      ...(json !== undefined ? { 'Content-Type': 'application/json' } : {}),
      ...headers,
    },
    body: json !== undefined ? JSON.stringify(json) : body,
  });
  if (response.status === 401) onUnauthorized?.();
  return response;
}

/** Perform an API request. Throws {@link ApiError} on any non-2xx response. */
export async function apiFetch<T>(path: string, options: ApiFetchOptions = {}): Promise<T> {
  const response = await rawFetch(path, options);
  return parseResponse<T>(response);
}

/** Upload a multipart body with progress (fetch can't report upload progress). */
async function apiUpload<T>(
  path: string,
  form: FormData,
  onProgress?: (fraction: number) => void,
): Promise<T> {
  // Same-origin XHR sends the session cookie (and `Origin`, which the
  // server's CSRF check wants on writes) without being asked.
  const result = await new Promise<{ status: number; statusText: string; text: string }>(
    (resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', path);
      xhr.setRequestHeader('Accept', 'application/json');
      xhr.upload.onprogress = (event) => {
        if (event.lengthComputable && onProgress) onProgress(event.loaded / event.total);
      };
      xhr.onload = () =>
        resolve({ status: xhr.status, statusText: xhr.statusText, text: xhr.responseText });
      xhr.onerror = () => reject(errorFromBody(undefined, 0));
      xhr.onabort = () => reject(new ApiError('Upload cancelled', 0));
      xhr.send(form);
    },
  );
  if (result.status === 401) onUnauthorized?.();

  let body: unknown;
  if (result.text) {
    try {
      body = JSON.parse(result.text);
    } catch {
      body = result.text;
    }
  }
  if (result.status < 200 || result.status >= 300) {
    throw errorFromBody(body, result.status, result.statusText);
  }
  return body as T;
}

const enc = encodeURIComponent;

/** Encode a published path for a URL, keeping its `/` separators. */
export function encodeFilePath(path: string): string {
  return path.split('/').map(enc).join('/');
}

type QueryValue = string | number | undefined | null;

/**
 * A query string from `params`, leaving out empty values. An array repeats
 * its key (`tag=a&tag=b`), which is how the API takes several tags.
 */
export function query(params: Record<string, QueryValue | readonly QueryValue[]>): string {
  const search = new URLSearchParams();
  for (const [key, raw] of Object.entries(params)) {
    const values: readonly QueryValue[] = Array.isArray(raw) ? raw : [raw as QueryValue];
    for (const value of values) {
      if (value !== undefined && value !== null && value !== '') search.append(key, String(value));
    }
  }
  const s = search.toString();
  return s ? `?${s}` : '';
}

/** Filters shared by `GET /parcels` and `GET /tags`. */
export interface ListFilters {
  scope?: ListScope;
  tag?: readonly string[];
  q?: string;
  archived?: ArchivedFilter;
}

/**
 * What `GET /tags` filters by: the list's filters, optionally a tag prefix,
 * and how `q` matches (left out: `auto`).
 */
export type TagFilters = ListFilters & { prefix?: string; mode?: TagsMode };

/** Fetch a file through the files API and hand it to the browser as a download. */
export async function downloadFile(id: string, version: number, path: string): Promise<void> {
  const response = await rawFetch(
    `/api/v1/parcels/${enc(id)}/versions/${version}/files/${encodeFilePath(path)}`,
    { headers: { Accept: '*/*' } },
  );
  if (!response.ok) await parseResponse(response);
  const blob = await response.blob();
  const url = URL.createObjectURL(blob);
  try {
    const a = document.createElement('a');
    a.href = url;
    a.download = path.split('/').pop() || 'download';
    a.rel = 'noopener';
    document.body.appendChild(a);
    a.click();
    a.remove();
  } finally {
    // Give the browser a moment to start the download before revoking.
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }
}

export const api = {
  me: () => apiFetch<Me>('/api/v1/me'),

  /** Change the signed-in user's handle (400 malformed, 409 taken). */
  updateMe: (body: { handle: string }) =>
    apiFetch<Me>('/api/v1/me', { method: 'PATCH', json: body }),

  listParcels: (params: ListFilters & { limit?: number; offset?: number } = {}) =>
    apiFetch<ParcelList>(`/api/v1/parcels${query({ ...params })}`),

  getParcel: (id: string) => apiFetch<ParcelDetail>(`/api/v1/parcels/${enc(id)}`),

  /** Resolve a clean URL's handle and slug (every miss is the same 404). */
  bySlug: (handle: string, slug: string) =>
    apiFetch<ParcelDetail>(`/api/v1/parcels/by-slug${query({ handle, slug })}`),

  /** Tag counts over the parcels matching the same filters as `listParcels`. */
  listTags: (params: TagFilters & { limit?: number; offset?: number } = {}) =>
    apiFetch<TagList>(`/api/v1/tags${query({ ...params })}`),

  /** Rename a tag on every parcel the caller owns (into an existing one merges them). */
  renameTag: (from: string, to: string) =>
    apiFetch<TagsUpdated>('/api/v1/tags/rename', { method: 'POST', json: { from, to } }),

  /** Remove a tag from every parcel the caller owns. */
  deleteTag: (tag: string) =>
    apiFetch<TagsUpdated>('/api/v1/tags/delete', { method: 'POST', json: { tag } }),

  updateParcel: (id: string, body: UpdateParcelRequest) =>
    apiFetch<ParcelSummary>(`/api/v1/parcels/${enc(id)}`, { method: 'PATCH', json: body }),

  deleteParcel: (id: string) => apiFetch<void>(`/api/v1/parcels/${enc(id)}`, { method: 'DELETE' }),

  /** A content URL for one page of a version (its entry unless `path` names another page). */
  view: (id: string, version?: number | null, path?: string | null) =>
    apiFetch<ViewGrant>(`/api/v1/parcels/${enc(id)}/view${query({ version, path })}`),

  listVersions: (id: string) => apiFetch<VersionList>(`/api/v1/parcels/${enc(id)}/versions`),

  getVersion: (id: string, version: number | 'latest') =>
    apiFetch<VersionDetail>(`/api/v1/parcels/${enc(id)}/versions/${version}`),

  restoreVersion: (id: string, version: number) =>
    apiFetch<PublishResult>(`/api/v1/parcels/${enc(id)}/versions/${version}/restore`, {
      method: 'POST',
    }),

  createParcel: (form: FormData, onProgress?: (fraction: number) => void) =>
    apiUpload<PublishResult>('/api/v1/parcels', form, onProgress),

  publishVersion: (id: string, form: FormData, onProgress?: (fraction: number) => void) =>
    apiUpload<PublishResult>(`/api/v1/parcels/${enc(id)}/versions`, form, onProgress),

  activity: (id: string) =>
    apiFetch<ParcelActivity>(`/api/v1/parcels/${enc(id)}/activity`, { cache: 'no-store' }),

  listComments: (id: string, status: CommentStatus = 'open') =>
    apiFetch<CommentList>(`/api/v1/parcels/${enc(id)}/comments${query({ status })}`),

  createComment: (id: string, body: NewComment) =>
    apiFetch<Comment>(`/api/v1/parcels/${enc(id)}/comments`, { method: 'POST', json: body }),

  updateComment: (id: string, number: number, body: UpdateComment) =>
    apiFetch<Comment>(`/api/v1/parcels/${enc(id)}/comments/${number}`, {
      method: 'PATCH',
      json: body,
    }),

  deleteComment: (id: string, number: number) =>
    apiFetch<void>(`/api/v1/parcels/${enc(id)}/comments/${number}`, { method: 'DELETE' }),

  dataSummary: (id: string) => apiFetch<DataSummary>(`/api/v1/parcels/${enc(id)}/data`),

  listShares: (id: string) => apiFetch<ShareList>(`/api/v1/parcels/${enc(id)}/shares`),

  /** Share with someone, or change their role. */
  putShare: (id: string, email: string, role: ShareRole) =>
    apiFetch<Share>(`/api/v1/parcels/${enc(id)}/shares/${enc(email)}`, {
      method: 'PUT',
      json: { role },
    }),

  deleteShare: (id: string, email: string) =>
    apiFetch<void>(`/api/v1/parcels/${enc(id)}/shares/${enc(email)}`, { method: 'DELETE' }),

  /** Parcels other people shared with the caller by email. */

  listTokens: () => apiFetch<TokenList>('/api/v1/tokens'),

  createToken: (body: CreateTokenRequest) =>
    apiFetch<CreatedToken>('/api/v1/tokens', { method: 'POST', json: body }),

  revokeToken: (id: string) => apiFetch<void>(`/api/v1/tokens/${enc(id)}`, { method: 'DELETE' }),

  getDevice: (userCode: string) => apiFetch<DeviceInfo>(`/api/v1/cli/device/${enc(userCode)}`),

  approveDevice: (userCode: string) =>
    apiFetch<void>(`/api/v1/cli/device/${enc(userCode)}/approve`, { method: 'POST' }),

  denyDevice: (userCode: string) =>
    apiFetch<void>(`/api/v1/cli/device/${enc(userCode)}/deny`, { method: 'POST' }),
};
