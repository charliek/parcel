/**
 * `GET /api/v1/info`, fetched once at boot.
 *
 * Auth settings (slauth issuer, client id, audience, or dev mode) and upload
 * limits come from the server at runtime instead of VITE_* build variables,
 * so one built bundle works in every environment.
 */

import { create } from 'zustand';
import { parseResponse } from '@/lib/http';
import type { AuthInfo, ServerInfo } from '@/lib/types';

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

/** The loaded server info. Throws if called before the boot gate resolved it. */
export function requireServerInfo(): ServerInfo {
  const info = useServerInfoStore.getState().info;
  if (!info) throw new Error('Server settings are not loaded yet');
  return info;
}

/** The auth block with a normalized (no trailing slash) issuer. */
export function authConfig(): AuthInfo {
  const { auth } = requireServerInfo();
  return { ...auth, issuer: auth.issuer.replace(/\/+$/, '') };
}

/** The current auth mode, or null before server info loads. */
export function currentAuthMode(): string | null {
  return useServerInfoStore.getState().info?.auth.mode ?? null;
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
