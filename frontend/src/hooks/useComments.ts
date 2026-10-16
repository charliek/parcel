import { type QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { removeComment, upsertThread } from '@/lib/comments';
import type { Comment, CommentList, NewComment, UpdateComment } from '@/lib/types';
import { queryKeys } from './queryKeys';

/** Every thread on a parcel, open and resolved; the panel filters client-side. */
export function useComments(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.comments(id),
    queryFn: () => api.listComments(id, 'all'),
    enabled,
    // The activity poll says when comments change; refetch then, not on a timer.
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Apply a mutation's result to the cached list now; the poll catches up after. */
function settle(client: QueryClient, id: string, update: (list: CommentList) => CommentList) {
  client.setQueryData<CommentList>(queryKeys.comments(id), (list) => (list ? update(list) : list));
  void client.invalidateQueries({ queryKey: queryKeys.activity(id) });
  // The dashboard shows open counts; it refetches when next shown.
  void client.invalidateQueries({ queryKey: queryKeys.allParcels, refetchType: 'none' });
}

export function useCreateComment(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: NewComment) => api.createComment(id, body),
    onSuccess: (thread: Comment) => settle(client, id, (list) => upsertThread(list, thread)),
  });
}

export function useUpdateComment(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ number, body }: { number: number; body: UpdateComment }) =>
      api.updateComment(id, number, body),
    onSuccess: (thread: Comment) => settle(client, id, (list) => upsertThread(list, thread)),
  });
}

export function useDeleteComment(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (number: number) => api.deleteComment(id, number),
    onSuccess: (_result, number) => settle(client, id, (list) => removeComment(list, number)),
  });
}
