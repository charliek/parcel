import { describe, expect, it } from 'bun:test';
import { formatUserCodeInput, normalizeUserCode } from './user-code';

describe('normalizeUserCode', () => {
  it('accepts the canonical form', () => {
    expect(normalizeUserCode('BCDF-GHJK')).toBe('BCDF-GHJK');
  });

  it('ignores case, spaces and the dash', () => {
    expect(normalizeUserCode('bcdf-ghjk')).toBe('BCDF-GHJK');
    expect(normalizeUserCode('bcdfghjk')).toBe('BCDF-GHJK');
    expect(normalizeUserCode('  bcdf ghjk  ')).toBe('BCDF-GHJK');
    expect(normalizeUserCode('BC-DF–GH JK')).toBe('BCDF-GHJK');
  });

  it('keeps digits', () => {
    expect(normalizeUserCode('ab12cd34')).toBe('AB12-CD34');
  });

  it('rejects the wrong length or empty input', () => {
    expect(normalizeUserCode('BCDF-GHJ')).toBeNull();
    expect(normalizeUserCode('BCDF-GHJKL')).toBeNull();
    expect(normalizeUserCode('')).toBeNull();
    expect(normalizeUserCode(null)).toBeNull();
    expect(normalizeUserCode(undefined)).toBeNull();
    expect(normalizeUserCode('----')).toBeNull();
  });
});

describe('formatUserCodeInput', () => {
  it('formats partial input as the user types', () => {
    expect(formatUserCodeInput('b')).toBe('B');
    expect(formatUserCodeInput('bcdf')).toBe('BCDF');
    expect(formatUserCodeInput('bcdfg')).toBe('BCDF-G');
    expect(formatUserCodeInput('BCDF-')).toBe('BCDF');
    expect(formatUserCodeInput('bcdf-ghjk')).toBe('BCDF-GHJK');
  });

  it('drops characters beyond a full code and junk', () => {
    expect(formatUserCodeInput('bcdfghjkxyz')).toBe('BCDF-GHJK');
    expect(formatUserCodeInput('b!c@d#f')).toBe('BCDF');
  });
});
