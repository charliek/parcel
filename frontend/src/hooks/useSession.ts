import { useEffect } from 'react';
import { useAuthStore } from '@/stores/auth.store';

/** Resolve the session (once, shared) and return its current state. */
export function useSession() {
  const status = useAuthStore((s) => s.status);
  const me = useAuthStore((s) => s.me);
  const error = useAuthStore((s) => s.error);
  const ensure = useAuthStore((s) => s.ensure);

  useEffect(() => {
    void ensure();
  }, [ensure]);

  return { status, me, error, signedIn: status === 'authenticated' };
}

/** The signed-in user (null when anonymous or still resolving). */
export function useMe() {
  return useAuthStore((s) => s.me);
}
