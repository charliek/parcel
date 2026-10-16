/**
 * OAuth2 Authorization Code + PKCE flow against slauth (Ory Hydra).
 *
 * Settings come from `GET /api/v1/info` at runtime (see server-info.store):
 * - authorize: `${issuer}/oauth2/auth` with `audience` = `auth.audience`
 * - token:     `${issuer}/oauth2/token` (public client + PKCE, no secret)
 *
 * startAuthorize() generates PKCE + state, stashes the transaction in
 * sessionStorage, and redirects to slauth. The /auth/callback route validates
 * state and calls exchangeCode(). refreshTokens() rotates the refresh token.
 *
 * Nothing here runs in `dev` auth mode: the backend authenticates every
 * request as the dev user, so the SPA never starts an OAuth flow.
 */

import { authConfig } from '@/stores/server-info.store';
import { challengeFromVerifier, generateState, generateVerifier } from './pkce';
import { safeReturnTo } from './return-to';
import { readStorage, removeStorage, STORAGE_KEYS, writeStorage } from './storage';

const SCOPE = 'openid offline_access email profile';

export interface OAuthTokens {
  access_token: string;
  refresh_token?: string;
  id_token?: string;
  expires_in?: number;
  token_type?: string;
}

export interface PkceTx {
  verifier: string;
  state: string;
  returnTo: string;
  /** Whether this was a silent prompt=none attempt (its failures are quiet). */
  silent?: boolean;
}

/** Token-endpoint error; `status` is set for HTTP responses, undefined for
 * network failures, so callers can distinguish definitive (4xx) from transient. */
export class OAuthError extends Error {
  status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.name = 'OAuthError';
    this.status = status;
  }
}

/** The OAuth redirect URI for this app (must match a registered redirect_uri). */
export function redirectUri(): string {
  return `${window.location.origin}/auth/callback`;
}

/** Whether interactive sign-in is possible (slauth mode with server info loaded). */
export function oauthEnabled(): boolean {
  try {
    return authConfig().mode === 'slauth';
  } catch {
    return false;
  }
}

/** Begin the Authorization Code + PKCE flow by redirecting to slauth. Pass
 * `prompt: 'none'` for a silent attempt that never shows UI. */
export async function startAuthorize(returnTo: string, opts?: { prompt?: 'none' }): Promise<void> {
  const cfg = authConfig();
  if (cfg.mode !== 'slauth') {
    throw new OAuthError('Sign-in is not used: the server is running in dev auth mode.');
  }
  const verifier = generateVerifier();
  const challenge = await challengeFromVerifier(verifier);
  const state = generateState();

  const tx: PkceTx = {
    verifier,
    state,
    returnTo: safeReturnTo(returnTo),
    silent: opts?.prompt === 'none',
  };
  writeStorage('session', STORAGE_KEYS.pkceTx, JSON.stringify(tx));
  if (readStorage('session', STORAGE_KEYS.pkceTx) === null) {
    // Without sessionStorage the callback could not validate state.
    throw new OAuthError('Sign-in needs browser storage, which is blocked in this window.');
  }

  const params = new URLSearchParams({
    response_type: 'code',
    client_id: cfg.client_id,
    redirect_uri: redirectUri(),
    scope: SCOPE,
    state,
    code_challenge: challenge,
    code_challenge_method: 'S256',
    audience: cfg.audience,
  });
  if (opts?.prompt) params.set('prompt', opts.prompt);
  window.location.assign(`${cfg.issuer}/oauth2/auth?${params.toString()}`);
}

/** Whether the once-per-session silent re-auth was already attempted. */
export function silentReauthTried(): boolean {
  return readStorage('session', STORAGE_KEYS.silentReauth) !== null;
}

/** Mark the silent attempt as used without trying (e.g. after a cross-tab logout). */
export function markSilentReauthTried(): void {
  writeStorage('session', STORAGE_KEYS.silentReauth, '1');
}

/**
 * Attempt a silent (prompt=none) re-auth against the live slauth session.
 *
 * In-memory tokens don't survive a reload, but the slauth session cookie does,
 * so we redirect through `/oauth2/auth?prompt=none`, which returns a code when
 * a session exists or `login_required` when it doesn't. Guarded to run at most
 * once per browser session to avoid a redirect loop.
 *
 * Returns the in-flight redirect promise, or null if no attempt was made.
 */
export function maybeStartSilentReauth(returnTo: string): Promise<void> | null {
  if (!oauthEnabled()) return null;
  if (window.location.pathname === '/auth/callback') return null;
  if (silentReauthTried()) return null;
  markSilentReauthTried();
  return startAuthorize(returnTo, { prompt: 'none' }).catch((err) => {
    clearSilentReauthGuard();
    throw err;
  });
}

/** Clear the silent-reauth guard — call once tokens are obtained. */
export function clearSilentReauthGuard(): void {
  removeStorage('session', STORAGE_KEYS.silentReauth);
}

/** Read and clear the pending PKCE transaction (single-use). */
export function takePkceTx(): PkceTx | null {
  const raw = readStorage('session', STORAGE_KEYS.pkceTx);
  removeStorage('session', STORAGE_KEYS.pkceTx);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as PkceTx;
  } catch {
    return null;
  }
}

async function postToken(body: URLSearchParams): Promise<OAuthTokens> {
  const { issuer } = authConfig();
  let response: Response;
  try {
    response = await fetch(`${issuer}/oauth2/token`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body,
    });
  } catch (e) {
    throw new OAuthError(`token endpoint unreachable: ${e}`);
  }
  if (!response.ok) {
    throw new OAuthError(`token endpoint returned ${response.status}`, response.status);
  }
  return response.json();
}

/** Exchange an authorization code (+ PKCE verifier) for tokens. */
export function exchangeCode(code: string, verifier: string): Promise<OAuthTokens> {
  return postToken(
    new URLSearchParams({
      grant_type: 'authorization_code',
      code,
      redirect_uri: redirectUri(),
      client_id: authConfig().client_id,
      code_verifier: verifier,
    }),
  );
}

/** Rotate tokens using a refresh token. */
export function refreshTokens(refreshToken: string): Promise<OAuthTokens> {
  return postToken(
    new URLSearchParams({
      grant_type: 'refresh_token',
      refresh_token: refreshToken,
      client_id: authConfig().client_id,
    }),
  );
}
