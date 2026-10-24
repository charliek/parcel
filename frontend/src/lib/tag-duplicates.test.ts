import { describe, expect, it } from 'bun:test';
import { editDistance, possibleDuplicates } from './tag-duplicates';

const t = (tag: string, count: number) => ({ tag, count });

describe('editDistance', () => {
  it('counts insertions, deletions and substitutions', () => {
    expect(editDistance('session', 'sessions')).toBe(1);
    expect(editDistance('kitten', 'sitting')).toBe(3);
    expect(editDistance('same', 'same')).toBe(0);
    expect(editDistance('', 'abc')).toBe(3);
  });

  it('gives up past the bound', () => {
    expect(editDistance('abcdefgh', 'zzzzzzzz', 2)).toBe(3);
    expect(editDistance('a', 'abcdef', 2)).toBe(3);
  });
});

describe('possibleDuplicates', () => {
  it('suggests merging the less used of a prefix or near-miss pair', () => {
    expect(
      possibleDuplicates([
        t('project:roost', 14),
        t('project:roost-app', 2),
        t('kind:research', 7),
        t('sessions', 4),
        t('session', 1),
        t('auth', 3),
      ]),
    ).toEqual([
      { from: 'project:roost-app', to: 'project:roost' },
      { from: 'session', to: 'sessions' },
    ]);
  });

  it('stays within a namespace and skips short values', () => {
    expect(possibleDuplicates([t('project:roost', 3), t('kind:roost', 1)])).toEqual([]);
    expect(possibleDuplicates([t('ui', 3), t('ui-kit', 1)])).toEqual([]);
    expect(possibleDuplicates([t('ios', 3), t('ios2', 1)])).toEqual([]);
  });

  it('uses edit distance two at most', () => {
    expect(possibleDuplicates([t('analytics', 2), t('analitycs', 1)])).toEqual([
      { from: 'analitycs', to: 'analytics' },
    ]);
    expect(possibleDuplicates([t('planning', 2), t('plotting', 1)])).toEqual([]);
  });

  it('breaks count ties toward the shorter tag', () => {
    expect(possibleDuplicates([t('deploys', 1), t('deploy', 1)])).toEqual([
      { from: 'deploys', to: 'deploy' },
    ]);
  });
});
