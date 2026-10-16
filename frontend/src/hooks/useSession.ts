import { useEffect, useMemo } from 'react';
import { type SignInTarget, signInTarget } from '@/lib/sign-in';
import { useAuthStore } from '@/stores/auth.store';
import { useAuthProviders, useServerInfoStore } from '@/stores/server-info.store';

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

/** Where "Sign in" goes here: nowhere (dev mode), the only provider, or the chooser. */
export function useSignInTarget(): SignInTarget {
  const mode = useServerInfoStore((s) => s.info?.auth.mode);
  const providers = useAuthProviders();
  return useMemo(() => signInTarget(mode, providers), [mode, providers]);
}
