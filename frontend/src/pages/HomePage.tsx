import { SessionError } from '@/components/auth/RequireAuth';
import { Splash } from '@/components/common/FullPage';
import { useSession } from '@/hooks/useSession';
import { useSilentReauth } from '@/hooks/useSilentReauth';
import { hasSignedInHint } from '@/lib/auth';
import { isResolving, shouldAttemptSilentReauth } from '@/lib/auth/policy';
import { silentReauthTried } from '@/lib/oauth';
import { useServerInfoStore } from '@/stores/server-info.store';
import { DashboardPage } from './DashboardPage';
import { LandingPage } from './LandingPage';

/**
 * `/`: the dashboard when signed in, otherwise the landing page. Returning
 * users (this browser signed in before) get one silent re-auth first, so a
 * reload doesn't drop them onto the landing page.
 */
export function HomePage() {
  const { status, error } = useSession();
  const mode = useServerInfoStore((s) => s.info?.auth.mode);
  const want =
    status === 'anonymous' &&
    shouldAttemptSilentReauth({
      mode,
      alreadyTried: silentReauthTried(),
      onCallbackRoute: false,
      need: 'optional',
      signedInHint: hasSignedInHint(),
    });
  const silent = useSilentReauth(want);

  if (isResolving(status) || silent === 'redirecting' || (want && silent === 'idle')) {
    return <Splash />;
  }
  if (status === 'authenticated') return <DashboardPage />;
  if (status === 'error') return <SessionError message={error} />;
  return <LandingPage />;
}
