import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRef } from 'react';
import { api, type TagFilters } from '@/lib/api';
import {
  type DashboardQuery,
  listFilters,
  mergeFacetCounts,
  swapFacetFilters,
} from '@/lib/search-query';
import type { TagCount } from '@/lib/types';
import { queryKeys } from './queryKeys';

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

/** Tag counts for the parcels matching `filters` (the dashboard's facets, autocomplete). */
export function useTagCounts(filters: TagFilters, enabled = true) {
  return useQuery({
    queryKey: queryKeys.tags(filters),
    queryFn: () => fetchAllTags(filters),
    enabled,
    placeholderData: (previous) => previous,
  });
}

/** Every tag the caller uses on their own parcels, archived ones included. */
export const OWN_TAGS: TagFilters = { scope: 'mine', archived: 'include' };

/**
 * The dashboard's facet counts: the main counts for the list's filters,
 * with the selected project's and kind's facets counted as a swap would
 * leave them (lib/search-query.ts). Undefined until all have loaded.
 */
export function useFacetCounts(query: DashboardQuery): TagCount[] | undefined {
  const main = useTagCounts(listFilters(query));
  const swaps = swapFacetFilters(query);
  const results = useQueries({
    queries: swaps.map((swap) => ({
      queryKey: queryKeys.tags(swap.filters),
      queryFn: () => fetchAllTags(swap.filters),
      placeholderData: (previous: TagCount[] | undefined) => previous,
    })),
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
    void client.invalidateQueries({ queryKey: queryKeys.allBySlug });
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
