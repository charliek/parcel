import type { ReactNode } from 'react';
import { Brand } from '@/components/layout/Brand';
import { currentReturnTo } from '@/lib/return-to';
import { useAuthProviders, useIsDevAuth } from '@/stores/server-info.store';
import { ProviderButtons } from './ProviderButtons';

interface SignInScreenProps {
  title?: string;
  description?: ReactNode;
  /** Where to land after sign-in; defaults to the current page. */
  returnTo?: string;
  /** Shown above the sign-in options (the `/signin` page's error). */
  notice?: ReactNode;
  /** Shown below them. */
  footer?: ReactNode;
}

/** The sign-in options, centered: account-only pages when signed out, and `/signin`. */
export function SignInScreen({
  title = 'Sign in to Parcel',
  description = 'Sign in to publish parcels, manage tokens and approve CLI logins.',
  returnTo,
  notice,
  footer,
}: SignInScreenProps) {
  const devAuth = useIsDevAuth();
  const providers = useAuthProviders();
  return (
    <div className="flex min-h-dvh items-center justify-center bg-background px-4 py-12">
      <div className="flex w-full max-w-sm flex-col items-center gap-6 text-center">
        <Brand className="scale-110" />
        <div className="space-y-2">
          <h1 className="font-semibold text-xl tracking-tight">{title}</h1>
          {description && <p className="text-muted-foreground text-sm">{description}</p>}
        </div>
        {notice}
        {devAuth ? (
          <p className="rounded-lg border bg-muted/50 px-4 py-3 text-muted-foreground text-sm">
            The server runs in dev auth mode, so there is nothing to sign in to — but it did not
            accept this request. Check the backend logs.
          </p>
        ) : providers.length === 0 ? (
          <p className="rounded-lg border bg-muted/50 px-4 py-3 text-muted-foreground text-sm">
            No sign-in providers are configured on this server.
          </p>
        ) : (
          <ProviderButtons providers={providers} returnTo={returnTo ?? currentReturnTo()} />
        )}
        {footer}
      </div>
    </div>
  );
}
