import { Brand } from '@/components/layout/Brand';
import { useIsDevAuth } from '@/stores/server-info.store';
import { SignInButton } from './SignInButton';

interface SignInScreenProps {
  title?: string;
  description?: string;
  /** Where to land after sign-in; defaults to the current page. */
  returnTo?: string;
}

/** Shown on account-only pages when there is no session. */
export function SignInScreen({
  title = 'Sign in to Parcel',
  description = 'Sign in to publish parcels, manage tokens and approve CLI logins.',
  returnTo,
}: SignInScreenProps) {
  const devAuth = useIsDevAuth();
  return (
    <div className="flex min-h-dvh items-center justify-center bg-background px-4 py-12">
      <div className="flex w-full max-w-sm flex-col items-center gap-6 text-center">
        <Brand className="scale-110" />
        <div className="space-y-2">
          <h1 className="font-semibold text-xl tracking-tight">{title}</h1>
          <p className="text-muted-foreground text-sm">{description}</p>
        </div>
        {devAuth ? (
          <p className="rounded-lg border bg-muted/50 px-4 py-3 text-muted-foreground text-sm">
            The server runs in dev auth mode, so there is nothing to sign in to — but it did not
            accept this request. Check the backend logs.
          </p>
        ) : (
          <SignInButton returnTo={returnTo} className="w-full" size="lg" />
        )}
      </div>
    </div>
  );
}
