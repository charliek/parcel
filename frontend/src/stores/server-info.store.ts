/**
 * `GET /api/v1/info`, fetched once at boot.
 *
 * Auth settings (the sign-in providers, or dev mode), whether public links
 * are allowed, and upload limits come from the server at runtime instead of
 * VITE_* build variables, so one built bundle works in every environment.
 */

import { create } from 'zustand';
import { parseResponse } from '@/lib/http';
import type { AuthProvider, ServerInfo } from '@/lib/types';

type InfoStatus = 'idle' | 'loading' | 'ready' | 'error';

interface ServerInfoState {
  status: InfoStatus;
  info: ServerInfo | null;
  error: string | null;
  load: () => Promise<void>;
}

let inflight: Promise<void> | null = null;

export const useServerInfoStore = create<ServerInfoState>((set, get) => ({
  status: 'idle',
  info: null,
  error: null,

  load: () => {
    if (get().status === 'ready') return Promise.resolve();
    if (inflight) return inflight;
    set({ status: 'loading', error: null });
    inflight = (async () => {
      try {
        const response = await fetch('/api/v1/info', { headers: { Accept: 'application/json' } });
        const info = await parseResponse<ServerInfo>(response);
        set({ status: 'ready', info, error: null });
      } catch (err) {
        const message =
          err instanceof TypeError
            ? 'The Parcel server could not be reached.'
            : err instanceof Error
              ? err.message
              : 'Failed to load server settings.';
        set({ status: 'error', error: message });
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  },
}));

/** Hook: the app's origin, for URLs the app spells out itself (this page's until info loads). */
export function useAppOrigin(): string {
  return useServerInfoStore((s) => s.info?.app_origin) ?? window.location.origin;
}

/** Hook: the loaded server info (non-null inside the boot gate). */
export function useServerInfo(): ServerInfo {
  const info = useServerInfoStore((s) => s.info);
  if (!info) throw new Error('useServerInfo used outside the boot gate');
  return info;
}

/** Hook: whether the backend authenticates everyone as a dev user. */
export function useIsDevAuth(): boolean {
  return useServerInfoStore((s) => s.info?.auth.mode === 'dev');
}

const NO_PROVIDERS: AuthProvider[] = [];

/** Hook: the sign-in providers to offer (none in dev mode). */
export function useAuthProviders(): AuthProvider[] {
  return useServerInfoStore((s) =>
    s.info?.auth.mode === 'dev' ? NO_PROVIDERS : (s.info?.auth.providers ?? NO_PROVIDERS),
  );
}

/** Hook: whether parcels may be made public on this server. */
export function useAllowPublic(): boolean {
  return useServerInfoStore((s) => s.info?.allow_public ?? false);
}
