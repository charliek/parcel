/**
 * Tag normalization: the TypeScript twin of `crates/parcel-types/src/tags.rs`.
 *
 * The server normalizes every tag it stores; the web app applies the same
 * rule as you type, so a chip shows exactly what will be saved. Both
 * implementations are tested against one case list,
 * `crates/parcel-types/tests/fixtures/tag-cases.json`. Only ASCII is
 * folded and trimmed, matching the Rust side byte for byte.
 */

/** Most tags one parcel may carry. */
export const MAX_TAGS = 20;
/** Longest tag, after normalization. */
export const MAX_TAG_CHARS = 64;

const EDGE_SPACE = /^[ \t\n\r\f]+|[ \t\n\r\f]+$/g;
const SEPARATOR_RUN = /[ \t\n\r\f_]+/g;
const SHAPE = /^[a-z0-9][a-z0-9.+-]*(:[a-z0-9][a-z0-9.+/-]*)?$/;

/**
 * Normalize one tag: trim, lowercase, turn each run of whitespace and `_`
 * into `-`, then require the tag shape and at most 64 characters. `null`
 * when it can't be a tag.
 */
export function normalizeTag(raw: string): string | null {
  const tag = raw
    .replace(EDGE_SPACE, '')
    .replace(SEPARATOR_RUN, '-')
    .replace(/[A-Z]/g, (c) => c.toLowerCase());
  return tag.length <= MAX_TAG_CHARS && SHAPE.test(tag) ? tag : null;
}

export type TagsResult =
  | { ok: true; tags: string[] }
  | { ok: false; error: 'invalid'; tag: string }
  | { ok: false; error: 'too_many'; count: number };

/**
 * Normalize a set: each tag as {@link normalizeTag}, duplicates dropped,
 * sorted by bytes (the default sort compares code units, which is the same
 * thing for ASCII), then held to {@link MAX_TAGS}.
 */
export function normalizeTags(raw: readonly string[]): TagsResult {
  const set = new Set<string>();
  for (const r of raw) {
    const tag = normalizeTag(r);
    if (tag === null) return { ok: false, error: 'invalid', tag: r };
    set.add(tag);
  }
  if (set.size > MAX_TAGS) return { ok: false, error: 'too_many', count: set.size };
  return { ok: true, tags: [...set].sort() };
}
