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
import { Link, useNavigate, useParams } from 'react-router-dom';
import { toast } from 'sonner';
import { SignInButton } from '@/components/auth/SignInButton';
import { copyText } from '@/components/common/CopyButton';
import { FullPageMessage } from '@/components/common/FullPage';
import { IconButton } from '@/components/common/IconButton';
import { RelativeTime } from '@/components/common/RelativeTime';
import { VisibilityBadge } from '@/components/common/VisibilityBadge';
import { Brand } from '@/components/layout/Brand';
import { ThemeMenu } from '@/components/layout/ThemeMenu';
import { UserMenu } from '@/components/layout/UserMenu';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { type CommentFilter, CommentsPanel } from '@/components/viewer/CommentsPanel';
import { ContentFrame } from '@/components/viewer/ContentFrame';
import { DeleteParcelDialog } from '@/components/viewer/DeleteParcelDialog';
import { EditDetailsDialog } from '@/components/viewer/EditDetailsDialog';
import { InfoPanel } from '@/components/viewer/InfoPanel';
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
import { useSession } from '@/hooks/useSession';
import { useSilentReauth } from '@/hooks/useSilentReauth';
import { api, downloadFile } from '@/lib/api';
import { hasSignedInHint } from '@/lib/auth';
import { viewerAccess } from '@/lib/auth/policy';
import { type CommentDraft, newThread, pinsForPage } from '@/lib/comments';
import { isExpiringSoon } from '@/lib/format';
import { contentToken, toPage } from '@/lib/frame-protocol';
import { errorMessage, errorStatus } from '@/lib/http';
import { isValidParcelId, parseVersionParam } from '@/lib/ids';
import { decideLiveUpdate, parseAutoReload, shouldApplyFreshGrant } from '@/lib/live-update';
import { silentReauthTried } from '@/lib/oauth';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import type { Comment, Me, ViewGrant } from '@/lib/types';
import { useServerInfoStore } from '@/stores/server-info.store';

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

  const session = useSession();
  const mode = useServerInfoStore((s) => s.info?.auth.mode);

  // Anonymous first: a public parcel renders without any sign-in. A token is
  // attached only if this tab already has one.
  const view = useQuery({
    queryKey: queryKeys.view(id, version),
    queryFn: () => api.view(id, version ?? undefined),
    enabled: valid,
    staleTime: Number.POSITIVE_INFINITY,
    // Keep showing the previous version of the same parcel while switching.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[1] === id ? keepPreviousData(previous) : undefined,
  });

  const access = valid
    ? viewerAccess({
        view: view.status,
        errorStatus: errorStatus(view.error),
        session: session.status,
        mode,
        silentTried: silentReauthTried(),
        signedInHint: hasSignedInHint(),
      })
    : 'not-found';
  const silent = useSilentReauth(access === 'silent-reauth');

  useDocumentTitle(view.data?.parcel.title ?? (access === 'ready' ? null : 'Parcel'));

  if (access === 'ready' && view.data) {
    return (
      <ParcelViewer
        // Comment drafts, pins and poll state belong to one parcel.
        key={view.data.parcel.id}
        grant={view.data}
        pinnedVersion={version}
        switching={view.isPlaceholderData}
        refetchGrant={async () => (await view.refetch()).data}
        me={session.me}
        signInAvailable={mode === 'slauth' && session.status !== 'authenticated'}
      />
    );
  }

  if (
    access === 'loading' ||
    silent === 'redirecting' ||
    (access === 'silent-reauth' && silent === 'idle')
  ) {
    return <ViewerSkeleton />;
  }

  if (access === 'sign-in' || access === 'silent-reauth') {
    return (
      <FullPageMessage
        icon={Lock}
        title="Sign in to view this parcel"
        description="This parcel is private, or it doesn't exist. If it's yours, sign in to open it."
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

  if (access === 'not-found') {
    const versionMissing = valid && version !== null;
    return (
      <FullPageMessage
        icon={FileQuestion}
        title={versionMissing ? `Version ${version} isn't available` : 'Parcel not found'}
        description={
          versionMissing
            ? 'This version does not exist, or the parcel is not shared with you.'
            : 'It may have been deleted, or it is private to someone else. Check the link and try again.'
        }
        actions={
          <>
            {versionMissing && (
              <Button asChild>
                <Link to={`/p/${id}`}>Open the current version</Link>
              </Button>
            )}
            <Button asChild variant="outline">
              <Link to="/">{session.signedIn ? 'My parcels' : 'Go to Parcel'}</Link>
            </Button>
          </>
        }
      />
    );
  }

  return (
    <FullPageMessage
      icon={AlertCircle}
      tone="destructive"
      title="Couldn't open this parcel"
      description={errorMessage(view.error, 'The server returned an unexpected error.')}
      actions={
        <Button variant="outline" onClick={() => void view.refetch()}>
          Try again
        </Button>
      }
    />
  );
}

function ViewerSkeleton() {
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

interface ParcelViewerProps {
  grant: ViewGrant;
  /** The version in the URL (`/v/N`), or null for "current". */
  pinnedVersion: number | null;
  /** True while a different version's grant loads (the old one is still shown). */
  switching: boolean;
  refetchGrant: () => Promise<ViewGrant | undefined>;
  me: Me | null;
  signInAvailable: boolean;
}

function ParcelViewer({
  grant,
  pinnedVersion,
  switching,
  refetchGrant,
  me,
  signInAvailable,
}: ParcelViewerProps) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const { parcel, version, is_owner: isOwner } = grant;
  const [panel, setPanel] = useState<Panel>(readPanel);
  const [frameNonce, setFrameNonce] = useState(0);
  const [dialog, setDialog] = useState<Dialog>(null);

  // ---- live updates: the poll, and what the frame should do about it

  const activity = useActivity(parcel.id, true);
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
  const versionDetail = useVersionDetail(parcel.id, version.version, !isCurrent);
  const update = useUpdateParcel(parcel.id);
  const restore = useRestoreVersion(parcel.id);

  // ---- comments

  // The owner, and signed-in viewers of a public parcel (the only other
  // people who can open it), may read and write comments. Anonymous viewers
  // see none; where sign-in is possible they are offered it instead.
  const canComment = me !== null;
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
  const commentsPanelOpen = canComment && panel === 'comments';
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

  const frameKey = `${grant.content_url}#${frameNonce}`;
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
      }
    },
  });
  const liveEpoch = live?.epoch ?? null;
  const framePath = live?.path ?? null;
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
  const [arrived, setArrived] = useState<{ grant: ViewGrant; manual: boolean } | null>(null);
  const fetching = useRef(false);
  const fetchLatestGrant = useCallback(
    async (manual: boolean) => {
      if (fetching.current) return;
      fetching.current = true;
      try {
        setArrived({ grant: await api.view(parcel.id), manual });
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
      client.setQueryData(queryKeys.view(parcel.id, null), fresh);
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
  }, [
    arrived,
    version.version,
    latest,
    pinned,
    composing,
    autoReload,
    autoReloadFailed,
    client,
    parcel.id,
  ]);

  const changeAutoReload = (on: boolean) => {
    setAutoReload(on);
    setAutoReloadFailed(false);
    writeStorage('local', STORAGE_KEYS.viewerAutoReload, on ? '1' : '0');
  };

  // ---- comment mode

  const startCommenting = () => {
    if (!canComment) {
      if (signInAvailable) setSignInPrompt(true);
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
      if (event.key === 'c' && (canComment || signInAvailable)) {
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

  const files = isCurrent ? (detail.data?.files ?? null) : (versionDetail.data?.files ?? null);
  const versions = detail.data?.versions ?? null;
  const baseUrl = parcel.url || `${window.location.origin}/p/${parcel.id}`;
  const link = pinnedVersion !== null ? `${baseUrl}/v/${pinnedVersion}` : baseUrl;
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
      setFrameNonce((n) => n + 1);
    },
    copyLink: async () => {
      if (await copyText(link)) toast.success('Link copied');
      else toast.error('Could not copy the link');
    },
    openInNewTab: (event: MouseEvent<HTMLAnchorElement>) => {
      if (!isExpiringSoon(grant.expires_at, GRANT_REFRESH_WINDOW_MS)) return;
      // Open synchronously (popup blockers), then point it at a fresh URL
      // without reloading the frame in this tab.
      event.preventDefault();
      const win = window.open('about:blank', '_blank');
      api
        .view(parcel.id, version.version)
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
      downloadFile(parcel.id, version.version, version.entry).catch((err) =>
        toast.error('Download failed', { description: errorMessage(err) }),
      );
    },
    share: () => setDialog('share'),
    edit: () => setDialog('edit'),
    uploadVersion: () => setDialog('upload'),
    remove: () => setDialog('delete'),
    toggleVisibility: () => {
      const next = parcel.visibility === 'public' ? 'private' : 'public';
      update.mutate(
        { visibility: next },
        {
          onSuccess: () =>
            toast.success(next === 'public' ? 'Parcel is now public' : 'Parcel is now private', {
              description:
                next === 'public'
                  ? 'Anyone with the link can view it.'
                  : 'Only you can view it now.',
              action:
                next === 'public'
                  ? { label: 'Copy link', onClick: () => void copyText(link) }
                  : undefined,
            }),
          onError: (err) =>
            toast.error('Could not change visibility', { description: errorMessage(err) }),
        },
      );
    },
    restore: () => {
      restore.mutate(version.version, {
        onSuccess: (result) => {
          toast.success(`Restored v${version.version} as v${result.version.version}`);
          navigate(`/p/${parcel.id}`);
        },
        onError: (err) => toast.error('Could not restore', { description: errorMessage(err) }),
      });
    },
  };

  const commentLabel = canComment && openCount > 0 ? `Comment (${openCount} open)` : 'Comment';

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
          <VisibilityBadge visibility={parcel.visibility} className="hidden sm:inline-flex" />
          {versions && versions.length > 0 ? (
            <VersionMenu
              parcelId={parcel.id}
              versions={versions}
              viewing={version.version}
              current={latest}
            />
          ) : (
            <span className="shrink-0 px-2 font-mono text-muted-foreground text-xs">
              v{version.version}
            </span>
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
          {(canComment || signInAvailable) && (
            <IconButton
              label={commentLabel}
              aria-pressed={commentsOn}
              aria-keyshortcuts="c"
              variant={commentsOn ? 'secondary' : 'ghost'}
              className="relative"
              onClick={toggleComments}
            >
              <MessageSquare />
              {canComment && openCount > 0 && (
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
            visibility={parcel.visibility}
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
              <Link to={`/p/${parcel.id}`}>View current</Link>
            </Button>
          </div>
        </div>
      )}

      <div className="relative flex min-h-0 flex-1">
        <ContentFrame
          key={frameKey}
          src={grant.content_url}
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
            onEdit={actions.edit}
            onClose={toggleDetails}
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
            viewedVersion={version.version}
            framePath={framePath}
            missing={missing}
            active={active}
            onActivate={activate}
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

      {!canComment && signInAvailable && (
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
