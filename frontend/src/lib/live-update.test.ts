import { describe, expect, it } from 'bun:test';
import { decideLiveUpdate, type LiveUpdateInput, parseAutoReload } from './live-update';

const base: LiveUpdateInput = {
  shown: 4,
  latest: 5,
  pinned: false,
  composing: false,
  autoReload: true,
};

describe('decideLiveUpdate', () => {
  it('does nothing when the frame already shows the newest version', () => {
    expect(decideLiveUpdate({ ...base, latest: 4 })).toBe('up-to-date');
    // A publish from this tab can make the grant newer than the last poll.
    expect(decideLiveUpdate({ ...base, latest: 3 })).toBe('up-to-date');
    expect(decideLiveUpdate({ ...base, latest: 4, pinned: true, composing: true })).toBe(
      'up-to-date',
    );
  });

  it('reloads the current-version view when nothing is in progress', () => {
    expect(decideLiveUpdate(base)).toBe('reload');
    expect(decideLiveUpdate({ ...base, latest: 9 })).toBe('reload');
  });

  it('never moves a viewer pinned to a version', () => {
    expect(decideLiveUpdate({ ...base, pinned: true })).toBe('pinned');
    expect(decideLiveUpdate({ ...base, pinned: true, autoReload: false })).toBe('pinned');
    expect(decideLiveUpdate({ ...base, pinned: true, composing: true })).toBe('pinned');
  });

  it('offers the reload instead while composing a comment', () => {
    expect(decideLiveUpdate({ ...base, composing: true })).toBe('offer-reload');
  });

  it('offers the reload instead when auto-reload is off', () => {
    expect(decideLiveUpdate({ ...base, autoReload: false })).toBe('offer-reload');
    expect(decideLiveUpdate({ ...base, autoReload: false, composing: true })).toBe('offer-reload');
  });
});

describe('parseAutoReload', () => {
  it('defaults to on', () => {
    expect(parseAutoReload(null)).toBe(true);
    expect(parseAutoReload('1')).toBe(true);
    expect(parseAutoReload('garbage')).toBe(true);
  });

  it('is off only when stored as 0', () => {
    expect(parseAutoReload('0')).toBe(false);
  });
});
