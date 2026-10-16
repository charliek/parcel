import { describe, expect, it } from 'bun:test';
import { normalizeEmail, removeShare, shareDisplay, upsertShare } from './shares';
import type { Share } from './types';

const at = '2026-10-01T00:00:00Z';
const ada: Share = {
  email: 'ada@example.com',
  role: 'viewer',
  name: 'Ada Lovelace',
  created_at: at,
};
const bob: Share = { email: 'bob@example.com', role: 'commenter', created_at: at };
const cy: Share = { email: 'cy@example.com', role: 'viewer', name: null, created_at: at };

describe('normalizeEmail', () => {
  it('trims and lowercases, like the server', () => {
    expect(normalizeEmail('  Ada@Example.COM ')).toBe('ada@example.com');
    expect(normalizeEmail('')).toBe('');
  });
});

describe('upsertShare', () => {
  it('adds in email order', () => {
    expect(upsertShare({ shares: [ada, cy] }, bob).shares.map((s) => s.email)).toEqual([
      'ada@example.com',
      'bob@example.com',
      'cy@example.com',
    ]);
  });

  it('replaces the share with the same email (a role change)', () => {
    const changed = { ...ada, role: 'commenter' as const };
    const list = upsertShare({ shares: [ada, bob] }, changed);
    expect(list.shares).toHaveLength(2);
    expect(list.shares[0]).toEqual(changed);
  });
});

describe('removeShare', () => {
  it('drops only that email', () => {
    expect(removeShare({ shares: [ada, bob] }, 'ada@example.com').shares).toEqual([bob]);
    expect(removeShare({ shares: [ada] }, 'nobody@example.com').shares).toEqual([ada]);
  });
});

describe('shareDisplay', () => {
  it('shows the name and email once they have signed in', () => {
    expect(shareDisplay(ada)).toEqual({
      primary: 'Ada Lovelace',
      secondary: 'ada@example.com',
      pending: false,
    });
  });

  it('shows the email alone, pending, until then', () => {
    for (const share of [bob, cy, { ...cy, name: '  ' }]) {
      expect(shareDisplay(share)).toEqual({ primary: share.email, secondary: null, pending: true });
    }
  });
});
