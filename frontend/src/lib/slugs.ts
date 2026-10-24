/**
 * Handles and slugs: the TypeScript twin of `crates/parcel-types/src/slugs.rs`.
 *
 * A clean parcel URL is `/@<handle>/<slug>`. The server checks both on every
 * write; the web app applies the same rules as you type, so the inline
 * message matches what the server would answer. ASCII-only, like the Rust side.
 */

import { isValidParcelId } from './ids';

/** Longest slug, in characters. */
export const MAX_SLUG_CHARS = 100;
/** Most `/`-separated segments in a slug. */
export const MAX_SLUG_SEGMENTS = 4;
/** Longest handle, in characters. */
export const MAX_HANDLE_CHARS = 32;

/** What the inline UI says about a slug it can't accept (the server's wording). */
export const SLUG_RULES =
  `Use 1–${MAX_SLUG_SEGMENTS} '/'-separated segments of lowercase letters, digits and single '-' ` +
  `(at most ${MAX_SLUG_CHARS} characters); a segment may not be 'p', 'v' or shaped like a parcel id.`;

/** What the inline UI says about a handle it can't accept. */
export const HANDLE_RULES =
  `Use 1–${MAX_HANDLE_CHARS} lowercase letters, digits and '-', ` +
  'starting and ending with a letter or digit.';

const EDGE_SPACE = /^[ \t\n\r\f]+|[ \t\n\r\f]+$/g;

/** ASCII-trimmed and ASCII-lowercased, as `slugs.rs`'s `folded`. */
function folded(raw: string): string {
  return raw.replace(EDGE_SPACE, '').replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** `[a-z0-9]+(-[a-z0-9]+)*` */
const WORD_SEGMENT = /^[a-z0-9]+(-[a-z0-9]+)*$/;

export type Checked = { ok: true; value: string } | { ok: false; message: string };

/**
 * Normalize a slug (trim, lowercase) and check it: 1–100 characters, 1–4
 * `/`-separated segments of `[a-z0-9]+(-[a-z0-9]+)*`, none of them `p`, `v`
 * or shaped like a parcel id.
 */
export function normalizeSlug(raw: string): Checked {
  const slug = folded(raw);
  const segments = slug.split('/');
  const ok =
    slug !== '' &&
    slug.length <= MAX_SLUG_CHARS &&
    segments.length <= MAX_SLUG_SEGMENTS &&
    segments.every((s) => WORD_SEGMENT.test(s) && s !== 'p' && s !== 'v' && !isValidParcelId(s));
  return ok ? { ok: true, value: slug } : { ok: false, message: SLUG_RULES };
}

/**
 * Whether a slug field's text can be saved: empty (no slug) or valid. The
 * normalized value is what gets sent.
 */
export function checkSlugText(
  text: string,
): { ok: true; slug: string } | { ok: false; message: string } {
  if (!text.trim()) return { ok: true, slug: '' };
  const checked = normalizeSlug(text);
  return checked.ok ? { ok: true, slug: checked.value } : { ok: false, message: checked.message };
}

/** Normalize a handle (trim, lowercase) and check `^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$`. */
export function normalizeHandle(raw: string): Checked {
  const handle = folded(raw);
  const ok = /^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$/.test(handle);
  return ok ? { ok: true, value: handle } : { ok: false, message: HANDLE_RULES };
}
