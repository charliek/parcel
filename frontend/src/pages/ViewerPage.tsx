import { keepPreviousData, useQuery } from '@tanstack/react-query';
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
  PanelRight,
  RotateCw,
  Share2,
} from 'lucide-react';
import { type MouseEvent, useState } from 'react';
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
import { ContentFrame } from '@/components/viewer/ContentFrame';
import { DeleteParcelDialog } from '@/components/viewer/DeleteParcelDialog';
import { EditDetailsDialog } from '@/components/viewer/EditDetailsDialog';
import { InfoPanel } from '@/components/viewer/InfoPanel';
import { ShareDialog } from '@/components/viewer/ShareDialog';
import { UploadVersionDialog } from '@/components/viewer/UploadVersionDialog';
import { VersionMenu } from '@/components/viewer/VersionMenu';
import { type ViewerActions, ViewerMoreMenu } from '@/components/viewer/ViewerMoreMenu';
import { queryKeys } from '@/hooks/queryKeys';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
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
import { isExpiringSoon } from '@/lib/format';
import { errorMessage, errorStatus } from '@/lib/http';
import { isValidParcelId, parseVersionParam } from '@/lib/ids';
import { silentReauthTried } from '@/lib/oauth';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import type { Me, ViewGrant } from '@/lib/types';
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

function readPanelOpen(): boolean {
  if (typeof window !== 'undefined' && window.innerWidth < 768) return false;
  return readStorage('local', STORAGE_KEYS.viewerPanel) === '1';
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
  const { parcel, version, is_owner: isOwner } = grant;
  const [panelOpen, setPanelOpen] = useState(readPanelOpen);
  const [frameNonce, setFrameNonce] = useState(0);
  const [dialog, setDialog] = useState<Dialog>(null);

  const isCurrent = version.version === parcel.current_version;
  const detail = useParcelDetail(parcel.id, true);
  const versionDetail = useVersionDetail(parcel.id, version.version, !isCurrent);
  const update = useUpdateParcel(parcel.id);
  const restore = useRestoreVersion(parcel.id);

  const files = isCurrent ? (detail.data?.files ?? null) : (versionDetail.data?.files ?? null);
  const versions = detail.data?.versions ?? null;
  const baseUrl = parcel.url || `${window.location.origin}/p/${parcel.id}`;
  const link = pinnedVersion !== null ? `${baseUrl}/v/${pinnedVersion}` : baseUrl;

  const togglePanel = () => {
    setPanelOpen((open) => {
      writeStorage('local', STORAGE_KEYS.viewerPanel, open ? '0' : '1');
      return !open;
    });
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
              current={parcel.current_version}
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
          <IconButton
            label={panelOpen ? 'Hide details' : 'Show details'}
            aria-pressed={panelOpen}
            onClick={togglePanel}
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
            isCurrent={isCurrent}
            visibility={parcel.visibility}
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

      {!isCurrent && (
        <div
          className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b bg-muted/50 px-3 py-2 text-sm sm:px-4"
          role="status"
        >
          <History className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            Viewing <span className="font-medium">v{version.version}</span> from{' '}
            <RelativeTime value={version.created_at} />. The current version is v
            {parcel.current_version}.
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
          key={`${grant.content_url}#${frameNonce}`}
          src={grant.content_url}
          title={parcel.title}
        />
        {panelOpen && (
          <InfoPanel
            parcel={parcel}
            version={version}
            versions={versions}
            files={files}
            isOwner={isOwner}
            onEdit={actions.edit}
            onClose={togglePanel}
          />
        )}
      </div>

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
