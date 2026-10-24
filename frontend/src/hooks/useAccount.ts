import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import type { ParcelDetail, ViewGrant } from '@/lib/types';
import { useAuthStore } from '@/stores/auth.store';
import { queryKeys } from './queryKeys';

/** Change the signed-in user's handle; every clean URL on screen changes with it. */
export function useUpdateHandle() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (handle: string) => api.updateMe({ handle }),
    onSuccess: (me) => {
      useAuthStore.setState({ me });
      void client.invalidateQueries({ queryKey: queryKeys.allParcels });
      void client.invalidateQueries({ queryKey: queryKeys.allBySlug });
      // Grants carry the summary (and its canonical URL), but refetching an
      // active one would reload the frame; mark them stale for next time...
      void client.invalidateQueries({ queryKey: queryKeys.everyView, refetchType: 'none' });
      // ...and patch the summaries in from the details the viewer on screen
      // refetches, so the link it shows and copies follows now.
      void client.invalidateQueries({ queryKey: queryKeys.allParcelDetails }).then(() => {
        for (const [key, grant] of client.getQueriesData<ViewGrant>({
          queryKey: queryKeys.everyView,
        })) {
          if (!grant) continue;
          const detailKey = queryKeys.parcel(grant.parcel.id);
          const state = client.getQueryState<ParcelDetail>(detailKey);
          // Only a detail fetched since the change (an inactive one is still the old).
          if (!state?.data || state.isInvalidated || state.status !== 'success') continue;
          client.setQueryData<ViewGrant>(key, { ...grant, parcel: state.data.parcel });
        }
      });
    },
  });
}
