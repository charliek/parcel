import { describe, expect, it } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { Snippet } from './Snippet';

describe('Snippet', () => {
  it('renders segments as escaped text with hits marked', () => {
    const html = renderToStaticMarkup(
      <Snippet
        segments={[
          { text: 'before <img src=x onerror=alert(1)> ', hit: false },
          { text: 'refresh', hit: true },
          { text: ' after\u0007\u001b[31m 🧪 é', hit: false },
        ]}
      />,
    );
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
    expect(html).toContain('<mark class="');
    expect(html).toContain('>refresh</mark>');
    expect(html).not.toContain('\u0007');
    expect(html).not.toContain('\u001b');
    expect(html).toContain('[31m 🧪 é');
  });

  it('renders nothing without segments', () => {
    expect(renderToStaticMarkup(<Snippet segments={[]} />)).toBe('');
  });
});
