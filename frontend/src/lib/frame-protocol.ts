/**
 * The postMessage protocol between the viewer and the page helper that the
 * content origin injects into every HTML page
 * (`crates/parcel-server/assets/parcel-helper.js`; table in PLAN.md).
 *
 * Everything a page sends is untrusted: a parcel is arbitrary HTML and can
 * forge any of these messages. So incoming messages are parsed field by
 * field into fresh objects (never spread), anything out of shape is
 * dropped, and the viewer renders the strings it keeps only as text.
 * Outgoing messages carry anchors, numbers and flags, never comment text or
 * author names.
 *
 * The frame can also be navigated away from the parcel (a link to another
 * site that allows framing). That document is still sandboxed, so it posts
 * from origin `"null"` through the same `contentWindow`, and the origin and
 * source checks can't tell it apart. So the bridge goes live only after a
 * `ready` carrying the capability token of the URL the viewer loaded: the
 * helper reads it from its own location, and a page from anywhere else
 * can't know it. Until then the viewer sends nothing but `hello`.
 */

import type { CommentAnchor } from './types';
import { validatePublishedPath } from './upload';

/** `source` on messages the page helper sends. */
export const PAGE_SOURCE = 'parcel-helper';
/** `source` on messages the viewer sends. */
export const VIEWER_SOURCE = 'parcel-viewer';

/**
 * `MessageEvent.origin` of a document in a sandbox without
 * `allow-same-origin`: its origin is opaque and serializes as `"null"`
 * (checked in Chromium). The content origin always sandboxes, so a real
 * parcel page can't post with anything else.
 */
export const SANDBOXED_ORIGIN = 'null';

/** Same bounds the server enforces (`parcel_types::comments`). */
export const MAX_SELECTOR_CHARS = 1000;
export const MAX_ANCHOR_TEXT_CHARS = 500;
/** More numbers than a parcel can hold comments means the message is junk. */
const MAX_NUMBERS = 2000;
const MAX_TITLE_CHARS = 300;
/** Content tokens are ~100 characters; anything much longer is junk. */
export const MAX_TOKEN_CHARS = 512;

export type PageMessage =
  | { type: 'ready'; path: string; title: string; version: number | null; token: string }
  | { type: 'anchor'; anchor: CommentAnchor }
  | { type: 'pin-click'; number: number }
  | { type: 'located'; found: number[]; missing: number[] }
  | { type: 'escape' }
  | { type: 'bye' };

/** A comment pin as the page draws it. */
export interface FramePin {
  number: number;
  anchor: CommentAnchor;
  resolved: boolean;
  active: boolean;
}

export type ViewerMessage =
  | { source: typeof VIEWER_SOURCE; type: 'hello' }
  | { source: typeof VIEWER_SOURCE; type: 'comment-mode'; enabled: boolean }
  | { source: typeof VIEWER_SOURCE; type: 'pins'; pins: FramePin[] }
  | { source: typeof VIEWER_SOURCE; type: 'draft'; anchor: CommentAnchor | null }
  | { source: typeof VIEWER_SOURCE; type: 'focus'; number: number | null };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Length in Unicode scalar values, which is what the server counts. */
function charCount(s: string): number {
  return Array.from(s).length;
}

function isCommentNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

function isFraction(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1;
}

function numberList(value: unknown): number[] | null {
  if (!Array.isArray(value) || value.length > MAX_NUMBERS) return null;
  return value.every(isCommentNumber) ? [...value] : null;
}

/**
 * An anchor in the shape the server accepts (`CommentAnchor::validate`, and
 * no unknown fields), copied into a new object; null when anything is off.
 */
export function parseAnchor(value: unknown): CommentAnchor | null {
  if (!isRecord(value)) return null;
  const { path, selector, text, x, y } = value;
  if (typeof path !== 'string' || validatePublishedPath(path) !== null) return null;
  const anchor: CommentAnchor = { path };
  if (selector !== undefined) {
    if (
      typeof selector !== 'string' ||
      selector.trim() === '' ||
      charCount(selector) > MAX_SELECTOR_CHARS
    ) {
      return null;
    }
    anchor.selector = selector;
  }
  if (text !== undefined) {
    if (typeof text !== 'string' || charCount(text) > MAX_ANCHOR_TEXT_CHARS) return null;
    if (text !== '') anchor.text = text;
  }
  if (x !== undefined) {
    if (!isFraction(x)) return null;
    anchor.x = x;
  }
  if (y !== undefined) {
    if (!isFraction(y)) return null;
    anchor.y = y;
  }
  return anchor;
}

/** A content token as the helper reports it: opaque, non-empty and bounded. */
function isToken(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= MAX_TOKEN_CHARS;
}

/**
 * The capability token in a content URL (`…/v/{token}/{path}`), as the
 * page helper reads it from `location.pathname`; null if there is none.
 */
export function contentToken(contentUrl: string): string | null {
  let pathname: string;
  try {
    pathname = new URL(contentUrl).pathname;
  } catch {
    return null;
  }
  const match = /^\/v\/([^/]+)(?:\/|$)/.exec(pathname);
  return match && isToken(match[1]) ? match[1] : null;
}

/**
 * Whether a `ready` comes from the parcel page the viewer loaded: its token
 * must equal the one in the frame's URL. Never true without an expected token.
 */
export function isVerifiedReady(
  message: PageMessage,
  expectedToken: string | null,
): message is Extract<PageMessage, { type: 'ready' }> {
  return message.type === 'ready' && expectedToken !== null && message.token === expectedToken;
}

/** Parse a message from the page, or null if it isn't a well-formed helper message. */
export function parsePageMessage(data: unknown): PageMessage | null {
  if (!isRecord(data) || data.source !== PAGE_SOURCE) return null;
  switch (data.type) {
    case 'ready': {
      if (typeof data.path !== 'string' || validatePublishedPath(data.path) !== null) return null;
      if (!isToken(data.token)) return null;
      const title =
        typeof data.title === 'string' ? Array.from(data.title).slice(0, MAX_TITLE_CHARS) : [];
      return {
        type: 'ready',
        path: data.path,
        title: title.join(''),
        version: isCommentNumber(data.version) ? data.version : null,
        token: data.token,
      };
    }
    case 'anchor': {
      const anchor = parseAnchor(data.anchor);
      return anchor ? { type: 'anchor', anchor } : null;
    }
    case 'pin-click':
      return isCommentNumber(data.number) ? { type: 'pin-click', number: data.number } : null;
    case 'located': {
      const found = numberList(data.found);
      const missing = numberList(data.missing);
      return found && missing ? { type: 'located', found, missing } : null;
    }
    case 'escape':
      return { type: 'escape' };
    case 'bye':
      return { type: 'bye' };
    default:
      return null;
  }
}

/** Copy an anchor field by field, so nothing but the anchor reaches the page. */
export function copyAnchor(anchor: CommentAnchor): CommentAnchor {
  const out: CommentAnchor = { path: anchor.path };
  if (anchor.selector !== undefined) out.selector = anchor.selector;
  if (anchor.text !== undefined) out.text = anchor.text;
  if (anchor.x !== undefined) out.x = anchor.x;
  if (anchor.y !== undefined) out.y = anchor.y;
  return out;
}

/** Builders for everything the viewer sends to the page. */
export const toPage = {
  hello: (): ViewerMessage => ({ source: VIEWER_SOURCE, type: 'hello' }),
  commentMode: (enabled: boolean): ViewerMessage => ({
    source: VIEWER_SOURCE,
    type: 'comment-mode',
    enabled,
  }),
  pins: (pins: FramePin[]): ViewerMessage => ({
    source: VIEWER_SOURCE,
    type: 'pins',
    pins: pins.map((p) => ({
      number: p.number,
      anchor: copyAnchor(p.anchor),
      resolved: p.resolved,
      active: p.active,
    })),
  }),
  draft: (anchor: CommentAnchor | null): ViewerMessage => ({
    source: VIEWER_SOURCE,
    type: 'draft',
    anchor: anchor ? copyAnchor(anchor) : null,
  }),
  focus: (number: number | null): ViewerMessage => ({
    source: VIEWER_SOURCE,
    type: 'focus',
    number,
  }),
};
