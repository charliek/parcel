import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { removeShare, upsertShare } from '@/lib/shares';
import type { ShareList, ShareRole } from '@/lib/types';
import { queryKeys } from './queryKeys';
import { PAGE_SIZE } from './useParcels';

/** The people a parcel is shared with (owner only). */
export function useShares(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.shares(id),
    queryFn: () => api.listShares(id),
    enabled,
  });
}

/** Share with someone, or change their role; the list updates from the answer. */
export function usePutShare(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ email, role }: { email: string; role: ShareRole }) =>
      api.putShare(id, email, role),
    onSuccess: (share) =>
      client.setQueryData<ShareList>(queryKeys.shares(id), (list) =>
        list ? upsertShare(list, share) : { shares: [share] },
      ),
  });
}

export function useDeleteShare(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (email: string) => api.deleteShare(id, email),
    onSuccess: (_result, email) =>
      client.setQueryData<ShareList>(queryKeys.shares(id), (list) =>
        list ? removeShare(list, email) : list,
      ),
  });
}

/** Parcels other people shared with the caller by email, paged with "load more". */
export function useSharedWithMe() {
  return useInfiniteQuery({
    queryKey: queryKeys.shared,
    queryFn: ({ pageParam }) => api.listShared({ limit: PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((n, page) => n + page.parcels.length, 0);
      return loaded < last.total && last.parcels.length > 0 ? loaded : undefined;
    },
  });
}
