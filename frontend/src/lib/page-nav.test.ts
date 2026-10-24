import { describe, expect, it } from 'bun:test';
import {
  addressForFrame,
  initialPageNav,
  nextPageNav,
  readPageParam,
  versionListing,
} from './page-nav';
import type { ParcelDetail, VersionDetail } from './types';

const PAGES = [
  { path: 'index.html', title: 'One' },
  { path: 'two.html', title: 'Two' },
];

describe('nextPageNav', () => {
  const start = initialPageNav('abcdefghijkm', null, null);
  const at = (urlPage: string | null, version: number | null = null) => ({
    id: 'abcdefghijkm',
    version,
    urlPage,
  });

  it('requests a page the viewer navigates to', () => {
    const next = nextPageNav(start, at('two.html'), {
      framePath: 'index.html',
      entry: 'index.html',
    });
    expect(next).toMatchObject({ urlPage: 'two.html', requested: 'two.html', nonce: 1 });
  });

  it('requests nothing when the frame already shows the page', () => {
    // A link inside the page went to two.html, and the address followed.
    const followed = nextPageNav(start, at('two.html'), {
      framePath: 'two.html',
      entry: 'index.html',
    });
    expect(followed).toEqual({ ...start, urlPage: 'two.html' });
    // Its own Back returns to the entry: still nothing to fetch.
    const back = nextPageNav(followed, at(null), { framePath: 'index.html', entry: 'index.html' });
    expect(back).toEqual(start);
  });

  it('reloads when the address leaves the page the frame shows', () => {
    const followed = { ...start, urlPage: 'two.html' };
    // The Pages menu goes back to the entry while the frame is on two.html:
    // `requested` was already the entry, so only the nonce says "reload".
    const next = nextPageNav(followed, at(null), { framePath: 'two.html', entry: 'index.html' });
    expect(next).toMatchObject({ urlPage: null, requested: null, nonce: 1 });
  });

  it('reloads while the frame has not said where it is', () => {
    const next = nextPageNav(start, at('two.html'), { framePath: null, entry: 'index.html' });
    expect(next.nonce).toBe(1);
  });

  it('starts over for another version or parcel', () => {
    const followed = { ...start, urlPage: 'two.html' };
    const next = nextPageNav(followed, at('two.html', 2), {
      framePath: 'two.html',
      entry: 'index.html',
    });
    expect(next).toMatchObject({ version: 2, requested: 'two.html', nonce: 1 });
  });

  it('returns the same state when nothing changed', () => {
    expect(nextPageNav(start, at(null), { framePath: null, entry: null })).toBe(start);
  });

  it('reads ?page=', () => {
    expect(readPageParam(new URLSearchParams('page=a%20b.html'))).toBe('a b.html');
    expect(readPageParam(new URLSearchParams('page='))).toBeNull();
    expect(readPageParam(new URLSearchParams(''))).toBeNull();
  });
});

describe('addressForFrame', () => {
  const base = { pages: PAGES, entry: 'index.html', urlPage: null, urlHash: '', frameHash: '' };

  it('follows a page the frame went to, dropping ?page= for the entry', () => {
    expect(addressForFrame({ ...base, framePath: 'two.html' })).toEqual({
      page: 'two.html',
      hash: '',
    });
    expect(addressForFrame({ ...base, framePath: 'index.html', urlPage: 'two.html' })).toEqual({
      page: null,
      hash: '',
    });
  });

  it('keeps the fragment in step', () => {
    expect(addressForFrame({ ...base, framePath: 'index.html', frameHash: '#s2' })).toEqual({
      page: null,
      hash: '#s2',
    });
    expect(
      addressForFrame({ ...base, framePath: 'index.html', urlHash: '#s2', frameHash: '' }),
    ).toEqual({ page: null, hash: '' });
  });

  it('changes nothing when the address matches', () => {
    expect(addressForFrame({ ...base, framePath: 'index.html' })).toBeNull();
    expect(addressForFrame({ ...base, framePath: 'index.html', urlPage: 'index.html' })).toBeNull();
    expect(
      addressForFrame({
        ...base,
        framePath: 'two.html',
        urlPage: 'two.html',
        urlHash: '#a',
        frameHash: '#a',
      }),
    ).toBeNull();
  });

  it('ignores a path that is not one of the pages, or before the pages are known', () => {
    expect(addressForFrame({ ...base, framePath: 'evil.html' })).toBeNull();
    expect(addressForFrame({ ...base, framePath: 'two.html', pages: null })).toBeNull();
    expect(
      addressForFrame({ ...base, framePath: 'index.html', frameHash: 'javascript:x' }),
    ).toBeNull();
  });
});

describe('versionListing', () => {
  const V2_PAGES = [...PAGES, { path: 'three.html', title: 'Three' }];
  const file = (path: string) => ({ path, size_bytes: 1, content_type: 'text/html', sha256: '' });
  // Only the fields versionListing reads.
  const detail = (current: number, pages: typeof PAGES) =>
    ({
      parcel: { current_version: current },
      files: pages.map((p) => file(p.path)),
      pages,
    }) as unknown as ParcelDetail;
  const versionDetail = (version: number, pages: typeof PAGES) =>
    ({ version, files: pages.map((p) => file(p.path)), pages }) as unknown as VersionDetail;

  it('uses the parcel detail when it describes the shown version', () => {
    expect(versionListing(2, detail(2, V2_PAGES), undefined)?.pages).toEqual(V2_PAGES);
  });

  it("does not vouch for a new version's pages with the old version's detail", () => {
    // A live update showed v2 before the detail (still v1's) refreshed.
    expect(versionListing(2, detail(1, PAGES), undefined)).toBeNull();
  });

  it("falls back to the shown version's own detail", () => {
    expect(versionListing(2, detail(3, PAGES), versionDetail(2, V2_PAGES))?.pages).toEqual(
      V2_PAGES,
    );
    expect(versionListing(2, detail(3, PAGES), versionDetail(1, PAGES))).toBeNull();
  });

  it('has no pages for a server that lists none', () => {
    const old = { ...detail(1, PAGES), pages: undefined } as ParcelDetail;
    expect(versionListing(1, old, undefined)).toEqual({ files: old.files, pages: null });
  });
});
