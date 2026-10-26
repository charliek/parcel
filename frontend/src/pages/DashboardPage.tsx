import {
  AlertCircle,
  Info,
  LayoutGrid,
  List,
  Loader2,
  Package,
  Plus,
  Search,
  SlidersHorizontal,
  Users,
} from 'lucide-react';
import { type ReactNode, useEffect, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CodeBlock } from '@/components/common/CodeBlock';
import { EmptyState } from '@/components/common/EmptyState';
import { FacetSidebar } from '@/components/dashboard/FacetSidebar';
import { SearchBox } from '@/components/dashboard/SearchBox';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { ParcelCard, ParcelRow } from '@/components/parcels/ParcelCard';
import { RemovableChip } from '@/components/tags/TagChip';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { useDebouncedValue } from '@/hooks/useDebouncedValue';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import { useParcelList } from '@/hooks/useParcels';
import { useMe } from '@/hooks/useSession';
import { useFacetCounts, useTagCounts } from '@/hooks/useTags';
import { formatBytes, pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import {
  type DashboardQuery,
  dashboardSearch,
  facetMode,
  isFiltered,
  isLegacyDashboardSearch,
  listFilters,
  parseDashboardQuery,
  RELAXED_NOTICE,
  showsRelaxedNotice,
  toggleTag,
} from '@/lib/search-query';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import type { ListScope, ParcelListItem } from '@/lib/types';
import { cn } from '@/lib/utils';

/**
 * Cards at least 18rem wide, as many as fit the column (the facet sidebar
 * takes part of the page, so the viewport says little about the room left).
 */
const CARD_GRID = 'grid grid-cols-[repeat(auto-fill,minmax(min(100%,18rem),1fr))] gap-4';

type Layout = 'grid' | 'list';

function readLayout(): Layout {
  return readStorage('local', STORAGE_KEYS.dashboardLayout) === 'list' ? 'list' : 'grid';
}

const TITLES: Record<ListScope, string> = {
  mine: 'My parcels',
  shared: 'Shared with me',
  all: 'All parcels',
};

function UsageLine() {
  const me = useMe();
  if (!me) return null;
  const quota = me.limits.user_quota_bytes;
  return (
    <span>
      {pluralize(me.usage.parcel_count, 'parcel')} · {formatBytes(me.usage.stored_bytes)}
      {quota > 0 ? ` of ${formatBytes(quota)}` : ''} stored
    </span>
  );
}

const SEGMENT =
  'px-3 py-1.5 text-sm transition-colors focus-visible:relative focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring';

function ScopeToggle({ scope, onChange }: { scope: ListScope; onChange: (s: ListScope) => void }) {
  return (
    <fieldset className="inline-flex shrink-0 overflow-hidden rounded-lg border">
      <legend className="sr-only">Whose parcels</legend>
      {(['mine', 'shared', 'all'] as const).map((value) => (
        <button
          key={value}
          type="button"
          aria-pressed={scope === value}
          onClick={() => onChange(value)}
          className={cn(
            SEGMENT,
            scope === value
              ? 'bg-muted font-semibold text-foreground'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {value === 'mine' ? 'Mine' : value === 'shared' ? 'Shared' : 'All'}
        </button>
      ))}
    </fieldset>
  );
}

function LayoutToggle({ layout, onChange }: { layout: Layout; onChange: (next: Layout) => void }) {
  return (
    <fieldset className="flex shrink-0 rounded-lg border p-0.5" aria-label="Layout">
      {(
        [
          { value: 'grid', label: 'Grid view', icon: LayoutGrid },
          { value: 'list', label: 'List view', icon: List },
        ] as const
      ).map(({ value, label, icon: Icon }) => (
        <button
          key={value}
          type="button"
          onClick={() => onChange(value)}
          aria-pressed={layout === value}
          aria-label={label}
          title={label}
          className={cn(
            'rounded-md p-1.5 text-muted-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
            layout === value ? 'bg-muted text-foreground' : 'hover:text-foreground',
          )}
        >
          <Icon className="h-4 w-4" />
        </button>
      ))}
    </fieldset>
  );
}

/** Cards or rows, with skeletons, an error, an empty state and "load more". */
function ParcelGrid({
  items,
  layout,
  loading,
  error,
  onRetry,
  empty,
  hasMore,
  loadingMore,
  onLoadMore,
  activeTags,
  onTagClick,
}: {
  items: ParcelListItem[];
  layout: Layout;
  loading: boolean;
  error: unknown;
  onRetry: () => void;
  empty: ReactNode;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  activeTags: readonly string[];
  onTagClick: (tag: string) => void;
}) {
  if (loading) {
    return (
      <div className={CARD_GRID}>
        {['a', 'b', 'c', 'd', 'e', 'f'].map((k) => (
          <Skeleton key={k} className="h-[148px] rounded-xl" />
        ))}
      </div>
    );
  }
  if (error) {
    return (
      <EmptyState
        icon={AlertCircle}
        title="Couldn't load the parcels"
        description={errorMessage(error)}
        action={
          <Button variant="outline" onClick={onRetry}>
            Try again
          </Button>
        }
      />
    );
  }
  if (items.length === 0) return <>{empty}</>;
  return (
    <>
      {layout === 'grid' ? (
        <div className={CARD_GRID}>
          {items.map((parcel) => (
            <ParcelCard
              key={parcel.id}
              parcel={parcel}
              activeTags={activeTags}
              onTagClick={onTagClick}
            />
          ))}
        </div>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {items.map((parcel) => (
            <ParcelRow
              key={parcel.id}
              parcel={parcel}
              activeTags={activeTags}
              onTagClick={onTagClick}
            />
          ))}
        </ul>
      )}
      {hasMore && (
        <div className="mt-6 flex justify-center">
          <Button variant="outline" onClick={onLoadMore} disabled={loadingMore}>
            {loadingMore && <Loader2 className="animate-spin" />}
            Load more
          </Button>
        </div>
      )}
    </>
  );
}

/**
 * `/` when signed in: the caller's parcels, the ones shared with them, or
 * both, narrowed by tag facets, a full-text search and the archive toggle.
 * The whole state lives in the address (`?scope=&tag=&q=&archived=`).
 */
export function DashboardPage() {
  const [params, setParams] = useSearchParams();
  const query = parseDashboardQuery(params);
  useDocumentTitle(TITLES[query.scope]);
  const [layout, setLayout] = useState<Layout>(readLayout);
  const [filtersOpen, setFiltersOpen] = useState(false);
  // Below this the facets fold behind the Filters button.
  const wide = useMediaQuery('(min-width: 760px)');
  const filtersButton = useRef<HTMLButtonElement>(null);

  // Old links (`?tab=shared`) move to the new address.
  useEffect(() => {
    if (isLegacyDashboardSearch(params)) {
      setParams(dashboardSearch(parseDashboardQuery(params), params), { replace: true });
    }
  }, [params, setParams]);

  /**
   * Change the filters. They push a history entry (Back undoes them);
   * typing replaces it. Read from the address itself, not this render: two
   * quick keystrokes must not undo each other.
   */
  const updateQuery = (change: (current: DashboardQuery) => DashboardQuery, replace = false) => {
    const current = new URLSearchParams(window.location.search);
    setParams(dashboardSearch(change(parseDashboardQuery(current)), current), { replace });
  };

  // ---- the search text: local while typing, then `?q=` once it settles
  const [search, setSearch] = useState(query.q);
  const settled = useDebouncedValue(search.trim(), 250);
  // The `q` the box and the address last agreed on (this page wrote it, or
  // took it from the address), to tell typing from Back/Forward.
  const writtenQ = useRef(query.q);
  // Only text typed since then is written. After Back the box still holds
  // the text from before, settled, until the effect below takes the
  // address's: comparing with the address would write the old text over it.
  useEffect(() => {
    if (settled !== search.trim() || settled === writtenQ.current) return;
    writtenQ.current = settled;
    const current = new URLSearchParams(window.location.search);
    setParams(dashboardSearch({ ...parseDashboardQuery(current), q: settled }, current), {
      replace: true,
    });
  }, [settled, search, setParams]);
  useEffect(() => {
    if (query.q !== writtenQ.current) {
      writtenQ.current = query.q;
      setSearch(query.q);
    }
  }, [query.q]);

  const filters = listFilters(query);
  const list = useParcelList(filters);
  const parcels = list.data?.pages.flatMap((page) => page.parcels) ?? [];
  const total = list.data?.pages[0]?.total ?? 0;
  // The first page says whether the search was relaxed; until it is in for
  // these filters (the previous filters' results stay up meanwhile), the
  // facets wait and the notice stays off. A failed list lets the server
  // decide the facets' mode.
  const first = list.data?.pages[0];
  const counts = useFacetCounts(
    query,
    list.isError ? undefined : facetMode(query, first, list.isPlaceholderData),
  );
  const relaxed = showsRelaxedNotice(first, list.isPlaceholderData);
  // Autocomplete offers everything in scope, not only what the filters leave.
  const vocabulary = useTagCounts({ scope: query.scope, archived: query.archived });
  const filtered = isFiltered(query);

  const onToggleTag = (tag: string) =>
    updateQuery((current) => ({ ...current, tags: toggleTag(current.tags, tag) }));
  /** A facet pick. On a phone the folded panel closes, so the results are in view. */
  const onFacetToggle = (tag: string) => {
    onToggleTag(tag);
    if (!wide) {
      setFiltersOpen(false);
      filtersButton.current?.focus();
    }
  };
  const facets = (
    <aside
      id="dashboard-filters"
      aria-label="Filters"
      className={wide ? undefined : 'mb-4 rounded-xl border p-3'}
    >
      <FacetSidebar counts={counts} selected={query.tags} onToggle={onFacetToggle} />
    </aside>
  );
  const onPickTag = (tag: string, rest: string) => {
    const text = rest.trimEnd() ? `${rest.trimEnd()} ` : '';
    setSearch(text);
    writtenQ.current = text.trim();
    updateQuery((current) => ({ ...current, q: text.trim(), tags: toggleTag(current.tags, tag) }));
  };
  const clearFilters = () => {
    setSearch('');
    writtenQ.current = '';
    updateQuery((current) => ({ ...current, tags: [], q: '', archived: 'exclude' }));
  };

  const chooseLayout = (next: Layout) => {
    setLayout(next);
    writeStorage('local', STORAGE_KEYS.dashboardLayout, next);
  };

  const empty = filtered ? (
    <EmptyState
      icon={Search}
      title="No matching parcels"
      description="Nothing matches these filters. Try fewer tags or a different search."
      action={
        <Button variant="outline" onClick={clearFilters}>
          Clear filters
        </Button>
      }
    />
  ) : query.scope === 'shared' ? (
    <EmptyState
      icon={Users}
      title="Nothing shared with you yet"
      description="When someone shares a parcel with your email address, it shows up here. Parcels open to everyone signed in aren’t listed; open those from their link."
    />
  ) : (
    <EmptyState
      icon={Package}
      title="No parcels yet"
      description="Upload an HTML or Markdown page here, or publish straight from your terminal (or your agent):"
      action={
        <>
          <Button asChild>
            <Link to="/new">
              <Plus />
              New parcel
            </Link>
          </Button>
          <Button asChild variant="outline">
            <Link to="/cli">Set up the CLI</Link>
          </Button>
        </>
      }
    >
      <CodeBlock code="parcel publish page.html" className="w-full max-w-xs text-left" />
    </EmptyState>
  );

  return (
    <AppShell wide>
      <PageHeader
        title={TITLES[query.scope]}
        description={
          query.scope === 'shared' ? (
            'Parcels other people shared with you by email.'
          ) : (
            <UsageLine />
          )
        }
        actions={
          // The header already has "New parcel" on wider screens.
          <Button asChild className="sm:hidden">
            <Link to="/new">
              <Plus />
              New parcel
            </Link>
          </Button>
        }
      />

      <div className="min-[760px]:grid min-[760px]:grid-cols-[13rem_minmax(0,1fr)] min-[760px]:gap-8">
        {wide && facets}

        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <Button
              ref={filtersButton}
              variant="outline"
              size="sm"
              className="h-9 min-[760px]:hidden"
              aria-expanded={filtersOpen}
              aria-controls="dashboard-filters"
              onClick={() => setFiltersOpen((open) => !open)}
            >
              <SlidersHorizontal />
              Filters
              {query.tags.length > 0 && (
                <span className="rounded-full bg-primary/12 px-1.5 text-[11px] text-primary tabular-nums">
                  {query.tags.length}
                </span>
              )}
            </Button>
            <ScopeToggle
              scope={query.scope}
              onChange={(scope) => updateQuery((current) => ({ ...current, scope }))}
            />
            <SearchBox
              text={search}
              onTextChange={setSearch}
              tags={query.tags}
              onPickTag={onPickTag}
              onRemoveLastTag={() =>
                updateQuery((current) => ({ ...current, tags: current.tags.slice(0, -1) }))
              }
              vocabulary={vocabulary.data ?? []}
            />
            <label className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 text-muted-foreground text-sm">
              <input
                type="checkbox"
                className="h-4 w-4 accent-primary"
                checked={query.archived !== 'exclude'}
                onChange={(event) => {
                  const archived = event.target.checked ? 'include' : 'exclude';
                  updateQuery((current) => ({ ...current, archived }));
                }}
              />
              Show archived
            </label>
            <div className="ml-auto flex items-center gap-2">
              {list.isFetching && !list.isLoading && (
                <Loader2
                  className="h-4 w-4 animate-spin text-muted-foreground"
                  aria-label="Updating"
                />
              )}
              <LayoutToggle layout={layout} onChange={chooseLayout} />
            </div>
          </div>

          <div className="mt-3 flex min-h-6 flex-wrap items-center gap-1.5 text-muted-foreground text-xs">
            {query.tags.length > 0 && (
              <>
                <span>Filtering by</span>
                <ul className="contents" aria-label="Tag filters">
                  {query.tags.map((tag) => (
                    <li key={tag}>
                      <RemovableChip
                        tag={tag}
                        onRemove={() => onToggleTag(tag)}
                        removeLabel={`Remove the ${tag} filter`}
                      />
                    </li>
                  ))}
                </ul>
                <button
                  type="button"
                  onClick={clearFilters}
                  className="rounded px-1 text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  Clear
                </button>
              </>
            )}
          </div>

          {/* Folded (phone): under the toolbar and its chips, above the results. */}
          {!wide && filtersOpen && facets}

          {!list.isLoading && !list.isError && parcels.length > 0 && (
            <p
              className="mt-1 mb-3 text-muted-foreground text-xs"
              aria-live="polite"
              data-testid="result-count"
            >
              {pluralize(total, 'parcel')}
              {filtered ? ' match' : ''}
            </p>
          )}
          {relaxed && !list.isError && (
            <p
              className="mb-3 flex items-center gap-1.5 text-muted-foreground text-sm"
              role="status"
              data-testid="relaxed-notice"
            >
              <Info className="h-4 w-4 shrink-0" aria-hidden />
              {RELAXED_NOTICE}
            </p>
          )}
          <div className={cn(parcels.length === 0 && 'mt-3')}>
            <ParcelGrid
              items={parcels}
              layout={layout}
              loading={list.isLoading}
              error={list.isError ? list.error : null}
              onRetry={() => void list.refetch()}
              hasMore={list.hasNextPage}
              loadingMore={list.isFetchingNextPage}
              onLoadMore={() => void list.fetchNextPage()}
              empty={empty}
              activeTags={query.tags}
              onTagClick={onToggleTag}
            />
          </div>
        </div>
      </div>
    </AppShell>
  );
}
