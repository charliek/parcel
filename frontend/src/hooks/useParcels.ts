import {
  keepPreviousData,
  type QueryClient,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import { api, type ListFilters } from '@/lib/api';
import type { UpdateParcelRequest, ViewGrant } from '@/lib/types';
import { useAuthStore } from '@/stores/auth.store';
import { queryKeys } from './queryKeys';

export const PAGE_SIZE = 48;

/** A filtered list of parcels (mine, shared or both), paged with "load more". */
export function useParcelList(filters: ListFilters) {
  return useInfiniteQuery({
    queryKey: queryKeys.parcels(filters),
    queryFn: ({ pageParam }) =>
      api.listParcels({ ...filters, limit: PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    // Keep the old results on screen while a new filter loads.
    placeholderData: keepPreviousData,
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((n, page) => n + page.parcels.length, 0);
      return loaded < last.total && last.parcels.length > 0 ? loaded : undefined;
    },
  });
}

export function useParcelDetail(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.parcel(id),
    queryFn: () => api.getParcel(id),
    enabled,
  });
}

export function useVersionDetail(id: string, version: number | null, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.version(id, version ?? 0),
    queryFn: () => api.getVersion(id, version ?? 'latest'),
    enabled: enabled && version !== null,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * Refresh everything that shows a parcel after a publish or restore. View
 * grants are refetched too, because the current version changed.
 */
export async function invalidateParcel(client: QueryClient, id: string) {
  await Promise.all([
    client.invalidateQueries({ queryKey: queryKeys.parcel(id) }),
    client.invalidateQueries({ queryKey: queryKeys.allViews(id) }),
    client.invalidateQueries({ queryKey: queryKeys.allParcels }),
    client.invalidateQueries({ queryKey: queryKeys.allTags }),
    // A publish can claim a slug another parcel's cached resolution points at.
    refreshSlugResolutions(client),
  ]);
  void useAuthStore.getState().reload();
}

/**
 * After anything that can move a name (publish, slug or handle change, tag
 * edits that refresh summaries): resolutions nobody is showing are dropped,
 * not just marked stale, because the slug viewer routes on cached data while
 * a refetch runs and would follow a name to the parcel that used to hold it.
 * The one on screen (if any) refetches.
 */
export function refreshSlugResolutions(client: QueryClient) {
  client.removeQueries({ queryKey: queryKeys.allBySlug, type: 'inactive' });
  return client.invalidateQueries({ queryKey: queryKeys.allBySlug });
}

/** Drop every cached query for a deleted parcel (call after leaving its page). */
export function forgetParcel(client: QueryClient, id: string) {
  client.removeQueries({ queryKey: queryKeys.parcel(id) });
  client.removeQueries({ queryKey: queryKeys.allViews(id) });
  client.removeQueries({ queryKey: ['version', id] });
  client.removeQueries({ queryKey: queryKeys.activity(id) });
  client.removeQueries({ queryKey: queryKeys.comments(id) });
  client.removeQueries({ queryKey: queryKeys.data(id) });
  client.removeQueries({ queryKey: queryKeys.shares(id) });
  // The deleted parcel's names must resolve afresh: its slug can be taken by
  // the next publish within the cache's lifetime. Every entry goes (cheap, and
  // no key says which names pointed at this parcel).
  client.removeQueries({ queryKey: queryKeys.allBySlug });
}

export function useUpdateParcel(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: UpdateParcelRequest) => api.updateParcel(id, body),
    onSuccess: (summary) => {
      // Patch cached grants in place: refetching one would mint a new content
      // URL and reload the iframe under the user.
      client.setQueriesData<ViewGrant>({ queryKey: queryKeys.allViews(id) }, (grant) =>
        grant ? { ...grant, parcel: summary } : grant,
      );
      void client.invalidateQueries({ queryKey: queryKeys.parcel(id) });
      void client.invalidateQueries({ queryKey: queryKeys.allParcels });
      void client.invalidateQueries({ queryKey: queryKeys.allTags });
      void refreshSlugResolutions(client);
    },
  });
}

export function useDeleteParcel(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.deleteParcel(id),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: queryKeys.allParcels });
      void client.invalidateQueries({ queryKey: queryKeys.allTags });
      void useAuthStore.getState().reload();
    },
  });
}

export function useRestoreVersion(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (version: number) => api.restoreVersion(id, version),
    onSuccess: () => invalidateParcel(client, id),
  });
}
