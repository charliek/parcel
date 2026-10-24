import { describe, expect, it } from 'bun:test';
import { QueryClient } from '@tanstack/react-query';
import { queryKeys } from './queryKeys';
import { forgetParcel, refreshSlugResolutions } from './useParcels';

describe('forgetParcel', () => {
  it('drops every cached clean-URL resolution, so a reused slug is asked again', () => {
    const client = new QueryClient();
    const detail = { parcel: { id: 'abcdefghijkm' } };
    client.setQueryData(queryKeys.parcel('abcdefghijkm'), detail);
    // The deleted parcel's slug (and an old name of it) resolve to it...
    client.setQueryData(queryKeys.bySlug('ada', 'plan'), detail);
    client.setQueryData(queryKeys.bySlug('ada', 'old-plan'), detail);
    // ...and so might another name, which the cache can't tie to an id.
    client.setQueryData(queryKeys.bySlug('ada', 'other'), { parcel: { id: 'nnnnnnnnnnnn' } });

    forgetParcel(client, 'abcdefghijkm');

    expect(client.getQueryData(queryKeys.parcel('abcdefghijkm'))).toBeUndefined();
    expect(client.getQueryData(queryKeys.bySlug('ada', 'plan'))).toBeUndefined();
    expect(client.getQueryData(queryKeys.bySlug('ada', 'old-plan'))).toBeUndefined();
    expect(client.getQueryData(queryKeys.bySlug('ada', 'other'))).toBeUndefined();
  });
});

describe('refreshSlugResolutions', () => {
  it('drops resolutions nobody shows, so a name taken over by a PATCH is asked again', async () => {
    const client = new QueryClient();
    // `plan` was A's slug until A was renamed; B has just taken it.
    client.setQueryData(queryKeys.bySlug('ada', 'plan'), { parcel: { id: 'aaaaaaaaaaaa' } });

    await refreshSlugResolutions(client);

    // Stale-but-kept data would route the slug viewer to A while it refetched.
    expect(client.getQueryData(queryKeys.bySlug('ada', 'plan'))).toBeUndefined();
  });
});
