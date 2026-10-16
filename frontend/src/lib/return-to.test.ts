import { describe, expect, it } from 'bun:test';
import { safeReturnTo } from './return-to';

describe('safeReturnTo', () => {
  it('keeps same-origin paths with query and hash', () => {
    expect(safeReturnTo('/')).toBe('/');
    expect(safeReturnTo('/p/abcdefghijkm')).toBe('/p/abcdefghijkm');
    expect(safeReturnTo('/device?code=BCDF-GHJK')).toBe('/device?code=BCDF-GHJK');
    expect(safeReturnTo('/p/abcdefghijkm/v/2#x')).toBe('/p/abcdefghijkm/v/2#x');
  });

  it('rejects anything that could leave the origin', () => {
    expect(safeReturnTo('https://evil.example')).toBe('/');
    expect(safeReturnTo('//evil.example/path')).toBe('/');
    expect(safeReturnTo('/\\evil.example')).toBe('/');
    expect(safeReturnTo('javascript:alert(1)')).toBe('/');
    expect(safeReturnTo('relative/path')).toBe('/');
  });

  it('rejects control characters, empties and the callback itself', () => {
    expect(safeReturnTo('/p/x\nSet-Cookie:a')).toBe('/');
    expect(safeReturnTo('')).toBe('/');
    expect(safeReturnTo(null)).toBe('/');
    expect(safeReturnTo('/auth/callback')).toBe('/');
    expect(safeReturnTo('/auth/callback?code=x')).toBe('/');
  });
});
