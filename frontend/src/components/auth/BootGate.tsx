import { ServerCrash } from 'lucide-react';
import { type ReactNode, useEffect } from 'react';
import { FullPageMessage, Splash } from '@/components/common/FullPage';
import { Button } from '@/components/ui/button';
import { useServerInfoStore } from '@/stores/server-info.store';

/**
 * Load `GET /api/v1/info` once before rendering anything: auth settings and
 * upload limits are runtime configuration, not build-time.
 */
export function BootGate({ children }: { children: ReactNode }) {
  const status = useServerInfoStore((s) => s.status);
  const error = useServerInfoStore((s) => s.error);
  const load = useServerInfoStore((s) => s.load);

  useEffect(() => {
    void load();
  }, [load]);

  if (status === 'ready') return <>{children}</>;
  if (status === 'error') {
    return (
      <FullPageMessage
        icon={ServerCrash}
        tone="destructive"
        title="Can't reach Parcel"
        description={error ?? 'The server is unavailable. Check that the backend is running.'}
        actions={
          <Button variant="outline" onClick={() => void load()}>
            Try again
          </Button>
        }
      />
    );
  }
  return <Splash />;
}
