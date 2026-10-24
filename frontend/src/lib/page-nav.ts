/**
 * Which page the viewer's frame shows, and how that stays in step with the
 * address (`?page=<path>#fragment`). Pure, so it is unit-tested.
 *
 * Two things move the page. The viewer navigates (the Pages menu, a Files
 * row, a comment on another page, Back/Forward over those, a reload): it
 * fetches a view grant for that page and loads it in a fresh frame. Or the
 * page navigates itself (a relative link, its own history): the frame is
 * already there, so the viewer only rewrites its address with `replace`
 * and must not fetch a grant or remount the frame.
 *
 * So the grant is keyed by `requested` (the page the viewer last asked
 * for), not by the address: when the address changes to the page the frame
 * already shows, nothing is requested.
 */

import type { FileEntry, PageEntry, ParcelDetail, VersionDetail } from './types';

export interface PageNav {
  /** The parcel and version this state belongs to. */
  id: string;
  version: number | null;
  /** `?page=` as last seen (null: the entry). */
  urlPage: string | null;
  /** The page the view grant is fetched for (null: the entry). */
  requested: string | null;
  /** Bumped whenever the frame must load `requested` afresh. */
  nonce: number;
}

export function initialPageNav(
  id: string,
  version: number | null,
  urlPage: string | null,
): PageNav {
  return { id, version, urlPage, requested: urlPage, nonce: 0 };
}

/** `?page=` as the viewer reads it: absent or empty means the entry. */
export function readPageParam(params: URLSearchParams): string | null {
  const page = params.get('page');
  return page ? page : null;
}

/**
 * The state after the address (or the parcel or version) changed. `framePath`
 * is the page the frame shows right now (null while unknown) and `entry`
 * the shown version's entry page. Returns `prev` itself when nothing changes.
 */
export function nextPageNav(
  prev: PageNav,
  next: { id: string; version: number | null; urlPage: string | null },
  frame: { framePath: string | null; entry: string | null },
): PageNav {
  if (next.id !== prev.id || next.version !== prev.version) {
    // Another parcel or version: a new grant whatever the frame shows.
    return { ...next, requested: next.urlPage, nonce: prev.nonce + 1 };
  }
  if (next.urlPage === prev.urlPage) return prev;
  const target = next.urlPage ?? frame.entry;
  if (frame.framePath !== null && target !== null && frame.framePath === target) {
    // The page got there by itself (a link inside it, its own Back).
    return { ...prev, urlPage: next.urlPage };
  }
  return { ...prev, urlPage: next.urlPage, requested: next.urlPage, nonce: prev.nonce + 1 };
}

/** A `#fragment` the page reported, if it is one the viewer will put in its address. */
export function safeHash(hash: unknown): string {
  return typeof hash === 'string' && hash.length <= 200 && /^#[\x21-\x7e]+$/.test(hash) ? hash : '';
}

/**
 * How the address should change after the frame reported its page and
 * fragment, or null when it already matches. A path that isn't one of the
 * shown version's pages changes nothing (the report is untrusted).
 */
export function addressForFrame(input: {
  framePath: string;
  frameHash: string;
  pages: readonly PageEntry[] | null;
  entry: string;
  urlPage: string | null;
  urlHash: string;
}): { page: string | null; hash: string } | null {
  const { framePath, pages, entry } = input;
  if (!pages?.some((p) => p.path === framePath)) return null;
  const page = framePath === entry ? null : framePath;
  const hash = safeHash(input.frameHash);
  const samePage = (input.urlPage ?? entry) === framePath;
  if (samePage && hash === input.urlHash) return null;
  // Keep `?page=index.html` when someone wrote it so; it names the same page.
  return { page: samePage ? input.urlPage : page, hash };
}

/** A version's files and pages (null pages: a server that doesn't list them). */
export interface VersionListing {
  files: FileEntry[];
  pages: PageEntry[] | null;
}

/**
 * The files and pages of the version the frame shows, from whichever
 * listing describes that version: the parcel detail (the current version's)
 * only while its current version is `shown`, else that version's own
 * detail. Null while neither does: right after a live update the detail can
 * still be the previous version's, and its pages must not vouch for (or
 * against) a page of the new one.
 */
export function versionListing(
  shown: number,
  detail: ParcelDetail | undefined,
  versionDetail: VersionDetail | undefined,
): VersionListing | null {
  if (detail && detail.parcel.current_version === shown) {
    return { files: detail.files, pages: detail.pages ?? null };
  }
  if (versionDetail && versionDetail.version === shown) {
    return { files: versionDetail.files, pages: versionDetail.pages ?? null };
  }
  return null;
}
