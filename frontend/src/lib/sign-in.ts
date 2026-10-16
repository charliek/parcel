/**
 * Pure decisions behind server-side sign-in, kept free of DOM and stores so
 * they are unit-tested.
 *
 * The server is the OIDC client (PLAN.md, "Sign-in providers and sharing"):
 * the SPA only navigates to `/auth/login?provider=&return_to=`, the server
 * sets an HttpOnly session cookie and lands back on `return_to`, and a
 * failure lands on `/signin?error=<code>&return_to=`. The browser never
 * holds a provider token.
 */

import { safeReturnTo } from './return-to';
import type { AuthProvider } from './types';

/** Where the session stands, as resolved from `GET /api/v1/me`. */
export type SessionStatus = 'idle' | 'loading' | 'authenticated' | 'anonymous' | 'error';

/** Whether the session is still being resolved. */
export function isResolving(status: SessionStatus): boolean {
  return status === 'idle' || status === 'loading';
}

/** `/auth/login` for one provider (omitted: the server's only one). */
export function loginHref(returnTo: string, providerId?: string): string {
  const params = new URLSearchParams();
  if (providerId) params.set('provider', providerId);
  params.set('return_to', safeReturnTo(returnTo));
  return `/auth/login?${params.toString()}`;
}

/** The SPA's sign-in page, which offers every provider. */
export function signInPageHref(returnTo: string): string {
  return `/signin?${new URLSearchParams({ return_to: safeReturnTo(returnTo) }).toString()}`;
}

/** Where a "Sign in" button goes. */
export type SignInTarget =
  /** Nothing to sign in to: dev auth mode, or no provider configured. */
  | { kind: 'none' }
  /** Straight to the only provider. */
  | { kind: 'provider'; provider: AuthProvider }
  /** The `/signin` page, to pick one of several. */
  | { kind: 'choose' };

export function signInTarget(
  mode: string | null | undefined,
  providers: readonly AuthProvider[],
): SignInTarget {
  if (mode === 'dev' || providers.length === 0) return { kind: 'none' };
  if (providers.length === 1) return { kind: 'provider', provider: providers[0] };
  return { kind: 'choose' };
}

/** Providers in button order: the one used last first, then as configured. */
export function orderProviders(
  providers: readonly AuthProvider[],
  lastUsed: string | null,
): AuthProvider[] {
  const last = providers.find((p) => p.id === lastUsed);
  return last ? [last, ...providers.filter((p) => p !== last)] : [...providers];
}

/** What the `/signin` page says about a failed sign-in. */
export interface SignInErrorText {
  title: string;
  description: string;
}

/** Error text, given how to name the provider ("that provider" when unknown). */
const ERRORS: Record<string, (provider: string) => SignInErrorText> = {
  denied: () => ({
    title: 'Sign-in was cancelled',
    description: 'The sign-in was cancelled before it finished. Try again when you’re ready.',
  }),
  expired: () => ({
    title: 'Sign-in took too long',
    description:
      'That sign-in expired or was already used, so it couldn’t be completed. Please start again.',
  }),
  domain: (provider) => ({
    title: 'That account can’t sign in here',
    description: `Accounts with that email domain can’t sign in with ${provider}. Try another sign-in option, or a different account.`,
  }),
  unverified: (provider) => ({
    title: 'Your email couldn’t be confirmed',
    description: `${capitalize(provider)} didn’t confirm your email address, which Parcel needs to know who you are. Verify it there, or try another sign-in option.`,
  }),
  failed: () => ({
    title: 'Sign-in didn’t complete',
    description: 'Something went wrong while signing you in. Please try again.',
  }),
  unavailable: (provider) => ({
    title: 'The sign-in provider is unavailable',
    description: `Parcel couldn’t reach ${provider}. Try again in a moment.`,
  }),
  disabled: () => ({
    title: 'Your account is disabled',
    description:
      'This Parcel account has been turned off, so it can’t sign in. Ask whoever runs this Parcel to turn it back on.',
  }),
};

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * Friendly text for an `error` code from the server; null when there is
 * none. `providerLabel` names the provider it happened with, when known.
 */
export function signInError(
  code: string | null | undefined,
  providerLabel?: string | null,
): SignInErrorText | null {
  if (!code) return null;
  const text = Object.hasOwn(ERRORS, code) ? ERRORS[code] : ERRORS.failed;
  return text(providerLabel || 'that provider');
}

/**
 * After an account page sent this tab off to sign in by itself, it won't
 * do so again for this long. If the session cookie doesn't stick (a proxy
 * eating `Set-Cookie`, a blocked cookie), the user sees the sign-in screen
 * instead of bouncing between Parcel and the provider forever.
 */
export const AUTO_SIGN_IN_COOLDOWN_MS = 2 * 60_000;

/** Parse the stored time of the last automatic sign-in. */
export function parseAttempt(raw: string | null): number | null {
  if (!raw) return null;
  const at = Number(raw);
  return Number.isFinite(at) ? at : null;
}

/**
 * Whether an account-only page should go straight to the provider instead
 * of showing the sign-in screen. Only for a returning browser (it has
 * signed in before and didn't sign out), only with one provider to pick,
 * and never twice within the cooldown.
 */
export function shouldAutoSignIn(input: {
  target: SignInTarget;
  signedInHint: boolean;
  lastAttemptAt: number | null;
  now: number;
}): boolean {
  if (input.target.kind !== 'provider' || !input.signedInHint) return false;
  if (input.lastAttemptAt === null) return true;
  const since = input.now - input.lastAttemptAt;
  // A clock that went backwards counts as recent.
  return since < 0 ? false : since >= AUTO_SIGN_IN_COOLDOWN_MS;
}

/**
 * Where to go after `POST /auth/logout`: the provider's logout page or `/`.
 * Anything that isn't http(s) (or doesn't parse) becomes `/`.
 */
export function safeLogoutRedirect(raw: unknown, origin: string): string {
  if (typeof raw !== 'string' || raw === '') return '/';
  try {
    const url = new URL(raw, origin);
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.href : '/';
  } catch {
    return '/';
  }
}
