/**
 * Viewer addresses: the links people copy and the in-app paths the viewer
 * navigates to, with the version, page and fragment they pin.
 *
 * The server says what a parcel's address is (`canonical_url`: the clean
 * `/@<handle>/<slug>` URL when it has a slug, else `/p/<id>`); this module
 * only appends what the viewer is showing, the same way the CLI's `links()`
 * does (`crates/parcel-cli/src/commands/mod.rs`): a version is `/v/<n>` on
 * `/p/<id>` and `?v=<n>` on a clean URL (a slug can contain `/`), the page
 * is `?page=<path>`, the fragment is kept as is.
 */

import type { PageEntry, ParcelSummary } from './types';

/** Escape everything but unreserved characters and `/` (the CLI's `percent_encode`). */
export function encodePagePath(path: string): string {
  let out = '';
  for (const byte of new TextEncoder().encode(path)) {
    const c = String.fromCharCode(byte);
    out += /[A-Za-z0-9\-._~/]/.test(c) ? c : `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
  }
  return out;
}

/** `?v=<n>&page=<path>#fragment`, each part only when set. */
function suffix(version: number | null, page: string | null, hash: string): string {
  const params: string[] = [];
  if (version !== null) params.push(`v=${version}`);
  if (page) params.push(`page=${encodePagePath(page)}`);
  const fragment = hash.startsWith('#') ? hash.slice(1) : hash;
  return (params.length ? `?${params.join('&')}` : '') + (fragment ? `#${fragment}` : '');
}

export interface ViewerSpot {
  /** A pinned version (null: whatever is current). */
  version: number | null;
  /** A page other than the entry (null: the entry). */
  page: string | null;
  /** `#fragment` (with or without the `#`), or empty. */
  hash: string;
}

/**
 * The link to copy and share: the parcel's canonical URL plus the version,
 * page and fragment in view. An older server sends no `canonical_url`; then
 * it is the legacy `url`.
 */
export function shareLink(parcel: ParcelSummary, spot: ViewerSpot): string {
  if (parcel.slug && parcel.canonical_url) {
    return parcel.canonical_url + suffix(spot.version, spot.page, spot.hash);
  }
  const base = parcel.canonical_url || parcel.url;
  const pin = spot.version !== null ? `/v/${spot.version}` : '';
  return base + pin + suffix(null, spot.page, spot.hash);
}

/** Where the viewer for a parcel lives in this app: `/p/<id>`, or a clean `/@h/slug` path. */
export type ViewerRoute = { kind: 'id'; id: string } | { kind: 'slug'; path: string };

/**
 * The in-app path for a version and page of the parcel on `route`. The
 * current version is the bare address; so is the entry page.
 */
export function viewerPath(route: ViewerRoute, spot: ViewerSpot): string {
  if (route.kind === 'slug') return route.path + suffix(spot.version, spot.page, spot.hash);
  const pin = spot.version !== null ? `/v/${spot.version}` : '';
  return `/p/${route.id}${pin}${suffix(null, spot.page, spot.hash)}`;
}

/** The in-app path of a parcel's canonical address (`/@h/slug` or `/p/<id>`). */
export function canonicalPath(parcel: ParcelSummary): string {
  if (parcel.canonical_url) {
    try {
      return new URL(parcel.canonical_url).pathname;
    } catch {
      // Fall through to the id path.
    }
  }
  return `/p/${parcel.id}`;
}

/**
 * A clean URL as the app previews it before the server has named the
 * parcel (once saved, `canonical_url` is what is shown).
 */
export function cleanUrl(origin: string, handle: string, slug: string): string {
  return `${origin}/@${handle}/${slug}`;
}

/**
 * The handle and slug of a clean URL's path (`/@ada/parcel/plan` →
 * `{handle: 'ada', slug: 'parcel/plan'}`), or null when it isn't one.
 * Nothing is validated here: the server's answer to a bad one is a 404.
 */
export function parseSlugPath(pathname: string): { handle: string; slug: string } | null {
  if (!pathname.startsWith('/@')) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  const [first, ...rest] = decoded.slice(2).split('/');
  const slug = rest.filter(Boolean).join('/');
  if (!first || !slug) return null;
  return { handle: first, slug };
}

/**
 * A page's name for menus: its title, else its path made readable
 * (`docs/rust-build_settings.html` → `Rust build settings`).
 */
export function pageLabel(page: PageEntry): string {
  const title = page.title?.trim();
  if (title) return title;
  const base = page.path.split('/').pop() ?? page.path;
  const words = base
    .replace(/\.(html?|md|markdown)$/i, '')
    .replace(/[-_]+/g, ' ')
    .trim();
  if (!words) return page.path;
  return words.charAt(0).toUpperCase() + words.slice(1);
}
