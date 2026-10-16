import { LogIn } from 'lucide-react';
import type { ComponentProps } from 'react';
import { useNavigate } from 'react-router-dom';
import { Button } from '@/components/ui/button';
import { useSignInTarget } from '@/hooks/useSession';
import { startSignIn } from '@/lib/auth';
import { currentReturnTo } from '@/lib/return-to';
import { signInPageHref } from '@/lib/sign-in';

/**
 * "Sign in", returning to `returnTo` (default: here). With one provider it
 * goes straight there; with several, to the `/signin` page to pick one.
 * Renders nothing in dev auth mode.
 */
export function SignInButton({
  returnTo,
  label = 'Sign in',
  ...props
}: { returnTo?: string; label?: string } & Omit<
  ComponentProps<typeof Button>,
  'onClick' | 'children'
>) {
  const target = useSignInTarget();
  const navigate = useNavigate();
  if (target.kind === 'none') return null;
  return (
    <Button
      {...props}
      onClick={() => {
        const back = returnTo ?? currentReturnTo();
        if (target.kind === 'provider') startSignIn(target.provider.id, back);
        else navigate(signInPageHref(back));
      }}
    >
      <LogIn />
      {label}
    </Button>
  );
}
