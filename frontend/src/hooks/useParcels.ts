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
  ]);
  void useAuthStore.getState().reload();
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
      void client.invalidateQueries({ queryKey: queryKeys.allBySlug });
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
