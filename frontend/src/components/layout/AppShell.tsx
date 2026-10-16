import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';
import { AppHeader } from './AppHeader';

/** Header plus a centered content column. */
export function AppShell({
  children,
  className,
  wide,
}: {
  children: ReactNode;
  className?: string;
  wide?: boolean;
}) {
  return (
    <div className="flex min-h-dvh flex-col bg-background">
      <AppHeader />
      <main
        id="main"
        className={cn(
          'mx-auto w-full flex-1 px-4 py-8 sm:px-6 sm:py-10',
          wide ? 'max-w-6xl' : 'max-w-3xl',
          className,
        )}
      >
        {children}
      </main>
    </div>
  );
}

/** A page title row with optional description and actions. */
export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-8 flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
      <div className="min-w-0 space-y-1">
        <h1 className="font-semibold text-2xl tracking-tight">{title}</h1>
        {description && <div className="text-muted-foreground text-sm">{description}</div>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
