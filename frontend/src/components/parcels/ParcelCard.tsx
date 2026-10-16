import { Link } from 'react-router-dom';
import { RelativeTime } from '@/components/common/RelativeTime';
import { VisibilityBadge } from '@/components/common/VisibilityBadge';
import { formatBytes, pluralize } from '@/lib/format';
import type { ParcelSummary } from '@/lib/types';
import { ParcelIcon } from './ParcelIcon';

const LINK_FOCUS =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background';

/** A parcel in the dashboard grid. The whole card is the link. */
export function ParcelCard({ parcel }: { parcel: ParcelSummary }) {
  return (
    <Link
      to={`/p/${parcel.id}`}
      className={`group flex min-h-[148px] flex-col rounded-xl border bg-card p-4 shadow-xs transition hover:border-foreground/20 hover:shadow-md ${LINK_FOCUS}`}
      data-testid="parcel-card"
    >
      <div className="flex items-start gap-3">
        <ParcelIcon entry={parcel.entry} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate font-medium text-[15px] leading-tight group-hover:text-primary">
            {parcel.title || parcel.entry}
          </h3>
          <p className="mt-1 truncate font-mono text-muted-foreground text-xs">{parcel.id}</p>
        </div>
        <VisibilityBadge visibility={parcel.visibility} />
      </div>
      {parcel.description && (
        <p className="mt-3 line-clamp-2 text-muted-foreground text-sm">{parcel.description}</p>
      )}
      <div className="mt-auto flex items-center gap-2 pt-4 text-muted-foreground text-xs">
        <span className="rounded bg-muted px-1.5 py-0.5 font-medium font-mono text-[11px]">
          v{parcel.current_version}
        </span>
        <span>{formatBytes(parcel.total_bytes)}</span>
        <span aria-hidden="true">·</span>
        <span>{pluralize(parcel.file_count, 'file')}</span>
        <span className="ml-auto">
          <RelativeTime value={parcel.updated_at} />
        </span>
      </div>
    </Link>
  );
}

/** A parcel in the dashboard list layout. */
export function ParcelRow({ parcel }: { parcel: ParcelSummary }) {
  return (
    <li>
      <Link
        to={`/p/${parcel.id}`}
        className={`group flex items-center gap-3 px-4 py-3 transition-colors hover:bg-muted/50 ${LINK_FOCUS}`}
        data-testid="parcel-card"
      >
        <ParcelIcon entry={parcel.entry} className="h-8 w-8" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate font-medium text-sm group-hover:text-primary">
              {parcel.title || parcel.entry}
            </h3>
            <VisibilityBadge visibility={parcel.visibility} className="hidden sm:inline-flex" />
          </div>
          <p className="truncate font-mono text-muted-foreground text-xs">{parcel.id}</p>
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
      </Link>
    </li>
  );
}
