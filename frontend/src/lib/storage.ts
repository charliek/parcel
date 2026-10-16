/**
 * Web Storage wrappers that never throw.
 *
 * `localStorage`/`sessionStorage` can be missing or throw (private windows,
 * blocked site data, sandboxed previews), so every read and write goes through
 * these helpers and the app keeps working, just without persistence.
 *
 * Every key the app stores is prefixed `parcel_`.
 */

type StorageKind = 'local' | 'session';

function backend(kind: StorageKind): Storage | null {
  try {
    if (typeof window === 'undefined') return null;
    return kind === 'local' ? window.localStorage : window.sessionStorage;
  } catch {
    return null;
  }
}

export function readStorage(kind: StorageKind, key: string): string | null {
  try {
    return backend(kind)?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function writeStorage(kind: StorageKind, key: string, value: string): void {
  try {
    backend(kind)?.setItem(key, value);
  } catch {
    // Storage unavailable or full; persistence is best-effort.
  }
}

export function removeStorage(kind: StorageKind, key: string): void {
  try {
    backend(kind)?.removeItem(key);
  } catch {
    // Nothing to remove.
  }
}

/** All storage keys used by the app, in one place. */
export const STORAGE_KEYS = {
  /** sessionStorage: the pending PKCE transaction (verifier, state, return path). */
  pkceTx: 'parcel_pkce_tx',
  /** sessionStorage: set once a silent prompt=none re-auth was tried this session. */
  silentReauth: 'parcel_silent_reauth',
  /** localStorage: bumped on logout so other tabs drop their in-memory tokens. */
  authSync: 'parcel_auth_sync',
  /** localStorage: this browser has signed in before (lets `/` try silent re-auth). */
  signedInHint: 'parcel_signed_in',
  /** localStorage: `light` | `dark` | `system`. Also read by public/theme-init.js. */
  theme: 'parcel_theme',
  /** localStorage: dashboard layout, `grid` | `list`. */
  dashboardLayout: 'parcel_dashboard_layout',
  /** localStorage: whether the viewer's info panel is open. */
  viewerPanel: 'parcel_viewer_panel',
} as const;
