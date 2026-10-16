import { Loader2 } from 'lucide-react';
import { type FormEvent, useEffect, useState } from 'react';
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
import { Textarea } from '@/components/ui/textarea';
import { useUpdateParcel } from '@/hooks/useParcels';
import { errorMessage } from '@/lib/http';
import type { ParcelSummary } from '@/lib/types';

/** Edit a parcel's title and description. */
export function EditDetailsDialog({
  parcel,
  open,
  onOpenChange,
}: {
  parcel: ParcelSummary;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const update = useUpdateParcel(parcel.id);
  const [title, setTitle] = useState(parcel.title);
  const [description, setDescription] = useState(parcel.description);

  // Start from the latest values each time the dialog opens.
  useEffect(() => {
    if (open) {
      setTitle(parcel.title);
      setDescription(parcel.description);
    }
  }, [open, parcel.title, parcel.description]);

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = title.trim();
    if (!trimmed) return;
    const body: { title?: string; description?: string } = {};
    if (trimmed !== parcel.title) body.title = trimmed;
    if (description.trim() !== parcel.description) body.description = description.trim();
    if (Object.keys(body).length === 0) {
      onOpenChange(false);
      return;
    }
    update.mutate(body, {
      onSuccess: () => {
        toast.success('Details saved');
        onOpenChange(false);
      },
      onError: (err) => toast.error('Could not save', { description: errorMessage(err) }),
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={onSubmit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>Edit details</DialogTitle>
            <DialogDescription>Changes apply to every version of this parcel.</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="edit-title">Title</Label>
            <Input
              id="edit-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              maxLength={200}
              required
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-description">Description</Label>
            <Textarea
              id="edit-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={4}
              maxLength={2000}
              placeholder="What is this parcel for?"
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={update.isPending || !title.trim()}>
              {update.isPending && <Loader2 className="animate-spin" />}
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
