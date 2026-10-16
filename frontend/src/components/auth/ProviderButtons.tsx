import { LogIn } from 'lucide-react';
import { useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { lastProvider, rememberProvider } from '@/lib/auth';
import { loginHref, orderProviders } from '@/lib/sign-in';
import type { AuthProvider } from '@/lib/types';

/**
 * One sign-in link per provider, the one used last first. They are plain
 * links to `/auth/login` (a full-page navigation: the server does the rest).
 */
export function ProviderButtons({
  providers,
  returnTo,
}: {
  providers: AuthProvider[];
  returnTo: string;
}) {
  const last = useMemo(() => lastProvider(), []);
  const ordered = useMemo(() => orderProviders(providers, last), [providers, last]);
  const several = providers.length > 1;

  return (
    <ul className="flex w-full flex-col gap-2" aria-label="Sign-in options">
      {ordered.map((provider, index) => {
        const isLast = several && provider.id === last;
        return (
          <li key={provider.id}>
            <Button
              asChild
              size="lg"
              variant={index === 0 ? 'default' : 'outline'}
              className="w-full"
            >
              <a
                href={loginHref(returnTo, provider.id)}
                onClick={() => rememberProvider(provider.id)}
              >
                <LogIn />
                <span className="truncate">Sign in with {provider.label}</span>
                {isLast && (
                  <>
                    <span
                      className="ml-auto rounded-full bg-background/20 px-2 py-0.5 font-medium text-[11px]"
                      aria-hidden="true"
                    >
                      Last used
                    </span>
                    <span className="sr-only"> (last used)</span>
                  </>
                )}
              </a>
            </Button>
          </li>
        );
      })}
    </ul>
  );
}
