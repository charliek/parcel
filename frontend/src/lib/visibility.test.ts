import { describe, expect, it } from 'bun:test';
import {
  PUBLIC_OFF_REASON,
  VISIBILITIES,
  VISIBILITY,
  visibilityChangedText,
  visibilityOptions,
} from './visibility';

describe('visibilityOptions', () => {
  it('offers all three, narrowest first', () => {
    const options = visibilityOptions(true);
    expect(options.map((o) => o.value)).toEqual(['private', 'internal', 'public']);
    expect(options.map((o) => o.label)).toEqual([
      'Private',
      'Everyone signed in',
      'Anyone with the link',
    ]);
    expect(options.every((o) => o.disabledReason === null)).toBe(true);
  });

  it('disables public, with the reason, when the server turned it off', () => {
    const options = visibilityOptions(false);
    expect(options.find((o) => o.value === 'public')?.disabledReason).toBe(PUBLIC_OFF_REASON);
    expect(options.filter((o) => o.disabledReason !== null)).toHaveLength(1);
  });
});

describe('visibility words', () => {
  it('has a badge, hint and tooltip for each', () => {
    for (const v of VISIBILITIES) {
      expect(VISIBILITY[v].badge.length).toBeGreaterThan(0);
      expect(VISIBILITY[v].hint.length).toBeGreaterThan(0);
      expect(VISIBILITY[v].who.length).toBeGreaterThan(0);
    }
    expect(VISIBILITY.internal.badge).toBe('Internal');
  });

  it('says what a change means', () => {
    expect(visibilityChangedText('public').title).toBe('Parcel is now public');
    expect(visibilityChangedText('internal').title).toBe(
      'Parcel is now open to everyone signed in',
    );
    expect(visibilityChangedText('private').description).toContain('people you add');
  });
});
