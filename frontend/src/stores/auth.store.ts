/**
 * The signed-in user, resolved lazily from `GET /api/v1/me`.
 *
 * Unlike spendwise, the app does not block on this at boot: the viewer can
 * render a public parcel for an anonymous visitor. Pages call `ensure()` (via
 * `useSession`) and decide what an `anonymous` session means for them.
 *
 * The session itself is the server's HttpOnly cookie, which this tab can't
 * see; `/me` answering is how it learns who it is.
 */

import { create } from 'zustand';
import { api, setUnauthorizedHandler } from '@/lib/api';
import { clearAutoSignIn, rememberSignedIn } from '@/lib/auth';
import { ApiError } from '@/lib/http';
import { queryClient } from '@/lib/query-client';
import type { SessionStatus } from '@/lib/sign-in';
import { STORAGE_KEYS } from '@/lib/storage';
import type { Me } from '@/lib/types';
import { useServerInfoStore } from './server-info.store';

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
        if (useServerInfoStore.getState().info?.auth.mode !== 'dev') {
          rememberSignedIn();
          // The sign-in worked, so a later expiry may go round again.
          clearAutoSignIn();
        }
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

/**
 * The session ended under a signed-in tab (it expired, was signed out
 * everywhere, or another tab signed out). Forget the user, and refetch
 * what's on screen without them: everything cached was fetched as them, and
 * a public parcel should go on showing, signed out.
 */
function sessionEnded(): void {
  if (useAuthStore.getState().status !== 'authenticated') return;
  useAuthStore.getState().setAnonymous();
  void queryClient.resetQueries();
}

// A 401 from any call means the server no longer accepts the session (and
// has cleared its cookie). While still resolving, `/me` decides instead.
setUnauthorizedHandler(sessionEnded);

// Another tab signed out (the cookie is shared, so it's gone here too).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    if (event.key === STORAGE_KEYS.authSync && event.newValue) sessionEnded();
  });
}
