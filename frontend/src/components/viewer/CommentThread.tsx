import {
  AlertTriangle,
  Check,
  Crosshair,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  Trash2,
} from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { IconButton } from '@/components/common/IconButton';
import { RelativeTime } from '@/components/common/RelativeTime';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useCreateComment, useUpdateComment } from '@/hooks/useComments';
import {
  authorTags,
  replyPermissions,
  summarizeAnchor,
  type ThreadPermissions,
  threadPermissions,
  wasEdited,
} from '@/lib/comments';
import { errorMessage } from '@/lib/http';
import type { Comment, CommentAuthor, CommentVia } from '@/lib/types';
import { cn } from '@/lib/utils';
import { CommentComposer } from './CommentComposer';
import type { DeleteTarget } from './DeleteCommentDialog';

/** What a read-only caller may do with any comment: nothing. */
const NO_PERMISSIONS: ThreadPermissions = { canEdit: false, canResolve: false, canDelete: false };

interface CommentThreadProps {
  parcelId: string;
  thread: Comment;
  /** The viewer owns the parcel. */
  isOwner: boolean;
  /** The caller may only read (shared as a viewer): no replies, edits or resolving. */
  readOnly?: boolean;
  /** The version in the frame. */
  viewedVersion: number;
  /** The page in the frame, once it has said. */
  framePath: string | null;
  /** The page reported it can't find this thread's spot. */
  missing: boolean;
  active: boolean;
  /** Highlight the thread and its pin, and scroll the page to it. */
  onActivate: () => void;
  onDelete: (target: DeleteTarget) => void;
  /** Tell the viewer a reply or edit is in progress (it holds off reloading). */
  onComposingChange: (key: string, composing: boolean) => void;
}

/** One thread: the comment, where it points, its replies and a reply box. */
export function CommentThread({
  parcelId,
  thread,
  isOwner,
  readOnly = false,
  viewedVersion,
  framePath,
  missing,
  active,
  onActivate,
  onDelete,
  onComposingChange,
}: CommentThreadProps) {
  const update = useUpdateComment(parcelId);
  const createReply = useCreateComment(parcelId);
  const [replying, setReplying] = useState(false);
  const [replyText, setReplyText] = useState('');
  const [editing, setEditing] = useState<number | null>(null);
  const [editText, setEditText] = useState('');

  const composing = replyText.trim() !== '' || editing !== null;
  const key = `thread-${thread.number}`;
  useEffect(() => {
    onComposingChange(key, composing);
  }, [onComposingChange, key, composing]);
  useEffect(() => () => onComposingChange(key, false), [onComposingChange, key]);

  const perms = readOnly ? NO_PERMISSIONS : threadPermissions(thread.author, isOwner);
  const anchor = thread.anchor ? summarizeAnchor(thread.anchor, framePath) : null;
  const onThisPage = thread.anchor !== undefined && thread.anchor.path === framePath;

  const setResolved = (resolved: boolean, undoable = true) => {
    update.mutate(
      { number: thread.number, body: { resolved } },
      {
        onSuccess: () => {
          if (!undoable) return;
          toast.success(`${resolved ? 'Resolved' : 'Reopened'} #${thread.number}`, {
            action: { label: 'Undo', onClick: () => setResolved(!resolved, false) },
          });
        },
        onError: (err) =>
          toast.error(`Could not ${resolved ? 'resolve' : 'reopen'} #${thread.number}`, {
            description: errorMessage(err),
          }),
      },
    );
  };

  const startEdit = (number: number, body: string) => {
    setEditing(number);
    setEditText(body);
  };

  const saveEdit = () => {
    if (editing === null) return;
    update.mutate(
      { number: editing, body: { body: editText.trim() } },
      {
        onSuccess: () => setEditing(null),
        onError: (err) => toast.error('Could not save', { description: errorMessage(err) }),
      },
    );
  };

  const sendReply = () => {
    createReply.mutate(
      { body: replyText.trim(), reply_to: thread.number },
      {
        onSuccess: () => {
          setReplyText('');
          setReplying(false);
        },
        onError: (err) => toast.error('Could not reply', { description: errorMessage(err) }),
      },
    );
  };

  const entry = (
    number: number,
    author: CommentAuthor,
    via: CommentVia,
    body: string,
    created: string,
    updated: string,
    entryPerms: ThreadPermissions,
    isThread: boolean,
    badge?: ReactNode,
  ) => (
    <CommentEntry
      number={number}
      author={author}
      via={via}
      body={body}
      created={created}
      updated={updated}
      perms={entryPerms}
      badge={badge}
      editing={editing === number}
      editText={editText}
      onEditTextChange={setEditText}
      onStartEdit={() => startEdit(number, body)}
      onCancelEdit={() => setEditing(null)}
      onSaveEdit={saveEdit}
      saving={update.isPending && editing === number}
      onDelete={() =>
        onDelete({
          number,
          isThread,
          replies: isThread ? thread.replies.length : 0,
          moderating: !author.is_you,
        })
      }
    />
  );

  const numberBadge = (
    <span
      className={cn(
        'inline-flex h-6 min-w-6 shrink-0 items-center justify-center rounded-full px-1.5 font-semibold text-[11px]',
        thread.resolved ? 'bg-muted text-muted-foreground' : 'bg-primary text-primary-foreground',
      )}
    >
      {thread.number}
    </span>
  );

  return (
    <article
      data-comment-number={thread.number}
      tabIndex={-1}
      aria-label={`Comment #${thread.number} by ${thread.author.name}`}
      className={cn(
        'space-y-2 rounded-lg border bg-card p-3 outline-none transition-shadow',
        active && 'border-primary/50 ring-2 ring-primary/30',
      )}
    >
      {entry(
        thread.number,
        thread.author,
        thread.via,
        thread.body,
        thread.created_at,
        thread.updated_at,
        perms,
        true,
        onThisPage ? (
          <button
            type="button"
            onClick={onActivate}
            aria-label={`Show #${thread.number} on the page`}
            className="rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {numberBadge}
          </button>
        ) : (
          numberBadge
        ),
      )}

      {thread.version !== viewedVersion || thread.resolved ? (
        <div className="flex flex-wrap gap-1.5 pl-8">
          {thread.version !== viewedVersion && (
            <Badge variant="muted" title={`Written on version ${thread.version}`}>
              v{thread.version}
            </Badge>
          )}
          {thread.resolved && (
            <Badge variant="success">
              <Check />
              Resolved
            </Badge>
          )}
        </div>
      ) : null}

      {anchor && (
        <div className="pl-8">
          {onThisPage ? (
            <button
              type="button"
              onClick={onActivate}
              className="group flex w-full items-start gap-1.5 rounded-md border-primary/40 border-l-2 bg-muted/50 px-2 py-1 text-left text-muted-foreground text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              title="Show on the page"
            >
              <span className="sr-only">Show on the page: </span>
              <Crosshair
                className="mt-0.5 h-3 w-3 shrink-0 group-hover:text-primary"
                aria-hidden="true"
              />
              <AnchorText text={anchor.text} page={anchor.page} />
            </button>
          ) : (
            <div className="flex items-start gap-1.5 rounded-md border-l-2 bg-muted/50 px-2 py-1 text-muted-foreground text-xs">
              <Crosshair className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
              <AnchorText text={anchor.text} page={anchor.page} />
            </div>
          )}
          {missing && onThisPage && (
            <p className="mt-1 flex items-center gap-1.5 text-amber-700 text-xs dark:text-amber-400">
              <AlertTriangle className="h-3 w-3 shrink-0" aria-hidden="true" />
              Can't find this spot on v{viewedVersion}
            </p>
          )}
        </div>
      )}

      {thread.replies.length > 0 && (
        <ol
          className="ml-8 space-y-3 border-l pt-1 pl-3"
          aria-label={`Replies to #${thread.number}`}
        >
          {thread.replies.map((r) => (
            <li key={r.number}>
              {entry(
                r.number,
                r.author,
                r.via,
                r.body,
                r.created_at,
                r.updated_at,
                readOnly ? NO_PERMISSIONS : replyPermissions(r.author, isOwner),
                false,
              )}
            </li>
          ))}
        </ol>
      )}

      {!readOnly && (
        <div className="pl-8">
          {replying ? (
            <CommentComposer
              value={replyText}
              onChange={setReplyText}
              onSubmit={sendReply}
              onCancel={() => {
                setReplying(false);
                setReplyText('');
              }}
              pending={createReply.isPending}
              label={`Reply to #${thread.number}`}
              placeholder="Reply…"
              submitLabel="Reply"
              autoFocus
            />
          ) : (
            <div className="flex gap-1">
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-muted-foreground text-xs"
                onClick={() => setReplying(true)}
                aria-label={`Reply to #${thread.number}`}
              >
                Reply
              </Button>
              {perms.canResolve && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 px-2 text-muted-foreground text-xs"
                  onClick={() => setResolved(!thread.resolved)}
                  disabled={update.isPending}
                  aria-label={`${thread.resolved ? 'Reopen' : 'Resolve'} #${thread.number}`}
                >
                  {thread.resolved ? <RotateCcw /> : <Check />}
                  {thread.resolved ? 'Reopen' : 'Resolve'}
                </Button>
              )}
            </div>
          )}
        </div>
      )}
    </article>
  );
}

function AnchorText({ text, page }: { text: string | null; page: string | null }) {
  return (
    <span className="min-w-0 break-words">
      {text ? <span className="italic">“{text}”</span> : 'A spot on the page'}
      {page && (
        <>
          {' '}
          on <span className="font-mono">{page}</span>
        </>
      )}
    </span>
  );
}

interface CommentEntryProps {
  number: number;
  author: CommentAuthor;
  via: CommentVia;
  body: string;
  created: string;
  updated: string;
  perms: ThreadPermissions;
  /** Leading element (the thread's number). */
  badge?: ReactNode;
  editing: boolean;
  editText: string;
  onEditTextChange: (text: string) => void;
  onStartEdit: () => void;
  onCancelEdit: () => void;
  onSaveEdit: () => void;
  saving: boolean;
  onDelete: () => void;
}

/** Author line, actions and text of one comment or reply. Text is never parsed as HTML. */
function CommentEntry({
  number,
  author,
  via,
  body,
  created,
  updated,
  perms,
  badge,
  editing,
  editText,
  onEditTextChange,
  onStartEdit,
  onCancelEdit,
  onSaveEdit,
  saving,
  onDelete,
}: CommentEntryProps) {
  const tags = authorTags(author, via);
  return (
    <div className="space-y-1">
      <div className="flex items-start gap-2">
        {badge}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-sm leading-6">
            <span className="min-w-0 truncate font-medium" title={author.name}>
              {author.name}
            </span>
            {tags.map((tag) => (
              <Badge
                key={tag}
                variant={tag === 'via CLI' ? 'outline' : 'muted'}
                className="px-1.5 py-0 text-[10px]"
              >
                {tag}
              </Badge>
            ))}
          </div>
          <p className="text-muted-foreground text-xs">
            {/* Replies are numbered too (the CLI addresses comments by number). */}
            {badge === undefined && <span className="font-mono">#{number} · </span>}
            <RelativeTime value={created} />
            {wasEdited(created, updated) && ' · edited'}
          </p>
        </div>
        {(perms.canEdit || perms.canDelete) && !editing && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <IconButton
                label={`Comment #${number} options`}
                className="-mr-1 h-7 w-7 shrink-0 text-muted-foreground"
              >
                <MoreHorizontal />
              </IconButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-40">
              {perms.canEdit && (
                <DropdownMenuItem onSelect={onStartEdit}>
                  <Pencil />
                  Edit
                </DropdownMenuItem>
              )}
              {perms.canDelete && (
                <DropdownMenuItem
                  onSelect={onDelete}
                  className="text-destructive focus:bg-destructive/10 focus:text-destructive [&_svg]:text-destructive"
                >
                  <Trash2 />
                  Delete…
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      <div className={cn(badge !== undefined && 'pl-8')}>
        {editing ? (
          <CommentComposer
            value={editText}
            onChange={onEditTextChange}
            onSubmit={onSaveEdit}
            onCancel={onCancelEdit}
            pending={saving}
            label={`Edit comment #${number}`}
            placeholder="Comment"
            submitLabel="Save"
            autoFocus
          />
        ) : (
          <p className="whitespace-pre-wrap break-words text-sm">{body}</p>
        )}
      </div>
    </div>
  );
}
