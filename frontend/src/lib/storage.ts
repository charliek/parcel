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
  /** localStorage: bumped on sign-out so other tabs forget the user too. */
  authSync: 'parcel_auth_sync',
  /** localStorage: this browser has signed in before (lets account pages go straight to the provider). */
  signedInHint: 'parcel_signed_in',
  /** localStorage: id of the sign-in provider used last, offered first. */
  lastProvider: 'parcel_last_provider',
  /** sessionStorage: when an account page last sent this tab to sign in on its own (loop guard). */
  autoSignIn: 'parcel_auto_sign_in',
  /** localStorage: `light` | `dark` | `system`. Also read by public/theme-init.js. */
  theme: 'parcel_theme',
  /** localStorage: dashboard layout, `grid` | `list`. */
  dashboardLayout: 'parcel_dashboard_layout',
  /** localStorage: whether the viewer's info panel is open. */
  viewerPanel: 'parcel_viewer_panel',
  /** localStorage: `0` stops the viewer reloading itself when a new version is published. */
  viewerAutoReload: 'parcel_viewer_auto_reload',
} as const;
