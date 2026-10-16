import { Loader2, LogOut } from 'lucide-react';
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
import { useSignOut } from '@/hooks/useSignOut';

/** Confirm ending every web session of the user, this one included. */
export function SignOutEverywhereDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { signOut, pending } = useSignOut();
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Sign out everywhere?</AlertDialogTitle>
          <AlertDialogDescription>
            This ends your Parcel sessions in every browser, this one included. API tokens keep
            working; revoke them separately.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>Cancel</AlertDialogCancel>
          <Button variant="destructive" onClick={() => signOut(true)} disabled={pending}>
            {pending ? <Loader2 className="animate-spin" /> : <LogOut />}
            Sign out everywhere
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
