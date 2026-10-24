import type { ComponentProps } from 'react';
import { cn } from '@/lib/utils';

/** The "Archived" marker beside a parcel's title (callers show it only when archived). */
export function ArchivedBadge({ className, ...props }: ComponentProps<'span'>) {
  return (
    <span
      className={cn(
        'rounded bg-muted px-1.5 py-0.5 font-mono font-semibold text-[10px] text-muted-foreground uppercase tracking-wide',
        className,
      )}
      {...props}
    >
      Archived
    </span>
  );
}
