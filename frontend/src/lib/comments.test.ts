import { describe, expect, it } from 'bun:test';
import {
  anchorLabel,
  authorTags,
  countThreads,
  findThread,
  isValidBody,
  MAX_COMMENT_CHARS,
  newThread,
  pinsForPage,
  removeComment,
  replyPermissions,
  snippet,
  summarizeAnchor,
  threadPermissions,
  upsertThread,
  wasEdited,
} from './comments';
import type { Comment, CommentAuthor, CommentList } from './types';

const you: CommentAuthor = { name: 'You Person', is_owner: false, is_you: true };
const owner: CommentAuthor = { name: 'Owner Person', is_owner: true, is_you: false };
const other: CommentAuthor = { name: 'Someone', is_owner: false, is_you: false };

function thread(number: number, overrides: Partial<Comment> = {}): Comment {
  return {
    number,
    version: 1,
    author: other,
    body: `body ${number}`,
    via: 'web',
    resolved: false,
    resolved_at: null,
    created_at: '2026-10-16T10:00:00Z',
    updated_at: '2026-10-16T10:00:00Z',
    replies: [],
    ...overrides,
  };
}

describe('snippet', () => {
  it('collapses whitespace', () => {
    expect(snippet('  Hello \n\t world  ')).toBe('Hello world');
  });

  it('cuts long text with an ellipsis', () => {
    expect(snippet('abcdefghij', 5)).toBe('abcd…');
    expect(snippet('abcde', 5)).toBe('abcde');
    expect(snippet('ab  cdefgh', 4)).toBe('ab…');
  });

  it('never splits a surrogate pair', () => {
    expect(snippet('😀😀😀😀', 3)).toBe('😀😀…');
  });
});

describe('summarizeAnchor / anchorLabel', () => {
  it('shows the text snippet, and the page only when it is another one', () => {
    const a = { path: 'index.html', text: 'Pricing  table' };
    expect(summarizeAnchor(a, 'index.html')).toEqual({ text: 'Pricing table', page: null });
    expect(summarizeAnchor(a, 'about.html')).toEqual({ text: 'Pricing table', page: 'index.html' });
    expect(anchorLabel(a, 'index.html')).toBe('“Pricing table”');
    expect(anchorLabel(a, 'about.html')).toBe('“Pricing table” on index.html');
  });

  it('names the page while the frame has not reported one', () => {
    expect(summarizeAnchor({ path: 'a.html' }, null)).toEqual({ text: null, page: 'a.html' });
  });

  it('falls back for anchors without text', () => {
    expect(anchorLabel({ path: 'a.html' }, 'a.html')).toBe('A spot on this page');
    expect(anchorLabel({ path: 'a.html', text: '   ' }, 'b.html')).toBe('A spot on a.html');
  });

  it('shortens long element text', () => {
    const label = anchorLabel({ path: 'a.html', text: 'x'.repeat(500) }, 'a.html');
    expect(label.length).toBeLessThan(90);
    expect(label.endsWith('…”')).toBe(true);
  });
});

describe('authorTags', () => {
  it('marks you, the owner and the CLI', () => {
    expect(authorTags(other, 'web')).toEqual([]);
    expect(authorTags({ ...owner, is_you: true }, 'cli')).toEqual(['You', 'Owner', 'via CLI']);
    expect(authorTags(owner, 'cli')).toEqual(['Owner', 'via CLI']);
    expect(authorTags(owner, 'mcp')).toEqual(['Owner', 'via MCP']);
  });
});

describe('permissions', () => {
  it('lets only the author edit', () => {
    expect(threadPermissions(you, false).canEdit).toBe(true);
    expect(threadPermissions(other, true).canEdit).toBe(false);
  });

  it('lets the owner or the author resolve and delete', () => {
    expect(threadPermissions(other, true)).toEqual({
      canEdit: false,
      canResolve: true,
      canDelete: true,
    });
    expect(threadPermissions(you, false)).toEqual({
      canEdit: true,
      canResolve: true,
      canDelete: true,
    });
    expect(threadPermissions(other, false)).toEqual({
      canEdit: false,
      canResolve: false,
      canDelete: false,
    });
  });

  it('never resolves replies', () => {
    expect(replyPermissions(you, true)).toEqual({
      canEdit: true,
      canResolve: false,
      canDelete: true,
    });
  });
});

describe('pinsForPage', () => {
  const threads = [
    thread(1, { anchor: { path: 'index.html', text: 'A', x: 0.1 } }),
    thread(2, { anchor: { path: 'index.html' }, resolved: true }),
    thread(3, { anchor: { path: 'other.html' } }),
    thread(4),
  ];

  it('pins open anchored threads on this page', () => {
    expect(pinsForPage(threads, 'index.html', { includeResolved: false, active: null })).toEqual([
      {
        number: 1,
        anchor: { path: 'index.html', text: 'A', x: 0.1 },
        resolved: false,
        active: false,
      },
    ]);
  });

  it('adds resolved ones when the panel shows them, and flags the active one', () => {
    expect(pinsForPage(threads, 'index.html', { includeResolved: true, active: 2 })).toEqual([
      {
        number: 1,
        anchor: { path: 'index.html', text: 'A', x: 0.1 },
        resolved: false,
        active: false,
      },
      { number: 2, anchor: { path: 'index.html' }, resolved: true, active: true },
    ]);
  });

  it('pins nothing until the page has said where it is', () => {
    expect(pinsForPage(threads, null, { includeResolved: true, active: null })).toEqual([]);
  });

  it('never carries bodies or authors', () => {
    const json = JSON.stringify(
      pinsForPage(threads, 'index.html', { includeResolved: true, active: null }),
    );
    expect(json).not.toContain('body');
    expect(json).not.toContain('Someone');
  });
});

describe('cache updates', () => {
  const reply = { ...thread(5), number: 5 };
  const list: CommentList = {
    comments: [thread(1), thread(3, { resolved: true, replies: [reply] })],
    open: 1,
  };

  it('counts threads by status', () => {
    expect(countThreads(list.comments)).toEqual({ open: 1, resolved: 1 });
  });

  it('finds a thread by its number or a reply number', () => {
    expect(findThread(list.comments, 3)?.number).toBe(3);
    expect(findThread(list.comments, 5)?.number).toBe(3);
    expect(findThread(list.comments, 9)).toBeUndefined();
  });

  it('appends new threads in number order and replaces existing ones', () => {
    const added = upsertThread(list, thread(6));
    expect(added.comments.map((t) => t.number)).toEqual([1, 3, 6]);
    expect(added.open).toBe(2);
    const reopened = upsertThread(list, thread(3));
    expect(reopened.comments.map((t) => t.number)).toEqual([1, 3]);
    expect(reopened.open).toBe(2);
  });

  it('removes a thread or a single reply', () => {
    expect(removeComment(list, 1).comments.map((t) => t.number)).toEqual([3]);
    const withoutReply = removeComment(list, 5);
    expect(withoutReply.comments[1].replies).toEqual([]);
    expect(withoutReply.open).toBe(1);
  });
});

describe('isValidBody', () => {
  it('needs non-blank text within the limit', () => {
    expect(isValidBody('hi')).toBe(true);
    expect(isValidBody('  \n ')).toBe(false);
    expect(isValidBody(` ${'x'.repeat(MAX_COMMENT_CHARS)} `)).toBe(true);
    expect(isValidBody('x'.repeat(MAX_COMMENT_CHARS + 1))).toBe(false);
  });
});

describe('wasEdited', () => {
  it('compares the timestamps', () => {
    expect(wasEdited('2026-10-16T10:00:00Z', '2026-10-16T10:00:00Z')).toBe(false);
    expect(wasEdited('2026-10-16T10:00:00Z', '2026-10-16T10:05:00Z')).toBe(true);
    expect(wasEdited('nope', '2026-10-16T10:05:00Z')).toBe(false);
  });
});

describe('newThread', () => {
  const anchor = { path: 'index.html', selector: 'h1', text: 'Title' };

  it('posts an anchored comment on the version its spot was picked on', () => {
    // Picked on v1; the frame has since moved to v2.
    expect(newThread('  Fix this  ', { anchor, version: 1 }, 2)).toEqual({
      body: 'Fix this',
      version: 1,
      anchor,
    });
  });

  it('posts a general comment on the version in view', () => {
    expect(newThread('Looks good', null, 2)).toEqual({ body: 'Looks good', version: 2 });
  });
});
