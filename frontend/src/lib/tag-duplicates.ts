/**
 * "Possible duplicates" on the Tags page: pairs of tags in the same
 * namespace that look like one tag spelled two ways (`roost` / `roost-app`,
 * `session` / `sessions`). Computed in the browser over the caller's own
 * vocabulary, so it costs the server nothing.
 */

import { tagNamespace, tagValue } from './tags';
import type { TagCount } from './types';

/** Values shorter than this are left alone (`ui` is a prefix of too much). */
export const MIN_DUPLICATE_CHARS = 4;
/** Most edits apart two values may be. */
export const MAX_DUPLICATE_DISTANCE = 2;

/** Levenshtein distance, giving up (returning `max + 1`) once it exceeds `max`. */
export function editDistance(a: string, b: string, max = Number.POSITIVE_INFINITY): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    let best = i;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      const value = Math.min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + cost);
      row.push(value);
      best = Math.min(best, value);
    }
    if (best > max) return max + 1;
    prev = row;
  }
  return prev[b.length];
}

/** A suggested merge: `from` (the less used) into `to`. */
export interface DuplicateHint {
  from: string;
  to: string;
}

/**
 * Pairs in the same namespace where both values are at least four
 * characters and one is a prefix of the other or they are at most two
 * edits apart. Each suggests merging the less used tag into the more used
 * one (ties: the shorter, then the first by name). Each tag appears as
 * `from` at most once.
 */
export function possibleDuplicates(tags: readonly TagCount[]): DuplicateHint[] {
  const hints: DuplicateHint[] = [];
  const merged = new Set<string>();
  const sorted = [...tags].sort(
    (a, b) =>
      b.count - a.count ||
      a.tag.length - b.tag.length ||
      (a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0),
  );
  // `sorted` puts the tag to keep first, so each pair is (keep, drop).
  for (let i = 0; i < sorted.length; i++) {
    const keep = sorted[i];
    if (merged.has(keep.tag)) continue;
    const ns = tagNamespace(keep.tag);
    const a = tagValue(keep.tag);
    if (a.length < MIN_DUPLICATE_CHARS) continue;
    for (let j = i + 1; j < sorted.length; j++) {
      const drop = sorted[j];
      if (merged.has(drop.tag) || tagNamespace(drop.tag) !== ns) continue;
      const b = tagValue(drop.tag);
      if (b.length < MIN_DUPLICATE_CHARS) continue;
      const similar =
        a.startsWith(b) ||
        b.startsWith(a) ||
        editDistance(a, b, MAX_DUPLICATE_DISTANCE) <= MAX_DUPLICATE_DISTANCE;
      if (similar) {
        hints.push({ from: drop.tag, to: keep.tag });
        merged.add(drop.tag);
      }
    }
  }
  return hints;
}
