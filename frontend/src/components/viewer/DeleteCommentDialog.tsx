import { Loader2 } from 'lucide-react';
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
import { useDeleteComment } from '@/hooks/useComments';
import { pluralize } from '@/lib/format';
import { errorMessage } from '@/lib/http';

/** What is about to be deleted. */
export interface DeleteTarget {
  number: number;
  /** A thread's first comment: deleting it deletes its replies too. */
  isThread: boolean;
  replies: number;
  /** The caller isn't the author (the owner removing someone else's comment). */
  moderating: boolean;
}

/** Confirm, then delete a comment (and, for a thread, its replies). */
export function DeleteCommentDialog({
  parcelId,
  target,
  onOpenChange,
}: {
  parcelId: string;
  target: DeleteTarget | null;
  onOpenChange: (open: boolean) => void;
}) {
  const remove = useDeleteComment(parcelId);

  const onConfirm = () => {
    if (!target) return;
    remove.mutate(target.number, {
      onSuccess: () => {
        toast.success(`Deleted comment #${target.number}`);
        onOpenChange(false);
      },
      onError: (err) => toast.error('Could not delete', { description: errorMessage(err) }),
    });
  };

  const what =
    target?.isThread && target.replies > 0
      ? `This deletes the comment and its ${pluralize(target.replies, 'reply', 'replies')}.`
      : 'This deletes the comment.';

  return (
    <AlertDialog open={target !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete comment #{target?.number}?</AlertDialogTitle>
          <AlertDialogDescription>
            {what}
            {target?.moderating && ' You are removing someone else’s comment as the owner.'} This
            can't be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel>
          {/* A plain button (not AlertDialogAction) so the dialog stays open while deleting. */}
          <Button variant="destructive" onClick={onConfirm} disabled={remove.isPending}>
            {remove.isPending && <Loader2 className="animate-spin" />}
            Delete comment
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
