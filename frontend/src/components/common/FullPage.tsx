import { Loader2, type LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { Brand } from '@/components/layout/Brand';
import { cn } from '@/lib/utils';

/** Centered splash while something resolves. */
export function Splash({ message }: { message?: string }) {
  return (
    <div
      className="flex min-h-dvh flex-col items-center justify-center gap-5 bg-background"
      role="status"
      aria-live="polite"
    >
      <Brand />
      <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
      <span className={cn(message ? 'text-muted-foreground text-sm' : 'sr-only')}>
        {message ?? 'Loading'}
      </span>
    </div>
  );
}

interface FullPageMessageProps {
  icon: LucideIcon;
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  tone?: 'default' | 'destructive';
  /** Show the brand link above the message (for pages without a header). */
  showBrand?: boolean;
  className?: string;
}

/** A centered status screen: not found, sign in to view, errors. */
export function FullPageMessage({
  icon: Icon,
  title,
  description,
  actions,
  tone = 'default',
  showBrand = true,
  className,
}: FullPageMessageProps) {
  return (
    <div
      className={cn(
        'flex min-h-dvh flex-col items-center justify-center bg-background px-4 py-12',
        className,
      )}
    >
      {showBrand && (
        <Link
          to="/"
          className="mb-10 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Brand />
        </Link>
      )}
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        <div
          className={cn(
            'flex h-12 w-12 items-center justify-center rounded-full',
            tone === 'destructive'
              ? 'bg-destructive/10 text-destructive'
              : 'bg-primary/10 text-primary',
          )}
        >
          <Icon className="h-6 w-6" aria-hidden="true" />
        </div>
        <div className="space-y-1.5">
          <h1 className="font-semibold text-xl tracking-tight">{title}</h1>
          {description && <div className="text-muted-foreground text-sm">{description}</div>}
        </div>
        {actions && <div className="mt-2 flex flex-wrap justify-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}
