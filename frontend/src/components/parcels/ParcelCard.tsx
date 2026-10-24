import { MessageSquare } from 'lucide-react';
import { Link } from 'react-router-dom';
import { ArchivedBadge } from '@/components/common/ArchivedBadge';
import { RelativeTime } from '@/components/common/RelativeTime';
import { VisibilityBadge } from '@/components/common/VisibilityBadge';
import { Snippet } from '@/components/tags/Snippet';
import { TagChipButton } from '@/components/tags/TagChip';
import { ROLE_LABEL } from '@/lib/access';
import { formatBytes, formatNumber, pluralize } from '@/lib/format';
import { canonicalPath } from '@/lib/refs';
import type { ParcelListItem } from '@/lib/types';
import { cn } from '@/lib/utils';
import { ParcelIcon } from './ParcelIcon';

// The title link stretches over the whole card (`after:inset-0`), so the
// card is one big link while its tag chips stay separate buttons on top.
const STRETCHED_LINK =
  'after:absolute after:inset-0 after:rounded-[inherit] after:content-[""] focus-visible:outline-none';
const CARD_FOCUS =
  'has-[a:focus-visible]:ring-2 has-[a:focus-visible]:ring-ring has-[a:focus-visible]:ring-offset-2 has-[a:focus-visible]:ring-offset-background';

/** Unresolved comment threads, shown only when there are some. */
function OpenComments({ count }: { count: number }) {
  if (count <= 0) return null;
  const label = pluralize(count, 'open comment');
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1 rounded bg-primary/12 px-1.5 py-0.5 font-medium text-[11px] text-primary"
      title={label}
      data-testid="open-comments"
    >
      <MessageSquare className="h-3 w-3" aria-hidden="true" />
      <span aria-hidden="true">{formatNumber(count)}</span>
      <span className="sr-only">{label}</span>
    </span>
  );
}

/**
 * The line under the title: who shared it (someone else's parcel), else its
 * clean address (`@handle/slug`), else its id.
 */
function Subtitle({ parcel }: { parcel: ParcelListItem }) {
  if (parcel.shared) {
    return (
      <p className="truncate text-muted-foreground text-xs" data-testid="shared-by">
        {parcel.shared.owner_name} · {ROLE_LABEL[parcel.shared.role]}
      </p>
    );
  }
  if (parcel.slug && parcel.owner_handle) {
    return (
      <p className="truncate font-mono text-muted-foreground text-xs">
        @{parcel.owner_handle}/{parcel.slug}
      </p>
    );
  }
  return <p className="truncate font-mono text-muted-foreground text-xs">{parcel.id}</p>;
}

interface ParcelItemProps {
  parcel: ParcelListItem;
  /** Tag filters in force (their chips are outlined). */
  activeTags: readonly string[];
  /** A chip was clicked: add or remove that filter. */
  onTagClick: (tag: string) => void;
}

function Tags({ parcel, activeTags, onTagClick }: ParcelItemProps) {
  const tags = parcel.tags ?? [];
  if (tags.length === 0) return null;
  return (
    <ul className="relative z-10 flex flex-wrap gap-1" aria-label="Tags">
      {tags.map((tag) => (
        <li key={tag} className="min-w-0 max-w-full">
          <TagChipButton tag={tag} on={activeTags.includes(tag)} onClick={() => onTagClick(tag)} />
        </li>
      ))}
    </ul>
  );
}

/** A parcel in the dashboard grid. The whole card opens it. */
export function ParcelCard(props: ParcelItemProps) {
  const { parcel } = props;
  return (
    <article
      className={cn(
        'group relative flex min-h-[148px] flex-col gap-3 rounded-xl border bg-card p-4 shadow-xs transition hover:border-foreground/20 hover:shadow-md',
        CARD_FOCUS,
      )}
      data-testid="parcel-card"
    >
      <div className="flex items-start gap-3">
        <ParcelIcon entry={parcel.entry} />
        <div className="min-w-0 flex-1">
          {/* Two lines before it's cut: the badges beside it take room. */}
          <h3
            className={cn(
              'line-clamp-2 break-words font-medium text-[15px] leading-tight group-hover:text-primary',
              parcel.archived_at && 'text-muted-foreground',
            )}
            data-testid="card-title"
          >
            <Link to={canonicalPath(parcel)} className={STRETCHED_LINK}>
              {parcel.title || parcel.entry}
            </Link>
          </h3>
          <div className="mt-1">
            <Subtitle parcel={parcel} />
          </div>
        </div>
        {parcel.archived_at && <ArchivedBadge className="shrink-0" />}
        <VisibilityBadge visibility={parcel.visibility} />
      </div>
      {parcel.description && (
        <p className="line-clamp-2 text-muted-foreground text-sm">{parcel.description}</p>
      )}
      {parcel.match && <Snippet segments={parcel.match.segments} />}
      <Tags {...props} />
      {/* Each item stays whole; a narrow card wraps between them, not inside. */}
      <div
        className="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1 whitespace-nowrap text-muted-foreground text-xs"
        data-testid="card-footer"
      >
        <span className="rounded bg-muted px-1.5 py-0.5 font-medium font-mono text-[11px]">
          v{parcel.current_version}
        </span>
        <span>{formatBytes(parcel.total_bytes)}</span>
        <span aria-hidden="true">·</span>
        <span>{pluralize(parcel.file_count, 'file')}</span>
        <OpenComments count={parcel.open_comments} />
        <span className="ml-auto">
          <RelativeTime value={parcel.updated_at} />
        </span>
      </div>
    </article>
  );
}

/** A parcel in the dashboard list layout. */
export function ParcelRow(props: ParcelItemProps) {
  const { parcel } = props;
  return (
    <li
      className={cn('group relative px-4 py-3 transition-colors hover:bg-muted/50', CARD_FOCUS)}
      data-testid="parcel-card"
    >
      <div className="flex items-center gap-3">
        <ParcelIcon entry={parcel.entry} className="h-8 w-8" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3
              className={cn(
                'truncate font-medium text-sm group-hover:text-primary',
                parcel.archived_at && 'text-muted-foreground',
              )}
            >
              <Link to={canonicalPath(parcel)} className={STRETCHED_LINK}>
                {parcel.title || parcel.entry}
              </Link>
            </h3>
            {parcel.archived_at && <ArchivedBadge className="shrink-0" />}
            <VisibilityBadge visibility={parcel.visibility} className="hidden sm:inline-flex" />
            <OpenComments count={parcel.open_comments} />
          </div>
          <Subtitle parcel={parcel} />
        </div>
        <span className="hidden w-12 text-right font-mono text-muted-foreground text-xs sm:block">
          v{parcel.current_version}
        </span>
        <span className="hidden w-16 text-right text-muted-foreground text-xs md:block">
          {formatBytes(parcel.total_bytes)}
        </span>
        <span className="w-24 text-right text-muted-foreground text-xs">
          <RelativeTime value={parcel.updated_at} />
        </span>
      </div>
      {(parcel.match || (parcel.tags?.length ?? 0) > 0) && (
        <div className="mt-2 space-y-1.5 pl-11">
          {parcel.match && <Snippet segments={parcel.match.segments} />}
          <Tags {...props} />
        </div>
      )}
    </li>
  );
}
