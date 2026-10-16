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
import { NativeSelect } from '@/components/ui/select';
import { useCreateToken } from '@/hooks/useTokens';
import { errorMessage } from '@/lib/http';
import type { CreatedToken } from '@/lib/types';

const EXPIRY_PRESETS = [
  { value: 'never', label: 'Never' },
  { value: '7', label: '7 days' },
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '365', label: '1 year' },
  { value: 'custom', label: 'Custom…' },
];

/** Name + optional expiry → `POST /api/v1/tokens`. */
export function CreateTokenDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (created: CreatedToken) => void;
}) {
  const create = useCreateToken();
  const [name, setName] = useState('');
  const [preset, setPreset] = useState('never');
  const [customDays, setCustomDays] = useState('30');

  const days = preset === 'never' ? undefined : Number(preset === 'custom' ? customDays : preset);
  const daysValid = days === undefined || (Number.isInteger(days) && days >= 1 && days <= 3650);

  const reset = () => {
    setName('');
    setPreset('never');
    setCustomDays('30');
  };

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (!name.trim() || !daysValid) return;
    create.mutate(
      { name: name.trim(), ...(days !== undefined ? { expires_in_days: days } : {}) },
      {
        onSuccess: (created) => {
          reset();
          onOpenChange(false);
          onCreated(created);
        },
        onError: (err) => toast.error('Could not create token', { description: errorMessage(err) }),
      },
    );
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset();
        onOpenChange(next);
      }}
    >
      <DialogContent>
        <form onSubmit={onSubmit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>Create token</DialogTitle>
            <DialogDescription>
              The token acts as you: it can publish, change and delete your parcels.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="token-name">Name</Label>
            <Input
              id="token-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. CI on build server"
              maxLength={100}
              required
              autoFocus
            />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="token-expiry">Expires</Label>
              <NativeSelect
                id="token-expiry"
                value={preset}
                onChange={(e) => setPreset(e.target.value)}
              >
                {EXPIRY_PRESETS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </NativeSelect>
            </div>
            {preset === 'custom' && (
              <div className="space-y-2">
                <Label htmlFor="token-days">Days</Label>
                <Input
                  id="token-days"
                  type="number"
                  inputMode="numeric"
                  min={1}
                  max={3650}
                  value={customDays}
                  onChange={(e) => setCustomDays(e.target.value)}
                  aria-invalid={!daysValid}
                />
              </div>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending || !name.trim() || !daysValid}>
              {create.isPending && <Loader2 className="animate-spin" />}
              Create token
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
