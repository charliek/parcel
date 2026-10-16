import { ServerCrash } from 'lucide-react';
import { type ReactNode, useEffect, useState } from 'react';
import { FullPageMessage, Splash } from '@/components/common/FullPage';
import { Button } from '@/components/ui/button';
import { useSession, useSignInTarget } from '@/hooks/useSession';
import { hasSignedInHint, lastAutoSignIn, recordAutoSignIn, startSignIn } from '@/lib/auth';
import { isResolving, shouldAutoSignIn } from '@/lib/sign-in';
import { useAuthStore } from '@/stores/auth.store';
import { SignInScreen } from './SignInScreen';

/** Session error screen with a retry. */
export function SessionError({ message }: { message: string | null }) {
  const reload = useAuthStore((s) => s.reload);
  return (
    <FullPageMessage
      icon={ServerCrash}
      tone="destructive"
      title="Couldn't load your account"
      description={message ?? 'The server returned an unexpected error.'}
      actions={
        <Button variant="outline" onClick={() => void reload()}>
          Try again
        </Button>
      }
    />
  );
}

/**
 * Gate for account-only pages. Resolves `/api/v1/me`; when signed out, a
 * returning browser with one provider goes straight to it (at most once per
 * cooldown, so a session that won't stick can't loop), and everyone else
 * gets the sign-in screen. Either way the return path, including `?code=`
 * on /device, survives the round trip.
 */
export function RequireAuth({
  children,
  signInTitle,
  signInDescription,
}: {
  children: ReactNode;
  signInTitle?: string;
  signInDescription?: string;
}) {
  const { status, error } = useSession();
  const target = useSignInTarget();
  const anonymous = status === 'anonymous';
  // Decided once the session is known to be signed out.
  const [auto, setAuto] = useState<'undecided' | 'going' | 'no'>('undecided');

  useEffect(() => {
    if (!anonymous || auto !== 'undecided') return;
    const go =
      target.kind === 'provider' &&
      shouldAutoSignIn({
        target,
        signedInHint: hasSignedInHint(),
        lastAttemptAt: lastAutoSignIn(),
        now: Date.now(),
      }) &&
      recordAutoSignIn();
    if (go && target.kind === 'provider') {
      setAuto('going');
      startSignIn(target.provider.id);
    } else {
      setAuto('no');
    }
  }, [anonymous, auto, target]);

  // Back from the provider's page into this one (bfcache): offer the screen.
  useEffect(() => {
    const onShow = (event: PageTransitionEvent) => {
      if (event.persisted) setAuto((a) => (a === 'going' ? 'no' : a));
    };
    window.addEventListener('pageshow', onShow);
    return () => window.removeEventListener('pageshow', onShow);
  }, []);

  if (isResolving(status)) return <Splash />;
  if (status === 'authenticated') return <>{children}</>;
  if (status === 'error') return <SessionError message={error} />;
  if (auto === 'going') return <Splash message="Signing you in…" />;
  if (auto === 'undecided') return <Splash />;
  return <SignInScreen title={signInTitle} description={signInDescription} />;
}
