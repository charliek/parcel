import { describe, expect, it } from 'bun:test';
import { query } from './api';

describe('query', () => {
  it('repeats array values and leaves out empty ones', () => {
    expect(query({ scope: 'all', tag: ['project:a', 'kind:plan'], q: '', limit: 48 })).toBe(
      '?scope=all&tag=project%3Aa&tag=kind%3Aplan&limit=48',
    );
    expect(query({ tag: [], q: undefined, x: null })).toBe('');
    expect(query({ path: 'docs/a b.html' })).toBe('?path=docs%2Fa+b.html');
  });
});
