import { ChevronDown, KeyRound, LayoutGrid, LogOut, Plus, Terminal } from 'lucide-react';
import { Link } from 'react-router-dom';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { logout } from '@/lib/auth';
import { initials } from '@/lib/format';
import type { Me } from '@/lib/types';
import { useIsDevAuth } from '@/stores/server-info.store';
import { ThemeRadioItems } from './ThemeMenu';

/** Avatar button with navigation, theme and sign-out. */
export function UserMenu({ me, compact }: { me: Me; compact?: boolean }) {
  const devAuth = useIsDevAuth();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" className="h-9 gap-2 px-1.5" aria-label="Account menu">
          <span className="flex h-7 w-7 items-center justify-center rounded-full bg-primary/12 font-semibold text-primary text-xs">
            {initials(me.name, me.email)}
          </span>
          {!compact && (
            <span
              className="hidden max-w-[14rem] truncate text-sm sm:inline"
              data-testid="user-email"
            >
              {me.email}
            </span>
          )}
          <ChevronDown className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <div className="flex flex-col gap-0.5 px-2 py-2">
          <span className="truncate font-medium text-sm">{me.name || me.email}</span>
          {me.name && <span className="truncate text-muted-foreground text-xs">{me.email}</span>}
          {devAuth && (
            <Badge variant="accent" className="mt-1 w-fit">
              Dev auth mode
            </Badge>
          )}
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/">
            <LayoutGrid />
            My parcels
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/new">
            <Plus />
            New parcel
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/tokens">
            <KeyRound />
            Tokens
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link to="/cli">
            <Terminal />
            CLI setup
          </Link>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Theme</DropdownMenuLabel>
        <ThemeRadioItems />
        {!devAuth && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => void logout()}>
              <LogOut />
              Sign out
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
