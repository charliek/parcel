/**
 * Typed client for the Parcel JSON API (`/api/v1`).
 *
 * The SPA talks to the backend over relative URLs, proxied to the Rust server
 * by Vite in dev and served from the same origin in production. A slauth
 * bearer token is attached when this tab has one; in dev auth mode there is
 * none and the backend authenticates every request as the dev user. Public
 * parcels are readable with no token at all.
 */

import { ApiError, errorFromBody, parseResponse } from './http';
import { useTokenStore } from './token-store';
import type {
  Comment,
  CommentList,
  CommentStatus,
  CreatedToken,
  CreateTokenRequest,
  DataSummary,
  DeviceInfo,
  Me,
  NewComment,
  ParcelActivity,
  ParcelDetail,
  ParcelList,
  ParcelSummary,
  PublishResult,
  TokenList,
  UpdateComment,
  UpdateParcelRequest,
  VersionDetail,
  VersionList,
  ViewGrant,
} from './types';

let onUnauthorized: (() => void) | null = null;

/** Called whenever the API answers 401 (the auth store downgrades the session). */
export function setUnauthorizedHandler(handler: (() => void) | null): void {
  onUnauthorized = handler;
}

interface ApiFetchOptions extends Omit<RequestInit, 'body'> {
  /** JSON body to send; sets `Content-Type` and serializes for you. */
  json?: unknown;
  body?: BodyInit;
}

async function send(path: string, options: ApiFetchOptions, token: string | null) {
  const { json, headers, body, ...rest } = options;
  return fetch(path, {
    ...rest,
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json',
      ...(json !== undefined ? { 'Content-Type': 'application/json' } : {}),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...headers,
    },
    body: json !== undefined ? JSON.stringify(json) : body,
  });
}

/**
 * Perform an API request. Retries once with a refreshed token if the server
 * rejects the current one. Throws {@link ApiError} on any non-2xx response.
 */
async function rawFetch(path: string, options: ApiFetchOptions = {}): Promise<Response> {
  const tokens = useTokenStore.getState();
  const token = await tokens.getValidToken();
  let response = await send(path, options, token);
  if (response.status === 401 && token) {
    const fresh = await useTokenStore.getState().refresh();
    if (fresh) response = await send(path, options, fresh);
  }
  if (response.status === 401) onUnauthorized?.();
  return response;
}

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
  const attempt = (token: string | null) =>
    new Promise<{ status: number; statusText: string; text: string }>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', path);
      xhr.setRequestHeader('Accept', 'application/json');
      if (token) xhr.setRequestHeader('Authorization', `Bearer ${token}`);
      xhr.upload.onprogress = (event) => {
        if (event.lengthComputable && onProgress) onProgress(event.loaded / event.total);
      };
      xhr.onload = () =>
        resolve({ status: xhr.status, statusText: xhr.statusText, text: xhr.responseText });
      xhr.onerror = () => reject(errorFromBody(undefined, 0));
      xhr.onabort = () => reject(new ApiError('Upload cancelled', 0));
      xhr.send(form);
    });

  const token = await useTokenStore.getState().getValidToken();
  let result = await attempt(token);
  if (result.status === 401 && token) {
    const fresh = await useTokenStore.getState().refresh();
    if (fresh) result = await attempt(fresh);
  }
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

function query(params: Record<string, string | number | undefined | null>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : '';
}

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

  listParcels: (params: { q?: string; limit?: number; offset?: number } = {}) =>
    apiFetch<ParcelList>(`/api/v1/parcels${query(params)}`),

  getParcel: (id: string) => apiFetch<ParcelDetail>(`/api/v1/parcels/${enc(id)}`),

  updateParcel: (id: string, body: UpdateParcelRequest) =>
    apiFetch<ParcelSummary>(`/api/v1/parcels/${enc(id)}`, { method: 'PATCH', json: body }),

  deleteParcel: (id: string) => apiFetch<void>(`/api/v1/parcels/${enc(id)}`, { method: 'DELETE' }),

  view: (id: string, version?: number) =>
    apiFetch<ViewGrant>(`/api/v1/parcels/${enc(id)}/view${query({ version })}`),

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
