import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { queryKeys } from './queryKeys';

/** A parcel's data collections and usage (read-only, for the details panel). */
export function useDataSummary(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.data(id),
    queryFn: () => api.dataSummary(id),
    enabled,
    staleTime: 10_000,
  });
}
