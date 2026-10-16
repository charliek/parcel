/**
 * Where to send the user after sign-in.
 *
 * Only same-origin, absolute-path destinations are allowed, so a crafted
 * `state`/session value can never bounce the user off-site after login.
 */

const FALLBACK = '/';

export function safeReturnTo(raw: string | null | undefined): string {
  if (!raw || typeof raw !== 'string') return FALLBACK;
  // Must be an absolute path on this origin: "/x", never "//host" or "/\host".
  if (!raw.startsWith('/') || raw.startsWith('//') || raw.startsWith('/\\')) return FALLBACK;
  // No control characters (header/URL smuggling).
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i);
    if (code < 0x20 || code === 0x7f) return FALLBACK;
  }
  // Never loop back into the OAuth callback.
  if (raw === '/auth/callback' || raw.startsWith('/auth/callback?')) return FALLBACK;
  return raw;
}

/** The current location as a return path (path + query + hash). */
export function currentReturnTo(): string {
  if (typeof window === 'undefined') return FALLBACK;
  const { pathname, search, hash } = window.location;
  return safeReturnTo(`${pathname}${search}${hash}`);
}
