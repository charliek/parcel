import { describe, expect, it } from 'bun:test';
import { hashKey } from '@tanstack/react-query';
import { queryKeys } from './queryKeys';

describe('queryKeys.view', () => {
  it('keeps the entry apart from a page literally named "entry"', () => {
    // Same hash would mean `?page=entry` reuses the entry's grant without asking.
    expect(hashKey(queryKeys.view('abcdefghijkm', null, null))).not.toBe(
      hashKey(queryKeys.view('abcdefghijkm', null, 'entry')),
    );
  });

  it('stays under the per-parcel prefix', () => {
    expect(queryKeys.view('abcdefghijkm', 2, 'two.html').slice(0, 2)).toEqual([
      ...queryKeys.allViews('abcdefghijkm'),
    ]);
  });
});
