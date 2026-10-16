import { Plus } from 'lucide-react';
import { Link, NavLink } from 'react-router-dom';
import { SignInButton } from '@/components/auth/SignInButton';
import { Button } from '@/components/ui/button';
import { useSession } from '@/hooks/useSession';
import { cn } from '@/lib/utils';
import { Brand } from './Brand';
import { ThemeMenu } from './ThemeMenu';
import { UserMenu } from './UserMenu';

const NAV = [
  { to: '/', label: 'Parcels', end: true },
  { to: '/tokens', label: 'Tokens', end: false },
  { to: '/cli', label: 'CLI', end: false },
];

/** The top bar on every page except the viewer. */
export function AppHeader() {
  const { me, status } = useSession();

  return (
    <header className="sticky top-0 z-40 border-b bg-background/85 backdrop-blur-md">
      <div className="mx-auto flex h-14 max-w-6xl items-center gap-4 px-4 sm:px-6">
        <Link
          to="/"
          className="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Parcel home"
        >
          <Brand />
        </Link>

        {me && (
          <nav className="hidden items-center gap-1 md:flex" aria-label="Main">
            {NAV.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                end={item.end}
                className={({ isActive }) =>
                  cn(
                    'rounded-md px-2.5 py-1.5 text-sm transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                    isActive ? 'font-medium text-foreground' : 'text-muted-foreground',
                  )
                }
              >
                {item.label}
              </NavLink>
            ))}
          </nav>
        )}

        <div className="ml-auto flex items-center gap-2">
          {me ? (
            <>
              <Button asChild size="sm">
                <Link to="/new">
                  <Plus />
                  <span className="hidden sm:inline">New parcel</span>
                  <span className="sr-only sm:hidden">New parcel</span>
                </Link>
              </Button>
              <UserMenu me={me} />
            </>
          ) : (
            <>
              <ThemeMenu />
              {status === 'anonymous' && <SignInButton size="sm" />}
            </>
          )}
        </div>
      </div>
    </header>
  );
}
