import { describe, expect, it } from 'bun:test';
import {
  canonicalPath,
  encodePagePath,
  pageLabel,
  parseSlugPath,
  shareLink,
  viewerPath,
} from './refs';
import type { ParcelSummary } from './types';

function summary(slug: string | null): ParcelSummary {
  return {
    id: 'abcdefghijkm',
    title: 'T',
    description: '',
    visibility: 'private',
    current_version: 3,
    entry: 'index.html',
    file_count: 1,
    total_bytes: 1,
    created_at: '1970-01-01T00:00:00Z',
    updated_at: '1970-01-01T00:00:00Z',
    url: 'https://x.test/p/abcdefghijkm',
    open_comments: 0,
    slug,
    owner_handle: 'ada',
    canonical_url: slug ? `https://x.test/@ada/${slug}` : 'https://x.test/p/abcdefghijkm',
  };
}

const at = (version: number | null, page: string | null, hash = '') => ({ version, page, hash });

// The same cases as the CLI's `links_pin_versions_and_pages`.
describe('shareLink', () => {
  it('pins versions and pages like the CLI', () => {
    const plain = summary(null);
    expect(shareLink(plain, at(null, null))).toBe(plain.url);
    expect(shareLink(plain, at(2, 'a b.html'))).toBe(
      'https://x.test/p/abcdefghijkm/v/2?page=a%20b.html',
    );
    const slugged = summary('parcel/plan');
    expect(shareLink(slugged, at(2, 'b.md', '#s'))).toBe(
      'https://x.test/@ada/parcel/plan?v=2&page=b.md#s',
    );
    expect(shareLink(slugged, at(null, null))).toBe('https://x.test/@ada/parcel/plan');
  });

  it('falls back to the legacy url for an older server', () => {
    const old = { ...summary(null), canonical_url: undefined, slug: undefined };
    expect(shareLink(old, at(null, 'docs/b.html'))).toBe(
      'https://x.test/p/abcdefghijkm?page=docs/b.html',
    );
  });
});

describe('viewerPath', () => {
  it('builds in-app paths for both routes', () => {
    const id = { kind: 'id', id: 'abcdefghijkm' } as const;
    expect(viewerPath(id, at(null, null))).toBe('/p/abcdefghijkm');
    expect(viewerPath(id, at(4, 'two.html', '#x'))).toBe('/p/abcdefghijkm/v/4?page=two.html#x');
    const slug = { kind: 'slug', path: '/@ada/parcel/plan' } as const;
    expect(viewerPath(slug, at(null, 'two.html'))).toBe('/@ada/parcel/plan?page=two.html');
    expect(viewerPath(slug, at(4, null))).toBe('/@ada/parcel/plan?v=4');
  });

  it('encodes pages as the CLI does', () => {
    expect(encodePagePath('docs/a b&c?.html')).toBe('docs/a%20b%26c%3F.html');
    expect(encodePagePath('café.md')).toBe('caf%C3%A9.md');
  });
});

describe('canonicalPath and parseSlugPath', () => {
  it('reads the path of the canonical URL', () => {
    expect(canonicalPath(summary('parcel/plan'))).toBe('/@ada/parcel/plan');
    expect(canonicalPath(summary(null))).toBe('/p/abcdefghijkm');
    expect(canonicalPath({ ...summary(null), canonical_url: '' })).toBe('/p/abcdefghijkm');
  });

  it('splits a clean path into handle and slug', () => {
    expect(parseSlugPath('/@ada/parcel/plan')).toEqual({ handle: 'ada', slug: 'parcel/plan' });
    expect(parseSlugPath('/@ada/plan/')).toEqual({ handle: 'ada', slug: 'plan' });
    expect(parseSlugPath('/@ada')).toBeNull();
    expect(parseSlugPath('/@/plan')).toBeNull();
    expect(parseSlugPath('/p/abcdefghijkm')).toBeNull();
    expect(parseSlugPath('/@ada/%E0%A4%A')).toBeNull();
  });
});

describe('pageLabel', () => {
  it('prefers the title, else a readable path', () => {
    expect(pageLabel({ path: 'index.html', title: 'Overview' })).toBe('Overview');
    expect(pageLabel({ path: 'docs/rust-build_settings.html', title: null })).toBe(
      'Rust build settings',
    );
    expect(pageLabel({ path: 'notes.md', title: '  ' })).toBe('Notes');
    expect(pageLabel({ path: '.html', title: null })).toBe('.html');
  });
});
