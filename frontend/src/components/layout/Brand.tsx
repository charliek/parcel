import { Package } from 'lucide-react';
import { cn } from '@/lib/utils';

/** The Parcel wordmark: a package glyph on the accent tile. */
export function Brand({ className, compact }: { className?: string; compact?: boolean }) {
  return (
    <span className={cn('inline-flex items-center gap-2', className)}>
      <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary text-primary-foreground shadow-sm">
        <Package className="h-4 w-4" strokeWidth={2.25} aria-hidden="true" />
      </span>
      {!compact && <span className="font-semibold text-[15px] tracking-tight">Parcel</span>}
    </span>
  );
}
