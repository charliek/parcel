import { describe, expect, it } from 'bun:test';
import { checkSlugText, MAX_SLUG_CHARS, normalizeHandle, normalizeSlug } from './slugs';

// The same cases as `crates/parcel-types/src/slugs.rs`'s tests.
describe('normalizeSlug', () => {
  it('normalizes what the server accepts', () => {
    for (const [raw, expected] of [
      ['plan', 'plan'],
      ['  Parcel/Plan-001 ', 'parcel/plan-001'],
      ['a/b/c/d', 'a/b/c/d'],
      ['q3-2026', 'q3-2026'],
      ['x', 'x'],
      // Ten or fourteen id-alphabet characters aren't an id.
      ['management', 'management'],
      ['vp/pv/p2/v-1', 'vp/pv/p2/v-1'],
      ['a'.repeat(MAX_SLUG_CHARS), 'a'.repeat(MAX_SLUG_CHARS)],
    ]) {
      expect([raw, normalizeSlug(raw)]).toEqual([raw, { ok: true, value: expected }]);
    }
  });

  it('refuses what the server refuses, with its rule', () => {
    for (const bad of [
      '',
      ' ',
      'a/b/c/d/e',
      'a'.repeat(MAX_SLUG_CHARS + 1),
      '/plan',
      'plan/',
      'a//b',
      '-plan',
      'plan-',
      'pl--an',
      'pl_an',
      'pl an',
      'plän',
      'pl.an',
      'plan?v=2',
      '@ada/plan',
      'p',
      'v',
      'p/abcdefghijkm',
      'plans/v/2',
      'abcdefghijkm',
      'plans/abcdefghijkm',
      'ABCDEFGHIJKM',
    ]) {
      const got = normalizeSlug(bad);
      expect([bad, got.ok]).toEqual([bad, false]);
      if (!got.ok) expect(got.message).toContain("may not be 'p', 'v'");
    }
  });
});

describe('normalizeHandle', () => {
  it('normalizes what the server accepts', () => {
    for (const [raw, expected] of [
      ['Ada', 'ada'],
      ['a', 'a'],
      [' ada-2 ', 'ada-2'],
      ['a--b', 'a--b'],
      ['a'.repeat(32), 'a'.repeat(32)],
    ]) {
      expect(normalizeHandle(raw)).toEqual({ ok: true, value: expected });
    }
  });

  it('refuses what the server refuses', () => {
    for (const bad of ['', '-ada', 'ada-', 'a.b', 'a_b', 'a/b', '@ada', 'adà', 'a'.repeat(33)]) {
      expect([bad, normalizeHandle(bad).ok]).toEqual([bad, false]);
    }
  });
});

describe('checkSlugText', () => {
  it('accepts no slug or a valid one, normalized', () => {
    expect(checkSlugText('  ')).toEqual({ ok: true, slug: '' });
    expect(checkSlugText(' Roost/Plan ')).toEqual({ ok: true, slug: 'roost/plan' });
    expect(checkSlugText('a_b').ok).toBe(false);
  });
});
