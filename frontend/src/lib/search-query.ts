/**
 * The dashboard's filter state and its URL form (`?scope=&tag=&tag=&q=&archived=`),
 * so a filtered list is a link that can be shared, reloaded and walked
 * through with Back and Forward. Pure, so it is unit-tested.
 */

import type { ListFilters } from './api';
import { SINGLE_VALUE_NAMESPACES, tagNamespace } from './tags';
import type { ArchivedFilter, ListScope, TagCount } from './types';

export interface DashboardQuery {
  scope: ListScope;
  /** Tag filters, in the order they were added (all must match). */
  tags: string[];
  /** Free text (title, description, id, slug, page text). */
  q: string;
  archived: ArchivedFilter;
}

export const EMPTY_QUERY: DashboardQuery = { scope: 'mine', tags: [], q: '', archived: 'exclude' };

function parseScope(params: URLSearchParams): ListScope {
  const scope = params.get('scope');
  if (scope === 'mine' || scope === 'shared' || scope === 'all') return scope;
  // Links from before the scope toggle: `?tab=shared`.
  return params.get('tab') === 'shared' ? 'shared' : 'mine';
}

function parseArchived(raw: string | null): ArchivedFilter {
  if (raw === 'include' || raw === 'only') return raw;
  // A hand-written `?archived=1` reads as "show archived".
  return raw === '1' || raw === 'true' ? 'include' : 'exclude';
}

/** Read the dashboard state from the address. Unknown values fall back to defaults. */
export function parseDashboardQuery(params: URLSearchParams): DashboardQuery {
  const tags: string[] = [];
  for (const tag of params.getAll('tag')) {
    const t = tag.trim();
    if (t && !tags.includes(t)) tags.push(t);
  }
  return {
    scope: parseScope(params),
    tags,
    q: (params.get('q') ?? '').trim(),
    archived: parseArchived(params.get('archived')),
  };
}

/**
 * The address for a state, defaults left out, in a fixed order. Parameters
 * the dashboard doesn't own are kept; the legacy `tab` is dropped.
 */
export function dashboardSearch(
  query: DashboardQuery,
  base: URLSearchParams = new URLSearchParams(),
): URLSearchParams {
  const next = new URLSearchParams(base);
  for (const key of ['tab', 'scope', 'tag', 'q', 'archived']) next.delete(key);
  if (query.scope !== 'mine') next.set('scope', query.scope);
  for (const tag of query.tags) next.append('tag', tag);
  if (query.q.trim()) next.set('q', query.q.trim());
  if (query.archived !== 'exclude') next.set('archived', query.archived);
  return next;
}

/** A link to the dashboard with these filters (the rest at their defaults). */
export function dashboardHref(query: Partial<DashboardQuery>): string {
  const search = dashboardSearch({ ...EMPTY_QUERY, ...query }).toString();
  return search ? `/?${search}` : '/';
}

/** Whether the address still uses a legacy form (`?tab=shared`) worth rewriting. */
export function isLegacyDashboardSearch(params: URLSearchParams): boolean {
  return params.has('tab');
}

/**
 * Add a tag filter, or remove it when it is already on. In the
 * single-value namespaces (`project:`, `kind:`) a new value replaces the
 * selected one, because a parcel normally has one of each.
 */
export function toggleTag(tags: readonly string[], tag: string): string[] {
  if (tags.includes(tag)) return tags.filter((t) => t !== tag);
  const ns = tagNamespace(tag);
  const single = (SINGLE_VALUE_NAMESPACES as readonly string[]).includes(ns);
  return [...tags.filter((t) => !single || tagNamespace(t) !== ns), tag];
}

/**
 * The API filters for a state (shared by the list and the facet counts).
 * Unset filters are left undefined, so the counts for an unfiltered list
 * share a cache entry with the search box's vocabulary.
 */
export function listFilters(query: DashboardQuery): ListFilters {
  return {
    scope: query.scope,
    tag: query.tags.length ? query.tags : undefined,
    q: query.q || undefined,
    archived: query.archived,
  };
}

/** Whether anything narrows the list (so an empty result says "no match", not "nothing yet"). */
export function isFiltered(query: DashboardQuery): boolean {
  return query.tags.length > 0 || query.q !== '' || query.archived === 'only';
}

/**
 * The search box's text split at its last word: what autocomplete looks up
 * (`last`) and what stays as free text when a suggestion is picked (`rest`).
 */
export function splitLastWord(text: string): { rest: string; last: string } {
  const match = /^(.*?)(\S*)$/s.exec(text);
  const rest = match?.[1] ?? '';
  const last = match?.[2] ?? '';
  return { rest, last };
}

/**
 * The extra tag-count queries the facets need. Picking another project (or
 * kind) replaces the one selected, so that facet's counts must say what
 * the list would hold *after* the swap: the same filters without its own
 * selection, limited to its namespace. One query per such namespace with a
 * selection; the other facets use the main counts.
 */
export function swapFacetFilters(
  query: DashboardQuery,
): { namespace: string; filters: ListFilters & { prefix: string } }[] {
  const base = listFilters(query);
  return SINGLE_VALUE_NAMESPACES.filter((ns) => query.tags.some((t) => tagNamespace(t) === ns)).map(
    (ns) => ({
      namespace: ns,
      filters: { ...base, tag: query.tags.filter((t) => tagNamespace(t) !== ns), prefix: `${ns}:` },
    }),
  );
}

/** The main counts with each swap facet's own counts put in its place. */
export function mergeFacetCounts(
  main: readonly TagCount[],
  swapped: readonly { namespace: string; counts: readonly TagCount[] }[],
): TagCount[] {
  const replaced = new Set(swapped.map((s) => s.namespace));
  return [
    ...main.filter((t) => !replaced.has(tagNamespace(t.tag))),
    ...swapped.flatMap((s) => s.counts),
  ];
}
