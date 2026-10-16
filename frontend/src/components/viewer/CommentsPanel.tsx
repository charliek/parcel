import { Crosshair, MessageSquare, MousePointerClick, X } from 'lucide-react';
import { type Ref, useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { anchorLabel, countThreads } from '@/lib/comments';
import { errorMessage } from '@/lib/http';
import type { Comment, CommentAnchor } from '@/lib/types';
import { cn } from '@/lib/utils';
import { CommentComposer } from './CommentComposer';
import { CommentThread } from './CommentThread';
import { DeleteCommentDialog, type DeleteTarget } from './DeleteCommentDialog';

export type CommentFilter = 'open' | 'resolved';

interface CommentsPanelProps {
  parcelId: string;
  /** Every thread, open and resolved; null while loading. */
  threads: Comment[] | null;
  error: unknown;
  onRetry: () => void;
  filter: CommentFilter;
  onFilterChange: (filter: CommentFilter) => void;
  isOwner: boolean;
  viewedVersion: number;
  framePath: string | null;
  /** Threads on this page whose spot the page couldn't find. */
  missing: ReadonlySet<number>;
  active: number | null;
  onActivate: (thread: Comment) => void;
  /** Scroll to and focus this thread (the nonce repeats a reveal of the same one). */
  reveal: { number: number; nonce: number } | null;
  commentMode: boolean;
  onStartCommenting: () => void;
  onClose: () => void;
  onComposingChange: (key: string, composing: boolean) => void;
  // The new-thread composer; its state lives in the viewer, which also
  // drives the draft pin in the page.
  draftAnchor: CommentAnchor | null;
  /** The version the spot was picked on (the comment goes there). */
  draftVersion: number | null;
  composerText: string;
  onComposerTextChange: (text: string) => void;
  onCancelDraft: () => void;
  onSubmit: () => void;
  submitting: boolean;
  composerRef: Ref<HTMLTextAreaElement>;
}

/** The comment threads on a parcel, with a composer for new ones. */
export function CommentsPanel({
  parcelId,
  threads,
  error,
  onRetry,
  filter,
  onFilterChange,
  isOwner,
  viewedVersion,
  framePath,
  missing,
  active,
  onActivate,
  reveal,
  commentMode,
  onStartCommenting,
  onClose,
  onComposingChange,
  draftAnchor,
  draftVersion,
  composerText,
  onComposerTextChange,
  onCancelDraft,
  onSubmit,
  submitting,
  composerRef,
}: CommentsPanelProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const [deleting, setDeleting] = useState<DeleteTarget | null>(null);
  const counts = threads ? countThreads(threads) : null;
  const shown = threads?.filter((t) => t.resolved === (filter === 'resolved')) ?? null;

  const loaded = shown !== null;

  // Runs when asked to reveal a thread (or once the list arrives), not on
  // every list change.
  useEffect(() => {
    if (!reveal || !loaded) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-comment-number="${reveal.number}"]`,
    );
    if (!el) return;
    el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    el.focus({ preventScroll: true });
  }, [reveal, loaded]);

  return (
    <aside
      className="absolute inset-y-0 right-0 z-20 flex w-full max-w-sm flex-col border-l bg-background shadow-xl md:static md:w-96 md:max-w-none md:shadow-none"
      aria-label="Comments"
    >
      <div className="flex h-11 shrink-0 items-center gap-2 border-b px-4">
        <h2 className="font-medium text-sm">Comments</h2>
        <fieldset className="ml-auto inline-flex h-7 items-center rounded-md bg-muted p-0.5 text-xs">
          <legend className="sr-only">Show comments</legend>
          {(['open', 'resolved'] as const).map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={filter === value}
              onClick={() => onFilterChange(value)}
              className={cn(
                'h-6 rounded px-2 font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                filter === value
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              {value === 'open' ? 'Open' : 'Resolved'}
              {counts && <span className="ml-1 tabular-nums opacity-70">{counts[value]}</span>}
            </button>
          ))}
        </fieldset>
        <Button
          variant="ghost"
          size="icon-sm"
          className="-mr-2"
          onClick={onClose}
          aria-label="Close comments"
        >
          <X />
        </Button>
      </div>

      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto p-4">
        {!commentMode && (
          <Button variant="outline" size="sm" className="w-full" onClick={onStartCommenting}>
            <MousePointerClick />
            Comment on a spot in the page
          </Button>
        )}
        {error ? (
          <div className="space-y-2 rounded-lg border border-destructive/30 p-3 text-sm">
            <p className="text-destructive">
              {errorMessage(error, 'Could not load the comments.')}
            </p>
            <Button variant="outline" size="sm" onClick={onRetry}>
              Try again
            </Button>
          </div>
        ) : shown === null ? (
          <div className="space-y-3" role="status" aria-label="Loading comments">
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </div>
        ) : shown.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-4 py-10 text-center text-muted-foreground text-sm">
            <MessageSquare className="h-6 w-6" aria-hidden="true" />
            {filter === 'open' ? (
              <p>
                No open comments.
                {commentMode && ' Click anything on the page to comment on it.'}
              </p>
            ) : (
              <p>No resolved comments.</p>
            )}
          </div>
        ) : (
          <ol
            className="space-y-3"
            aria-label={filter === 'open' ? 'Open comments' : 'Resolved comments'}
          >
            {shown.map((thread) => (
              <li key={thread.number}>
                <CommentThread
                  parcelId={parcelId}
                  thread={thread}
                  isOwner={isOwner}
                  viewedVersion={viewedVersion}
                  framePath={framePath}
                  missing={missing.has(thread.number)}
                  active={active === thread.number}
                  onActivate={() => onActivate(thread)}
                  onDelete={setDeleting}
                  onComposingChange={onComposingChange}
                />
              </li>
            ))}
          </ol>
        )}
      </div>

      <div className="shrink-0 border-t p-3">
        <CommentComposer
          value={composerText}
          onChange={onComposerTextChange}
          onSubmit={onSubmit}
          onCancel={draftAnchor || composerText ? onCancelDraft : undefined}
          pending={submitting}
          label={draftAnchor ? 'Comment on this spot' : 'Add a comment'}
          placeholder={
            draftAnchor ? 'Comment on this spot…' : 'Add a comment about the whole page…'
          }
          submitLabel="Comment"
          textareaRef={composerRef}
          context={
            draftAnchor ? (
              <p className="flex items-start gap-1.5 text-muted-foreground text-xs">
                <Crosshair className="mt-0.5 h-3 w-3 shrink-0 text-primary" aria-hidden="true" />
                <span className="min-w-0 break-words">
                  Commenting on {anchorLabel(draftAnchor, framePath)}
                  {draftVersion !== null && draftVersion !== viewedVersion && ` (v${draftVersion})`}
                </span>
              </p>
            ) : null
          }
        />
      </div>

      <DeleteCommentDialog
        parcelId={parcelId}
        target={deleting}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      />
    </aside>
  );
}
