import {
  AlertCircle,
  LayoutGrid,
  List,
  Loader2,
  Package,
  Plus,
  Search,
  Users,
  X,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CodeBlock } from '@/components/common/CodeBlock';
import { EmptyState } from '@/components/common/EmptyState';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { ParcelCard, ParcelRow, type SharedBy } from '@/components/parcels/ParcelCard';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Skeleton } from '@/components/ui/skeleton';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useDebouncedValue } from '@/hooks/useDebouncedValue';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useParcelList } from '@/hooks/useParcels';
import { useMe } from '@/hooks/useSession';
import { useSharedWithMe } from '@/hooks/useShares';
import { formatBytes, formatNumber, pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import type { ParcelSummary } from '@/lib/types';
import { cn } from '@/lib/utils';

type Layout = 'grid' | 'list';
type Tab = 'mine' | 'shared';

function readLayout(): Layout {
  return readStorage('local', STORAGE_KEYS.dashboardLayout) === 'list' ? 'list' : 'grid';
}

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

function LayoutToggle({ layout, onChange }: { layout: Layout; onChange: (next: Layout) => void }) {
  return (
    <fieldset className="ml-auto flex rounded-lg border p-0.5" aria-label="Layout">
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

interface ListItem {
  parcel: ParcelSummary;
  sharedBy?: SharedBy;
}

/** Cards or rows, with skeletons, an error, an empty state and "load more". */
function ParcelGrid({
  items,
  layout,
  loading,
  error,
  onRetry,
  errorTitle,
  empty,
  hasMore,
  loadingMore,
  onLoadMore,
}: {
  items: ListItem[];
  layout: Layout;
  loading: boolean;
  error: unknown;
  onRetry: () => void;
  errorTitle: string;
  empty: ReactNode;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
}) {
  if (loading) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
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
        title={errorTitle}
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
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {items.map(({ parcel, sharedBy }) => (
            <ParcelCard key={parcel.id} parcel={parcel} sharedBy={sharedBy} />
          ))}
        </div>
      ) : (
        <ul className="divide-y overflow-hidden rounded-xl border bg-card">
          {items.map(({ parcel, sharedBy }) => (
            <ParcelRow key={parcel.id} parcel={parcel} sharedBy={sharedBy} />
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

/** `/` when signed in: the user's parcels, and the ones shared with them. */
export function DashboardPage() {
  const [params, setParams] = useSearchParams();
  const tab: Tab = params.get('tab') === 'shared' ? 'shared' : 'mine';
  useDocumentTitle(tab === 'shared' ? 'Shared with me' : 'My parcels');
  const [search, setSearch] = useState(params.get('q') ?? '');
  const q = useDebouncedValue(search.trim(), 250);
  const [layout, setLayout] = useState<Layout>(readLayout);

  // Keep ?q= in the URL (shareable, survives reload) without adding history entries.
  useEffect(() => {
    const current = params.get('q') ?? '';
    if (current === q) return;
    const next = new URLSearchParams(params);
    if (q) next.set('q', q);
    else next.delete('q');
    setParams(next, { replace: true });
  }, [q, params, setParams]);

  const chooseTab = (next: string) => {
    const nextParams = new URLSearchParams(params);
    if (next === 'shared') nextParams.set('tab', 'shared');
    else nextParams.delete('tab');
    setParams(nextParams);
  };

  const chooseLayout = (next: Layout) => {
    setLayout(next);
    writeStorage('local', STORAGE_KEYS.dashboardLayout, next);
  };

  const list = useParcelList(q);
  const parcels = list.data?.pages.flatMap((page) => page.parcels) ?? [];
  const total = list.data?.pages[0]?.total ?? 0;
  const searching = q.length > 0;

  // Fetched on either tab: its count is on the tab.
  const shared = useSharedWithMe();
  const sharedItems: ListItem[] =
    shared.data?.pages.flatMap((page) =>
      page.parcels.map((s) => ({
        parcel: s.parcel,
        sharedBy: { ownerName: s.owner_name, role: s.role },
      })),
    ) ?? [];
  const sharedTotal = shared.data?.pages[0]?.total ?? 0;

  return (
    <AppShell wide>
      <PageHeader
        title={tab === 'shared' ? 'Shared with me' : 'My parcels'}
        description={
          tab === 'shared' ? 'Parcels other people shared with you by email.' : <UsageLine />
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

      <Tabs value={tab} onValueChange={chooseTab}>
        <TabsList className="mb-5">
          <TabsTrigger value="mine">My parcels</TabsTrigger>
          <TabsTrigger value="shared">
            Shared with me
            {sharedTotal > 0 && (
              <span
                className="rounded-full bg-primary/12 px-1.5 font-semibold text-[11px] text-primary tabular-nums"
                data-testid="shared-count"
              >
                {formatNumber(sharedTotal)}
              </span>
            )}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="mine" className="mt-0">
          <div className="mb-5 flex items-center gap-2">
            <div className="relative flex-1 sm:max-w-sm">
              <Search
                className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                type="search"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Search by title, description or id"
                aria-label="Search parcels"
                className="pr-9 pl-9 [&::-webkit-search-cancel-button]:hidden"
              />
              {search && (
                <button
                  type="button"
                  onClick={() => setSearch('')}
                  className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label="Clear search"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            {list.isFetching && !list.isLoading && (
              <Loader2
                className="h-4 w-4 animate-spin text-muted-foreground"
                aria-label="Updating"
              />
            )}
            <LayoutToggle layout={layout} onChange={chooseLayout} />
          </div>

          {searching && !list.isLoading && !list.isError && parcels.length > 0 && (
            <p className="mb-3 text-muted-foreground text-sm" aria-live="polite">
              {pluralize(total, 'result')}
            </p>
          )}
          <ParcelGrid
            items={parcels.map((parcel) => ({ parcel }))}
            layout={layout}
            loading={list.isLoading}
            error={list.isError ? list.error : null}
            onRetry={() => void list.refetch()}
            errorTitle="Couldn't load your parcels"
            hasMore={list.hasNextPage}
            loadingMore={list.isFetchingNextPage}
            onLoadMore={() => void list.fetchNextPage()}
            empty={
              searching ? (
                <EmptyState
                  icon={Search}
                  title="No matching parcels"
                  description={<>Nothing matches “{q}”. Try a different search.</>}
                  action={
                    <Button variant="outline" onClick={() => setSearch('')}>
                      Clear search
                    </Button>
                  }
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
                  <CodeBlock
                    code="parcel publish page.html"
                    className="w-full max-w-xs text-left"
                  />
                </EmptyState>
              )
            }
          />
        </TabsContent>

        <TabsContent value="shared" className="mt-0">
          <div className="mb-5 flex items-center gap-2">
            {shared.isFetching && !shared.isLoading && (
              <Loader2
                className="h-4 w-4 animate-spin text-muted-foreground"
                aria-label="Updating"
              />
            )}
            <LayoutToggle layout={layout} onChange={chooseLayout} />
          </div>
          <ParcelGrid
            items={sharedItems}
            layout={layout}
            loading={shared.isLoading}
            error={shared.isError ? shared.error : null}
            onRetry={() => void shared.refetch()}
            errorTitle="Couldn't load what's shared with you"
            hasMore={shared.hasNextPage}
            loadingMore={shared.isFetchingNextPage}
            onLoadMore={() => void shared.fetchNextPage()}
            empty={
              <EmptyState
                icon={Users}
                title="Nothing shared with you yet"
                description="When someone shares a parcel with your email address, it shows up here. Parcels open to everyone signed in aren’t listed; open those from their link."
              />
            }
          />
        </TabsContent>
      </Tabs>
    </AppShell>
  );
}
