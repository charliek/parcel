import { describe, expect, it } from 'bun:test';
import {
  dashboardHref,
  dashboardSearch,
  EMPTY_QUERY,
  facetFilters,
  facetMode,
  isFiltered,
  listFilters,
  mergeFacetCounts,
  parseDashboardQuery,
  RELAXED_NOTICE,
  showsRelaxedNotice,
  splitLastWord,
  swapFacetFilters,
  toggleTag,
} from './search-query';
import type { ParcelList, ParcelListItem } from './types';

const parse = (s: string) => parseDashboardQuery(new URLSearchParams(s));

describe('dashboard query ⇄ URL', () => {
  it('reads every part, with defaults', () => {
    expect(parse('')).toEqual(EMPTY_QUERY);
    expect(parse('scope=all&tag=project:roost&tag=auth&q=refresh+token&archived=include')).toEqual({
      scope: 'all',
      tags: ['project:roost', 'auth'],
      q: 'refresh token',
      archived: 'include',
    });
    expect(parse('scope=nope&archived=maybe&tag=&tag=a&tag=a')).toEqual({
      ...EMPTY_QUERY,
      tags: ['a'],
    });
    expect(parse('archived=1').archived).toBe('include');
    expect(parse('archived=only').archived).toBe('only');
  });

  it('maps the old ?tab=shared links', () => {
    expect(parse('tab=shared').scope).toBe('shared');
    expect(parse('tab=shared&scope=all').scope).toBe('all');
  });

  it('writes a canonical address and round-trips', () => {
    const query = {
      scope: 'shared',
      tags: ['kind:plan', 'auth'],
      q: ' a b ',
      archived: 'include',
    } as const;
    const search = dashboardSearch({ ...query, tags: [...query.tags] });
    expect(search.toString()).toBe('scope=shared&tag=kind%3Aplan&tag=auth&q=a+b&archived=include');
    expect(parse(search.toString())).toEqual({ ...query, tags: [...query.tags], q: 'a b' });
    expect(dashboardSearch(EMPTY_QUERY).toString()).toBe('');
    // Other parameters stay; the legacy tab goes.
    expect(dashboardSearch(EMPTY_QUERY, new URLSearchParams('tab=shared&x=1')).toString()).toBe(
      'x=1',
    );
  });

  it('links to a filtered dashboard', () => {
    expect(dashboardHref({})).toBe('/');
    expect(dashboardHref({ scope: 'all', tags: ['project:roost'] })).toBe(
      '/?scope=all&tag=project%3Aroost',
    );
    expect(dashboardHref({ tags: ['a'], archived: 'include' })).toBe('/?tag=a&archived=include');
  });
});

describe('toggleTag', () => {
  it('adds and removes', () => {
    expect(toggleTag([], 'auth')).toEqual(['auth']);
    expect(toggleTag(['auth', 'ios'], 'auth')).toEqual(['ios']);
    expect(toggleTag(['auth'], 'team:web')).toEqual(['auth', 'team:web']);
    expect(toggleTag(['team:web'], 'team:ios')).toEqual(['team:web', 'team:ios']);
  });

  it('keeps one project and one kind', () => {
    expect(toggleTag(['project:a', 'auth', 'kind:plan'], 'project:b')).toEqual([
      'auth',
      'kind:plan',
      'project:b',
    ]);
    expect(toggleTag(['kind:plan'], 'kind:report')).toEqual(['kind:report']);
  });
});

describe('list filters', () => {
  it('passes the state to the API', () => {
    expect(listFilters({ scope: 'all', tags: ['a'], q: '', archived: 'exclude' })).toEqual({
      scope: 'all',
      tag: ['a'],
      q: undefined,
      archived: 'exclude',
    });
    expect(listFilters(EMPTY_QUERY).tag).toBeUndefined();
    expect(isFiltered(EMPTY_QUERY)).toBe(false);
    expect(isFiltered({ ...EMPTY_QUERY, archived: 'include' })).toBe(false);
    expect(isFiltered({ ...EMPTY_QUERY, q: 'x' })).toBe(true);
  });

  it('splits the search text at its last word', () => {
    expect(splitLastWord('refresh tok')).toEqual({ rest: 'refresh ', last: 'tok' });
    expect(splitLastWord('proj')).toEqual({ rest: '', last: 'proj' });
    expect(splitLastWord('done ')).toEqual({ rest: 'done ', last: '' });
    expect(splitLastWord('')).toEqual({ rest: '', last: '' });
  });
});

describe('swap facets', () => {
  it('counts a selected project or kind without its own selection', () => {
    const query = {
      scope: 'all',
      tags: ['project:a', 'auth', 'kind:plan'],
      q: 'x',
      archived: 'exclude',
    } as const;
    expect(swapFacetFilters({ ...query, tags: [...query.tags] })).toEqual([
      {
        namespace: 'project',
        filters: {
          scope: 'all',
          tag: ['auth', 'kind:plan'],
          q: 'x',
          archived: 'exclude',
          prefix: 'project:',
        },
      },
      {
        namespace: 'kind',
        filters: {
          scope: 'all',
          tag: ['project:a', 'auth'],
          q: 'x',
          archived: 'exclude',
          prefix: 'kind:',
        },
      },
    ]);
    expect(swapFacetFilters({ ...EMPTY_QUERY, tags: ['auth'] })).toEqual([]);
  });

  it('puts those counts in place of the main ones', () => {
    const main = [
      { tag: 'project:a', count: 2 },
      { tag: 'kind:plan', count: 2 },
      { tag: 'auth', count: 2 },
    ];
    const swapped = [
      {
        namespace: 'project',
        counts: [
          { tag: 'project:a', count: 2 },
          { tag: 'project:b', count: 5 },
        ],
      },
    ];
    expect(mergeFacetCounts(main, swapped)).toEqual([
      { tag: 'kind:plan', count: 2 },
      { tag: 'auth', count: 2 },
      { tag: 'project:a', count: 2 },
      { tag: 'project:b', count: 5 },
    ]);
  });
});

describe('relaxed search', () => {
  const item = { id: 'abcdefghijkm' } as ParcelListItem;
  const relaxed: ParcelList = { parcels: [item], total: 1, relaxed: true };
  const exact: ParcelList = { parcels: [item], total: 1 };
  const searching = { ...EMPTY_QUERY, q: 'faster rust builds' };

  it('shows the notice only over a relaxed list for the current query, with results', () => {
    expect(showsRelaxedNotice(relaxed, false)).toBe(true);
    // The query changed: the old relaxed results stay up while the new ones
    // load, without the notice.
    expect(showsRelaxedNotice(relaxed, true)).toBe(false);
    expect(showsRelaxedNotice(exact, false)).toBe(false);
    expect(showsRelaxedNotice({ parcels: [], total: 0, relaxed: true }, false)).toBe(false);
    expect(showsRelaxedNotice(undefined, false)).toBe(false);
    expect(RELAXED_NOTICE).toBe('No exact matches; showing parcels that match any of the words.');
  });

  it('gives the facets the list’s mode, and waits until it is known', () => {
    expect(facetMode(EMPTY_QUERY, relaxed, false)).toBeUndefined();
    expect(facetMode(searching, relaxed, false)).toBe('relaxed');
    expect(facetMode(searching, exact, false)).toBe('exact');
    expect(facetMode(searching, relaxed, true)).toBeNull();
    expect(facetMode(searching, undefined, false)).toBeNull();
  });

  it('sends that mode on the main and every swap request', () => {
    const query = { ...searching, tags: ['project:a', 'kind:plan'] };
    expect(facetFilters(query, 'relaxed')).toEqual({
      scope: 'mine',
      tag: ['project:a', 'kind:plan'],
      q: 'faster rust builds',
      archived: 'exclude',
      mode: 'relaxed',
    });
    expect(swapFacetFilters(query, 'relaxed').map((swap) => swap.filters.mode)).toEqual([
      'relaxed',
      'relaxed',
    ]);
    // No mode (no query): the filters are the list's, so the cache entry is shared.
    expect(facetFilters(EMPTY_QUERY)).toEqual(listFilters(EMPTY_QUERY));
    expect(swapFacetFilters(query)[0].filters).not.toHaveProperty('mode');
  });
});
