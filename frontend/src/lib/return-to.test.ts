import { describe, expect, it } from 'bun:test';
import { safeReturnTo } from './return-to';

describe('safeReturnTo', () => {
  it('keeps same-origin paths with query and hash', () => {
    expect(safeReturnTo('/')).toBe('/');
    expect(safeReturnTo('/p/abcdefghijkm')).toBe('/p/abcdefghijkm');
    expect(safeReturnTo('/device?code=BCDF-GHJK')).toBe('/device?code=BCDF-GHJK');
    expect(safeReturnTo('/p/abcdefghijkm/v/2#x')).toBe('/p/abcdefghijkm/v/2#x');
    // Only the exact prefixes are refused.
    expect(safeReturnTo('/authors')).toBe('/authors');
    expect(safeReturnTo('/signing')).toBe('/signing');
  });

  it('rejects anything that could leave the origin', () => {
    expect(safeReturnTo('https://evil.example')).toBe('/');
    expect(safeReturnTo('//evil.example/path')).toBe('/');
    expect(safeReturnTo('/\\evil.example')).toBe('/');
    expect(safeReturnTo('/a\\b')).toBe('/');
    expect(safeReturnTo('javascript:alert(1)')).toBe('/');
    expect(safeReturnTo('relative/path')).toBe('/');
  });

  it('rejects control characters, empties and overlong paths', () => {
    expect(safeReturnTo('/p/x\nSet-Cookie:a')).toBe('/');
    expect(safeReturnTo('')).toBe('/');
    expect(safeReturnTo(null)).toBe('/');
    expect(safeReturnTo(`/${'a'.repeat(2048)}`)).toBe('/');
  });

  it('never returns into sign-in itself', () => {
    expect(safeReturnTo('/auth/callback')).toBe('/');
    expect(safeReturnTo('/auth/callback?code=x')).toBe('/');
    expect(safeReturnTo('/auth/login?provider=okta')).toBe('/');
    expect(safeReturnTo('/auth')).toBe('/');
    expect(safeReturnTo('/signin')).toBe('/');
    expect(safeReturnTo('/signin?error=denied&return_to=%2Fsignin')).toBe('/');
  });
});
