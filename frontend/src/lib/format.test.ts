import { describe, expect, it } from 'bun:test';
import {
  asSentence,
  formatBytes,
  formatDate,
  formatDateTime,
  formatExpiry,
  formatNumber,
  formatRelativeTime,
  initials,
  isExpiringSoon,
  percentOf,
  pluralize,
} from './format';

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

describe('formatBytes', () => {
  it('shows bytes below one kilobyte', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1)).toBe('1 B');
    expect(formatBytes(1023)).toBe('1023 B');
  });

  it('uses one decimal below 10 units and drops trailing .0', () => {
    expect(formatBytes(KB)).toBe('1 KB');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(9.94 * KB)).toBe('9.9 KB');
    expect(formatBytes(2.25 * MB)).toBe('2.3 MB');
  });

  it('rounds to whole numbers from 10 units up', () => {
    expect(formatBytes(15 * MB)).toBe('15 MB');
    expect(formatBytes(16 * MB)).toBe('16 MB');
    expect(formatBytes(64 * MB)).toBe('64 MB');
    expect(formatBytes(512.4 * KB)).toBe('512 KB');
  });

  it('matches the server limits exactly', () => {
    expect(formatBytes(256 * MB)).toBe('256 MB');
    expect(formatBytes(2 * GB)).toBe('2 GB');
  });

  it('rolls over to the next unit instead of printing 1024', () => {
    expect(formatBytes(1023.9 * KB)).toBe('1 MB');
  });

  it('treats invalid and negative input as zero', () => {
    expect(formatBytes(-5)).toBe('0 B');
    expect(formatBytes(Number.NaN)).toBe('0 B');
  });
});

describe('pluralize / formatNumber / percentOf', () => {
  it('pluralizes by count', () => {
    expect(pluralize(1, 'file')).toBe('1 file');
    expect(pluralize(0, 'file')).toBe('0 files');
    expect(pluralize(1200, 'parcel')).toBe('1,200 parcels');
    expect(pluralize(2, 'entry', 'entries')).toBe('2 entries');
  });

  it('adds thousands separators', () => {
    expect(formatNumber(1234567)).toBe('1,234,567');
  });

  it('computes a clamped percentage, 0 with no limit', () => {
    expect(percentOf(32, 64)).toBe(50);
    expect(percentOf(100, 64)).toBe(100);
    expect(percentOf(5, 0)).toBe(0);
  });
});

describe('formatRelativeTime', () => {
  const now = new Date('2026-10-16T12:00:00Z');
  const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();
  const ahead = (ms: number) => new Date(now.getTime() + ms).toISOString();
  const MIN = 60_000;
  const HOUR = 60 * MIN;
  const DAY = 24 * HOUR;

  it('says just now for the last few seconds', () => {
    expect(formatRelativeTime(ago(10_000), now)).toBe('just now');
    expect(formatRelativeTime(now, now)).toBe('just now');
  });

  it('counts minutes, hours and days', () => {
    expect(formatRelativeTime(ago(MIN), now)).toBe('1 minute ago');
    expect(formatRelativeTime(ago(5 * MIN), now)).toBe('5 minutes ago');
    expect(formatRelativeTime(ago(2 * HOUR), now)).toBe('2 hours ago');
    expect(formatRelativeTime(ago(DAY), now)).toBe('yesterday');
    expect(formatRelativeTime(ago(3 * DAY), now)).toBe('3 days ago');
    expect(formatRelativeTime(ago(14 * DAY), now)).toBe('2 weeks ago');
  });

  it('handles the future', () => {
    expect(formatRelativeTime(ahead(10 * MIN), now)).toBe('in 10 minutes');
    expect(formatRelativeTime(ahead(DAY), now)).toBe('tomorrow');
    expect(formatRelativeTime(ahead(5_000), now)).toBe('in a few seconds');
  });

  it('falls back to a date after four weeks', () => {
    expect(formatRelativeTime('2026-08-03T12:00:00Z', now, { timeZone: 'UTC' })).toBe('Aug 3');
    expect(formatRelativeTime('2025-03-03T12:00:00Z', now, { timeZone: 'UTC' })).toBe(
      'Mar 3, 2025',
    );
  });

  it('handles missing and malformed input', () => {
    expect(formatRelativeTime(null, now)).toBe('never');
    expect(formatRelativeTime('not a date', now)).toBe('unknown');
  });
});

describe('formatDate / formatDateTime', () => {
  const now = new Date('2026-10-16T12:00:00Z');
  it('omits the year in the current year', () => {
    expect(formatDate('2026-01-05T08:00:00Z', now, { timeZone: 'UTC' })).toBe('Jan 5');
    expect(formatDate('2024-12-31T08:00:00Z', now, { timeZone: 'UTC' })).toBe('Dec 31, 2024');
  });

  it('formats an absolute date and time', () => {
    expect(formatDateTime('2026-03-03T14:15:00Z', { timeZone: 'UTC' })).toBe(
      'Mar 3, 2026, 2:15 PM',
    );
    expect(formatDateTime(null)).toBe('');
  });
});

describe('expiry helpers', () => {
  const now = new Date('2026-10-16T12:00:00Z');

  it('formats token expiry', () => {
    expect(formatExpiry(null, now)).toBe('Never');
    expect(formatExpiry('2026-10-15T12:00:00Z', now)).toBe('Expired');
    expect(formatExpiry('2026-10-19T12:00:00Z', now)).toBe('in 3 days');
  });

  it('detects grants close to expiry', () => {
    const fiveMin = 5 * 60_000;
    expect(isExpiringSoon('2026-10-16T12:04:00Z', fiveMin, now)).toBe(true);
    expect(isExpiringSoon('2026-10-16T11:00:00Z', fiveMin, now)).toBe(true);
    expect(isExpiringSoon('2026-10-16T12:30:00Z', fiveMin, now)).toBe(false);
    expect(isExpiringSoon(null, fiveMin, now)).toBe(false);
    expect(isExpiringSoon('garbage', fiveMin, now)).toBe(true);
  });
});

describe('initials', () => {
  it('uses the name, else the email local part', () => {
    expect(initials('Charlie Knudsen', 'c@x.com')).toBe('CK');
    expect(initials('', 'charlie.knudsen@x.com')).toBe('CK');
    expect(initials(null, 'dev@localhost')).toBe('DE');
  });
});

describe('asSentence', () => {
  it('capitalizes the first letter only', () => {
    expect(asSentence('that’s you: owners always have access')).toBe(
      'That’s you: owners always have access',
    );
    expect(asSentence('"x" is not an email address')).toBe('"x" is not an email address');
    expect(asSentence('')).toBe('');
  });
});
