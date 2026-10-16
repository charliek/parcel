import { describe, expect, it } from 'bun:test';
import {
  contentToken,
  copyAnchor,
  isVerifiedReady,
  MAX_ANCHOR_TEXT_CHARS,
  MAX_SELECTOR_CHARS,
  MAX_TOKEN_CHARS,
  PAGE_SOURCE,
  type PageMessage,
  parseAnchor,
  parsePageMessage,
  toPage,
  VIEWER_SOURCE,
} from './frame-protocol';

// The shape of a real content token: base64url claims, a dot, a signature.
const TOKEN = 'cDJ4a3FtbjR0YmNkLjMuMTc2MDYyNDAwMC5v.Zm9vYmFyYmF6cXV4';

const anchor = { path: 'index.html', selector: 'html > body > h1', text: 'Hello', x: 0.5, y: 0 };
const msg = (type: string, extra: Record<string, unknown> = {}) => ({
  source: PAGE_SOURCE,
  type,
  ...extra,
});

describe('parseAnchor', () => {
  it('accepts what the page helper sends', () => {
    expect(parseAnchor(anchor)).toEqual(anchor);
    expect(parseAnchor({ path: 'docs/a page.html' })).toEqual({ path: 'docs/a page.html' });
  });

  it('drops unknown fields (the server rejects them)', () => {
    const parsed = parseAnchor({ ...anchor, body: 'sneaky', __proto__x: 1 });
    expect(parsed).toEqual(anchor);
    expect(Object.keys(parsed ?? {})).toEqual(['path', 'selector', 'text', 'x', 'y']);
  });

  it('returns a new object', () => {
    const parsed = parseAnchor(anchor);
    expect(parsed).not.toBe(anchor);
  });

  it('rejects bad paths', () => {
    for (const path of ['', '/abs.html', '../x.html', 'a//b.html', 'a\\b.html', 'a\nb', 7, null]) {
      expect(parseAnchor({ ...anchor, path })).toBeNull();
    }
    expect(parseAnchor({ selector: 'h1' })).toBeNull();
  });

  it('rejects wrong types and out-of-range values', () => {
    expect(parseAnchor({ ...anchor, selector: 3 })).toBeNull();
    expect(parseAnchor({ ...anchor, selector: '   ' })).toBeNull();
    expect(parseAnchor({ ...anchor, selector: 'a'.repeat(MAX_SELECTOR_CHARS + 1) })).toBeNull();
    expect(parseAnchor({ ...anchor, text: { toString: 'x' } })).toBeNull();
    expect(parseAnchor({ ...anchor, text: 'é'.repeat(MAX_ANCHOR_TEXT_CHARS + 1) })).toBeNull();
    expect(parseAnchor({ ...anchor, x: 1.5 })).toBeNull();
    expect(parseAnchor({ ...anchor, y: -0.1 })).toBeNull();
    expect(parseAnchor({ ...anchor, x: Number.NaN })).toBeNull();
    expect(parseAnchor({ ...anchor, y: '0.5' })).toBeNull();
  });

  it('counts characters, not UTF-16 units, like the server', () => {
    const emoji = '😀'.repeat(MAX_ANCHOR_TEXT_CHARS);
    expect(parseAnchor({ ...anchor, text: emoji })?.text).toBe(emoji);
  });

  it('treats empty text as no text', () => {
    expect(parseAnchor({ path: 'index.html', text: '' })).toEqual({ path: 'index.html' });
  });

  it('rejects non-objects', () => {
    for (const value of [null, undefined, 'index.html', [anchor], 1]) {
      expect(parseAnchor(value)).toBeNull();
    }
  });
});

describe('parsePageMessage', () => {
  it('ignores anything not from the page helper', () => {
    expect(parsePageMessage(null)).toBeNull();
    expect(parsePageMessage('ready')).toBeNull();
    expect(parsePageMessage([msg('escape')])).toBeNull();
    expect(parsePageMessage({ type: 'escape' })).toBeNull();
    expect(parsePageMessage({ source: VIEWER_SOURCE, type: 'escape' })).toBeNull();
    expect(parsePageMessage(msg('unknown'))).toBeNull();
  });

  it('parses ready', () => {
    expect(
      parsePageMessage(msg('ready', { path: 'index.html', title: 'Hi', version: 3, token: TOKEN })),
    ).toEqual({ type: 'ready', path: 'index.html', title: 'Hi', version: 3, token: TOKEN });
    expect(
      parsePageMessage(msg('ready', { path: 'a.html', version: Number.NaN, token: TOKEN })),
    ).toEqual({ type: 'ready', path: 'a.html', title: '', version: null, token: TOKEN });
    expect(
      parsePageMessage(msg('ready', { path: '../etc', title: 'x', version: 1, token: TOKEN })),
    ).toBeNull();
    const long = parsePageMessage(
      msg('ready', { path: 'a.html', title: 'x'.repeat(5000), token: TOKEN }),
    );
    expect(long?.type === 'ready' && long.title.length).toBe(300);
  });

  it('requires a token on ready, as an opaque bounded string', () => {
    // The helper sends null when its URL isn't a parcel URL.
    for (const token of [
      undefined,
      null,
      '',
      42,
      ['t'],
      { t: 1 },
      'x'.repeat(MAX_TOKEN_CHARS + 1),
    ]) {
      expect(parsePageMessage(msg('ready', { path: 'index.html', token }))).toBeNull();
    }
    const opaque = 'any thing/at all?';
    expect(parsePageMessage(msg('ready', { path: 'index.html', token: opaque }))).toMatchObject({
      token: opaque,
    });
    expect(
      parsePageMessage(msg('ready', { path: 'index.html', token: 'x'.repeat(MAX_TOKEN_CHARS) })),
    ).not.toBeNull();
  });

  it('parses bye', () => {
    expect(parsePageMessage(msg('bye'))).toEqual({ type: 'bye' });
    expect(parsePageMessage({ type: 'bye' })).toBeNull();
  });

  it('parses anchor', () => {
    expect(parsePageMessage(msg('anchor', { anchor }))).toEqual({ type: 'anchor', anchor });
    expect(parsePageMessage(msg('anchor', { anchor: { ...anchor, x: 2 } }))).toBeNull();
    expect(parsePageMessage(msg('anchor'))).toBeNull();
  });

  it('parses pin-click with a positive integer number only', () => {
    expect(parsePageMessage(msg('pin-click', { number: 4 }))).toEqual({
      type: 'pin-click',
      number: 4,
    });
    for (const number of [0, -1, 1.5, '4', null, Number.POSITIVE_INFINITY, 2 ** 60]) {
      expect(parsePageMessage(msg('pin-click', { number }))).toBeNull();
    }
  });

  it('parses located', () => {
    expect(parsePageMessage(msg('located', { found: [1, 3], missing: [] }))).toEqual({
      type: 'located',
      found: [1, 3],
      missing: [],
    });
    expect(parsePageMessage(msg('located', { found: [1, 'x'], missing: [] }))).toBeNull();
    expect(parsePageMessage(msg('located', { found: [1] }))).toBeNull();
    const many = Array.from({ length: 2001 }, (_, i) => i + 1);
    expect(parsePageMessage(msg('located', { found: many, missing: [] }))).toBeNull();
  });

  it('parses escape', () => {
    expect(parsePageMessage(msg('escape'))).toEqual({ type: 'escape' });
  });
});

describe('toPage', () => {
  it('tags every message as the viewer', () => {
    expect(toPage.hello()).toEqual({ source: VIEWER_SOURCE, type: 'hello' });
    expect(toPage.commentMode(true)).toEqual({
      source: VIEWER_SOURCE,
      type: 'comment-mode',
      enabled: true,
    });
    expect(toPage.focus(null)).toEqual({ source: VIEWER_SOURCE, type: 'focus', number: null });
    expect(toPage.draft(null)).toEqual({ source: VIEWER_SOURCE, type: 'draft', anchor: null });
  });

  it('sends only anchor fields, numbers and flags in pins', () => {
    const leaky = {
      number: 2,
      anchor: { ...anchor, body: 'secret' } as typeof anchor,
      resolved: false,
      active: true,
      body: 'secret',
      author: { name: 'Ann' },
    };
    const out = toPage.pins([leaky]);
    expect(JSON.stringify(out)).not.toContain('secret');
    expect(JSON.stringify(out)).not.toContain('Ann');
    expect(out).toEqual({
      source: VIEWER_SOURCE,
      type: 'pins',
      pins: [{ number: 2, anchor, resolved: false, active: true }],
    });
  });

  it('copies draft anchors', () => {
    const out = toPage.draft({ ...anchor, extra: 1 } as typeof anchor);
    expect(out).toEqual({ source: VIEWER_SOURCE, type: 'draft', anchor });
  });
});

describe('copyAnchor', () => {
  it('keeps only known fields', () => {
    expect(copyAnchor({ path: 'a.html', x: 0 })).toEqual({ path: 'a.html', x: 0 });
  });
});

describe('contentToken', () => {
  it('reads the token from a content URL, as the helper reads its location', () => {
    expect(contentToken(`https://parcelcontent.example.com/v/${TOKEN}/`)).toBe(TOKEN);
    expect(contentToken(`http://127.0.0.1:18110/v/${TOKEN}/docs/a.html?x=1#y`)).toBe(TOKEN);
    expect(contentToken(`http://127.0.0.1:18110/v/${TOKEN}`)).toBe(TOKEN);
  });

  it('is null for anything else', () => {
    for (const url of [
      'not a url',
      'https://parcelcontent.example.com/',
      'https://parcelcontent.example.com/d/abc/votes',
      'https://parcelcontent.example.com/x/v/abc/',
      'https://parcelcontent.example.com/v//index.html',
      `https://parcelcontent.example.com/v/${'x'.repeat(MAX_TOKEN_CHARS + 1)}/`,
    ]) {
      expect(contentToken(url)).toBeNull();
    }
  });
});

describe('isVerifiedReady', () => {
  const ready = (token: string): PageMessage => ({
    type: 'ready',
    path: 'index.html',
    title: '',
    version: 1,
    token,
  });

  it('accepts only the token of the URL the frame was given', () => {
    expect(isVerifiedReady(ready(TOKEN), TOKEN)).toBe(true);
    expect(isVerifiedReady(ready(`${TOKEN}x`), TOKEN)).toBe(false);
    expect(isVerifiedReady(ready(TOKEN.toUpperCase()), TOKEN)).toBe(false);
    // Another parcel's page (another valid token) on the same content origin.
    expect(isVerifiedReady(ready('b3RoZXIuMS4yLm8.c2ln'), TOKEN)).toBe(false);
  });

  it('never verifies without an expected token, or for other messages', () => {
    expect(isVerifiedReady(ready(TOKEN), null)).toBe(false);
    expect(isVerifiedReady({ type: 'escape' }, TOKEN)).toBe(false);
    expect(isVerifiedReady({ type: 'bye' }, TOKEN)).toBe(false);
  });

  it('round-trips with contentToken', () => {
    const expected = contentToken(`http://127.0.0.1:18110/v/${TOKEN}/index.html`);
    expect(isVerifiedReady(ready(TOKEN), expected)).toBe(true);
  });
});
