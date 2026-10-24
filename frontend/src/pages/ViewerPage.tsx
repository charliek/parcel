import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  AlertCircle,
  ArrowLeft,
  Download,
  ExternalLink,
  FileQuestion,
  History,
  Link2,
  Loader2,
  Lock,
  MessageSquare,
  MousePointerClick,
  PanelRight,
  RotateCw,
  Share2,
  Sparkles,
} from 'lucide-react';
import {
  type MouseEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Link, useLocation, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { SignInButton } from '@/components/auth/SignInButton';
import { ArchivedBadge } from '@/components/common/ArchivedBadge';
import { copyText } from '@/components/common/CopyButton';
import { FullPageMessage } from '@/components/common/FullPage';
import { IconButton } from '@/components/common/IconButton';
import { RelativeTime } from '@/components/common/RelativeTime';
import { VisibilityBadge } from '@/components/common/VisibilityBadge';
import { Brand } from '@/components/layout/Brand';
import { ThemeMenu } from '@/components/layout/ThemeMenu';
import { UserMenu } from '@/components/layout/UserMenu';
import { TagChip } from '@/components/tags/TagChip';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { type CommentFilter, CommentsPanel } from '@/components/viewer/CommentsPanel';
import { ContentFrame } from '@/components/viewer/ContentFrame';
import { DeleteParcelDialog } from '@/components/viewer/DeleteParcelDialog';
import { DiscardDraftDialog } from '@/components/viewer/DiscardDraftDialog';
import { EditDetailsDialog } from '@/components/viewer/EditDetailsDialog';
import { InfoPanel } from '@/components/viewer/InfoPanel';
import { PagesMenu } from '@/components/viewer/PagesMenu';
import { ShareDialog } from '@/components/viewer/ShareDialog';
import { SignInToCommentDialog } from '@/components/viewer/SignInToCommentDialog';
import { UploadVersionDialog } from '@/components/viewer/UploadVersionDialog';
import { VersionMenu } from '@/components/viewer/VersionMenu';
import { type ViewerActions, ViewerMoreMenu } from '@/components/viewer/ViewerMoreMenu';
import { queryKeys } from '@/hooks/queryKeys';
import { useActivity } from '@/hooks/useActivity';
import { useComments, useCreateComment } from '@/hooks/useComments';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useFrameBridge } from '@/hooks/useFrameBridge';
import {
  useParcelDetail,
  useRestoreVersion,
  useUpdateParcel,
  useVersionDetail,
} from '@/hooks/useParcels';
import { useSession, useSignInTarget } from '@/hooks/useSession';
import { useChangeVisibility } from '@/hooks/useVisibility';
import { viewerAbilities, viewerPageState } from '@/lib/access';
import { api, downloadFile } from '@/lib/api';
import { type CommentDraft, newThread, pinsForPage } from '@/lib/comments';
import { isExpiringSoon } from '@/lib/format';
import { contentPath, contentToken, contentUrlFor, toPage } from '@/lib/frame-protocol';
import { errorMessage, errorStatus } from '@/lib/http';
import { isValidParcelId, parseVersionParam } from '@/lib/ids';
import { decideLiveUpdate, parseAutoReload, shouldApplyFreshGrant } from '@/lib/live-update';
import {
  addressForFrame,
  initialPageNav,
  nextPageNav,
  type PageNav,
  readPageParam,
  versionListing,
} from '@/lib/page-nav';
import { shareLink, type ViewerRoute, viewerPath } from '@/lib/refs';
import { dashboardHref } from '@/lib/search-query';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import type { Comment, Me, PageEntry, ViewGrant } from '@/lib/types';
import { useAllowPublic } from '@/stores/server-info.store';

/** Refetch a content URL that expires within this window before reusing it. */
const GRANT_REFRESH_WINDOW_MS = 5 * 60_000;

type Dialog = 'share' | 'edit' | 'upload' | 'delete' | null;

/** `/p/:id` and `/p/:id/v/:version`: the full-height sandboxed viewer. */
export function ViewerPage() {
  const params = useParams();
  const id = params.id ?? '';
  const parsedVersion = parseVersionParam(params.version);
  const valid = isValidParcelId(id) && parsedVersion !== 'invalid';
  const version = parsedVersion === 'invalid' ? null : parsedVersion;
  return <ParcelView id={id} version={version} valid={valid} route={{ kind: 'id', id }} />;
}

/** Where the frame is, as the viewer last heard (read when the address changes). */
interface FrameReport {
  framePath: string | null;
  entry: string | null;
}

/**
 * The viewer for one parcel, whichever address it was reached by (`/p/<id>`
 * or a clean `/@handle/slug`): the view grant for the version and page in
 * the address, and the page states (loading, sign in, not found).
 *
 * The grant is keyed by the page the viewer asked for, not by `?page=`
 * itself: when a link inside the page moves the frame, the address follows
 * (`replace`) but nothing is fetched and the frame isn't remounted
 * (lib/page-nav.ts).
 */
export function ParcelView({
  id,
  version,
  valid,
  route,
}: {
  id: string;
  /** A pinned version, or null for the current one. */
  version: number | null;
  /** The id and version are well-formed (otherwise: not found, no request). */
  valid: boolean;
  route: ViewerRoute;
}) {
  const [searchParams] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const client = useQueryClient();
  const urlPage = readPageParam(searchParams);

  const session = useSession();
  const canSignIn = useSignInTarget().kind !== 'none';

  const frame = useRef<FrameReport>({ framePath: null, entry: null });
  const [storedNav, setNav] = useState<PageNav>(() => initialPageNav(id, version, urlPage));
  // Adjusted while rendering (not in an effect) so the grant query below
  // never sees an address and a page request from different moments.
  const nav = nextPageNav(storedNav, { id, version, urlPage }, frame.current);
  if (nav !== storedNav) setNav(nav);

  // Signed in or not: the session cookie rides along when there is one, and
  // a public parcel renders without. What the caller may do comes back in
  // the grant (`access`).
  const view = useQuery({
    queryKey: queryKeys.view(id, version, nav.requested),
    queryFn: () => api.view(id, version, nav.requested),
    enabled: valid,
    staleTime: Number.POSITIVE_INFINITY,
    // A 401 here means the session cookie stopped working, and the server
    // cleared it in that same answer: asking again is signed out, so a
    // public parcel still shows. Otherwise the app-wide rule (no 4xx).
    retry: (failureCount, error) => {
      const status = errorStatus(error);
      return failureCount < 1 && (status === 401 || status === undefined || status >= 500);
    },
    retryDelay: (_attempt, error) => (errorStatus(error) === 401 ? 0 : 1000),
    // Keep showing the previous version or page of the same parcel while switching.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[1] === id ? keepPreviousData(previous) : undefined,
  });

  // The frame reloads for a new request only once its grant is here, so a
  // switch doesn't load the old page again in the meantime.
  const [frameNonce, setFrameNonce] = useState(nav.nonce);
  if (view.data && !view.isPlaceholderData && frameNonce !== nav.nonce) setFrameNonce(nav.nonce);

  // What the callbacks below read, so the ones effects depend on stay stable.
  const where = useRef({ route, version });
  where.current = { route, version };
  const onFrame = useCallback((path: string | null, entry: string) => {
    frame.current = { framePath: path, entry };
  }, []);
  const followFrame = useCallback(
    (page: string | null, hash: string) => {
      const { route: r, version: v } = where.current;
      navigate(viewerPath(r, { version: v, page, hash }), { replace: true });
    },
    [navigate],
  );

  const pageControl: PageControl = {
    route,
    urlPage,
    urlHash: location.hash,
    frameNonce,
    onFrame,
    followFrame,
    openPage: (page) => {
      navigate(viewerPath(route, { version, page, hash: '' }));
    },
    reloadPage: () => {
      setNav((n) => ({ ...n, requested: urlPage, nonce: n.nonce + 1 }));
    },
    applyGrant: (grant, page) => {
      client.setQueryData(queryKeys.view(id, null, page), grant);
      if (page !== urlPage) {
        // The page in view is gone from the new version: the entry, and an
        // address to match (the nav state follows the address).
        navigate(viewerPath(route, { version: null, page, hash: '' }), { replace: true });
      } else {
        setNav((n) => ({ ...n, requested: page, nonce: n.nonce + 1 }));
      }
    },
  };

  const state = valid
    ? viewerPageState({
        view: view.status,
        errorStatus: errorStatus(view.error),
        session: session.status,
        canSignIn,
      })
    : 'not-found';
  // A page that isn't in this version is a 404 too; say so whoever asks.
  const pageMissing =
    valid && nav.requested !== null && view.status === 'error' && errorStatus(view.error) === 404;

  useDocumentTitle(view.data?.parcel.title ?? (state === 'ready' ? null : 'Parcel'));

  if (state === 'ready' && view.data) {
    return (
      <ParcelViewer
        // Comment drafts, pins and poll state belong to one parcel.
        key={view.data.parcel.id}
        grant={view.data}
        pinnedVersion={version}
        switching={view.isPlaceholderData}
        refetchGrant={async () => (await view.refetch()).data}
        me={session.me}
        // The server saw no session (`public`); the tab may be about to learn the same.
        signInAvailable={
          canSignIn && view.data.access === 'public' && session.status !== 'authenticated'
        }
        pages={pageControl}
      />
    );
  }

  if (pageMissing) {
    return (
      <FullPageMessage
        icon={FileQuestion}
        title="Page not found"
        description={
          <>
            This version of the parcel has no page{' '}
            <span className="font-mono">{nav.requested}</span>, or the parcel isn’t shared with you.
          </>
        }
        actions={
          <>
            <Button asChild>
              <Link to={viewerPath(route, { version, page: null, hash: '' })}>
                Open the first page
              </Link>
            </Button>
            <HomeButton signedIn={session.signedIn} />
          </>
        }
      />
    );
  }

  if (state === 'loading') return <ViewerSkeleton />;

  if (state === 'sign-in') {
    return <SignInToView />;
  }

  if (state === 'not-found') {
    if (!valid || version === null) return <ParcelNotFound signedIn={session.signedIn} />;
    return (
      <FullPageMessage
        icon={FileQuestion}
        title={`Version ${version} isn't available`}
        description="This version does not exist, or the parcel is not shared with you."
        actions={
          <>
            <Button asChild>
              <Link to={viewerPath(route, { version: null, page: null, hash: '' })}>
                Open the current version
              </Link>
            </Button>
            <HomeButton signedIn={session.signedIn} />
          </>
        }
      />
    );
  }

  return <CouldNotOpen error={view.error} onRetry={() => void view.refetch()} />;
}

/** Signed out, and the parcel needs an account (or doesn't exist). */
export function SignInToView() {
  return (
    <FullPageMessage
      icon={Lock}
      title="Sign in to view this parcel"
      description="It’s private, open only to people signed in, or it doesn’t exist. Sign in to open it if it’s yours or was shared with you."
      actions={
        <>
          <SignInButton />
          <Button asChild variant="outline">
            <Link to="/">About Parcel</Link>
          </Button>
        </>
      }
    />
  );
}

/** Back to the dashboard (or the landing page, signed out). */
function HomeButton({ signedIn }: { signedIn: boolean }) {
  return (
    <Button asChild variant="outline">
      <Link to="/">{signedIn ? 'My parcels' : 'Go to Parcel'}</Link>
    </Button>
  );
}

/** Not found for a signed-in visitor (or where nobody can sign in). */
export function ParcelNotFound({ signedIn }: { signedIn: boolean }) {
  return (
    <FullPageMessage
      icon={FileQuestion}
      title="Parcel not found"
      description="It may have been deleted, or it isn’t shared with you. Check the link, or ask its owner to share it."
      actions={<HomeButton signedIn={signedIn} />}
    />
  );
}

export function CouldNotOpen({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  return (
    <FullPageMessage
      icon={AlertCircle}
      tone="destructive"
      title="Couldn't open this parcel"
      description={errorMessage(error, 'The server returned an unexpected error.')}
      actions={
        <Button variant="outline" onClick={onRetry}>
          Try again
        </Button>
      }
    />
  );
}

export function ViewerSkeleton() {
  return (
    <div className="flex h-dvh flex-col bg-background" role="status" aria-label="Loading parcel">
      <div className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
        <Skeleton className="h-7 w-7 rounded-lg" />
        <Skeleton className="h-4 w-48" />
        <Skeleton className="ml-auto h-7 w-24" />
      </div>
      <div className="flex flex-1 items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
      </div>
    </div>
  );
}

type Panel = 'details' | 'comments' | null;

function readPanel(): Panel {
  if (typeof window !== 'undefined' && window.innerWidth < 768) return null;
  return readStorage('local', STORAGE_KEYS.viewerPanel) === '1' ? 'details' : null;
}

/** Wide enough that a side panel sits beside the page instead of over it. */
function isWide(): boolean {
  return typeof window !== 'undefined' && window.matchMedia('(min-width: 768px)').matches;
}

/** Keys typed here are text, not shortcuts. */
function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName);
}

/** An open dialog or menu owns the keyboard. */
function overlayOpen(): boolean {
  return (
    document.querySelector(
      '[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"], [role="menu"][data-state="open"]',
    ) !== null
  );
}

const NO_NUMBERS: ReadonlySet<number> = new Set();

/** Where the viewer goes once an unsent comment may be discarded. */
type PendingSwitch =
  | { kind: 'page'; page: string; focus?: number }
  | { kind: 'href'; href: string };

/** How the viewer moves between pages and keeps its address in step (from `ParcelView`). */
interface PageControl {
  route: ViewerRoute;
  /** `?page=` (null: the entry). */
  urlPage: string | null;
  /** The address's `#fragment`, or empty. */
  urlHash: string;
  /** Bumped whenever the frame must load its grant's page afresh. */
  frameNonce: number;
  /** Where the frame is now (null while a page loads), and the shown version's entry. */
  onFrame: (path: string | null, entry: string) => void;
  /** The page moved by itself: rewrite the address to match (`replace`). */
  followFrame: (page: string | null, hash: string) => void;
  /** Open a page (null: the entry) with a new history entry. */
  openPage: (page: string | null) => void;
  /** Load the page in the address afresh. */
  reloadPage: () => void;
  /** Show a grant fetched for a live reload, for `page` (null: the entry). */
  applyGrant: (grant: ViewGrant, page: string | null) => void;
}

interface ParcelViewerProps {
  grant: ViewGrant;
  /** The version in the URL (`/v/N`), or null for "current". */
  pinnedVersion: number | null;
  /** True while a different version's grant loads (the old one is still shown). */
  switching: boolean;
  refetchGrant: () => Promise<ViewGrant | undefined>;
  me: Me | null;
  signInAvailable: boolean;
  pages: PageControl;
}

function ParcelViewer({
  grant,
  pinnedVersion,
  switching,
  refetchGrant,
  me,
  signInAvailable,
  pages: nav,
}: ParcelViewerProps) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const { parcel, version } = grant;
  const { isOwner, canReadComments, canComment } = viewerAbilities(grant.access);
  const allowPublic = useAllowPublic();
  const [panel, setPanel] = useState<Panel>(readPanel);
  const [dialog, setDialog] = useState<Dialog>(null);
  const update = useUpdateParcel(parcel.id);

  // ---- live updates: the poll, and what the frame should do about it

  const activity = useActivity(parcel.id, true);
  // Losing access (unshared, made private, deleted) shows up as a 404 on the
  // poll. Asking for the grant again turns the page into the not-found state
  // instead of leaving the old page and comments on screen; the server has
  // already stopped serving it anything more.
  const lostAccess = errorStatus(activity.error) === 404;
  const askedAgain = useRef(false);
  useEffect(() => {
    if (lostAccess && !askedAgain.current) {
      askedAgain.current = true;
      void refetchGrant();
    }
  }, [lostAccess, refetchGrant]);
  // The grant can be newer than the last poll (a publish from this tab).
  const latest = Math.max(parcel.current_version, activity.data?.current_version ?? 0);
  const pinned = pinnedVersion !== null;
  const isCurrent = version.version === latest;
  const [autoReload, setAutoReload] = useState(() =>
    parseAutoReload(readStorage('local', STORAGE_KEYS.viewerAutoReload)),
  );
  // Set when an automatic reload failed, so the banner offers it instead.
  const [autoReloadFailed, setAutoReloadFailed] = useState(false);

  const detail = useParcelDetail(parcel.id, true);
  // The detail lists the current version's files; any other version shown
  // (pinned, or a live update the detail hasn't caught up with, or one it
  // has moved past) needs its own listing.
  const detailIsOther =
    detail.data !== undefined &&
    !detail.isFetching &&
    detail.data.parcel.current_version !== version.version;
  const versionDetail = useVersionDetail(parcel.id, version.version, !isCurrent || detailIsOther);
  const restore = useRestoreVersion(parcel.id);
  const listing = versionListing(version.version, detail.data, versionDetail.data);
  // The pages of the version in the frame (null while loading): the only
  // paths a page report is believed for.
  const pages: PageEntry[] | null = listing?.pages ?? null;

  // ---- comments

  // The server says who may do what (`grant.access`): owners and commenters
  // read and write comments, viewers only read them, and signed-out visitors
  // of a public parcel see none (where sign-in is possible they are offered
  // it instead).
  const [commentMode, setCommentMode] = useState(false);
  const [filter, setFilter] = useState<CommentFilter>('open');
  const [active, setActive] = useState<number | null>(null);
  const [reveal, setReveal] = useState<{ number: number; nonce: number } | null>(null);
  // The spot being commented on, with the version it was picked on.
  const [draft, setDraft] = useState<CommentDraft | null>(null);
  const draftAnchor = draft?.anchor ?? null;
  const [composerText, setComposerText] = useState('');
  const [composingKeys, setComposingKeys] = useState<ReadonlySet<string>>(() => new Set());
  const [signInPrompt, setSignInPrompt] = useState(false);
  const composerRef = useRef<HTMLTextAreaElement>(null);

  const commentModeOn = canComment && commentMode;
  const commentsPanelOpen = canReadComments && panel === 'comments';
  const commentsOn = commentModeOn || commentsPanelOpen;
  const comments = useComments(parcel.id, commentsOn);
  const threads = comments.data?.comments ?? null;
  const createComment = useCreateComment(parcel.id);
  const openCount = activity.data?.open_comments ?? parcel.open_comments ?? 0;

  const onComposingChange = useCallback((key: string, on: boolean) => {
    setComposingKeys((prev) => {
      if (prev.has(key) === on) return prev;
      const next = new Set(prev);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);

  const composing = draft !== null || composerText.trim() !== '' || composingKeys.size > 0;
  const decision = decideLiveUpdate({
    shown: version.version,
    latest,
    pinned,
    composing,
    autoReload: autoReload && !autoReloadFailed,
  });

  // ---- the page in the frame (see lib/frame-protocol.ts)

  const frameKey = `${grant.content_url}#${nav.frameNonce}`;
  // The address's fragment goes to the frame when it loads (not after:
  // later changes come from the page itself).
  const [frameSrc, setFrameSrc] = useState({ key: frameKey, src: grant.content_url + nav.urlHash });
  if (frameSrc.key !== frameKey)
    setFrameSrc({ key: frameKey, src: grant.content_url + nav.urlHash });
  // The page's fragment, as it last reported it.
  const [frameHash, setFrameHash] = useState<{ epoch: number; hash: string } | null>(null);
  // A thread on another page to show once that page has loaded.
  const [pendingFocus, setPendingFocus] = useState<{ number: number; path: string } | null>(null);
  // A page or version switch waiting on "discard your comment?".
  const [pendingSwitch, setPendingSwitch] = useState<PendingSwitch | null>(null);
  const [located, setLocated] = useState<{ epoch: number; missing: ReadonlySet<number> } | null>(
    null,
  );

  // `live` is set only while the frame holds the parcel page we loaded
  // (verified by its token); until then nothing is sent to it but `hello`,
  // and nothing it sends is acted on.
  const { frameRef, live, post, onLoad } = useFrameBridge({
    frameKey,
    token: contentToken(grant.content_url),
    version: version.version,
    onMessage: (message, session) => {
      switch (message.type) {
        case 'anchor':
          // Only clicks made in comment mode; a page could send this anytime.
          if (!commentModeOn) return;
          // The comment belongs to the version this page was loaded from,
          // even if the frame moves on before it is sent.
          setDraft({ anchor: message.anchor, version: session.version });
          setPanel('comments');
          break;
        case 'pin-click': {
          if (!commentsOn) return;
          const thread = threads?.find((t) => t.number === message.number);
          if (!thread) return;
          setPanel('comments');
          setFilter(thread.resolved ? 'resolved' : 'open');
          setActive(thread.number);
          setReveal({ number: thread.number, nonce: Date.now() });
          break;
        }
        case 'located':
          if (live) setLocated({ epoch: live.epoch, missing: new Set(message.missing) });
          break;
        case 'escape':
          setCommentMode(false);
          break;
        case 'hash':
          setFrameHash({ epoch: session.epoch, hash: message.hash });
          break;
      }
    },
  });
  const liveEpoch = live?.epoch ?? null;
  const framePath = live?.path ?? null;
  // The page's own word for where it is, believed only for a page of the shown version.
  const knownFramePath =
    framePath !== null && pages?.some((p) => p.path === framePath) ? framePath : null;
  const shownHash = live ? (frameHash?.epoch === live.epoch ? frameHash.hash : live.hash) : '';
  const shownPage = knownFramePath ?? nav.urlPage ?? version.entry;
  const missing = live && located?.epoch === live.epoch ? located.missing : NO_NUMBERS;

  const pins = useMemo(
    () =>
      commentsOn && threads
        ? pinsForPage(threads, framePath, { includeResolved: filter === 'resolved', active })
        : [],
    [commentsOn, threads, framePath, filter, active],
  );

  // Each new page (every `ready`) gets the current state, then each change.
  useEffect(() => {
    if (liveEpoch !== null) post(toPage.commentMode(commentModeOn));
  }, [liveEpoch, commentModeOn, post]);
  useEffect(() => {
    if (liveEpoch !== null) post(toPage.pins(pins));
  }, [liveEpoch, pins, post]);
  useEffect(() => {
    if (liveEpoch !== null) post(toPage.draft(draftAnchor));
  }, [liveEpoch, draftAnchor, post]);

  // A thread opened from another page: focus it once that page proves itself.
  useEffect(() => {
    if (!pendingFocus || !live || live.path !== pendingFocus.path) return;
    if (!pages?.some((p) => p.path === live.path)) return;
    setPendingFocus(null);
    setActive(pendingFocus.number);
    setReveal({ number: pendingFocus.number, nonce: Date.now() });
    post(toPage.focus(pendingFocus.number));
  }, [live, pendingFocus, pages, post]);

  // ---- the address follows the page (lib/page-nav.ts)

  const { onFrame, followFrame, urlHash } = nav;
  useEffect(() => {
    onFrame(framePath, version.entry);
  }, [onFrame, framePath, version.entry]);

  // Only when the frame reports (a new page, a new fragment) or the shown
  // version's pages arrive or change: an address change of the viewer's own
  // (a Pages menu switch) must not be undone by the page that is about to
  // be replaced.
  const addressInputs = useRef({ urlPage: nav.urlPage, urlHash: nav.urlHash, switching });
  addressInputs.current = { urlPage: nav.urlPage, urlHash: nav.urlHash, switching };
  useEffect(() => {
    const now = addressInputs.current;
    if (!live || now.switching || pages === null) return;
    const change = addressForFrame({
      framePath: live.path,
      frameHash: shownHash,
      pages,
      entry: version.entry,
      urlPage: now.urlPage,
      urlHash: now.urlHash,
    });
    if (change) followFrame(change.page, change.hash);
  }, [live, shownHash, pages, version.entry, followFrame]);

  // The address's fragment changed by itself (Back/Forward to a
  // fragment-only entry, an edited address): move the page to it. Not when
  // the frame reports (the address follows the page then, so they agree).
  // A fragment-only `location.replace` scrolls the page without reloading
  // it (no `load`: the bridge stays live) or adding a history entry; the
  // helper reports the new fragment back as `hash`.
  const fragmentInputs = useRef({ knownFramePath, shownHash, urlPage: nav.urlPage, live });
  fragmentInputs.current = { knownFramePath, shownHash, urlPage: nav.urlPage, live };
  const contentUrl = grant.content_url;
  useEffect(() => {
    const now = fragmentInputs.current;
    if (!now.live || now.knownFramePath === null || urlHash === now.shownHash) return;
    // A page change in the address loads that page with its fragment.
    if ((now.urlPage ?? version.entry) !== now.knownFramePath) return;
    const url = contentUrlFor(contentUrl, now.knownFramePath);
    if (!url) return;
    try {
      // `#` alone (not no fragment) keeps it a fragment navigation: back to the top.
      frameRef.current?.contentWindow?.location.replace(url + (urlHash || '#'));
    } catch {
      // The frame is gone or somewhere else; it keeps its fragment.
    }
  }, [urlHash, contentUrl, version.entry, frameRef]);

  // A new spot to comment on: put the cursor in the composer.
  useEffect(() => {
    if (draftAnchor) composerRef.current?.focus();
  }, [draftAnchor]);

  // ---- reacting to the poll

  // A comment changed somewhere: another viewer, the CLI, an agent.
  const revision = activity.data?.comments_revision;
  useEffect(() => {
    if (revision !== undefined) {
      void client.invalidateQueries({ queryKey: queryKeys.comments(parcel.id) });
    }
  }, [revision, client, parcel.id]);

  // A new version: refresh the version list (and, lazily, the dashboard).
  const seenLatest = useRef(latest);
  useEffect(() => {
    if (latest > seenLatest.current) {
      void client.invalidateQueries({ queryKey: queryKeys.parcel(parcel.id) });
      void client.invalidateQueries({ queryKey: queryKeys.allParcels, refetchType: 'none' });
    }
    seenLatest.current = latest;
  }, [latest, client, parcel.id]);

  // Reloading to a new version is two steps. The grant is fetched, and only
  // when it arrives is it decided, against the state at that moment,
  // whether to apply it (put it in the view query, which remounts the
  // frame). The user may have started a comment while it was in flight.
  const [arrived, setArrived] = useState<{
    grant: ViewGrant;
    manual: boolean;
    /** The page in the address when it was asked for. */
    askedFor: string | null;
    /** The page the grant is for (the entry when that page is gone). */
    page: string | null;
  } | null>(null);
  const fetching = useRef(false);
  const fetchLatestGrant = useCallback(
    async (manual: boolean) => {
      if (fetching.current) return;
      fetching.current = true;
      const askedFor = addressInputs.current.urlPage;
      try {
        let page = askedFor;
        let fresh: ViewGrant;
        try {
          fresh = await api.view(parcel.id, null, page);
        } catch (err) {
          // The page isn't in the new version: its entry instead.
          if (page === null || errorStatus(err) !== 404) throw err;
          page = null;
          fresh = await api.view(parcel.id);
        }
        setArrived({ grant: fresh, manual, askedFor, page });
      } catch (err) {
        if (manual) toast.error('Could not reload', { description: errorMessage(err) });
        else setAutoReloadFailed(true);
      } finally {
        fetching.current = false;
      }
    },
    [parcel.id],
  );

  useEffect(() => {
    if (decision === 'reload') void fetchLatestGrant(false);
  }, [decision, fetchLatestGrant]);

  // Layout phase: nothing (a page message, a keystroke) can be handled
  // between the render that sees the arrived grant and this decision.
  useLayoutEffect(() => {
    if (!arrived) return;
    setArrived(null);
    // The user moved to another page meanwhile; that page's grant is current.
    if (arrived.askedFor !== nav.urlPage) return;
    const fresh = arrived.grant;
    const apply = shouldApplyFreshGrant({
      shown: version.version,
      fresh: fresh.version.version,
      pinned,
      composing,
      autoReload: autoReload && !autoReloadFailed,
      manual: arrived.manual,
    });
    if (apply) {
      nav.applyGrant(fresh, arrived.page);
      if (arrived.manual) setAutoReloadFailed(false);
      else toast.success(`Updated to v${fresh.version.version}`);
    } else if (
      !arrived.manual &&
      !pinned &&
      fresh.version.version <= version.version &&
      latest > version.version
    ) {
      // The poll says there is a newer version but the grant isn't one; offer
      // the reload rather than retrying in a loop. (A grant the frame has
      // already moved past, say after a publish from this tab, is just dropped.)
      setAutoReloadFailed(true);
    }
    // Otherwise the frame stays and the banner offers vN (the decision above
    // is now 'offer-reload', or the view is pinned).
  }, [arrived, version.version, latest, pinned, composing, autoReload, autoReloadFailed, nav]);

  const changeAutoReload = (on: boolean) => {
    setAutoReload(on);
    setAutoReloadFailed(false);
    writeStorage('local', STORAGE_KEYS.viewerAutoReload, on ? '1' : '0');
  };

  // ---- comment mode

  const startCommenting = () => {
    if (!canComment) {
      // Viewers get the threads, read-only.
      if (canReadComments) setPanel('comments');
      else if (signInAvailable) setSignInPrompt(true);
      return;
    }
    setCommentMode(true);
    // Beside the page on wide screens. On phones the panel would cover the
    // page, so it opens once there is something to show (a click, a pin).
    if (isWide()) setPanel('comments');
  };

  const clearDraft = () => {
    setDraft(null);
    setComposerText('');
  };

  /** The header button and `c`: comments on (mode + panel) or off. */
  const toggleComments = () => {
    if (!commentsOn) {
      startCommenting();
      return;
    }
    setCommentMode(false);
    if (panel === 'comments') setPanel(null);
    // A spot with nothing written about it yet goes; a written draft stays.
    if (!composerText.trim()) setDraft(null);
  };

  const activate = (thread: Comment) => {
    setActive(thread.number);
    post(toPage.focus(thread.number));
  };

  const hasDraft = draft !== null || composerText.trim() !== '';

  /** Open a page; an unsent comment is only thrown away once the user says so. */
  const switchPage = (path: string, focus?: number) => {
    if (path === shownPage && focus === undefined) return;
    if (hasDraft) {
      setPendingSwitch({ kind: 'page', page: path, focus });
      return;
    }
    goToPage(path, focus);
  };
  const goToPage = (path: string, focus?: number) => {
    if (focus !== undefined) {
      setActive(focus);
      setPendingFocus({ number: focus, path });
    }
    nav.openPage(path === version.entry ? null : path);
  };

  /** A thread on another page of this version: open it there. */
  const openThreadPage = (thread: Comment) => {
    const path = thread.anchor?.path;
    if (!path || !pages?.some((p) => p.path === path)) return;
    switchPage(path, thread.number);
  };

  const submitComment = () => {
    createComment.mutate(newThread(composerText, draft, version.version), {
      onSuccess: (thread) => {
        clearDraft();
        setFilter('open');
        setActive(thread.number);
        setReveal({ number: thread.number, nonce: Date.now() });
      },
      onError: (err) =>
        toast.error('Could not add the comment', { description: errorMessage(err) }),
    });
  };

  // `c` toggles comments and Escape leaves comment mode, unless the user is
  // typing or a dialog or menu has the keyboard. (While focus is inside the
  // frame, the page helper reports Escape instead.)
  const keyHandler = useRef<(event: KeyboardEvent) => void>(() => undefined);
  useEffect(() => {
    keyHandler.current = (event) => {
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return;
      if (isTypingTarget(event.target) || overlayOpen()) return;
      if (event.key === 'c' && (canReadComments || signInAvailable)) {
        event.preventDefault();
        toggleComments();
      } else if (event.key === 'Escape' && commentModeOn) {
        setCommentMode(false);
      }
    };
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => keyHandler.current(event);
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  // ---- everything else

  const files = listing?.files ?? null;
  const versions = detail.data?.versions ?? null;
  const pageParam = shownPage === version.entry ? null : shownPage;
  // What "Copy link" copies: the canonical URL with the version, page and
  // fragment in the address, which the page and the address keep in step
  // (the server names the parcel; lib/refs.ts adds the rest).
  const pageLink = shareLink(parcel, {
    version: pinnedVersion,
    page: nav.urlPage === version.entry ? null : nav.urlPage,
    hash: nav.urlHash,
  });
  const link = shareLink(parcel, { version: pinnedVersion, page: null, hash: '' });
  const visibility = useChangeVisibility(parcel, link);
  const versionHref = (v: number) =>
    viewerPath(nav.route, { version: v === latest ? null : v, page: pageParam, hash: '' });
  const currentHref = viewerPath(nav.route, { version: null, page: pageParam, hash: '' });
  // Another version keeps the page in view, which that version may not
  // have: its "page not found" replaces the viewer, unsent comment and all.
  // (The entry is in every version, and the viewer stays for it.)
  const versionSwitchLosesDraft = hasDraft && pageParam !== null;
  const openVersion = (href: string) => {
    if (versionSwitchLosesDraft) setPendingSwitch({ kind: 'href', href });
    else navigate(href);
  };
  /** The same for a link to another version (a plain click; others open a new tab). */
  const onVersionLinkClick = (event: MouseEvent<HTMLAnchorElement>, href: string) => {
    if (!versionSwitchLosesDraft || event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    setPendingSwitch({ kind: 'href', href });
  };
  const tagHref = me
    ? (tag: string) => dashboardHref({ scope: isOwner ? 'mine' : 'all', tags: [tag] })
    : null;
  const tags = parcel.tags ?? [];
  // The grant's summary predates the poll; show the newest current version.
  const shownParcel =
    parcel.current_version === latest ? parcel : { ...parcel, current_version: latest };

  const toggleDetails = () => {
    const next = panel === 'details' ? null : 'details';
    writeStorage('local', STORAGE_KEYS.viewerPanel, next ? '1' : '0');
    setPanel(next);
  };

  const actions: ViewerActions = {
    contentUrl: grant.content_url,
    reload: async () => {
      // Content URLs expire; get a fresh one when this one is close to it.
      if (isExpiringSoon(grant.expires_at, GRANT_REFRESH_WINDOW_MS)) {
        try {
          await refetchGrant();
        } catch {
          // Fall through to a plain reload; the frame will show its own error.
        }
      }
      nav.reloadPage();
    },
    copyLink: async () => {
      if (await copyText(pageLink)) toast.success('Link copied');
      else toast.error('Could not copy the link');
    },
    openInNewTab: (event: MouseEvent<HTMLAnchorElement>) => {
      // The grant's URL is for the page the viewer asked for; the page in
      // view may have moved on, and an old URL may be about to expire.
      const samePage = (framePath ?? version.entry) === contentPath(grant.content_url);
      if (samePage && !isExpiringSoon(grant.expires_at, GRANT_REFRESH_WINDOW_MS)) return;
      // Open synchronously (popup blockers), then point it at a fresh URL
      // without reloading the frame in this tab.
      event.preventDefault();
      const win = window.open('about:blank', '_blank');
      api
        .view(parcel.id, version.version, framePath)
        .then((fresh) => {
          if (!win) return;
          win.opener = null;
          win.location.href = fresh.content_url;
        })
        .catch((err) => {
          win?.close();
          toast.error('Could not open the parcel', { description: errorMessage(err) });
        });
    },
    downloadEntry: () => {
      downloadFile(parcel.id, version.version, shownPage).catch((err) =>
        toast.error('Download failed', { description: errorMessage(err) }),
      );
    },
    share: () => setDialog('share'),
    edit: () => setDialog('edit'),
    uploadVersion: () => setDialog('upload'),
    remove: () => setDialog('delete'),
    toggleArchived: () => {
      const archived = !parcel.archived_at;
      update.mutate(
        { archived },
        {
          onSuccess: () =>
            toast.success(archived ? 'Archived' : 'Unarchived', {
              description: archived
                ? 'It’s left out of lists and search; its links keep working.'
                : undefined,
              action: archived
                ? { label: 'Undo', onClick: () => update.mutate({ archived: false }) }
                : undefined,
            }),
          onError: (err) =>
            toast.error(archived ? 'Could not archive' : 'Could not unarchive', {
              description: errorMessage(err),
            }),
        },
      );
    },
    setVisibility: visibility.change,
    restore: () => {
      restore.mutate(version.version, {
        onSuccess: (result) => {
          toast.success(`Restored v${version.version} as v${result.version.version}`);
          navigate(currentHref);
        },
        onError: (err) => toast.error('Could not restore', { description: errorMessage(err) }),
      });
    },
  };

  // Viewers open the threads to read; everyone else who sees the button comments.
  const commentNoun = canReadComments && !canComment ? 'Comments' : 'Comment';
  const commentLabel =
    canReadComments && openCount > 0 ? `${commentNoun} (${openCount} open)` : commentNoun;

  return (
    <div className="flex h-dvh flex-col overflow-hidden bg-background">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b px-2 sm:gap-2 sm:px-3">
        {me ? (
          <IconButton label="My parcels" asChild>
            <Link to="/">
              <ArrowLeft />
            </Link>
          </IconButton>
        ) : (
          <Link
            to="/"
            aria-label="Parcel home"
            className="mr-1 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Brand compact />
          </Link>
        )}

        <div className="flex min-w-0 flex-1 items-center gap-2">
          <h1 className="truncate font-medium text-sm" title={parcel.title}>
            {parcel.title}
          </h1>
          {parcel.archived_at && (
            <ArchivedBadge className="shrink-0" data-testid="archived-badge" />
          )}
          <VisibilityBadge visibility={parcel.visibility} className="hidden sm:inline-flex" />
          {tags.length > 0 && (
            <ul className="hidden shrink-0 items-center gap-1 2xl:flex" aria-label="Tags">
              {tags.slice(0, 3).map((tag) => (
                <li key={tag} className="max-w-40">
                  <TagChip tag={tag} to={tagHref?.(tag)} />
                </li>
              ))}
              {tags.length > 3 && (
                <li className="shrink-0 text-muted-foreground text-xs">+{tags.length - 3}</li>
              )}
            </ul>
          )}
          {!isOwner && grant.owner_name && (
            <span
              className="hidden min-w-0 max-w-48 shrink truncate text-muted-foreground text-xs lg:inline"
              title={`Shared by ${grant.owner_name}`}
              data-testid="shared-by"
            >
              Shared by {grant.owner_name}
            </span>
          )}
          {versions && versions.length > 0 ? (
            <VersionMenu
              versions={versions}
              viewing={version.version}
              current={latest}
              hrefFor={versionHref}
              onOpen={openVersion}
            />
          ) : (
            <span className="shrink-0 px-2 font-mono text-muted-foreground text-xs">
              v{version.version}
            </span>
          )}
          {pages && pages.length > 1 && (
            <PagesMenu pages={pages} current={shownPage} onOpen={(path) => switchPage(path)} />
          )}
          {(switching || restore.isPending) && (
            <Loader2
              className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground"
              aria-label="Loading"
            />
          )}
        </div>

        <div className="flex shrink-0 items-center gap-0.5">
          <div className="hidden items-center gap-0.5 md:flex">
            <IconButton label="Reload" onClick={() => void actions.reload()}>
              <RotateCw />
            </IconButton>
            <IconButton label="Copy link" onClick={() => void actions.copyLink()}>
              <Link2 />
            </IconButton>
            <IconButton label="Open in new tab" asChild>
              <a
                href={grant.content_url}
                target="_blank"
                rel="noopener noreferrer"
                onClick={actions.openInNewTab}
              >
                <ExternalLink />
              </a>
            </IconButton>
            <IconButton label="Download page" onClick={actions.downloadEntry}>
              <Download />
            </IconButton>
          </div>
          {(canReadComments || signInAvailable) && (
            <IconButton
              label={commentLabel}
              aria-pressed={commentsOn}
              aria-keyshortcuts="c"
              variant={commentsOn ? 'secondary' : 'ghost'}
              className="relative"
              onClick={toggleComments}
            >
              <MessageSquare />
              {canReadComments && openCount > 0 && (
                <span
                  className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 font-semibold text-[10px] text-primary-foreground tabular-nums"
                  aria-hidden="true"
                  data-testid="open-comment-count"
                >
                  {openCount > 99 ? '99+' : openCount}
                </span>
              )}
            </IconButton>
          )}
          <IconButton
            label={panel === 'details' ? 'Hide details' : 'Show details'}
            aria-pressed={panel === 'details'}
            onClick={toggleDetails}
          >
            <PanelRight />
          </IconButton>
          {isOwner && (
            <Button
              variant="outline"
              size="sm"
              className="mx-1 hidden sm:inline-flex"
              onClick={actions.share}
            >
              <Share2 />
              Share
            </Button>
          )}
          <ViewerMoreMenu
            actions={actions}
            isOwner={isOwner}
            isCurrent={isCurrent || !pinned}
            archived={Boolean(parcel.archived_at)}
            visibility={visibility.shown}
            allowPublic={allowPublic}
            autoReload={autoReload}
            onAutoReloadChange={changeAutoReload}
          />
          {me ? (
            <UserMenu me={me} compact />
          ) : (
            <>
              <ThemeMenu />
              {signInAvailable && <SignInButton size="sm" variant="outline" className="ml-1" />}
            </>
          )}
        </div>
      </header>

      {commentModeOn && (
        <div
          className="flex shrink-0 items-center gap-x-3 gap-y-1 border-b bg-primary/5 px-3 py-1.5 text-sm sm:px-4"
          role="status"
        >
          <MousePointerClick className="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className="sm:hidden">Tap a spot to comment on it.</span>
            <span className="hidden sm:inline">Pick a spot on the page to comment on it.</span>
          </span>
          <div className="flex gap-1">
            {panel !== 'comments' && (
              <Button
                size="sm"
                variant="ghost"
                className="h-7"
                onClick={() => setPanel('comments')}
              >
                Show comments
              </Button>
            )}
            <Button size="sm" variant="ghost" className="h-7" onClick={() => setCommentMode(false)}>
              Done
            </Button>
          </div>
        </div>
      )}

      {decision === 'offer-reload' && (
        <div
          className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b bg-muted/50 px-3 py-2 text-sm sm:px-4"
          role="status"
        >
          <Sparkles className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className="font-medium">v{latest}</span> is available.
            {composing && ' Reload when you’ve finished your comment.'}
          </span>
          <Button size="sm" variant="outline" onClick={() => void fetchLatestGrant(true)}>
            <RotateCw />
            Reload
          </Button>
        </div>
      )}

      {pinned && !isCurrent && (
        <div
          className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b bg-muted/50 px-3 py-2 text-sm sm:px-4"
          role="status"
        >
          <History className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            Viewing <span className="font-medium">v{version.version}</span> from{' '}
            <RelativeTime value={version.created_at} />. The current version is v{latest}.
          </span>
          <div className="flex gap-2">
            {isOwner && (
              <Button
                size="sm"
                variant="outline"
                onClick={actions.restore}
                disabled={restore.isPending}
              >
                {restore.isPending && <Loader2 className="animate-spin" />}
                Restore this version
              </Button>
            )}
            <Button size="sm" variant="ghost" asChild>
              <Link to={currentHref} onClick={(event) => onVersionLinkClick(event, currentHref)}>
                View current
              </Link>
            </Button>
          </div>
        </div>
      )}

      <div className="relative flex min-h-0 flex-1">
        <ContentFrame
          key={frameKey}
          src={frameSrc.src}
          title={parcel.title}
          frameRef={frameRef}
          onLoad={onLoad}
        />
        {panel === 'details' && (
          <InfoPanel
            parcel={shownParcel}
            version={version}
            versions={versions}
            files={files}
            isOwner={isOwner}
            access={grant.access}
            ownerName={grant.owner_name}
            onEdit={actions.edit}
            onClose={toggleDetails}
            versionHref={versionHref}
            onVersionLinkClick={onVersionLinkClick}
            pages={pages}
            currentPage={shownPage}
            onOpenPage={(path) => switchPage(path)}
            link={pageLink}
            tagHref={tagHref}
          />
        )}
        {commentsPanelOpen && (
          <CommentsPanel
            parcelId={parcel.id}
            threads={threads}
            error={comments.error}
            onRetry={() => void comments.refetch()}
            filter={filter}
            onFilterChange={setFilter}
            isOwner={isOwner}
            canComment={canComment}
            viewedVersion={version.version}
            framePath={framePath}
            missing={missing}
            active={active}
            onActivate={activate}
            pages={pages}
            onOpenThreadPage={openThreadPage}
            reveal={reveal}
            commentMode={commentModeOn}
            onStartCommenting={() => setCommentMode(true)}
            onClose={() => setPanel(null)}
            onComposingChange={onComposingChange}
            draftAnchor={draftAnchor}
            draftVersion={draft?.version ?? null}
            composerText={composerText}
            onComposerTextChange={setComposerText}
            onCancelDraft={clearDraft}
            onSubmit={submitComment}
            submitting={createComment.isPending}
            composerRef={composerRef}
          />
        )}
      </div>

      <DiscardDraftDialog
        open={pendingSwitch !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSwitch(null);
        }}
        onDiscard={() => {
          const next = pendingSwitch;
          setPendingSwitch(null);
          clearDraft();
          if (next?.kind === 'page') goToPage(next.page, next.focus);
          else if (next) navigate(next.href);
        }}
      />

      {!canReadComments && signInAvailable && (
        <SignInToCommentDialog open={signInPrompt} onOpenChange={setSignInPrompt} />
      )}

      {isOwner && (
        <>
          <ShareDialog
            parcel={parcel}
            link={link}
            open={dialog === 'share'}
            onOpenChange={(open) => setDialog(open ? 'share' : null)}
          />
          <EditDetailsDialog
            parcel={parcel}
            open={dialog === 'edit'}
            onOpenChange={(open) => setDialog(open ? 'edit' : null)}
          />
          <UploadVersionDialog
            parcel={parcel}
            currentFiles={detail.data?.files ?? []}
            open={dialog === 'upload'}
            onOpenChange={(open) => setDialog(open ? 'upload' : null)}
          />
          <DeleteParcelDialog
            parcel={parcel}
            open={dialog === 'delete'}
            onOpenChange={(open) => setDialog(open ? 'delete' : null)}
          />
        </>
      )}
    </div>
  );
}
