import { describe, expect, it } from 'bun:test';
import {
  AUTO_SIGN_IN_COOLDOWN_MS,
  isResolving,
  loginHref,
  orderProviders,
  parseAttempt,
  type SignInTarget,
  safeLogoutRedirect,
  shouldAutoSignIn,
  signInError,
  signInPageHref,
  signInTarget,
} from './sign-in';

const okta = { id: 'okta', label: 'Acme Okta' };
const keycloak = { id: 'keycloak', label: 'Partner Keycloak' };
const slauth = { id: 'slauth', label: 'slauth' };

describe('isResolving', () => {
  it('is true until /me has answered', () => {
    expect(isResolving('idle')).toBe(true);
    expect(isResolving('loading')).toBe(true);
    expect(isResolving('authenticated')).toBe(false);
    expect(isResolving('anonymous')).toBe(false);
    expect(isResolving('error')).toBe(false);
  });
});

describe('loginHref', () => {
  it('names the provider and the way back', () => {
    expect(loginHref('/p/abcdefghijkm', 'okta')).toBe(
      '/auth/login?provider=okta&return_to=%2Fp%2Fabcdefghijkm',
    );
    expect(loginHref('/device?code=BCDF-GHJK', 'okta')).toBe(
      '/auth/login?provider=okta&return_to=%2Fdevice%3Fcode%3DBCDF-GHJK',
    );
  });

  it('leaves the provider out when the server has only one', () => {
    expect(loginHref('/tokens')).toBe('/auth/login?return_to=%2Ftokens');
  });

  it('never asks to return off-site or into sign-in', () => {
    expect(loginHref('https://evil.example', 'okta')).toBe(
      '/auth/login?provider=okta&return_to=%2F',
    );
    expect(loginHref('/signin?error=denied', 'okta')).toBe(
      '/auth/login?provider=okta&return_to=%2F',
    );
  });
});

describe('signInPageHref', () => {
  it('carries the return path', () => {
    expect(signInPageHref('/p/abcdefghijkm')).toBe('/signin?return_to=%2Fp%2Fabcdefghijkm');
    expect(signInPageHref('//evil.example')).toBe('/signin?return_to=%2F');
  });
});

describe('signInTarget', () => {
  it('offers nothing in dev mode or without providers', () => {
    expect(signInTarget('dev', [])).toEqual({ kind: 'none' });
    expect(signInTarget('dev', [okta])).toEqual({ kind: 'none' });
    expect(signInTarget('oidc', [])).toEqual({ kind: 'none' });
    expect(signInTarget(undefined, [])).toEqual({ kind: 'none' });
  });

  it('goes straight to a single provider', () => {
    expect(signInTarget('oidc', [slauth])).toEqual({ kind: 'provider', provider: slauth });
  });

  it('lets people choose between several', () => {
    expect(signInTarget('oidc', [okta, keycloak])).toEqual({ kind: 'choose' });
  });
});

describe('orderProviders', () => {
  it('puts the last-used provider first, keeping the rest in order', () => {
    expect(orderProviders([okta, keycloak, slauth], 'slauth')).toEqual([slauth, okta, keycloak]);
    expect(orderProviders([okta, keycloak, slauth], 'okta')).toEqual([okta, keycloak, slauth]);
  });

  it('keeps the configured order when the last one is unknown or unset', () => {
    expect(orderProviders([okta, keycloak], 'gone')).toEqual([okta, keycloak]);
    expect(orderProviders([okta, keycloak], null)).toEqual([okta, keycloak]);
  });

  it('does not change its input', () => {
    const list = [okta, keycloak];
    orderProviders(list, 'keycloak');
    expect(list).toEqual([okta, keycloak]);
  });
});

describe('signInError', () => {
  it('has words for every code the server sends', () => {
    for (const code of [
      'denied',
      'expired',
      'domain',
      'unverified',
      'failed',
      'unavailable',
      'disabled',
    ]) {
      const text = signInError(code);
      expect(text?.title.length).toBeGreaterThan(0);
      expect(text?.description.length).toBeGreaterThan(0);
    }
    expect(signInError('domain')?.description).toContain('email domain');
    expect(signInError('disabled')?.title).toBe('Your account is disabled');
  });

  it('names the provider when the server says which one', () => {
    expect(signInError('domain', 'EU office (Keycloak)')?.description).toContain(
      'can’t sign in with EU office (Keycloak).',
    );
    expect(signInError('unverified', 'acme Okta')?.description).toMatch(/^Acme Okta didn’t/);
    expect(signInError('domain')?.description).toContain('with that provider');
    expect(signInError('unverified')?.description).toMatch(/^That provider didn’t/);
  });

  it('falls back to a generic failure for unknown codes', () => {
    expect(signInError('teapot')).toEqual(signInError('failed'));
    // Not fooled by inherited property names.
    expect(signInError('constructor')).toEqual(signInError('failed'));
  });

  it('is null without an error', () => {
    expect(signInError(null)).toBeNull();
    expect(signInError('')).toBeNull();
  });
});

describe('shouldAutoSignIn', () => {
  const single: SignInTarget = { kind: 'provider', provider: okta };
  const now = 1_000_000_000;

  it('sends a returning browser straight to its only provider', () => {
    expect(shouldAutoSignIn({ target: single, signedInHint: true, lastAttemptAt: null, now })).toBe(
      true,
    );
  });

  it('shows the sign-in screen to first-time visitors and after sign-out', () => {
    expect(
      shouldAutoSignIn({ target: single, signedInHint: false, lastAttemptAt: null, now }),
    ).toBe(false);
  });

  it('never picks between several providers, or in dev mode', () => {
    for (const target of [{ kind: 'choose' }, { kind: 'none' }] as SignInTarget[]) {
      expect(shouldAutoSignIn({ target, signedInHint: true, lastAttemptAt: null, now })).toBe(
        false,
      );
    }
  });

  it('does not go round again within the cooldown', () => {
    const recent = now - AUTO_SIGN_IN_COOLDOWN_MS + 1;
    expect(
      shouldAutoSignIn({ target: single, signedInHint: true, lastAttemptAt: recent, now }),
    ).toBe(false);
    const old = now - AUTO_SIGN_IN_COOLDOWN_MS;
    expect(shouldAutoSignIn({ target: single, signedInHint: true, lastAttemptAt: old, now })).toBe(
      true,
    );
    // A clock that went backwards counts as recent.
    expect(
      shouldAutoSignIn({ target: single, signedInHint: true, lastAttemptAt: now + 5, now }),
    ).toBe(false);
  });
});

describe('parseAttempt', () => {
  it('reads a stored timestamp', () => {
    expect(parseAttempt('1700000000000')).toBe(1_700_000_000_000);
    expect(parseAttempt(null)).toBeNull();
    expect(parseAttempt('')).toBeNull();
    expect(parseAttempt('soon')).toBeNull();
  });
});

describe('safeLogoutRedirect', () => {
  const origin = 'https://parcel.example';

  it('follows the provider logout page or a path', () => {
    expect(safeLogoutRedirect('https://id.example/logout?id_token_hint=x', origin)).toBe(
      'https://id.example/logout?id_token_hint=x',
    );
    expect(safeLogoutRedirect('/', origin)).toBe('https://parcel.example/');
  });

  it('refuses anything that is not http(s)', () => {
    expect(safeLogoutRedirect('javascript:alert(1)', origin)).toBe('/');
    expect(safeLogoutRedirect('data:text/html,hi', origin)).toBe('/');
    expect(safeLogoutRedirect('', origin)).toBe('/');
    expect(safeLogoutRedirect(undefined, origin)).toBe('/');
    expect(safeLogoutRedirect(42, origin)).toBe('/');
  });
});
