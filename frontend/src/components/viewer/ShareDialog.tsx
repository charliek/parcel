import { toast } from 'sonner';
import { CopyButton } from '@/components/common/CopyButton';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { VisibilityPicker } from '@/components/upload/VisibilityPicker';
import { useUpdateParcel } from '@/hooks/useParcels';
import { errorMessage } from '@/lib/http';
import type { ParcelSummary, Visibility } from '@/lib/types';

/** Owner sharing: visibility plus the link. */
export function ShareDialog({
  parcel,
  link,
  open,
  onOpenChange,
}: {
  parcel: ParcelSummary;
  link: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const update = useUpdateParcel(parcel.id);

  const setVisibility = (visibility: Visibility) => {
    if (visibility === parcel.visibility) return;
    update.mutate(
      { visibility },
      {
        onSuccess: () =>
          toast.success(visibility === 'public' ? 'Parcel is now public' : 'Parcel is now private'),
        onError: (err) =>
          toast.error('Could not change visibility', { description: errorMessage(err) }),
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Share “{parcel.title}”</DialogTitle>
          <DialogDescription>
            Public parcels open for anyone with the link, no sign-in needed. Private parcels open
            only for you.
          </DialogDescription>
        </DialogHeader>
        <VisibilityPicker
          value={
            update.isPending
              ? (update.variables?.visibility ?? parcel.visibility)
              : parcel.visibility
          }
          onChange={setVisibility}
          disabled={update.isPending}
        />
        <div className="space-y-2">
          <Label htmlFor="share-link">Link</Label>
          <div className="flex gap-2">
            <Input
              id="share-link"
              value={link}
              readOnly
              className="font-mono text-xs"
              onFocus={(e) => e.currentTarget.select()}
            />
            <CopyButton value={link} label="Copy" size="default" toastMessage="Link copied" />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
