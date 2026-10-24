import { Fragment } from 'react';
import type { MatchSegment } from '@/lib/types';
import { cn } from '@/lib/utils';

/** Control characters never reach the screen (the server strips them too). */
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching them is the point.
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/g;

/**
 * Where a search matched a parcel's page text. The segments are the
 * parcel's own content, so they are rendered as React text nodes (escaped),
 * never as HTML: invariant 1, user content never runs on the app origin.
 */
export function Snippet({ segments, className }: { segments: MatchSegment[]; className?: string }) {
  if (segments.length === 0) return null;
  return (
    <p
      className={cn('line-clamp-2 text-muted-foreground text-xs', className)}
      data-testid="snippet"
    >
      <span aria-hidden="true">…</span>
      {segments.map((segment, index) => {
        const text = segment.text.replace(CONTROL, '');
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional and never reorder.
          <Fragment key={index}>
            {segment.hit ? (
              <mark className="rounded-sm bg-amber-200/70 px-0.5 text-foreground dark:bg-amber-400/30">
                {text}
              </mark>
            ) : (
              text
            )}
          </Fragment>
        );
      })}
      <span aria-hidden="true">…</span>
    </p>
  );
}
