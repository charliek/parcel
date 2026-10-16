/**
 * The three visibilities, in words: what the pickers, badges and toasts say.
 *
 * `private` is the owner plus the people it's shared with by email;
 * `internal` is anyone signed in to this Parcel; `public` is anyone with
 * the link. A server can turn public links off (`ServerInfo.allow_public`),
 * and then reports stored public parcels as `internal`.
 */

import type { Visibility } from './types';

export interface VisibilityText {
  /** The option's name in pickers and menus. */
  label: string;
  /** Short name on badges. */
  badge: string;
  /** One line under the label in pickers. */
  hint: string;
  /** Badge tooltip: who can open it. */
  who: string;
}

export const VISIBILITY: Record<Visibility, VisibilityText> = {
  private: {
    label: 'Private',
    badge: 'Private',
    hint: 'Only you and the people you add',
    who: 'Only the owner and the people it’s shared with can open it',
  },
  internal: {
    label: 'Everyone signed in',
    badge: 'Internal',
    hint: 'Anyone who signs in to this Parcel',
    who: 'Anyone signed in to this Parcel can open it',
  },
  public: {
    label: 'Anyone with the link',
    badge: 'Public',
    hint: 'No sign-in needed',
    who: 'Anyone with the link can open it, signed in or not',
  },
};

/** Narrowest first. */
export const VISIBILITIES: readonly Visibility[] = ['private', 'internal', 'public'];

/** Why `public` can't be picked when the server turned it off. */
export const PUBLIC_OFF_REASON = 'Public links are turned off on this server.';

export interface VisibilityOption extends VisibilityText {
  value: Visibility;
  /** Set when the option can't be picked here. */
  disabledReason: string | null;
}

/** The picker's options; `public` is disabled, with the reason, when the server refuses it. */
export function visibilityOptions(allowPublic: boolean): VisibilityOption[] {
  return VISIBILITIES.map((value) => ({
    value,
    ...VISIBILITY[value],
    disabledReason: value === 'public' && !allowPublic ? PUBLIC_OFF_REASON : null,
  }));
}

/** The toast after a parcel's visibility changes. */
export function visibilityChangedText(visibility: Visibility): {
  title: string;
  description: string;
} {
  switch (visibility) {
    case 'private':
      return {
        title: 'Parcel is now private',
        description: 'Only you and the people you add can open it.',
      };
    case 'internal':
      return {
        title: 'Parcel is now open to everyone signed in',
        description: 'Anyone who signs in to this Parcel can open the link.',
      };
    case 'public':
      return {
        title: 'Parcel is now public',
        description: 'Anyone with the link can view it, no sign-in needed.',
      };
  }
}
