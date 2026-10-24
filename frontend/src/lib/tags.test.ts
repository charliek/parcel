import { describe, expect, it } from 'bun:test';
import { normalizeTag, normalizeTags } from './tags';

// The case list the Rust implementation is tested against too.
interface TagCases {
  valid: [string, string][];
  invalid: string[];
  sets: { input: string[]; expected?: string[]; error?: 'too_many' | 'invalid' }[];
}

const cases: TagCases = await Bun.file(
  new URL('../../../crates/parcel-types/tests/fixtures/tag-cases.json', import.meta.url),
).json();

describe('normalizeTag', () => {
  it('normalizes what the server would store', () => {
    expect(cases.valid.length).toBeGreaterThan(0);
    for (const [input, expected] of cases.valid) {
      expect([input, normalizeTag(input)]).toEqual([input, expected]);
    }
  });

  it('refuses what the server refuses', () => {
    expect(cases.invalid.length).toBeGreaterThan(0);
    for (const input of cases.invalid) {
      expect([input, normalizeTag(input)]).toEqual([input, null]);
    }
  });
});

describe('normalizeTags', () => {
  it('dedupes, sorts by bytes and caps like the server', () => {
    for (const c of cases.sets) {
      const got = normalizeTags(c.input);
      if (c.error) {
        expect(got.ok ? 'ok' : got.error).toBe(c.error);
      } else {
        expect(got).toEqual({ ok: true, tags: c.expected ?? [] });
      }
    }
  });

  it('names the tag it refused', () => {
    expect(normalizeTags(['fine', 'not fine!'])).toEqual({
      ok: false,
      error: 'invalid',
      tag: 'not fine!',
    });
  });
});
