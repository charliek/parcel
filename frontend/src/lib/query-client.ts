import { QueryClient } from '@tanstack/react-query';
import { ApiError } from './http';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Retry transient failures once, never auth/not-found answers.
      retry: (failureCount, error) =>
        !(error instanceof ApiError && error.status >= 400 && error.status < 500) &&
        failureCount < 1,
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
});
