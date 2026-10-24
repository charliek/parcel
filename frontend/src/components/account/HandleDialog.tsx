import { Loader2 } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { toast } from 'sonner';
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
import { Label } from '@/components/ui/label';
import { useUpdateHandle } from '@/hooks/useAccount';
import { errorMessage, errorStatus } from '@/lib/http';
import { cleanUrl } from '@/lib/refs';
import { normalizeHandle } from '@/lib/slugs';
import type { Me } from '@/lib/types';
import { useAppOrigin } from '@/stores/server-info.store';

interface HandleDialogProps {
  me: Me;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Show and change the user's handle (the `@<handle>` in their clean URLs). */
export function HandleDialog(props: HandleDialogProps) {
  // Remount the form each time it opens so it starts from the current handle.
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent>{props.open && <HandleForm {...props} />}</DialogContent>
    </Dialog>
  );
}

function HandleForm({ me, onOpenChange }: HandleDialogProps) {
  const update = useUpdateHandle();
  const origin = useAppOrigin();
  const [text, setText] = useState(me.handle ?? '');
  const [serverError, setServerError] = useState<string | null>(null);

  const checked = normalizeHandle(text);
  const unchanged = checked.ok && checked.value === me.handle;
  const error = text.trim() && !checked.ok ? checked.message : serverError;

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (!checked.ok) return;
    if (unchanged) {
      onOpenChange(false);
      return;
    }
    update.mutate(checked.value, {
      onSuccess: (next) => {
        toast.success(`Your handle is now @${next.handle}`);
        onOpenChange(false);
      },
      onError: (err) => {
        const status = errorStatus(err);
        if (status === 409) setServerError('That handle is taken.');
        else if (status === 400) setServerError(errorMessage(err));
        else toast.error('Could not change your handle', { description: errorMessage(err) });
      },
    });
  };

  return (
    <form onSubmit={onSubmit} className="grid gap-5">
      <DialogHeader>
        <DialogTitle>Your handle</DialogTitle>
        <DialogDescription>
          It names you in the links of parcels you give a slug:{' '}
          {cleanUrl(origin, (checked.ok ? checked.value : me.handle) ?? '', '…')}
        </DialogDescription>
      </DialogHeader>
      <div className="space-y-2">
        <Label htmlFor="handle">Handle</Label>
        <div className="flex items-center gap-1.5">
          <span className="font-mono text-muted-foreground text-sm" aria-hidden="true">
            @
          </span>
          <Input
            id="handle"
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              setServerError(null);
            }}
            maxLength={40}
            autoFocus
            spellCheck={false}
            autoCapitalize="none"
            className="font-mono"
            aria-invalid={error ? true : undefined}
            aria-describedby="handle-help"
          />
        </div>
        <div id="handle-help" className="space-y-1 text-xs" aria-live="polite">
          {error ? (
            <p className="text-destructive" data-testid="handle-error">
              {error}
            </p>
          ) : (
            checked.ok && (
              <p className="break-all font-mono text-muted-foreground" data-testid="handle-preview">
                {cleanUrl(origin, checked.value, 'your-slug')}
              </p>
            )
          )}
          <p className="text-muted-foreground">
            Links with your old handle keep working, and nobody else can take it.
          </p>
        </div>
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button type="submit" disabled={!checked.ok || update.isPending}>
          {update.isPending && <Loader2 className="animate-spin" />}
          Save
        </Button>
      </DialogFooter>
    </form>
  );
}
