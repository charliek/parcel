/**
 * What the viewer does when its activity poll reports a newer version.
 *
 * Pure so the rules are unit-tested; `ViewerPage` feeds it the poll result
 * and acts on the answer.
 */

/** How often the viewer polls `GET /parcels/{id}/activity` while visible. */
export const ACTIVITY_POLL_MS = 5_000;

export type LiveUpdateDecision =
  /** The frame shows the newest version (or the poll hasn't answered yet). */
  | 'up-to-date'
  /** Swap the frame to the new version now. */
  | 'reload'
  /** Leave the frame alone and offer a "vN is available. Reload" banner. */
  | 'offer-reload'
  /**
   * The URL names a version (`/v/N`): never move it. The "viewing an older
   * version" banner shows the new current number instead.
   */
  | 'pinned';

export interface LiveUpdateInput {
  /** The version the frame shows. */
  shown: number;
  /** The newest version known (from the poll, or the grant when newer). */
  latest: number;
  /** The URL names a version (`/p/:id/v/:n`). */
  pinned: boolean;
  /** The user is writing a comment, a reply or an edit (or has a pin placed). */
  composing: boolean;
  /** The "Reload when a new version is published" preference. */
  autoReload: boolean;
}

export function decideLiveUpdate(input: LiveUpdateInput): LiveUpdateDecision {
  if (input.latest <= input.shown) return 'up-to-date';
  if (input.pinned) return 'pinned';
  // Reloading would throw away a half-written comment and the spot it points at.
  if (input.composing || !input.autoReload) return 'offer-reload';
  return 'reload';
}

/** The stored preference: on unless explicitly turned off. */
export function parseAutoReload(stored: string | null): boolean {
  return stored !== '0';
}
