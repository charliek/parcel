import { AlertCircle, KeyRound, LogOut, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { SignOutEverywhereDialog } from '@/components/auth/SignOutEverywhereDialog';
import { EmptyState } from '@/components/common/EmptyState';
import { RelativeTime } from '@/components/common/RelativeTime';
import { AppShell, PageHeader } from '@/components/layout/AppShell';
import { CreateTokenDialog } from '@/components/tokens/CreateTokenDialog';
import { RevokeTokenDialog } from '@/components/tokens/RevokeTokenDialog';
import { TokenCreatedDialog } from '@/components/tokens/TokenCreatedDialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useMe } from '@/hooks/useSession';
import { useTokens } from '@/hooks/useTokens';
import { formatDateTime, formatExpiry } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import type { ApiToken, CreatedToken } from '@/lib/types';
import { cn } from '@/lib/utils';
import { useIsDevAuth } from '@/stores/server-info.store';

function createdViaLabel(via: string): string {
  if (via === 'device') return 'CLI login';
  if (via === 'web') return 'Web';
  return via;
}

/** Who this browser is signed in as, and a way to end every web session. */
function WebSignIn() {
  const me = useMe();
  const devAuth = useIsDevAuth();
  const [confirm, setConfirm] = useState(false);
  // Dev mode has no sessions to end.
  if (devAuth || !me) return null;
  return (
    <section
      className="mt-10 flex flex-col gap-4 rounded-xl border bg-card p-5 sm:flex-row sm:items-center"
      aria-labelledby="web-sign-in"
    >
      <div className="min-w-0 flex-1 space-y-1">
        <h2 id="web-sign-in" className="font-medium text-base">
          Web sign-in
        </h2>
        <p className="text-muted-foreground text-sm">
          Signed in as <span className="font-medium text-foreground">{me.email}</span>
          {me.signed_in_with && (
            <>
              {' '}
              with <span className="font-medium text-foreground">{me.signed_in_with}</span>
            </>
          )}
          . Lost a laptop, or signed in on a shared computer? End every web session at once.
        </p>
      </div>
      <Button variant="outline" className="shrink-0" onClick={() => setConfirm(true)}>
        <LogOut />
        Sign out everywhere
      </Button>
      <SignOutEverywhereDialog open={confirm} onOpenChange={setConfirm} />
    </section>
  );
}

function Expiry({ value }: { value: string | null }) {
  const label = formatExpiry(value);
  return (
    <span
      className={cn(label === 'Expired' && 'text-destructive')}
      title={value ? formatDateTime(value) : undefined}
    >
      {label}
    </span>
  );
}

/** `/tokens`: list, create and revoke API tokens. */
export function TokensPage() {
  useDocumentTitle('Tokens');
  const tokens = useTokens();
  const [createOpen, setCreateOpen] = useState(false);
  const [created, setCreated] = useState<CreatedToken | null>(null);
  const [revoking, setRevoking] = useState<ApiToken | null>(null);
  const list = tokens.data?.tokens ?? [];

  const createButton = (
    <Button onClick={() => setCreateOpen(true)}>
      <Plus />
      Create token
    </Button>
  );

  return (
    <AppShell wide>
      <PageHeader
        title="Tokens"
        description={
          <>
            Tokens let the <code className="font-mono text-xs">parcel</code> CLI, scripts and CI
            publish as you.{' '}
            <Link className="text-primary hover:underline" to="/cli">
              CLI logins
            </Link>{' '}
            create one automatically.
          </>
        }
        actions={createButton}
      />

      {tokens.isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : tokens.isError ? (
        <EmptyState
          icon={AlertCircle}
          title="Couldn't load tokens"
          description={errorMessage(tokens.error)}
          action={
            <Button variant="outline" onClick={() => void tokens.refetch()}>
              Try again
            </Button>
          }
        />
      ) : list.length === 0 ? (
        <EmptyState
          icon={KeyRound}
          title="No tokens yet"
          description="Create a token for a script or CI job, or run parcel login to connect the CLI from your terminal."
          action={createButton}
        />
      ) : (
        <div className="overflow-hidden rounded-xl border bg-card">
          <table className="w-full text-sm">
            <thead className="border-b bg-muted/40 text-left text-muted-foreground text-xs">
              <tr>
                <th scope="col" className="px-4 py-2.5 font-medium">
                  Name
                </th>
                <th scope="col" className="hidden px-4 py-2.5 font-medium md:table-cell">
                  Token
                </th>
                <th scope="col" className="hidden px-4 py-2.5 font-medium sm:table-cell">
                  Created
                </th>
                <th scope="col" className="hidden px-4 py-2.5 font-medium lg:table-cell">
                  Last used
                </th>
                <th scope="col" className="hidden px-4 py-2.5 font-medium sm:table-cell">
                  Expires
                </th>
                <th scope="col" className="w-12 px-2 py-2.5">
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {list.map((token) => (
                <tr key={token.id} className="align-middle">
                  <td className="px-4 py-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{token.name}</span>
                      <Badge variant="muted">{createdViaLabel(token.created_via)}</Badge>
                    </div>
                    <p className="mt-0.5 font-mono text-muted-foreground text-xs md:hidden">
                      {token.prefix}…
                    </p>
                  </td>
                  <td className="hidden px-4 py-3 font-mono text-muted-foreground text-xs md:table-cell">
                    {token.prefix}…
                  </td>
                  <td className="hidden px-4 py-3 text-muted-foreground sm:table-cell">
                    <RelativeTime value={token.created_at} />
                  </td>
                  <td className="hidden px-4 py-3 text-muted-foreground lg:table-cell">
                    {token.last_used_at ? <RelativeTime value={token.last_used_at} /> : 'Never'}
                  </td>
                  <td className="hidden px-4 py-3 text-muted-foreground sm:table-cell">
                    <Expiry value={token.expires_at} />
                  </td>
                  <td className="px-2 py-3 text-right">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="text-muted-foreground hover:text-destructive"
                      onClick={() => setRevoking(token)}
                      aria-label={`Revoke ${token.name}`}
                      title="Revoke"
                    >
                      <Trash2 />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <WebSignIn />

      <CreateTokenDialog open={createOpen} onOpenChange={setCreateOpen} onCreated={setCreated} />
      <TokenCreatedDialog created={created} onDone={() => setCreated(null)} />
      <RevokeTokenDialog token={revoking} onOpenChange={(open) => !open && setRevoking(null)} />
    </AppShell>
  );
}
