/**
 * `/signin`: pick a sign-in provider. The server sends people here when it
 * has several and none was named, and when a sign-in fails
 * (`?error=<code>&provider=<id>&return_to=<path>`).
 */

import { AlertCircle } from 'lucide-react';
import { Link, Navigate, useSearchParams } from 'react-router-dom';
import { SignInScreen } from '@/components/auth/SignInScreen';
import { Splash } from '@/components/common/FullPage';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useSession } from '@/hooks/useSession';
import { safeReturnTo } from '@/lib/return-to';
import { isResolving, signInError } from '@/lib/sign-in';
import { useAuthProviders, useIsDevAuth } from '@/stores/server-info.store';

export function SignInPage() {
  useDocumentTitle('Sign in');
  const [params] = useSearchParams();
  const returnTo = safeReturnTo(params.get('return_to'));
  const { status, me } = useSession();
  const devAuth = useIsDevAuth();
  const providers = useAuthProviders();
  const failedWith = providers.find((p) => p.id === params.get('provider'))?.label;
  const error = signInError(params.get('error'), failedWith);

  // Nothing to pick: dev mode, or already signed in with nothing gone wrong.
  if (devAuth || (status === 'authenticated' && !error)) {
    return <Navigate to={returnTo} replace />;
  }
  if (isResolving(status) && !error) return <Splash />;

  return (
    <SignInScreen
      title="Sign in to Parcel"
      description={
        providers.length > 1
          ? 'Choose how to sign in. Use the one your email address belongs to.'
          : 'Sign in to open parcels shared with you, comment, and publish your own.'
      }
      returnTo={returnTo}
      notice={
        error && (
          <div
            role="alert"
            className="flex w-full items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-left text-sm"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" aria-hidden="true" />
            <div className="min-w-0 space-y-1">
              <p className="font-medium">{error.title}</p>
              <p className="text-muted-foreground">{error.description}</p>
              {me && (
                <p className="text-muted-foreground">
                  You’re still signed in as <span className="font-medium">{me.email}</span>.
                </p>
              )}
            </div>
          </div>
        )
      }
      footer={
        <Link
          to={returnTo}
          className="rounded text-muted-foreground text-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {returnTo === '/' ? 'Back to Parcel' : 'Go back'}
        </Link>
      }
    />
  );
}
