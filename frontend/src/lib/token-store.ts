/**
 * In-memory store for slauth OAuth2 access/refresh tokens.
 *
 * Tokens live in memory only (never localStorage); a full reload re-authorizes
 * silently against the live slauth session. In dev mode no tokens are ever set,
 * so `getValidToken()` returns null and the API sends no header.
 */

import { create } from 'zustand';
import { OAuthError, type OAuthTokens, refreshTokens } from './oauth';
import { STORAGE_KEYS, writeStorage } from './storage';

interface TokenState {
  accessToken: string | null;
  refreshToken: string | null;
  /** Kept for RP-initiated logout (id_token_hint). */
  idToken: string | null;
  expiresAt: Date | null;

  setTokens: (tokens: OAuthTokens) => void;
  refresh: () => Promise<string | null>;
  getValidToken: () => Promise<string | null>;
  /** Clear tokens here and tell other tabs to do the same. */
  clearTokens: () => void;
}

// Refresh 2 minutes before expiry.
const REFRESH_BUFFER_MS = 2 * 60 * 1000;

// Dedupe concurrent refreshes (avoids tripping Hydra's rotation reuse detection).
let refreshPromise: Promise<string | null> | null = null;
// Bumped on logout / cross-tab clear so a stale in-flight refresh is discarded.
let authGeneration = 0;

const EMPTY = { accessToken: null, refreshToken: null, idToken: null, expiresAt: null };

function computeExpiresAt(tokens: OAuthTokens): Date {
  if (typeof tokens.expires_in === 'number') {
    return new Date(Date.now() + tokens.expires_in * 1000);
  }
  try {
    const payload = JSON.parse(atob(tokens.access_token.split('.')[1]));
    if (typeof payload.exp === 'number') {
      return new Date(payload.exp * 1000);
    }
  } catch {
    // malformed token; fall through to a conservative default
  }
  return new Date(Date.now() + 15 * 60 * 1000);
}

export const useTokenStore = create<TokenState>((set, get) => ({
  ...EMPTY,

  setTokens: (tokens) => {
    set({
      accessToken: tokens.access_token,
      refreshToken: tokens.refresh_token ?? get().refreshToken,
      idToken: tokens.id_token ?? get().idToken,
      expiresAt: computeExpiresAt(tokens),
    });
  },

  refresh: async () => {
    if (refreshPromise) return refreshPromise;
    const { refreshToken } = get();
    if (!refreshToken) return null;

    const generation = authGeneration;
    refreshPromise = (async () => {
      try {
        const tokens = await refreshTokens(refreshToken);
        if (authGeneration !== generation) return null;
        set({
          accessToken: tokens.access_token,
          refreshToken: tokens.refresh_token ?? refreshToken,
          idToken: tokens.id_token ?? get().idToken,
          expiresAt: computeExpiresAt(tokens),
        });
        return tokens.access_token;
      } catch (error) {
        // Only clear on a definitive 4xx (refresh token invalid/expired);
        // transient errors keep the session.
        const status = error instanceof OAuthError ? error.status : undefined;
        const definitive = status !== undefined && status >= 400 && status < 500;
        if (definitive && authGeneration === generation) {
          set(EMPTY);
        }
        return null;
      } finally {
        refreshPromise = null;
      }
    })();
    return refreshPromise;
  },

  getValidToken: async () => {
    const { accessToken, expiresAt } = get();
    if (accessToken && expiresAt) {
      const msUntilExpiry = expiresAt.getTime() - Date.now();
      if (msUntilExpiry >= REFRESH_BUFFER_MS) {
        // Close to the buffer: refresh in the background, use the current token now.
        if (msUntilExpiry < REFRESH_BUFFER_MS * 2) void get().refresh();
        return accessToken;
      }
      return get().refresh();
    }
    return get().refresh();
  },

  clearTokens: () => {
    authGeneration++;
    set(EMPTY);
    writeStorage('local', STORAGE_KEYS.authSync, Date.now().toString());
  },
}));

/** Whether this tab holds slauth tokens (i.e. a real, sign-out-able session). */
export function hasTokens(): boolean {
  const { accessToken, refreshToken } = useTokenStore.getState();
  return Boolean(accessToken || refreshToken);
}

// Multi-tab synchronization: clear tokens when another tab logs out.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (event) => {
    if (event.key === STORAGE_KEYS.authSync && event.newValue) {
      authGeneration++;
      useTokenStore.setState(EMPTY);
    }
  });
}
