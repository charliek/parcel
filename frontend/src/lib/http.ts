/**
 * Response parsing shared by every API call.
 *
 * Errors use the backend envelope `{"error": {"message", "type"}}`; `message`
 * is human-readable and safe to show, so it is what toasts and inline errors
 * display.
 */

import type { ErrorBody } from './types';

/** Error thrown for any non-2xx API response. Carries the HTTP status. */
export class ApiError extends Error {
  readonly status: number;
  readonly type?: string;

  constructor(message: string, status: number, type?: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.type = type;
  }

  get isUnauthorized(): boolean {
    return this.status === 401;
  }

  get isNotFound(): boolean {
    return this.status === 404;
  }

  /**
   * 401, 403 or 404: the server either does not know who we are or will not
   * admit the parcel exists to us (private parcels look like 404s).
   */
  get isAccessDenied(): boolean {
    return this.status === 401 || this.status === 403 || this.status === 404;
  }
}

/** Status of an error if it came from the API, else undefined (network, bug). */
export function errorStatus(err: unknown): number | undefined {
  return err instanceof ApiError ? err.status : undefined;
}

/** A message suitable for a toast or inline error. */
export function errorMessage(err: unknown, fallback = 'Something went wrong'): string {
  if (err instanceof ApiError || err instanceof Error) return err.message || fallback;
  if (typeof err === 'string' && err) return err;
  return fallback;
}

/** Parse a fetch response body, throwing {@link ApiError} for non-2xx. */
export async function parseResponse<T>(response: Response): Promise<T> {
  const text = response.status === 204 ? '' : await response.text();
  let body: unknown;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  if (!response.ok) {
    throw errorFromBody(body, response.status, response.statusText);
  }
  return body as T;
}

/** Build an {@link ApiError} from a parsed error body (also used by XHR uploads). */
export function errorFromBody(body: unknown, status: number, statusText = ''): ApiError {
  const envelope = body as Partial<ErrorBody> | undefined;
  const message =
    envelope?.error?.message ||
    (typeof body === 'string' && body.length < 300 && body) ||
    statusText ||
    (status === 0
      ? 'Network error: the server could not be reached'
      : `Request failed (${status})`);
  return new ApiError(message, status, envelope?.error?.type);
}
