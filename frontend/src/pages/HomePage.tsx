import { SessionError } from '@/components/auth/RequireAuth';
import { Splash } from '@/components/common/FullPage';
import { useSession } from '@/hooks/useSession';
import { isResolving } from '@/lib/sign-in';
import { DashboardPage } from './DashboardPage';
import { LandingPage } from './LandingPage';

/**
 * `/`: the dashboard when signed in, otherwise the landing page. The session
 * is a cookie, so a reload keeps it; signed-out visitors are never sent off
 * to sign in from here.
 */
export function HomePage() {
  const { status, error } = useSession();
  if (isResolving(status)) return <Splash />;
  if (status === 'authenticated') return <DashboardPage />;
  if (status === 'error') return <SessionError message={error} />;
  return <LandingPage />;
}
