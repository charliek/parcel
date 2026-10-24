import { AlertCircle, ArrowLeft, GitMerge, Loader2, Pencil, Tags, Trash2 } from 'lucide-react';
import { type FormEvent, useEffect, useMemo, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { toast } from 'sonner';
import { EmptyState } from '@/components/common/EmptyState';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { TagChip } from '@/components/tags/TagChip';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { OWN_TAGS, useDeleteTag, useRenameTag, useTagCounts } from '@/hooks/useTags';
import { formatNumber, pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import { dashboardHref } from '@/lib/search-query';
import { possibleDuplicates } from '@/lib/tag-duplicates';
import { groupTags, normalizeTag, TAG_RULES } from '@/lib/tags';
import type { TagCount } from '@/lib/types';

/** What a row is doing: nothing, being renamed (into `to`), or awaiting a delete confirm. */
type RowMode =
  | { tag: string; kind: 'rename'; to: string; confirmMerge: boolean }
  | { tag: string; kind: 'delete' };

function RenameRow({
  item,
  mode,
  existing,
  onChange,
  onCancel,
}: {
  item: TagCount;
  mode: Extract<RowMode, { kind: 'rename' }>;
  existing: TagCount | undefined;
  onChange: (mode: RowMode | null) => void;
  onCancel: () => void;
}) {
  const rename = useRenameTag();
  // Opens on the old name, selected: typing replaces it, an arrow key edits it.
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const normalized = normalizeTag(mode.to);
  const unchanged = normalized === item.tag;
  const merging = existing !== undefined && !unchanged;

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!normalized || unchanged) return;
    // Renaming into a tag in use merges the two: say so before doing it.
    if (merging && !mode.confirmMerge) {
      onChange({ ...mode, confirmMerge: true });
      return;
    }
    rename.mutate(
      { from: item.tag, to: normalized },
      {
        onSuccess: ({ updated }) => {
          toast.success(merging ? `Merged into ${normalized}` : `Renamed to ${normalized}`, {
            description: `${pluralize(updated, 'parcel')} updated.`,
          });
          onChange(null);
        },
        onError: (err) => toast.error('Could not rename', { description: errorMessage(err) }),
      },
    );
  };

  return (
    <form onSubmit={submit} className="space-y-2 py-2" aria-label={`Rename ${item.tag}`}>
      <div className="flex flex-wrap items-center gap-2">
        <TagChip tag={item.tag} />
        <span className="text-muted-foreground text-xs">to</span>
        <Input
          ref={input}
          value={mode.to}
          onChange={(event) => onChange({ ...mode, to: event.target.value, confirmMerge: false })}
          aria-label={`New name for ${item.tag}`}
          className="h-8 w-56 font-mono text-xs"
          spellCheck={false}
          autoCapitalize="none"
          onKeyDown={(event) => {
            if (event.key === 'Escape') onCancel();
          }}
        />
        <Button
          type="submit"
          size="sm"
          variant={merging && mode.confirmMerge ? 'destructive' : 'default'}
          disabled={!normalized || unchanged || rename.isPending}
        >
          {rename.isPending && <Loader2 className="animate-spin" />}
          {merging ? (mode.confirmMerge ? 'Merge' : 'Merge…') : 'Rename'}
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      <p className="text-muted-foreground text-xs" aria-live="polite">
        {!mode.to.trim()
          ? 'Type the new name.'
          : normalized === null
            ? `Not a valid tag: ${TAG_RULES}`
            : unchanged
              ? 'That’s its name already.'
              : merging
                ? mode.confirmMerge
                  ? `Merge ${item.tag} (${pluralize(item.count, 'parcel')}) into ${normalized} (${pluralize(existing.count, 'parcel')})? Every parcel with ${item.tag} gets ${normalized} instead. This can’t be undone.`
                  : `${normalized} already exists: renaming merges the two.`
                : normalized !== mode.to.trim()
                  ? `Saved as “${normalized}”.`
                  : `Renames it on ${pluralize(item.count, 'parcel')}.`}
      </p>
    </form>
  );
}

function DeleteRow({ item, onDone }: { item: TagCount; onDone: () => void }) {
  const remove = useDeleteTag();
  return (
    <fieldset className="flex flex-wrap items-center gap-2 py-2" aria-label={`Delete ${item.tag}`}>
      <span className="text-sm">
        Remove <TagChip tag={item.tag} /> from {pluralize(item.count, 'parcel')}?
      </span>
      <Button
        size="sm"
        variant="destructive"
        disabled={remove.isPending}
        onClick={() =>
          remove.mutate(item.tag, {
            onSuccess: ({ updated }) => {
              toast.success(`Deleted ${item.tag}`, {
                description: `Removed from ${pluralize(updated, 'parcel')}.`,
              });
              onDone();
            },
            onError: (err) => toast.error('Could not delete', { description: errorMessage(err) }),
          })
        }
      >
        {remove.isPending && <Loader2 className="animate-spin" />}
        Delete tag
      </Button>
      <Button size="sm" variant="ghost" onClick={onDone}>
        Cancel
      </Button>
    </fieldset>
  );
}

const NO_TAGS: TagCount[] = [];

/**
 * `/tags`: every tag on the caller's parcels with how many carry it,
 * grouped by namespace, with rename (into an existing tag merges), delete
 * and hints at likely duplicates.
 */
export function TagsPage() {
  useDocumentTitle('Tags');
  const tags = useTagCounts(OWN_TAGS);
  const [mode, setMode] = useState<RowMode | null>(null);
  const all = tags.data ?? NO_TAGS;
  // Typing in a rename row re-renders the page; the duplicate scan is quadratic.
  const groups = useMemo(() => groupTags(all, (t) => t.tag), [all]);
  const hints = useMemo(() => possibleDuplicates(all), [all]);
  const find = (tag: string) => all.find((t) => t.tag === tag);

  const startMerge = (from: string, to: string) =>
    setMode({ tag: from, kind: 'rename', to, confirmMerge: true });

  return (
    <AppShell>
      <Link
        to="/"
        className="mb-6 inline-flex items-center gap-1.5 rounded text-muted-foreground text-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        My parcels
      </Link>
      <PageHeader
        title="Tags"
        description="Every tag on your parcels. Renaming into a tag you already use merges the two; changes apply to all your parcels at once."
      />

      {tags.isLoading ? (
        <div className="space-y-3" role="status" aria-label="Loading tags">
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : tags.isError ? (
        <EmptyState
          icon={AlertCircle}
          title="Couldn't load your tags"
          description={errorMessage(tags.error)}
          action={
            <Button variant="outline" onClick={() => void tags.refetch()}>
              Try again
            </Button>
          }
        />
      ) : all.length === 0 ? (
        <EmptyState
          icon={Tags}
          title="No tags yet"
          description="Tag a parcel from its details in the viewer, or publish with parcel publish --tag. Tags like project:roost and kind:plan get their own filters on the dashboard."
        />
      ) : (
        <div className="space-y-8">
          {hints.length > 0 && (
            <section
              className="space-y-2 rounded-lg bg-primary/8 p-3 text-sm"
              aria-label="Possible duplicates"
            >
              <p className="font-medium">Possible duplicates</p>
              <ul className="space-y-1.5">
                {hints.map((hint) => (
                  <li key={hint.from} className="flex flex-wrap items-center gap-2">
                    <TagChip tag={hint.from} />
                    <span className="text-muted-foreground text-xs">looks like</span>
                    <TagChip tag={hint.to} />
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-7"
                      onClick={() => startMerge(hint.from, hint.to)}
                    >
                      <GitMerge />
                      Merge into {hint.to}…
                    </Button>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {groups.map((group) => (
            <section key={group.namespace} aria-labelledby={`tags-${group.namespace || 'plain'}`}>
              <h2
                id={`tags-${group.namespace || 'plain'}`}
                className="mb-2 flex items-center justify-between font-mono font-semibold text-muted-foreground text-xs uppercase tracking-wider"
              >
                <span>{group.label}</span>
                <span className="tabular-nums">{formatNumber(group.items.length)}</span>
              </h2>
              <ul className="divide-y rounded-xl border bg-card px-3">
                {group.items.map((item) => {
                  const rowMode = mode?.tag === item.tag ? mode : null;
                  return (
                    <li key={item.tag} data-testid="tag-row" data-tag={item.tag}>
                      {rowMode?.kind === 'rename' ? (
                        <RenameRow
                          item={item}
                          mode={rowMode}
                          existing={find(normalizeTag(rowMode.to) ?? '')}
                          onChange={setMode}
                          onCancel={() => setMode(null)}
                        />
                      ) : rowMode?.kind === 'delete' ? (
                        <DeleteRow item={item} onDone={() => setMode(null)} />
                      ) : (
                        <div className="flex items-center gap-3 py-2">
                          <span className="min-w-0 flex-1">
                            <TagChip
                              tag={item.tag}
                              to={dashboardHref({ tags: [item.tag], archived: 'include' })}
                            />
                          </span>
                          <span
                            className="shrink-0 text-muted-foreground text-xs tabular-nums"
                            title={pluralize(item.count, 'parcel')}
                          >
                            {formatNumber(item.count)}
                          </span>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="h-7"
                            aria-label={`Rename ${item.tag}`}
                            onClick={() =>
                              setMode({
                                tag: item.tag,
                                kind: 'rename',
                                to: item.tag,
                                confirmMerge: false,
                              })
                            }
                          >
                            <Pencil />
                            <span className="hidden sm:inline">Rename</span>
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="h-7 text-muted-foreground hover:text-destructive"
                            aria-label={`Delete ${item.tag}`}
                            onClick={() => setMode({ tag: item.tag, kind: 'delete' })}
                          >
                            <Trash2 />
                            <span className="hidden sm:inline">Delete</span>
                          </Button>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </section>
          ))}
        </div>
      )}
    </AppShell>
  );
}
