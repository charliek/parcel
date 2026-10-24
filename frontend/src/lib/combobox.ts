/**
 * The keyboard half of the ARIA combobox pattern the tag inputs use, and
 * how suggestions are picked from a tag vocabulary. Pure, so it is
 * unit-tested; the components only map the result onto state.
 *
 * Focus never leaves the text input: ↑/↓ move the active option
 * (`aria-activedescendant`), Enter picks it, Escape closes the list, and
 * Backspace in an empty input removes the last chip. Typing makes the first
 * option active, and the first ↓ after that lands on it rather than past
 * it, so ↓ + Enter picks the best match just as Enter does.
 */

import { normalizeTag, tagValue } from './tags';
import type { TagCount } from './types';

export type ComboboxAction =
  | { type: 'move'; active: number }
  | { type: 'pick'; index: number }
  | { type: 'close' }
  | { type: 'remove-last' }
  | { type: 'none' };

export interface ComboboxState {
  /** Whether the suggestion list is showing. */
  open: boolean;
  /** Index of the active option (-1 for none). */
  active: number;
  /** How many options the list has. */
  count: number;
  /** The text in the input. */
  text: string;
  /** How many chips sit before the input. */
  chips: number;
  /** The active option is the one typing chose (the user hasn't moved yet). */
  fresh?: boolean;
}

/** What a key in the input does. `none` leaves the key to the browser. */
export function comboboxKey(key: string, state: ComboboxState): ComboboxAction {
  const { open, active, count } = state;
  switch (key) {
    case 'ArrowDown':
      if (count === 0) return { type: 'none' };
      if (!open) return { type: 'move', active: 0 };
      return { type: 'move', active: state.fresh ? active : (active + 1) % count };
    case 'ArrowUp':
      if (count === 0) return { type: 'none' };
      return { type: 'move', active: open ? (active - 1 + count) % count : count - 1 };
    case 'Enter':
      return open && active >= 0 && active < count
        ? { type: 'pick', index: active }
        : { type: 'none' };
    case 'Escape':
      return open ? { type: 'close' } : { type: 'none' };
    case 'Backspace':
      return state.text === '' && state.chips > 0 ? { type: 'remove-last' } : { type: 'none' };
    default:
      return { type: 'none' };
  }
}

/**
 * Tags from `vocabulary` that complete `text`: the tag or its value (the
 * part after `ns:`) starts with it. Already chosen tags are left out. An
 * exact match comes first, then the most used, then by name.
 */
export function matchingTags(
  vocabulary: readonly TagCount[],
  text: string,
  exclude: readonly string[],
  limit = 8,
): TagCount[] {
  // Typed text completes as the tag it would become (`Big Plan` → `big-plan`).
  const needle = normalizeTag(text) ?? text.trim().toLowerCase();
  if (!needle) return [];
  return vocabulary
    .filter(
      ({ tag }) =>
        !exclude.includes(tag) && (tag.startsWith(needle) || tagValue(tag).startsWith(needle)),
    )
    .sort(
      (a, b) =>
        Number(b.tag === needle) - Number(a.tag === needle) ||
        b.count - a.count ||
        (a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0),
    )
    .slice(0, limit);
}

/** One option in a tag chip input's list. */
export type TagOption =
  | { kind: 'existing'; tag: string; count: number }
  | { kind: 'create'; tag: string };

/**
 * The options for a tag chip input: existing tags first (with counts), then
 * "Create …" for the normalized text when it is a valid tag nobody uses yet.
 */
export function tagOptions(
  vocabulary: readonly TagCount[],
  text: string,
  chosen: readonly string[],
): TagOption[] {
  const options: TagOption[] = matchingTags(vocabulary, text, chosen).map((t) => ({
    kind: 'existing',
    tag: t.tag,
    count: t.count,
  }));
  const normalized = normalizeTag(text);
  if (
    normalized !== null &&
    !chosen.includes(normalized) &&
    !options.some((o) => o.tag === normalized) &&
    !vocabulary.some((t) => t.tag === normalized)
  ) {
    options.push({ kind: 'create', tag: normalized });
  }
  return options;
}
