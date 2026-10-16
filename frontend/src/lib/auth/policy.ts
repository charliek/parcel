/**
 * Pure decisions behind optional sign-in, kept free of DOM and stores so they
 * are unit-tested.
 *
 * Parcel differs from spendwise in that not every page needs an account:
 * public parcels must render for anonymous visitors, and only a failed view
 * (or an account-only page) should send someone through slauth.
 */

/** Where the session stands, as resolved from `GET /api/v1/me`. */
export type SessionStatus = 'idle' | 'loading' | 'authenticated' | 'anonymous' | 'error';

/** Whether the SPA may start an OAuth flow at all (never in `dev` mode). */
export function canStartOAuth(mode: string | null | undefined): boolean {
  return mode === 'slauth';
}

/** Whether the session is still being resolved. */
export function isResolving(status: SessionStatus): boolean {
  return status === 'idle' || status === 'loading';
}

export interface SilentReauthInput {
  mode: string | null | undefined;
  /** The once-per-browser-session guard is already spent. */
  alreadyTried: boolean;
  /** We are on /auth/callback (never redirect from there). */
  onCallbackRoute: boolean;
  /**
   * `required`: an account-only page (dashboard actions, /new, /tokens, /cli,
   * /device) — always worth one silent attempt.
   * `optional`: `/` — only try when this browser signed in before, so
   * first-time visitors see the landing page without a redirect bounce.
   */
  need: 'required' | 'optional';
  /** This browser has completed a sign-in before (localStorage hint). */
  signedInHint: boolean;
}

/** Whether to bounce through slauth with `prompt=none` right now. */
export function shouldAttemptSilentReauth(input: SilentReauthInput): boolean {
  if (!canStartOAuth(input.mode)) return false;
  if (input.alreadyTried || input.onCallbackRoute) return false;
  if (input.need === 'optional' && !input.signedInHint) return false;
  return true;
}

/** What an account-only page should render. */
export type RequiredAccess = 'loading' | 'ready' | 'silent-reauth' | 'sign-in' | 'error';

export function requiredPageAccess(input: {
  session: SessionStatus;
  mode: string | null | undefined;
  silentTried: boolean;
}): RequiredAccess {
  if (isResolving(input.session)) return 'loading';
  if (input.session === 'authenticated') return 'ready';
  if (input.session === 'error') return 'error';
  return shouldAttemptSilentReauth({
    mode: input.mode,
    alreadyTried: input.silentTried,
    onCallbackRoute: false,
    need: 'required',
    signedInHint: false,
  })
    ? 'silent-reauth'
    : 'sign-in';
}

/** What the viewer (`/p/:id`) should render. */
export type ViewerAccess =
  | 'loading'
  | 'ready'
  | 'silent-reauth'
  | 'sign-in'
  | 'not-found'
  | 'error';

export function viewerAccess(input: {
  /** State of `GET /api/v1/parcels/{id}/view`. */
  view: 'pending' | 'success' | 'error';
  /** HTTP status when the view failed (undefined for network errors). */
  errorStatus?: number;
  session: SessionStatus;
  mode: string | null | undefined;
  silentTried: boolean;
}): ViewerAccess {
  if (input.view === 'success') return 'ready';
  if (input.view === 'pending') return 'loading';
  const denied =
    input.errorStatus === 401 || input.errorStatus === 403 || input.errorStatus === 404;
  if (!denied) return 'error';
  // Wait until we know whether the visitor is signed in.
  if (isResolving(input.session)) return 'loading';
  // Signed in (or dev mode, where everyone is): it really is gone or not ours.
  if (input.session === 'authenticated' || !canStartOAuth(input.mode)) return 'not-found';
  return input.silentTried ? 'sign-in' : 'silent-reauth';
}
