import { Loader2 } from 'lucide-react';
import { useState } from 'react';

/**
 * The sandboxed iframe that renders a parcel.
 *
 * The content lives on a separate origin (served with a CSP `sandbox`), and
 * the iframe repeats the sandbox. `allow-same-origin` is deliberately absent:
 * with it, a parcel could script its way out of the sandbox.
 */
export const IFRAME_SANDBOX =
  'allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads';

export function ContentFrame({ src, title }: { src: string; title: string }) {
  // The parent keys this component by src + reload count, so state resets per load.
  const [loaded, setLoaded] = useState(false);
  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-white">
      {!loaded && (
        <div
          className="absolute inset-0 flex items-center justify-center bg-background"
          role="status"
          aria-label="Loading parcel"
        >
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
        </div>
      )}
      <iframe
        src={src}
        title={title}
        sandbox={IFRAME_SANDBOX}
        allow="clipboard-write; fullscreen"
        referrerPolicy="no-referrer"
        className="absolute inset-0 h-full w-full border-0"
        onLoad={() => setLoaded(true)}
        data-testid="parcel-frame"
      />
    </div>
  );
}
