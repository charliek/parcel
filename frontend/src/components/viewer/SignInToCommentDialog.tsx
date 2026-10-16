import { SignInButton } from '@/components/auth/SignInButton';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

/** What a signed-out visitor of a public parcel sees instead of comment mode. */
export function SignInToCommentDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Sign in to comment</DialogTitle>
          <DialogDescription>
            Comments on a public parcel are shared with everyone who is signed in. Sign in to read
            them and leave your own.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Not now
          </Button>
          <SignInButton />
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
