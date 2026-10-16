import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import type { CreateTokenRequest } from '@/lib/types';
import { queryKeys } from './queryKeys';

export function useTokens() {
  return useQuery({ queryKey: queryKeys.tokens, queryFn: api.listTokens });
}

export function useCreateToken() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateTokenRequest) => api.createToken(body),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.tokens }),
  });
}

export function useRevokeToken() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.revokeToken(id),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.tokens }),
  });
}
