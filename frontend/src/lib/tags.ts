/**
 * Tag normalization: the TypeScript twin of `crates/parcel-types/src/tags.rs`.
 *
 * The server normalizes every tag it stores; the web app applies the same
 * rule as you type, so a chip shows exactly what will be saved. Both
 * implementations are tested against one case list,
 * `crates/parcel-types/tests/fixtures/tag-cases.json`. Only ASCII is
 * folded and trimmed, matching the Rust side byte for byte.
 */

import { folded } from './slugs';

/** Most tags one parcel may carry. */
export const MAX_TAGS = 20;
/** Longest tag, after normalization. */
export const MAX_TAG_CHARS = 64;

/** What the inline UI says a tag may contain. */
export const TAG_RULES =
  'lowercase letters, digits, “.”, “+” and “-”, optionally as namespace:value.';

const SEPARATOR_RUN = /[ \t\n\r\f_]+/g;
const SHAPE = /^[a-z0-9][a-z0-9.+-]*(:[a-z0-9][a-z0-9.+/-]*)?$/;

/**
 * Normalize one tag: trim, lowercase, turn each run of whitespace and `_`
 * into `-`, then require the tag shape and at most 64 characters. `null`
 * when it can't be a tag.
 */
export function normalizeTag(raw: string): string | null {
  const tag = folded(raw).replace(SEPARATOR_RUN, '-');
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

/** The namespace of `ns:value` (`project`), or `''` for a bare topic tag. */
export function tagNamespace(tag: string): string {
  const i = tag.indexOf(':');
  return i === -1 ? '' : tag.slice(0, i);
}

/** The part after the namespace (`roost` in `project:roost`), or the whole bare tag. */
export function tagValue(tag: string): string {
  const i = tag.indexOf(':');
  return i === -1 ? tag : tag.slice(i + 1);
}

/**
 * Namespaces where a parcel normally has one value, so selecting a second
 * filter in them replaces the first (the API itself just ANDs them).
 */
export const SINGLE_VALUE_NAMESPACES = ['project', 'kind'] as const;

/** One group of tags: a namespace (`project`, any other `ns`), or `''` for bare tags. */
export interface TagGroup<T> {
  namespace: string;
  label: string;
  items: T[];
}

function groupLabel(namespace: string): string {
  if (namespace === 'project') return 'Projects';
  if (namespace === 'kind') return 'Kind';
  if (namespace === '') return 'Tags';
  return namespace.charAt(0).toUpperCase() + namespace.slice(1);
}

/**
 * Group tags by namespace in the order the sidebar and the Tags page show
 * them: Projects, Kind, every other namespace by name, then bare tags.
 * Items keep their order within a group.
 */
export function groupTags<T>(items: readonly T[], tagOf: (item: T) => string): TagGroup<T>[] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const ns = tagNamespace(tagOf(item));
    const list = groups.get(ns);
    if (list) list.push(item);
    else groups.set(ns, [item]);
  }
  const rank = (ns: string) => (ns === 'project' ? 0 : ns === 'kind' ? 1 : ns === '' ? 3 : 2);
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0))
    .map(([namespace, list]) => ({ namespace, label: groupLabel(namespace), items: list }));
}
