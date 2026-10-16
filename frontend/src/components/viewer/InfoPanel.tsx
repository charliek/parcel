import { Database, Download, FileIcon, Pencil, X } from 'lucide-react';
import { Link } from 'react-router-dom';
import { toast } from 'sonner';
import { CopyButton } from '@/components/common/CopyButton';
import { RelativeTime } from '@/components/common/RelativeTime';
import { VisibilityBadge } from '@/components/common/VisibilityBadge';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { useDataSummary } from '@/hooks/useParcelData';
import { accessLabel } from '@/lib/access';
import { downloadFile } from '@/lib/api';
import { formatBytes, percentOf, pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import type { FileEntry, ParcelAccess, ParcelSummary, VersionSummary } from '@/lib/types';
import { cn } from '@/lib/utils';
import { versionPath } from './VersionMenu';

interface InfoPanelProps {
  parcel: ParcelSummary;
  version: VersionSummary;
  /** Newest first; null while loading. */
  versions: VersionSummary[] | null;
  /** Files of the version being viewed; null while loading. */
  files: FileEntry[] | null;
  isOwner: boolean;
  /** What the caller may do (shown to everyone but the owner). */
  access: ParcelAccess;
  /** The owner's display name, for signed-in callers. */
  ownerName?: string;
  onEdit: () => void;
  onClose: () => void;
}

function Meta({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right">{children}</dd>
    </>
  );
}

function shortType(contentType: string): string {
  return contentType.split(';')[0];
}

const SECTION_HEADING = 'font-medium text-muted-foreground text-xs uppercase tracking-wide';

/** The parcel's stored data (what `window.parcel.data` keeps), when it has any. */
function DataSection({ parcelId }: { parcelId: string }) {
  const { data } = useDataSummary(parcelId, true);
  if (!data || data.collections.length === 0) return null;
  const percent = percentOf(data.usage_bytes, data.limit_bytes);
  return (
    <section className="space-y-2" aria-label="Data">
      <h3 className={SECTION_HEADING}>Data</h3>
      <div className="space-y-1.5">
        {/* Decorative: the line below says the same in words. */}
        <div className="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
          <div
            className={cn('h-full rounded-full', percent >= 90 ? 'bg-destructive' : 'bg-primary')}
            style={{ width: `${percent}%` }}
          />
        </div>
        <p className="text-muted-foreground text-xs">
          {formatBytes(data.usage_bytes)} of {formatBytes(data.limit_bytes)} used
        </p>
      </div>
      <ul className="divide-y rounded-lg border" aria-label="Data collections">
        {data.collections.map((c) => (
          <li key={c.name} className="flex items-center gap-2 py-1.5 pr-3 pl-3">
            <Database className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate font-mono text-xs" title={c.name}>
              {c.name}
            </span>
            <span className="shrink-0 text-muted-foreground text-xs">
              {pluralize(c.count, 'item')} · {formatBytes(c.usage_bytes)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Description, files and history for the parcel in view. */
export function InfoPanel({
  parcel,
  version,
  versions,
  files,
  isOwner,
  access,
  ownerName,
  onEdit,
  onClose,
}: InfoPanelProps) {
  const download = (path: string) => {
    downloadFile(parcel.id, version.version, path).catch((err) =>
      toast.error('Download failed', { description: errorMessage(err) }),
    );
  };

  return (
    <aside
      className="absolute inset-y-0 right-0 z-20 flex w-full max-w-sm flex-col border-l bg-background shadow-xl md:static md:w-80 md:max-w-none md:shadow-none"
      aria-label="Parcel details"
    >
      <div className="flex h-11 shrink-0 items-center justify-between border-b px-4">
        <h2 className="font-medium text-sm">Details</h2>
        <Button variant="ghost" size="icon-sm" onClick={onClose} aria-label="Close details">
          <X />
        </Button>
      </div>
      <div className="flex-1 space-y-6 overflow-y-auto p-4">
        <section className="space-y-2">
          <div className="flex items-start gap-2">
            <h3 className="min-w-0 flex-1 break-words font-semibold text-base leading-snug">
              {parcel.title}
            </h3>
            {isOwner && (
              <Button
                variant="ghost"
                size="icon-sm"
                className="-mt-1 shrink-0"
                onClick={onEdit}
                aria-label="Edit details"
              >
                <Pencil />
              </Button>
            )}
          </div>
          {parcel.description ? (
            <p className="whitespace-pre-wrap text-muted-foreground text-sm">
              {parcel.description}
            </p>
          ) : (
            <p className="text-muted-foreground text-sm italic">No description.</p>
          )}
        </section>

        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          <Meta label="ID">
            <span className="inline-flex items-center gap-1">
              <span className="font-mono text-xs">{parcel.id}</span>
              <CopyButton
                value={parcel.id}
                variant="ghost"
                className="h-6 w-6"
                aria-label="Copy parcel id"
                toastMessage="Parcel id copied"
              />
            </span>
          </Meta>
          <Meta label="Visibility">
            <VisibilityBadge visibility={parcel.visibility} />
          </Meta>
          {!isOwner && ownerName && (
            <Meta label="Owner">
              <span title={ownerName}>{ownerName}</span>
            </Meta>
          )}
          {!isOwner && <Meta label="Your access">{accessLabel(access)}</Meta>}
          <Meta label="Version">
            v{version.version}
            {version.version !== parcel.current_version && (
              <span className="text-muted-foreground"> of {parcel.current_version}</span>
            )}
          </Meta>
          <Meta label="Opens on">
            <span className="font-mono text-xs" title={version.entry}>
              {version.entry}
            </span>
          </Meta>
          <Meta label="Size">
            {formatBytes(version.total_bytes)} · {pluralize(version.file_count, 'file')}
          </Meta>
          <Meta label="Published">
            <RelativeTime value={version.created_at} />
          </Meta>
          <Meta label="Created">
            <RelativeTime value={parcel.created_at} />
          </Meta>
        </dl>

        <section className="space-y-2">
          <h3 className={SECTION_HEADING}>Files</h3>
          {files === null ? (
            <div className="space-y-2">
              <Skeleton className="h-8" />
              <Skeleton className="h-8" />
            </div>
          ) : (
            <ul className="divide-y rounded-lg border" aria-label="Files">
              {files.map((file) => (
                <li key={file.path} className="flex items-center gap-2 py-1.5 pr-1 pl-3">
                  <FileIcon
                    className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-mono text-xs" title={file.path}>
                      {file.path}
                    </p>
                    <p className="truncate text-[11px] text-muted-foreground">
                      {shortType(file.content_type)} · {formatBytes(file.size_bytes)}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-7 w-7 shrink-0 text-muted-foreground"
                    onClick={() => download(file.path)}
                    aria-label={`Download ${file.path}`}
                  >
                    <Download />
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <DataSection parcelId={parcel.id} />

        <section className="space-y-2">
          <h3 className={SECTION_HEADING}>History</h3>
          {versions === null ? (
            <Skeleton className="h-16" />
          ) : (
            <ol className="space-y-0.5" aria-label="Version history">
              {versions.map((v) => {
                const active = v.version === version.version;
                return (
                  <li key={v.version}>
                    <Link
                      to={versionPath(parcel.id, v.version, parcel.current_version)}
                      aria-current={active ? 'page' : undefined}
                      className={cn(
                        'flex items-start gap-3 rounded-md px-2 py-1.5 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                        active ? 'bg-muted' : 'hover:bg-muted/50',
                      )}
                    >
                      <span className="w-8 shrink-0 font-mono text-muted-foreground text-xs leading-5">
                        v{v.version}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate">
                          {v.label ||
                            (v.version === parcel.current_version
                              ? 'Current version'
                              : 'Untitled version')}
                        </span>
                        <span className="block text-muted-foreground text-xs">
                          <RelativeTime value={v.created_at} /> · {formatBytes(v.total_bytes)}
                        </span>
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ol>
          )}
        </section>
      </div>
    </aside>
  );
}
