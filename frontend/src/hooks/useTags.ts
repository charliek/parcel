import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRef } from 'react';
import { api, type TagFilters } from '@/lib/api';
import {
  type DashboardQuery,
  facetFilters,
  mergeFacetCounts,
  swapFacetFilters,
} from '@/lib/search-query';
import type { SearchMode, TagCount } from '@/lib/types';
import { queryKeys } from './queryKeys';
import { refreshSlugResolutions } from './useParcels';

/** The server's largest page of tags. */
const TAG_PAGE = 1000;

/**
 * Every tag (with counts) over the parcels matching `filters`: `GET /tags`
 * page by page until `total` is reached.
 */
export async function fetchAllTags(filters: TagFilters): Promise<TagCount[]> {
  const tags: TagCount[] = [];
  for (;;) {
    const page = await api.listTags({ ...filters, limit: TAG_PAGE, offset: tags.length });
    tags.push(...page.tags);
    if (page.tags.length === 0 || tags.length >= page.total) return tags;
  }
}

/** The query for {@link fetchAllTags}; the last counts stay up while new ones load. */
function tagCountsQuery(filters: TagFilters, enabled = true) {
  return {
    queryKey: queryKeys.tags(filters),
    queryFn: () => fetchAllTags(filters),
    placeholderData: (previous: TagCount[] | undefined) => previous,
    enabled,
  };
}

/** Tag counts for the parcels matching `filters` (the dashboard's facets, autocomplete). */
export function useTagCounts(filters: TagFilters, enabled = true) {
  return useQuery(tagCountsQuery(filters, enabled));
}

/** Every tag the caller uses on their own parcels, archived ones included. */
export const OWN_TAGS: TagFilters = { scope: 'mine', archived: 'include' };

/**
 * The dashboard's facet counts: the main counts for the list's filters,
 * with the selected project's and kind's facets counted as a swap would
 * leave them (lib/search-query.ts), all in the list's search `mode`
 * (`null`: not known yet, so nothing is fetched). Undefined until all have
 * loaded.
 */
export function useFacetCounts(
  query: DashboardQuery,
  mode: SearchMode | undefined | null,
): TagCount[] | undefined {
  const enabled = mode !== null;
  const main = useTagCounts(facetFilters(query, mode ?? undefined), enabled);
  const swaps = swapFacetFilters(query, mode ?? undefined);
  const results = useQueries({
    queries: swaps.map((swap) => tagCountsQuery(swap.filters, enabled)),
  });
  // While a new filter's counts load, the last complete set stays up (no
  // skeleton flashing in the sidebar at every click).
  const last = useRef<TagCount[] | undefined>(undefined);
  if (main.data && results.every((r) => r.data)) {
    last.current = mergeFacetCounts(
      main.data,
      swaps.map((swap, i) => ({ namespace: swap.namespace, counts: results[i].data ?? [] })),
    );
  }
  return last.current;
}

/** After a rename or delete: tags moved on many parcels at once. */
function useInvalidateTagged() {
  const client = useQueryClient();
  return () => {
    void client.invalidateQueries({ queryKey: queryKeys.allTags });
    void client.invalidateQueries({ queryKey: queryKeys.allParcels });
    void client.invalidateQueries({ queryKey: queryKeys.allParcelDetails });
    void client.invalidateQueries({ queryKey: queryKeys.everyView });
    void refreshSlugResolutions(client);
  };
}

export function useRenameTag() {
  const invalidate = useInvalidateTagged();
  return useMutation({
    mutationFn: ({ from, to }: { from: string; to: string }) => api.renameTag(from, to),
    onSuccess: invalidate,
  });
}

export function useDeleteTag() {
  const invalidate = useInvalidateTagged();
  return useMutation({
    mutationFn: (tag: string) => api.deleteTag(tag),
    onSuccess: invalidate,
  });
}
