import { Compass } from 'lucide-react';
import { Link } from 'react-router-dom';
import { AppShell } from '@/components/layout/AppShell';
import { Button } from '@/components/ui/button';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';

export function NotFoundPage() {
  useDocumentTitle('Not found');
  return (
    <AppShell className="flex items-center justify-center">
      <div className="flex max-w-md flex-col items-center gap-4 py-16 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Compass className="h-6 w-6" aria-hidden="true" />
        </div>
        <div className="space-y-1.5">
          <p className="font-mono text-muted-foreground text-xs">404</p>
          <h1 className="font-semibold text-xl tracking-tight">This page doesn't exist</h1>
          <p className="text-muted-foreground text-sm">
            The link may be mistyped, or the page may have moved.
          </p>
        </div>
        <Button asChild variant="outline">
          <Link to="/">Back to Parcel</Link>
        </Button>
      </div>
    </AppShell>
  );
}
