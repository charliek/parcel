/**
 * What the viewer shows and allows, from the caller's `access` to a parcel
 * (the server decides it: PLAN.md, "Sign-in providers and sharing") and
 * the state of the view request. Pure, so it is unit-tested.
 */

import { isResolving, type SessionStatus } from './sign-in';
import type { ParcelAccess, ShareRole } from './types';

/** What `/p/:id` renders. */
export type ViewerPageState = 'loading' | 'ready' | 'sign-in' | 'not-found' | 'error';

export function viewerPageState(input: {
  /** State of `GET /api/v1/parcels/{id}/view`. */
  view: 'pending' | 'success' | 'error';
  /** HTTP status when the view failed (undefined for network errors). */
  errorStatus?: number;
  session: SessionStatus;
  /** Signing in is possible here (not dev mode, and a provider exists). */
  canSignIn: boolean;
}): ViewerPageState {
  if (input.view === 'success') return 'ready';
  if (input.view === 'pending') return 'loading';
  const denied =
    input.errorStatus === 401 || input.errorStatus === 403 || input.errorStatus === 404;
  if (!denied) return 'error';
  // Wait until we know whether the visitor is signed in.
  if (isResolving(input.session)) return 'loading';
  // Signed in (or nowhere to sign in): it really is gone or not theirs.
  if (input.session === 'authenticated' || !input.canSignIn) return 'not-found';
  return 'sign-in';
}

/** What the viewer lets the caller do. */
export interface ViewerAbilities {
  /** Change, share, publish to and delete the parcel. */
  isOwner: boolean;
  /** See the comment threads (and their pins). */
  canReadComments: boolean;
  /** Comment mode, the composer, replies. */
  canComment: boolean;
}

export function viewerAbilities(access: ParcelAccess): ViewerAbilities {
  return {
    isOwner: access === 'owner',
    canReadComments: access !== 'public',
    canComment: access === 'owner' || access === 'commenter',
  };
}

/** A share role in words, for role pickers and the shared-with-me list. */
export const ROLE_LABEL: Record<ShareRole, string> = {
  viewer: 'Can view',
  commenter: 'Can comment',
};

/** The caller's access in words, for the details panel. */
export function accessLabel(access: ParcelAccess): string {
  switch (access) {
    case 'owner':
      return 'Owner';
    case 'commenter':
      return ROLE_LABEL.commenter;
    case 'viewer':
      return ROLE_LABEL.viewer;
    case 'public':
      return 'View only';
  }
}
