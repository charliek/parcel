import { Tags } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Skeleton } from '@/components/ui/skeleton';
import { formatNumber } from '@/lib/format';
import { groupTags, tagNamespace, tagValue } from '@/lib/tags';
import type { TagCount } from '@/lib/types';
import { cn } from '@/lib/utils';

/** Rows a facet shows before "Show all". */
export const FACET_ROWS = 6;

function Facet({
  namespace,
  label,
  items,
  selected,
  onToggle,
}: {
  namespace: string;
  label: string;
  items: TagCount[];
  selected: readonly string[];
  onToggle: (tag: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [filter, setFilter] = useState('');
  const needle = filter.trim().toLowerCase();
  const matching = needle ? items.filter((t) => tagValue(t.tag).includes(needle)) : items;
  const shown = expanded ? matching : matching.slice(0, FACET_ROWS);
  const headingId = `facet-${namespace || 'tags'}`;

  return (
    <section aria-labelledby={headingId} className="space-y-1">
      <h3
        id={headingId}
        className="flex items-center justify-between px-1.5 font-mono font-semibold text-[11px] text-muted-foreground uppercase tracking-wider"
      >
        <span>{label}</span>
        <span className="tabular-nums">{formatNumber(items.length)}</span>
      </h3>
      {items.length > FACET_ROWS && (
        <input
          type="search"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder={`Filter ${label.toLowerCase()}`}
          aria-label={`Filter ${label.toLowerCase()}`}
          className="mb-1 h-7 w-full rounded-md border border-input bg-background px-2 text-xs outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
        />
      )}
      <ul className="space-y-px">
        {shown.map(({ tag, count }) => {
          const on = selected.includes(tag);
          return (
            <li key={tag}>
              <button
                type="button"
                onClick={() => onToggle(tag)}
                aria-pressed={on}
                title={on ? `Remove the ${tag} filter` : `Filter by ${tag}`}
                className={cn(
                  'flex w-full items-center justify-between gap-2 rounded-md px-1.5 py-1 text-left text-[13px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                  on ? 'bg-primary/12 font-semibold text-foreground' : 'hover:bg-muted',
                )}
                data-tag={tag}
              >
                <span className="min-w-0 truncate">{tagNamespace(tag) ? tagValue(tag) : tag}</span>
                <span className="shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums">
                  {formatNumber(count)}
                </span>
              </button>
            </li>
          );
        })}
        {shown.length === 0 && (
          <li className="px-1.5 py-1 text-muted-foreground text-xs">None here</li>
        )}
      </ul>
      {matching.length > FACET_ROWS && (
        <button
          type="button"
          onClick={() => setExpanded((on) => !on)}
          className="rounded px-1.5 py-0.5 text-primary text-xs hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {expanded ? 'Show fewer' : `Show all ${formatNumber(matching.length)}`}
        </button>
      )}
    </section>
  );
}

/**
 * Facets over the tags of the parcels in the list: Projects, Kind, any
 * other namespace, then plain tags. Counts come from `GET /tags` with the
 * list's own filters, so each says how many parcels clicking it leaves.
 */
export function FacetSidebar({
  counts,
  selected,
  onToggle,
}: {
  /** Undefined while loading. */
  counts: TagCount[] | undefined;
  selected: readonly string[];
  onToggle: (tag: string) => void;
}) {
  if (!counts) {
    return (
      <div className="space-y-3" role="status" aria-label="Loading tags">
        <Skeleton className="h-24" />
        <Skeleton className="h-16" />
      </div>
    );
  }
  // A selected tag stays listed even when the filters leave it no parcels.
  const all = [...counts];
  for (const tag of selected) if (!all.some((t) => t.tag === tag)) all.push({ tag, count: 0 });
  const groups = groupTags(all, (t) => t.tag);

  return (
    <div className="space-y-5">
      {groups.map((group) => (
        <Facet
          key={group.namespace}
          namespace={group.namespace}
          label={group.label}
          items={group.items}
          selected={selected}
          onToggle={onToggle}
        />
      ))}
      {groups.length === 0 && (
        <p className="px-1.5 text-muted-foreground text-xs">
          No tags here yet. Add them in a parcel’s details, or publish with{' '}
          <code className="font-mono">--tag</code>.
        </p>
      )}
      <Link
        to="/tags"
        className="inline-flex items-center gap-1.5 rounded px-1.5 text-muted-foreground text-xs hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Tags className="h-3.5 w-3.5" aria-hidden="true" />
        Manage tags
      </Link>
    </div>
  );
}
