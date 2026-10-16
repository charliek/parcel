import { LogIn } from 'lucide-react';
import { type ComponentProps, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { beginLogin } from '@/lib/auth';
import { errorMessage } from '@/lib/http';
import { currentReturnTo } from '@/lib/return-to';
import { useIsDevAuth } from '@/stores/server-info.store';

/** Starts the slauth OAuth flow, returning to `returnTo` (default: here). */
export function SignInButton({
  returnTo,
  label = 'Sign in',
  ...props
}: { returnTo?: string; label?: string } & Omit<
  ComponentProps<typeof Button>,
  'onClick' | 'children'
>) {
  const [pending, setPending] = useState(false);
  const devAuth = useIsDevAuth();
  if (devAuth) return null;
  return (
    <Button
      {...props}
      disabled={pending || props.disabled}
      onClick={() => {
        setPending(true);
        beginLogin(returnTo ?? currentReturnTo()).catch((err) => {
          setPending(false);
          toast.error(errorMessage(err, 'Could not start sign-in'));
        });
      }}
    >
      <LogIn />
      {label}
    </Button>
  );
}
