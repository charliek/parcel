import { AlertCircle, LayoutGrid, List, Loader2, Package, Plus, Search, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CodeBlock } from '@/components/common/CodeBlock';
import { EmptyState } from '@/components/common/EmptyState';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { ParcelCard, ParcelRow } from '@/components/parcels/ParcelCard';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Skeleton } from '@/components/ui/skeleton';
import { useDebouncedValue } from '@/hooks/useDebouncedValue';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useParcelList } from '@/hooks/useParcels';
import { useMe } from '@/hooks/useSession';
import { formatBytes, pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';
import { cn } from '@/lib/utils';

type Layout = 'grid' | 'list';

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

/** `/` when signed in: the user's parcels. */
export function DashboardPage() {
  useDocumentTitle('My parcels');
  const [params, setParams] = useSearchParams();
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

  const chooseLayout = (next: Layout) => {
    setLayout(next);
    writeStorage('local', STORAGE_KEYS.dashboardLayout, next);
  };

  const list = useParcelList(q);
  const parcels = list.data?.pages.flatMap((page) => page.parcels) ?? [];
  const total = list.data?.pages[0]?.total ?? 0;
  const searching = q.length > 0;

  return (
    <AppShell wide>
      <PageHeader
        title="My parcels"
        description={<UsageLine />}
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
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-label="Updating" />
        )}
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
              onClick={() => chooseLayout(value)}
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
      </div>

      {list.isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {['a', 'b', 'c', 'd', 'e', 'f'].map((k) => (
            <Skeleton key={k} className="h-[148px] rounded-xl" />
          ))}
        </div>
      ) : list.isError ? (
        <EmptyState
          icon={AlertCircle}
          title="Couldn't load your parcels"
          description={errorMessage(list.error)}
          action={
            <Button variant="outline" onClick={() => void list.refetch()}>
              Try again
            </Button>
          }
        />
      ) : parcels.length === 0 ? (
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
            <CodeBlock code="parcel publish page.html" className="w-full max-w-xs text-left" />
          </EmptyState>
        )
      ) : (
        <>
          {searching && (
            <p className="mb-3 text-muted-foreground text-sm" aria-live="polite">
              {pluralize(total, 'result')}
            </p>
          )}
          {layout === 'grid' ? (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {parcels.map((parcel) => (
                <ParcelCard key={parcel.id} parcel={parcel} />
              ))}
            </div>
          ) : (
            <ul className="divide-y overflow-hidden rounded-xl border bg-card">
              {parcels.map((parcel) => (
                <ParcelRow key={parcel.id} parcel={parcel} />
              ))}
            </ul>
          )}
          {list.hasNextPage && (
            <div className="mt-6 flex justify-center">
              <Button
                variant="outline"
                onClick={() => void list.fetchNextPage()}
                disabled={list.isFetchingNextPage}
              >
                {list.isFetchingNextPage && <Loader2 className="animate-spin" />}
                Load more
              </Button>
            </div>
          )}
        </>
      )}
    </AppShell>
  );
}
