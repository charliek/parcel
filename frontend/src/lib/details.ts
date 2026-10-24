/**
 * The PATCH the Edit details dialog sends. Pure, so it is unit-tested.
 */

import type { ParcelSummary, UpdateParcelRequest } from './types';

function sameSet(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && [...a].sort().join('\n') === [...b].sort().join('\n');
}

/**
 * What changed, and only that. Built by hand rather than through
 * `compactMetadata` (which drops empty values): clearing the tags must send
 * `tags: []` and clearing the slug `slug: ""`, or nothing would be cleared.
 */
export function detailsPatch(
  parcel: ParcelSummary,
  edited: { title: string; description: string; tags: string[]; slug: string },
): UpdateParcelRequest {
  const body: UpdateParcelRequest = {};
  if (edited.title !== parcel.title) body.title = edited.title;
  if (edited.description !== parcel.description) body.description = edited.description;
  if (!sameSet(edited.tags, parcel.tags ?? [])) body.tags = edited.tags;
  if (edited.slug !== (parcel.slug ?? '')) body.slug = edited.slug;
  return body;
}
