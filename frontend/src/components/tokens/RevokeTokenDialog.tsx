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
import { useRevokeToken } from '@/hooks/useTokens';
import { errorMessage } from '@/lib/http';
import type { ApiToken } from '@/lib/types';

export function RevokeTokenDialog({
  token,
  onOpenChange,
}: {
  token: ApiToken | null;
  onOpenChange: (open: boolean) => void;
}) {
  const revoke = useRevokeToken();
  const onConfirm = () => {
    if (!token) return;
    revoke.mutate(token.id, {
      onSuccess: () => {
        toast.success('Token revoked', { description: token.name });
        onOpenChange(false);
      },
      onError: (err) => toast.error('Could not revoke token', { description: errorMessage(err) }),
    });
  };

  return (
    <AlertDialog open={token !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Revoke “{token?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            Anything using this token — the CLI on another machine, a script or a CI job — stops
            working immediately. This can't be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={revoke.isPending}>Cancel</AlertDialogCancel>
          <Button variant="destructive" onClick={onConfirm} disabled={revoke.isPending}>
            {revoke.isPending && <Loader2 className="animate-spin" />}
            Revoke token
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
