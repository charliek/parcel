/**
 * Where to send the user after sign-in.
 *
 * Only same-origin, absolute-path destinations are allowed, so a crafted
 * `return_to` can never bounce the user off-site after login. The server
 * applies the same rule (`http/auth.rs`, `safe_return_to`); this copy keeps
 * the SPA from even asking for anything else.
 */

const FALLBACK = '/';

/** Longest return path the server accepts. */
const MAX_LEN = 2048;

export function safeReturnTo(raw: string | null | undefined): string {
  if (!raw || typeof raw !== 'string' || raw.length > MAX_LEN) return FALLBACK;
  // Must be an absolute path on this origin: "/x", never "//host" or "/\host".
  if (!raw.startsWith('/') || raw.startsWith('//') || raw.includes('\\')) return FALLBACK;
  // No control characters (header/URL smuggling).
  for (let i = 0; i < raw.length; i++) {
    const code = raw.charCodeAt(i);
    if (code < 0x20 || code === 0x7f) return FALLBACK;
  }
  // Never back into sign-in itself: the server's /auth routes, or the
  // sign-in page (which would only send the user round again).
  if (isPath(raw, '/auth') || isPath(raw, '/signin')) return FALLBACK;
  return raw;
}

/** `raw` is `prefix`, or a path, query or fragment under it. */
function isPath(raw: string, prefix: string): boolean {
  if (!raw.startsWith(prefix)) return false;
  const next = raw.charAt(prefix.length);
  return next === '' || next === '/' || next === '?' || next === '#';
}

/** The current location as a return path (path + query + hash). */
export function currentReturnTo(): string {
  if (typeof window === 'undefined') return FALLBACK;
  const { pathname, search, hash } = window.location;
  return safeReturnTo(`${pathname}${search}${hash}`);
}
