import { describe, expect, it } from 'bun:test';
import { accessLabel, ROLE_LABEL, viewerAbilities, viewerPageState } from './access';

describe('viewerPageState', () => {
  const base = { session: 'anonymous', canSignIn: true } as const;

  it('renders whatever the server granted, signed in or not', () => {
    expect(viewerPageState({ ...base, view: 'success' })).toBe('ready');
    expect(viewerPageState({ ...base, view: 'success', session: 'loading' })).toBe('ready');
    expect(viewerPageState({ ...base, view: 'pending' })).toBe('loading');
  });

  it('offers sign-in to a signed-out visitor who was refused', () => {
    for (const status of [401, 403, 404]) {
      expect(viewerPageState({ ...base, view: 'error', errorStatus: status })).toBe('sign-in');
    }
  });

  it('waits for the session before deciding', () => {
    expect(viewerPageState({ ...base, view: 'error', errorStatus: 404, session: 'idle' })).toBe(
      'loading',
    );
    expect(viewerPageState({ ...base, view: 'error', errorStatus: 404, session: 'loading' })).toBe(
      'loading',
    );
  });

  it('is not found for signed-in callers and where nobody can sign in', () => {
    expect(
      viewerPageState({ ...base, view: 'error', errorStatus: 404, session: 'authenticated' }),
    ).toBe('not-found');
    expect(viewerPageState({ ...base, view: 'error', errorStatus: 404, canSignIn: false })).toBe(
      'not-found',
    );
  });

  it('reports other failures as errors', () => {
    expect(viewerPageState({ ...base, view: 'error', errorStatus: 500 })).toBe('error');
    expect(viewerPageState({ ...base, view: 'error' })).toBe('error');
  });
});

describe('viewerAbilities', () => {
  it('follows the access the server reports', () => {
    expect(viewerAbilities('owner')).toEqual({
      isOwner: true,
      canReadComments: true,
      canComment: true,
    });
    expect(viewerAbilities('commenter')).toEqual({
      isOwner: false,
      canReadComments: true,
      canComment: true,
    });
    expect(viewerAbilities('viewer')).toEqual({
      isOwner: false,
      canReadComments: true,
      canComment: false,
    });
    // Signed out on a public parcel: no comments at all.
    expect(viewerAbilities('public')).toEqual({
      isOwner: false,
      canReadComments: false,
      canComment: false,
    });
  });
});

describe('labels', () => {
  it('names roles and access in words', () => {
    expect(ROLE_LABEL.viewer).toBe('Can view');
    expect(ROLE_LABEL.commenter).toBe('Can comment');
    expect(accessLabel('owner')).toBe('Owner');
    expect(accessLabel('commenter')).toBe('Can comment');
    expect(accessLabel('viewer')).toBe('Can view');
    expect(accessLabel('public')).toBe('View only');
  });
});
