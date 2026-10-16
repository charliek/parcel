/**
 * Pure helpers behind the share dialog: the email as sent, how a person is
 * shown, and cache updates after a mutation.
 */

import type { Share, ShareList } from './types';

/**
 * The email as the server stores and matches it: trimmed and lowercased.
 * Whether it is a usable address is the server's call (its 400 says why).
 */
export function normalizeEmail(raw: string): string {
  return raw.trim().toLowerCase();
}

/** Add a share, or replace the one with the same email, keeping the list by email. */
export function upsertShare(list: ShareList, share: Share): ShareList {
  const rest = list.shares.filter((s) => s.email !== share.email);
  const shares = [...rest, share].sort((a, b) =>
    a.email < b.email ? -1 : a.email > b.email ? 1 : 0,
  );
  return { shares };
}

export function removeShare(list: ShareList, email: string): ShareList {
  return { shares: list.shares.filter((s) => s.email !== email) };
}

/** How a person with access is listed. */
export interface ShareDisplay {
  /** Their name once they've signed in, else the email. */
  primary: string;
  /** The email under the name, when the name is shown. */
  secondary: string | null;
  /** Nobody with this email has signed in yet. */
  pending: boolean;
}

export function shareDisplay(share: Share): ShareDisplay {
  const name = share.name?.trim();
  return name
    ? { primary: name, secondary: share.email, pending: false }
    : { primary: share.email, secondary: null, pending: true };
}
