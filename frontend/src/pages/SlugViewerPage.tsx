import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { queryKeys } from '@/hooks/queryKeys';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useSession, useSignInTarget } from '@/hooks/useSession';
import { viewerPageState } from '@/lib/access';
import { api } from '@/lib/api';
import { errorStatus } from '@/lib/http';
import { parseVersionParam } from '@/lib/ids';
import { readPageParam } from '@/lib/page-nav';
import { canonicalPath, parseSlugPath, type ViewerRoute, viewerPath } from '@/lib/refs';
import { NotFoundPage } from './NotFoundPage';
import {
  CouldNotOpen,
  ParcelNotFound,
  ParcelView,
  SignInToView,
  ViewerSkeleton,
} from './ViewerPage';

/**
 * Every path the other routes don't claim: a clean parcel URL
 * (`/@handle/slug`) or the not-found page. (A route can't match a segment
 * that merely starts with `@`, so this catch-all checks.)
 */
export function CatchAllPage() {
  const { pathname } = useLocation();
  const ref = parseSlugPath(pathname);
  if (!ref) return <NotFoundPage />;
  return <SlugViewerPage handle={ref.handle} slug={ref.slug} />;
}

/**
 * `/@handle/slug[?v=&page=][#fragment]`: resolve the name through
 * `GET /parcels/by-slug`, then show the same viewer as `/p/<id>`. When the
 * name was an old one (a renamed slug or handle), the address is replaced
 * with the parcel's canonical one, keeping `?v=`, `?page=` and the fragment.
 */
function SlugViewerPage({ handle, slug }: { handle: string; slug: string }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const location = useLocation();
  const [params] = useSearchParams();
  const session = useSession();
  const canSignIn = useSignInTarget().kind !== 'none';
  const parsedVersion = parseVersionParam(params.get('v'));

  const resolved = useQuery({
    queryKey: queryKeys.bySlug(handle, slug),
    queryFn: async () => {
      const detail = await api.bySlug(handle, slug);
      // The viewer asks for the same detail by id; and the canonical
      // address resolves to it without asking again.
      client.setQueryData(queryKeys.parcel(detail.parcel.id), detail);
      const canonical = parseSlugPath(canonicalPath(detail.parcel));
      if (canonical)
        client.setQueryData(queryKeys.bySlug(canonical.handle, canonical.slug), detail);
      return detail;
    },
    staleTime: 60_000,
  });

  const target = resolved.data ? canonicalPath(resolved.data.parcel) : null;
  const id = resolved.data?.parcel.id ?? '';
  const route = useMemo<ViewerRoute>(
    () =>
      target?.startsWith('/@')
        ? { kind: 'slug', path: target }
        : // A cleared slug's old name still resolves; its address is `/p/<id>`.
          { kind: 'id', id },
    [target, id],
  );
  const version = parsedVersion === 'invalid' ? null : parsedVersion;
  const page = readPageParam(params);
  useEffect(() => {
    if (target && target !== location.pathname) {
      navigate(viewerPath(route, { version, page, hash: location.hash }), { replace: true });
    }
  }, [target, location.pathname, location.hash, route, version, page, navigate]);

  const state = viewerPageState({
    view: resolved.status,
    errorStatus: errorStatus(resolved.error),
    session: session.status,
    canSignIn,
  });
  useDocumentTitle(resolved.data?.parcel.title ?? (state === 'ready' ? null : 'Parcel'));

  if (state === 'ready' && resolved.data) {
    return (
      <ParcelView id={id} version={version} valid={parsedVersion !== 'invalid'} route={route} />
    );
  }
  if (state === 'loading') return <ViewerSkeleton />;
  if (state === 'sign-in') return <SignInToView />;
  if (state === 'not-found') return <ParcelNotFound signedIn={session.signedIn} />;
  return <CouldNotOpen error={resolved.error} onRetry={() => void resolved.refetch()} />;
}
