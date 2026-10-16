/**
 * Human-friendly formatting for sizes, counts and times.
 *
 * Pure functions with an injectable `now` so they are deterministic in tests.
 */

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

/**
 * Format a byte count with binary (1024) steps: `0 B`, `512 B`, `1.5 KB`,
 * `16 MB`, `2 GB`. One decimal below 10 units, whole numbers above, and
 * trailing `.0` dropped. The server's limits are MiB-based, so `16 MB` here
 * matches a 16 MiB limit exactly.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  if (unit === 0) return `${Math.round(value)} B`;
  // Round first so 1023.96 KB becomes "1 MB" rather than "1024 KB".
  let rounded = value < 10 ? Math.round(value * 10) / 10 : Math.round(value);
  if (rounded >= 1024 && unit < BYTE_UNITS.length - 1) {
    rounded = 1;
    unit++;
  }
  return `${Number.isInteger(rounded) ? rounded.toFixed(0) : rounded.toFixed(1)} ${BYTE_UNITS[unit]}`;
}

/** `1 file`, `3 files`. Irregular plurals pass `plural`. */
export function pluralize(count: number, singular: string, plural = `${singular}s`): string {
  return `${formatNumber(count)} ${count === 1 ? singular : plural}`;
}

/** Thousands separators: 12345 -> "12,345". */
export function formatNumber(value: number): string {
  return new Intl.NumberFormat('en-US').format(value);
}

/** Percentage of `used` against `limit`, clamped to 0..100. 0 when there is no limit. */
export function percentOf(used: number, limit: number): number {
  if (!Number.isFinite(limit) || limit <= 0) return 0;
  return Math.min(100, Math.max(0, (used / limit) * 100));
}

function toDate(value: string | Date): Date | null {
  const date = value instanceof Date ? value : new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/**
 * Relative time like Claude's gallery: `just now`, `5 minutes ago`,
 * `2 hours ago`, `yesterday`, `3 days ago`, `in 10 minutes`. Beyond four weeks
 * it falls back to a short date (`Mar 3`, or `Mar 3, 2025` in another year).
 */
export function formatRelativeTime(
  value: string | Date | null | undefined,
  now: Date = new Date(),
  options: { timeZone?: string } = {},
): string {
  if (!value) return 'never';
  const date = toDate(value);
  if (!date) return 'unknown';

  const diff = date.getTime() - now.getTime();
  const abs = Math.abs(diff);
  if (abs < 45 * SECOND) return diff > 0 ? 'in a few seconds' : 'just now';

  const rtf = new Intl.RelativeTimeFormat('en-US', { numeric: 'auto' });
  const sign = diff < 0 ? -1 : 1;
  if (abs < 45 * MINUTE) return rtf.format(sign * Math.max(1, Math.round(abs / MINUTE)), 'minute');
  if (abs < 22 * HOUR) return rtf.format(sign * Math.max(1, Math.round(abs / HOUR)), 'hour');
  if (abs < 7 * DAY) return rtf.format(sign * Math.max(1, Math.round(abs / DAY)), 'day');
  if (abs < 28 * DAY) return rtf.format(sign * Math.round(abs / (7 * DAY)), 'week');
  return formatDate(date, now, options);
}

/** Short calendar date: `Mar 3` this year, `Mar 3, 2025` otherwise. */
export function formatDate(
  value: string | Date,
  now: Date = new Date(),
  options: { timeZone?: string } = {},
): string {
  const date = toDate(value);
  if (!date) return 'unknown';
  const sameYear =
    new Intl.DateTimeFormat('en-US', { year: 'numeric', timeZone: options.timeZone }).format(
      date,
    ) ===
    new Intl.DateTimeFormat('en-US', { year: 'numeric', timeZone: options.timeZone }).format(now);
  return new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    year: sameYear ? undefined : 'numeric',
    timeZone: options.timeZone,
  }).format(date);
}

/** Absolute date and time for tooltips: `Mar 3, 2026, 2:15 PM`. */
export function formatDateTime(
  value: string | Date | null | undefined,
  options: { timeZone?: string } = {},
): string {
  if (!value) return '';
  const date = toDate(value);
  if (!date) return '';
  return new Intl.DateTimeFormat('en-US', {
    dateStyle: 'medium',
    timeStyle: 'short',
    timeZone: options.timeZone,
  }).format(date);
}

/** Token / grant expiry: `Never`, `Expired`, or `in 30 days`. */
export function formatExpiry(
  value: string | Date | null | undefined,
  now: Date = new Date(),
): string {
  if (!value) return 'Never';
  const date = toDate(value);
  if (!date) return 'Never';
  if (date.getTime() <= now.getTime()) return 'Expired';
  return formatRelativeTime(date, now);
}

/** Whether `expiresAt` is in the past or within `withinMs` of `now`. */
export function isExpiringSoon(
  expiresAt: string | Date | null | undefined,
  withinMs: number,
  now: Date = new Date(),
): boolean {
  if (!expiresAt) return false;
  const date = toDate(expiresAt);
  if (!date) return true;
  return date.getTime() - now.getTime() <= withinMs;
}

/** Two-letter initials for an avatar, from a name or else an email. */
export function initials(name: string | null | undefined, email: string): string {
  const source = name?.trim() || email.split('@')[0] || email;
  const parts = source.split(/[\s._-]+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  return source.slice(0, 2).toUpperCase();
}

/**
 * A server message as a sentence: the API's messages start lowercase
 * ("that's you: owners always have access") because they also read well
 * after a prefix; shown on their own, they get a capital.
 */
export function asSentence(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1);
}
