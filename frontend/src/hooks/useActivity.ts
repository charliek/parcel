import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { errorStatus } from '@/lib/http';
import { ACTIVITY_POLL_MS } from '@/lib/live-update';
import { queryKeys } from './queryKeys';

/**
 * The viewer's poll for new versions and comment changes. Runs every few
 * seconds while the tab is visible, pauses while it's hidden, and refetches
 * as soon as it's visible again (React Query's focus manager listens for
 * `visibilitychange`).
 */
export function useActivity(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.activity(id),
    queryFn: () => api.activity(id),
    enabled,
    staleTime: 0,
    // Stop once the parcel is gone or no longer shared with us; the next
    // navigation shows the not-found page.
    refetchInterval: (query) => (errorStatus(query.state.error) === 404 ? false : ACTIVITY_POLL_MS),
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    retry: false,
  });
}
