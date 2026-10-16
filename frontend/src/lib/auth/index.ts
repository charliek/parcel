/**
 * Authentication boundary: sign-in, sign-out and the signed-in hint.
 *
 * In dev mode (`ServerInfo.auth.mode === 'dev'`) the backend authenticates every
 * request as the dev user, so `/api/v1/me` succeeds with no token, sign-in is
 * never started and sign-out is hidden. Under slauth, unauthenticated calls
 * return 401 and {@link beginLogin} starts the OAuth2 + PKCE flow.
 */

import { queryClient } from '@/lib/query-client';
import { authConfig } from '@/stores/server-info.store';
import { markSilentReauthTried, oauthEnabled, startAuthorize } from '../oauth';
import { currentReturnTo } from '../return-to';
import { readStorage, removeStorage, STORAGE_KEYS, writeStorage } from '../storage';
import { useTokenStore } from '../token-store';

/** Start the slauth Authorization Code + PKCE flow, returning to `returnTo` afterward. */
export function beginLogin(returnTo: string = currentReturnTo()): Promise<void> {
  if (!oauthEnabled()) return Promise.resolve();
  return startAuthorize(returnTo);
}

/** Remember that this browser has signed in (lets `/` try a silent re-auth). */
export function rememberSignedIn(): void {
  writeStorage('local', STORAGE_KEYS.signedInHint, '1');
}

export function hasSignedInHint(): boolean {
  return readStorage('local', STORAGE_KEYS.signedInHint) !== null;
}

/**
 * Revoke, clear, then OIDC RP-initiated logout (global SSO logout via slauth).
 * A no-op in dev mode, where there is no session to end.
 */
export async function logout(): Promise<void> {
  if (!oauthEnabled()) return;
  const { issuer, client_id } = authConfig();
  const { refreshToken, idToken } = useTokenStore.getState();
  if (refreshToken) {
    try {
      await fetch(`${issuer}/oauth2/revoke`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ token: refreshToken, client_id }),
      });
    } catch {
      // Best-effort: proceed with the logout redirect even if this fails.
    }
  }
  useTokenStore.getState().clearTokens();
  removeStorage('local', STORAGE_KEYS.signedInHint);
  // Coming back to `/` must not immediately sign in again.
  markSilentReauthTried();
  queryClient.clear();

  // post_logout_redirect_uri must be registered for this client (origin root);
  // id_token_hint lets Hydra validate it and skip a prompt.
  const params = new URLSearchParams({ post_logout_redirect_uri: `${window.location.origin}/` });
  if (idToken) params.set('id_token_hint', idToken);
  window.location.assign(`${issuer}/oauth2/sessions/logout?${params.toString()}`);
}
