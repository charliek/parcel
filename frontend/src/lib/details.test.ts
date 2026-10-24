import { describe, expect, it } from 'bun:test';
import { detailsPatch } from './details';
import type { ParcelSummary } from './types';

const parcel = {
  id: 'abcdefghijkm',
  title: 'Plan',
  description: 'About it',
  tags: ['auth', 'kind:plan'],
  slug: 'roost/plan',
} as ParcelSummary;

const same = {
  title: 'Plan',
  description: 'About it',
  tags: ['kind:plan', 'auth'],
  slug: 'roost/plan',
};

describe('detailsPatch', () => {
  it('sends nothing when nothing changed (tag order aside)', () => {
    expect(detailsPatch(parcel, same)).toEqual({});
  });

  it('sends only what changed', () => {
    expect(detailsPatch(parcel, { ...same, title: 'Plan v2', tags: ['auth'] })).toEqual({
      title: 'Plan v2',
      tags: ['auth'],
    });
  });

  it('sends an empty tag set and an empty slug to clear them', () => {
    const body = detailsPatch(parcel, { ...same, tags: [], slug: '' });
    expect(body).toEqual({ tags: [], slug: '' });
    expect(JSON.stringify(body)).toBe('{"tags":[],"slug":""}');
  });

  it('treats a parcel without tags or slug as empty', () => {
    const bare = { ...parcel, tags: undefined, slug: null };
    expect(detailsPatch(bare, { ...same, tags: [], slug: '' })).toEqual({});
  });
});
