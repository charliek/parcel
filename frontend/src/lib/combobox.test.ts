import { describe, expect, it } from 'bun:test';
import { comboboxKey, matchingTags, tagOptions } from './combobox';

const state = { open: true, active: 0, count: 3, text: 'ab', chips: 2 };

describe('comboboxKey', () => {
  it('moves the active option with the arrows, wrapping', () => {
    expect(comboboxKey('ArrowDown', state)).toEqual({ type: 'move', active: 1 });
    expect(comboboxKey('ArrowDown', { ...state, active: 2 })).toEqual({ type: 'move', active: 0 });
    expect(comboboxKey('ArrowUp', state)).toEqual({ type: 'move', active: 2 });
    // A closed list opens on the first (↓) or last (↑) option.
    expect(comboboxKey('ArrowDown', { ...state, open: false, active: -1 })).toEqual({
      type: 'move',
      active: 0,
    });
    expect(comboboxKey('ArrowUp', { ...state, open: false, active: -1 })).toEqual({
      type: 'move',
      active: 2,
    });
    expect(comboboxKey('ArrowDown', { ...state, count: 0 })).toEqual({ type: 'none' });
  });

  it('moves onto the option typing highlighted, not past it', () => {
    // Typing highlights the first match; ↓ + Enter must pick that one too,
    // not the next ("Create …" in a tag input).
    expect(comboboxKey('ArrowDown', { ...state, fresh: true })).toEqual({
      type: 'move',
      active: 0,
    });
    expect(comboboxKey('ArrowUp', { ...state, fresh: true })).toEqual({ type: 'move', active: 2 });
    expect(comboboxKey('Enter', { ...state, fresh: true })).toEqual({ type: 'pick', index: 0 });
  });

  it('picks with Enter and closes with Escape only while open', () => {
    expect(comboboxKey('Enter', { ...state, active: 1 })).toEqual({ type: 'pick', index: 1 });
    expect(comboboxKey('Enter', { ...state, open: false })).toEqual({ type: 'none' });
    expect(comboboxKey('Enter', { ...state, active: -1 })).toEqual({ type: 'none' });
    expect(comboboxKey('Escape', state)).toEqual({ type: 'close' });
    expect(comboboxKey('Escape', { ...state, open: false })).toEqual({ type: 'none' });
  });

  it('removes the last chip with Backspace in an empty input', () => {
    expect(comboboxKey('Backspace', { ...state, text: '' })).toEqual({ type: 'remove-last' });
    expect(comboboxKey('Backspace', state)).toEqual({ type: 'none' });
    expect(comboboxKey('Backspace', { ...state, text: '', chips: 0 })).toEqual({ type: 'none' });
  });
});

const VOCAB = [
  { tag: 'project:roost', count: 14 },
  { tag: 'project:parcel', count: 9 },
  { tag: 'sessions', count: 4 },
  { tag: 'session-tokens', count: 1 },
  { tag: 'kind:research', count: 7 },
  { tag: 'roost', count: 1 },
];

describe('matchingTags', () => {
  it('completes the tag or its value, most used first', () => {
    expect(matchingTags(VOCAB, 'ro', []).map((t) => t.tag)).toEqual(['project:roost', 'roost']);
    expect(matchingTags(VOCAB, 'project:', []).map((t) => t.tag)).toEqual([
      'project:roost',
      'project:parcel',
    ]);
    expect(matchingTags(VOCAB, 'Ses', ['sessions']).map((t) => t.tag)).toEqual(['session-tokens']);
    expect(matchingTags(VOCAB, '  ', [])).toEqual([]);
  });

  it('puts an exact match first', () => {
    expect(matchingTags(VOCAB, 'roost', []).map((t) => t.tag)).toEqual(['roost', 'project:roost']);
  });
});

describe('tagOptions', () => {
  it('offers existing tags with counts, then Create for a new valid one', () => {
    expect(tagOptions(VOCAB, 'ses', [])).toEqual([
      { kind: 'existing', tag: 'sessions', count: 4 },
      { kind: 'existing', tag: 'session-tokens', count: 1 },
      { kind: 'create', tag: 'ses' },
    ]);
  });

  it('shows the normalized form and never offers an existing or chosen tag again', () => {
    expect(tagOptions([], 'Big Plan', [])).toEqual([{ kind: 'create', tag: 'big-plan' }]);
    expect(tagOptions(VOCAB, 'sessions', [])).toEqual([
      { kind: 'existing', tag: 'sessions', count: 4 },
    ]);
    expect(tagOptions([], 'auth', ['auth'])).toEqual([]);
    expect(tagOptions([], 'not ok!', [])).toEqual([]);
  });
});
