/** React Query keys, in one place so invalidation stays consistent. */
export const queryKeys = {
  parcels: (q: string) => ['parcels', q] as const,
  allParcels: ['parcels'] as const,
  parcel: (id: string) => ['parcel', id] as const,
  view: (id: string, version: number | null) => ['view', id, version ?? 'current'] as const,
  allViews: (id: string) => ['view', id] as const,
  version: (id: string, version: number) => ['version', id, version] as const,
  activity: (id: string) => ['activity', id] as const,
  /** Every thread on a parcel (`status=all`); the panel filters client-side. */
  comments: (id: string) => ['comments', id] as const,
  data: (id: string) => ['data', id] as const,
  tokens: ['tokens'] as const,
  device: (code: string) => ['device', code] as const,
};
