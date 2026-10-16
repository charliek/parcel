import { useQueryClient } from '@tanstack/react-query';
import { Loader2 } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { Button } from '@/components/ui/button';
import { forgetParcel, useDeleteParcel } from '@/hooks/useParcels';
import { errorMessage } from '@/lib/http';
import type { ParcelSummary } from '@/lib/types';

/** Confirm, then permanently delete a parcel and all its versions. */
export function DeleteParcelDialog({
  parcel,
  open,
  onOpenChange,
}: {
  parcel: ParcelSummary;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const client = useQueryClient();
  const remove = useDeleteParcel(parcel.id);

  const onConfirm = () => {
    remove.mutate(undefined, {
      onSuccess: () => {
        toast.success('Parcel deleted', { description: parcel.title });
        onOpenChange(false);
        navigate('/', { replace: true });
        // After leaving the viewer, so its queries don't refetch into a 404.
        setTimeout(() => forgetParcel(client, parcel.id), 0);
      },
      onError: (err) => toast.error('Could not delete', { description: errorMessage(err) }),
    });
  };

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{parcel.title}”?</AlertDialogTitle>
          <AlertDialogDescription>
            This permanently deletes the parcel and all of its versions. Anyone with the link will
            get a not-found page. This can't be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel>
          {/* A plain button (not AlertDialogAction) so the dialog stays open while deleting. */}
          <Button variant="destructive" onClick={onConfirm} disabled={remove.isPending}>
            {remove.isPending && <Loader2 className="animate-spin" />}
            Delete parcel
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
