import { describe, expect, it } from 'bun:test';
import {
  canStartOAuth,
  requiredPageAccess,
  shouldAttemptSilentReauth,
  viewerAccess,
} from './policy';

describe('canStartOAuth', () => {
  it('only allows OAuth under slauth', () => {
    expect(canStartOAuth('slauth')).toBe(true);
    expect(canStartOAuth('dev')).toBe(false);
    expect(canStartOAuth(null)).toBe(false);
    expect(canStartOAuth('something-else')).toBe(false);
  });
});

describe('shouldAttemptSilentReauth', () => {
  const base = {
    mode: 'slauth',
    alreadyTried: false,
    onCallbackRoute: false,
    need: 'required' as const,
    signedInHint: false,
  };

  it('tries once on account pages', () => {
    expect(shouldAttemptSilentReauth(base)).toBe(true);
    expect(shouldAttemptSilentReauth({ ...base, alreadyTried: true })).toBe(false);
  });

  it('never runs in dev mode or from the callback', () => {
    expect(shouldAttemptSilentReauth({ ...base, mode: 'dev' })).toBe(false);
    expect(shouldAttemptSilentReauth({ ...base, onCallbackRoute: true })).toBe(false);
  });

  it('on optional pages, only tries for returning browsers', () => {
    expect(shouldAttemptSilentReauth({ ...base, need: 'optional' })).toBe(false);
    expect(shouldAttemptSilentReauth({ ...base, need: 'optional', signedInHint: true })).toBe(true);
  });
});

describe('requiredPageAccess', () => {
  const base = { mode: 'slauth', silentTried: false };

  it('waits while the session resolves', () => {
    expect(requiredPageAccess({ ...base, session: 'idle' })).toBe('loading');
    expect(requiredPageAccess({ ...base, session: 'loading' })).toBe('loading');
  });

  it('renders for signed-in users', () => {
    expect(requiredPageAccess({ ...base, session: 'authenticated' })).toBe('ready');
    expect(requiredPageAccess({ ...base, mode: 'dev', session: 'authenticated' })).toBe('ready');
  });

  it('tries silent re-auth once, then shows sign-in', () => {
    expect(requiredPageAccess({ ...base, session: 'anonymous' })).toBe('silent-reauth');
    expect(requiredPageAccess({ ...base, session: 'anonymous', silentTried: true })).toBe(
      'sign-in',
    );
  });

  it('never redirects in dev mode', () => {
    expect(requiredPageAccess({ ...base, mode: 'dev', session: 'anonymous' })).toBe('sign-in');
  });

  it('surfaces errors', () => {
    expect(requiredPageAccess({ ...base, session: 'error' })).toBe('error');
  });
});

describe('viewerAccess', () => {
  const base = { mode: 'slauth', silentTried: false, session: 'anonymous' as const };

  it('renders a parcel as soon as the view succeeds, signed in or not', () => {
    expect(viewerAccess({ ...base, view: 'success' })).toBe('ready');
    expect(viewerAccess({ ...base, view: 'success', session: 'loading' })).toBe('ready');
  });

  it('waits for the view', () => {
    expect(viewerAccess({ ...base, view: 'pending' })).toBe('loading');
  });

  it('gives a browser that signed in before one silent try, so owners keep their controls', () => {
    const hinted = { ...base, view: 'success' as const, signedInHint: true };
    expect(viewerAccess(hinted)).toBe('silent-reauth');
    expect(viewerAccess({ ...hinted, session: 'loading' })).toBe('loading');
    expect(viewerAccess({ ...hinted, silentTried: true })).toBe('ready');
    expect(viewerAccess({ ...hinted, session: 'authenticated' })).toBe('ready');
    expect(viewerAccess({ ...hinted, mode: 'dev' })).toBe('ready');
  });

  it('for anonymous visitors, tries silent re-auth on 401/403/404, then asks to sign in', () => {
    for (const status of [401, 403, 404]) {
      expect(viewerAccess({ ...base, view: 'error', errorStatus: status })).toBe('silent-reauth');
      expect(viewerAccess({ ...base, view: 'error', errorStatus: status, silentTried: true })).toBe(
        'sign-in',
      );
    }
  });

  it('waits for the session before deciding a denied view', () => {
    expect(viewerAccess({ ...base, view: 'error', errorStatus: 404, session: 'loading' })).toBe(
      'loading',
    );
  });

  it('treats a denied view as not found when signed in or in dev mode', () => {
    expect(
      viewerAccess({ ...base, view: 'error', errorStatus: 404, session: 'authenticated' }),
    ).toBe('not-found');
    expect(viewerAccess({ ...base, view: 'error', errorStatus: 404, mode: 'dev' })).toBe(
      'not-found',
    );
  });

  it('reports other failures as errors', () => {
    expect(viewerAccess({ ...base, view: 'error', errorStatus: 500 })).toBe('error');
    expect(viewerAccess({ ...base, view: 'error' })).toBe('error');
  });
});
