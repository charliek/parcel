import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  className?: string;
  children?: ReactNode;
}

/** A friendly placeholder for empty lists. */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
  children,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-4 rounded-xl border border-dashed px-6 py-14 text-center',
        className,
      )}
    >
      <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
        <Icon className="h-6 w-6" aria-hidden="true" />
      </div>
      <div className="space-y-1.5">
        <h2 className="font-semibold text-base">{title}</h2>
        {description && (
          <div className="mx-auto max-w-md text-muted-foreground text-sm">{description}</div>
        )}
      </div>
      {children}
      {action && <div className="flex flex-wrap justify-center gap-2">{action}</div>}
    </div>
  );
}
