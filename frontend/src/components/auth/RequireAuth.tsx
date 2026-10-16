import { ServerCrash } from 'lucide-react';
import type { ReactNode } from 'react';
import { FullPageMessage, Splash } from '@/components/common/FullPage';
import { Button } from '@/components/ui/button';
import { useSession } from '@/hooks/useSession';
import { useSilentReauth } from '@/hooks/useSilentReauth';
import { requiredPageAccess } from '@/lib/auth/policy';
import { silentReauthTried } from '@/lib/oauth';
import { useAuthStore } from '@/stores/auth.store';
import { useServerInfoStore } from '@/stores/server-info.store';
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
 * Gate for account-only pages. Resolves `/api/v1/me`; on 401 tries one silent
 * re-auth per browser session, then shows the sign-in screen. The return path
 * (including `?code=` on /device) survives the login round trip.
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
  const mode = useServerInfoStore((s) => s.info?.auth.mode);
  const access = requiredPageAccess({ session: status, mode, silentTried: silentReauthTried() });
  const silent = useSilentReauth(access === 'silent-reauth');

  if (silent === 'redirecting' || access === 'loading') return <Splash />;
  if (access === 'silent-reauth' && silent === 'idle') return <Splash />;
  if (access === 'ready') return <>{children}</>;
  if (access === 'error') return <SessionError message={error} />;
  return <SignInScreen title={signInTitle} description={signInDescription} />;
}
