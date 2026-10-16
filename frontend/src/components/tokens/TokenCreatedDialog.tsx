import { TriangleAlert } from 'lucide-react';
import { CodeBlock } from '@/components/common/CodeBlock';
import { CopyButton } from '@/components/common/CopyButton';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import type { CreatedToken } from '@/lib/types';

/** Shows a new token's secret exactly once. Only the explicit button closes it. */
export function TokenCreatedDialog({
  created,
  onDone,
}: {
  created: CreatedToken | null;
  onDone: () => void;
}) {
  return (
    <Dialog open={created !== null} onOpenChange={(open) => !open && onDone()}>
      <DialogContent
        hideClose
        onInteractOutside={(e) => e.preventDefault()}
        onEscapeKeyDown={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>Token created</DialogTitle>
          <DialogDescription>
            “{created?.token.name}” is ready. Use it with the CLI or in CI.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-start gap-2 rounded-lg border border-primary/30 bg-primary/5 px-3 py-2.5 text-sm">
          <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
          <span>
            Copy it now. For your security it is never shown again — if you lose it, revoke it and
            create another.
          </span>
        </div>
        <div className="flex gap-2">
          <Input
            value={created?.secret ?? ''}
            readOnly
            aria-label="Token secret"
            className="font-mono text-xs"
            onFocus={(e) => e.currentTarget.select()}
            data-testid="token-secret"
          />
          <CopyButton
            value={created?.secret ?? ''}
            label="Copy"
            size="default"
            toastMessage="Token copied"
          />
        </div>
        <div className="space-y-1.5">
          <p className="text-muted-foreground text-xs">Use it from a script or CI job:</p>
          <CodeBlock
            code={`PARCEL_TOKEN=${created?.secret ?? ''} parcel publish index.html`}
            label="token usage"
          />
        </div>
        <DialogFooter>
          <Button onClick={onDone}>I've saved my token</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
