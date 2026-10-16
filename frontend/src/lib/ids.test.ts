import { describe, expect, it } from 'bun:test';
import { isValidParcelId, parseVersionParam } from './ids';

describe('isValidParcelId', () => {
  it('accepts 12 characters from the id alphabet', () => {
    expect(isValidParcelId('abcdefghijkm')).toBe(true);
    expect(isValidParcelId('k7m2qx9vhd4n')).toBe(true);
  });

  it('rejects other shapes', () => {
    expect(isValidParcelId('short')).toBe(false);
    expect(isValidParcelId('abcdefghijk0')).toBe(false); // '0' is not in the alphabet
    expect(isValidParcelId('abcdefghijkl')).toBe(false); // nor 'l'
    expect(isValidParcelId('ABCDEFGHIJKM')).toBe(false);
    expect(isValidParcelId('abcdefghijkmn')).toBe(false);
    expect(isValidParcelId(undefined)).toBe(false);
  });
});

describe('parseVersionParam', () => {
  it('parses positive integers', () => {
    expect(parseVersionParam('1')).toBe(1);
    expect(parseVersionParam('42')).toBe(42);
  });

  it('returns null when absent', () => {
    expect(parseVersionParam(undefined)).toBeNull();
  });

  it('flags anything else as invalid', () => {
    expect(parseVersionParam('0')).toBe('invalid');
    expect(parseVersionParam('-1')).toBe('invalid');
    expect(parseVersionParam('01')).toBe('invalid');
    expect(parseVersionParam('latest')).toBe('invalid');
    expect(parseVersionParam('1.5')).toBe('invalid');
    expect(parseVersionParam('')).toBe('invalid');
  });
});
