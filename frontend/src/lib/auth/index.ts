/**
 * Authentication boundary: starting sign-in, signing out, and the few
 * browser-side hints that make them smooth.
 *
 * Sign-in belongs to the server (`/auth/login` → provider → `/auth/callback`
 * → back to `return_to`, with an HttpOnly session cookie), so starting it is
 * a full-page navigation. In dev mode (`ServerInfo.auth.mode === 'dev'`)
 * every request is the dev user: there is nothing to start, and the UI hides
 * sign-in and sign-out.
 */

import { parseResponse } from '../http';
import { currentReturnTo } from '../return-to';
import { loginHref, parseAttempt, safeLogoutRedirect } from '../sign-in';
import { readStorage, removeStorage, STORAGE_KEYS, writeStorage } from '../storage';

/**
 * Go to the provider (a full-page navigation); the server signs the user in
 * and lands back on `returnTo`. `providerId` may be omitted when the server
 * has only one.
 */
export function startSignIn(providerId?: string, returnTo: string = currentReturnTo()): void {
  if (providerId) rememberProvider(providerId);
  window.location.assign(loginHref(returnTo, providerId));
}

/** Remember the provider picked, so the sign-in page offers it first next time. */
export function rememberProvider(id: string): void {
  writeStorage('local', STORAGE_KEYS.lastProvider, id);
}

export function lastProvider(): string | null {
  return readStorage('local', STORAGE_KEYS.lastProvider);
}

/** Remember that this browser has signed in (lets account pages go straight to the provider). */
export function rememberSignedIn(): void {
  writeStorage('local', STORAGE_KEYS.signedInHint, '1');
}

export function hasSignedInHint(): boolean {
  return readStorage('local', STORAGE_KEYS.signedInHint) !== null;
}

/**
 * Note that an account page is sending this tab to sign in by itself (the
 * loop guard in `shouldAutoSignIn`). Returns false when it can't be noted
 * (storage blocked): then it mustn't go, or nothing would stop a loop.
 */
export function recordAutoSignIn(now: number = Date.now()): boolean {
  writeStorage('session', STORAGE_KEYS.autoSignIn, String(now));
  return readStorage('session', STORAGE_KEYS.autoSignIn) === String(now);
}

export function lastAutoSignIn(): number | null {
  return parseAttempt(readStorage('session', STORAGE_KEYS.autoSignIn));
}

export function clearAutoSignIn(): void {
  removeStorage('session', STORAGE_KEYS.autoSignIn);
}

/**
 * Sign out: the server clears this session (and with `everywhere`, every
 * other session of the user too), then the browser goes where it says:
 * the provider's logout page, or `/`.
 */
export async function logout({ everywhere = false }: { everywhere?: boolean } = {}): Promise<void> {
  const response = await fetch(`/auth/logout${everywhere ? '?everywhere=true' : ''}`, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  const body = await parseResponse<{ redirect?: unknown } | undefined>(response);
  // Coming back to an account page must not sign straight in again.
  removeStorage('local', STORAGE_KEYS.signedInHint);
  // Other tabs share the cookie that was just cleared; tell them.
  writeStorage('local', STORAGE_KEYS.authSync, Date.now().toString());
  window.location.assign(safeLogoutRedirect(body?.redirect, window.location.origin));
}
