/**
 * Pure helpers behind the comments panel: who may do what, how an anchor is
 * summarized, which pins a page gets, and cache updates after a mutation.
 */

import { copyAnchor, type FramePin } from './frame-protocol';
import type {
  Comment,
  CommentAnchor,
  CommentAuthor,
  CommentList,
  CommentVia,
  NewComment,
} from './types';

/** Longest comment or reply body, in characters (`comments::MAX_COMMENT_CHARS`). */
export const MAX_COMMENT_CHARS = 4000;

/** Longest anchor snippet the panel shows before cutting it with an ellipsis. */
export const SNIPPET_CHARS = 80;

/** Collapse whitespace and cut to `max` characters (by code point) with an ellipsis. */
export function snippet(text: string, max = SNIPPET_CHARS): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  const chars = Array.from(flat);
  if (chars.length <= max) return flat;
  return `${chars
    .slice(0, max - 1)
    .join('')
    .trimEnd()}…`;
}

export interface AnchorSummary {
  /** The clicked element's text, shortened; null when it had none. */
  text: string | null;
  /** The page, when it isn't the one in view; null otherwise. */
  page: string | null;
}

/** What the panel shows about where a comment points. */
export function summarizeAnchor(anchor: CommentAnchor, currentPath: string | null): AnchorSummary {
  const text = anchor.text ? snippet(anchor.text) : '';
  return {
    text: text || null,
    page: currentPath !== null && anchor.path === currentPath ? null : anchor.path,
  };
}

/** The same summary as one line: `“Pricing” on pricing.html`. */
export function anchorLabel(anchor: CommentAnchor, currentPath: string | null): string {
  const { text, page } = summarizeAnchor(anchor, currentPath);
  if (text && page) return `“${text}” on ${page}`;
  if (text) return `“${text}”`;
  if (page) return `A spot on ${page}`;
  return 'A spot on this page';
}

/** Whether a body is sendable: not blank and within the server's limit. */
export function isValidBody(body: string): boolean {
  const trimmed = body.trim();
  return trimmed !== '' && Array.from(trimmed).length <= MAX_COMMENT_CHARS;
}

/**
 * Whether a comment's text was changed after it was posted. The server
 * stamps both times together on insert and bumps `updated_at` only on edits.
 */
export function wasEdited(created: string, updated: string): boolean {
  const a = Date.parse(created);
  const b = Date.parse(updated);
  return Number.isFinite(a) && Number.isFinite(b) && b > a;
}

/** A spot picked on the page, with the version the frame showed when it was picked. */
export interface CommentDraft {
  anchor: CommentAnchor;
  version: number;
}

/**
 * The request for a new thread. An anchored comment goes on the version
 * its spot was picked on, whatever the frame shows by the time it is sent;
 * a general one on the version in view.
 */
export function newThread(
  body: string,
  draft: CommentDraft | null,
  shownVersion: number,
): NewComment {
  return draft
    ? { body: body.trim(), version: draft.version, anchor: draft.anchor }
    : { body: body.trim(), version: shownVersion };
}

/** Short labels next to an author's name. */
export function authorTags(author: CommentAuthor, via: CommentVia): string[] {
  const tags: string[] = [];
  if (author.is_you) tags.push('You');
  if (author.is_owner) tags.push('Owner');
  if (via === 'cli') tags.push('via CLI');
  return tags;
}

export interface ThreadPermissions {
  canEdit: boolean;
  canResolve: boolean;
  canDelete: boolean;
}

/**
 * The server's rules (`http/comments.rs`): only the author edits; the owner
 * or the thread's author resolves; the author or the owner deletes.
 */
export function threadPermissions(
  author: CommentAuthor,
  viewerIsOwner: boolean,
): ThreadPermissions {
  return {
    canEdit: author.is_you,
    canResolve: author.is_you || viewerIsOwner,
    canDelete: author.is_you || viewerIsOwner,
  };
}

/** Replies can't be resolved; otherwise the same rules as threads. */
export function replyPermissions(author: CommentAuthor, viewerIsOwner: boolean): ThreadPermissions {
  return { ...threadPermissions(author, viewerIsOwner), canResolve: false };
}

/**
 * Pins for the page at `path`: its open threads, plus resolved ones when the
 * panel is showing resolved threads. Only numbers, anchors and flags.
 */
export function pinsForPage(
  threads: readonly Comment[],
  path: string | null,
  options: { includeResolved: boolean; active: number | null },
): FramePin[] {
  if (path === null) return [];
  const pins: FramePin[] = [];
  for (const t of threads) {
    if (!t.anchor || t.anchor.path !== path) continue;
    if (t.resolved && !options.includeResolved) continue;
    pins.push({
      number: t.number,
      anchor: copyAnchor(t.anchor),
      resolved: t.resolved,
      active: t.number === options.active,
    });
  }
  return pins;
}

export function countThreads(threads: readonly Comment[]): { open: number; resolved: number } {
  let resolved = 0;
  for (const t of threads) if (t.resolved) resolved++;
  return { open: threads.length - resolved, resolved };
}

/** The thread holding `number` (a thread's own number or one of its replies'). */
export function findThread(threads: readonly Comment[], number: number): Comment | undefined {
  return threads.find((t) => t.number === number || t.replies.some((r) => r.number === number));
}

/** Put a thread the server returned into a cached list (replacing it, or appending). */
export function upsertThread(list: CommentList, thread: Comment): CommentList {
  const index = list.comments.findIndex((t) => t.number === thread.number);
  const comments =
    index === -1
      ? [...list.comments, thread].sort((a, b) => a.number - b.number)
      : list.comments.map((t, i) => (i === index ? thread : t));
  return { comments, open: countThreads(comments).open };
}

/** Drop a deleted comment from a cached list: a whole thread, or one reply. */
export function removeComment(list: CommentList, number: number): CommentList {
  const comments = list.comments
    .filter((t) => t.number !== number)
    .map((t) =>
      t.replies.some((r) => r.number === number)
        ? { ...t, replies: t.replies.filter((r) => r.number !== number) }
        : t,
    );
  return { comments, open: countThreads(comments).open };
}
