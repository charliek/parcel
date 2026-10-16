/**
 * The signed-in user, resolved lazily from `GET /api/v1/me`.
 *
 * Unlike spendwise, the app does not block on this at boot: the viewer can
 * render a public parcel for an anonymous visitor. Pages call `ensure()` (via
 * `useSession`) and decide what an `anonymous` session means for them.
 */

import { create } from 'zustand';
import { api, setUnauthorizedHandler } from '@/lib/api';
import type { SessionStatus } from '@/lib/auth/policy';
import { ApiError } from '@/lib/http';
import { markSilentReauthTried } from '@/lib/oauth';
import { queryClient } from '@/lib/query-client';
import { STORAGE_KEYS } from '@/lib/storage';
import type { Me } from '@/lib/types';

interface AuthState {
  status: SessionStatus;
  me: Me | null;
  error: string | null;
  /** Resolve the session once (deduped); later calls are no-ops. */
  ensure: () => Promise<void>;
  /** Re-fetch `/me` (e.g. to refresh usage after a publish). */
  reload: () => Promise<void>;
  setAnonymous: () => void;
}

let inflight: Promise<void> | null = null;

export const useAuthStore = create<AuthState>((set, get) => {
  const resolve = (): Promise<void> => {
    if (inflight) return inflight;
    if (get().status !== 'authenticated') set({ status: 'loading', error: null });
    inflight = (async () => {
      try {
        const me = await api.me();
        set({ status: 'authenticated', me, error: null });
      } catch (err) {
        if (err instanceof ApiError && (err.status === 401 || err.status === 403)) {
          set({ status: 'anonymous', me: null, error: null });
        } else {
          set({
            status: 'error',
            me: null,
            error: err instanceof Error ? err.message : 'Failed to load your account',
          });
        }
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  };

  return {
    status: 'idle',
    me: null,
    error: null,
    ensure: () => (get().status === 'idle' ? resolve() : (inflight ?? Promise.resolve())),
    reload: resolve,
    setAnonymous: () => set({ status: 'anonymous', me: null, error: null }),
  };
});

// A 401 from any call while signed in means the session is gone (refresh token
// expired or revoked): drop to anonymous so account pages show sign-in.
setUnauthorizedHandler(() => {
  if (useAuthStore.getState().status === 'authenticated') {
    useAuthStore.getState().setAnonymous();
  }
});

// Another tab signed out: forget the user here too, and don't immediately
// bounce through slauth to sign back in.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    if (event.key === STORAGE_KEYS.authSync && event.newValue) {
      markSilentReauthTried();
      queryClient.clear();
      if (useAuthStore.getState().status === 'authenticated') {
        useAuthStore.getState().setAnonymous();
      }
    }
  });
}
