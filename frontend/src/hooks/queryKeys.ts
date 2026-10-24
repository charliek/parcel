import type { ListFilters, TagFilters } from '@/lib/api';

/** React Query keys, in one place so invalidation stays consistent. */
export const queryKeys = {
  /** One filtered list (`GET /parcels`). */
  parcels: (filters: ListFilters) => ['parcels', filters] as const,
  allParcels: ['parcels'] as const,
  parcel: (id: string) => ['parcel', id] as const,
  allParcelDetails: ['parcel'] as const,
  /** A clean URL's resolution (`GET /parcels/by-slug`). */
  bySlug: (handle: string, slug: string) => ['by-slug', handle, slug] as const,
  allBySlug: ['by-slug'] as const,
  /**
   * A view grant: one page (null: the entry) of a version (null: the current
   * one). The entry stays `null` in the key: any string could be a page's path.
   */
  view: (id: string, version: number | null, page: string | null) =>
    ['view', id, version ?? 'current', page] as const,
  allViews: (id: string) => ['view', id] as const,
  /** Every view grant of every parcel. */
  everyView: ['view'] as const,
  version: (id: string, version: number) => ['version', id, version] as const,
  activity: (id: string) => ['activity', id] as const,
  /** Every thread on a parcel (`status=all`); the panel filters client-side. */
  comments: (id: string) => ['comments', id] as const,
  data: (id: string) => ['data', id] as const,
  /** The people a parcel is shared with (owner only). */
  shares: (id: string) => ['shares', id] as const,
  /** Tag counts for one set of filters (every page of `GET /tags`). */
  tags: (filters: TagFilters) => ['tags', filters] as const,
  allTags: ['tags'] as const,
  tokens: ['tokens'] as const,
  device: (code: string) => ['device', code] as const,
};
