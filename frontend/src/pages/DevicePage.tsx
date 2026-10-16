import { useMutation, useQuery } from '@tanstack/react-query';
import { CheckCircle2, Clock, Loader2, Terminal, XCircle } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { RelativeTime } from '@/components/common/RelativeTime';
import { AppShell } from '@/components/layout/AppShell';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Skeleton } from '@/components/ui/skeleton';
import { queryKeys } from '@/hooks/queryKeys';
import { useDocumentTitle } from '@/hooks/useDocumentTitle';
import { useMe } from '@/hooks/useSession';
import { api } from '@/lib/api';
import { ApiError, errorMessage } from '@/lib/http';
import { formatUserCodeInput, normalizeUserCode } from '@/lib/user-code';

type Outcome = 'approved' | 'denied' | null;

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <AppShell className="flex max-w-md items-start justify-center sm:pt-16">
      <div className="w-full">{children}</div>
    </AppShell>
  );
}

function StatusCard({
  icon: Icon,
  tone,
  title,
  children,
}: {
  icon: typeof CheckCircle2;
  tone: 'success' | 'muted' | 'destructive';
  title: string;
  children?: React.ReactNode;
}) {
  const toneClass =
    tone === 'success'
      ? 'bg-success/12 text-success'
      : tone === 'destructive'
        ? 'bg-destructive/10 text-destructive'
        : 'bg-muted text-muted-foreground';
  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-4 p-8 text-center">
        <div className={`flex h-12 w-12 items-center justify-center rounded-full ${toneClass}`}>
          <Icon className="h-6 w-6" aria-hidden="true" />
        </div>
        <div className="space-y-1.5">
          <h1 className="font-semibold text-xl tracking-tight">{title}</h1>
          {children && <div className="text-muted-foreground text-sm">{children}</div>}
        </div>
      </CardContent>
    </Card>
  );
}

function CodeEntry({ initial, onSubmit }: { initial: string; onSubmit: (code: string) => void }) {
  const [value, setValue] = useState(formatUserCodeInput(initial));
  const normalized = normalizeUserCode(value);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (normalized) onSubmit(normalized);
  };
  return (
    <Card>
      <CardContent className="p-6 sm:p-8">
        <form onSubmit={submit} className="flex flex-col items-center gap-5 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
            <Terminal className="h-6 w-6" aria-hidden="true" />
          </div>
          <div className="space-y-1.5">
            <h1 className="font-semibold text-xl tracking-tight">Connect the Parcel CLI</h1>
            <p className="text-muted-foreground text-sm">
              Enter the code shown in your terminal after running{' '}
              <code className="font-mono text-xs">parcel login</code>.
            </p>
          </div>
          <div className="w-full space-y-2 text-left">
            <Label htmlFor="user-code" className="sr-only">
              Code
            </Label>
            <Input
              id="user-code"
              value={value}
              onChange={(e) => setValue(formatUserCodeInput(e.target.value))}
              placeholder="XXXX-XXXX"
              autoComplete="one-time-code"
              autoCapitalize="characters"
              spellCheck={false}
              autoFocus
              className="h-12 text-center font-mono text-xl uppercase tracking-[0.3em]"
              aria-describedby="user-code-hint"
            />
            <p id="user-code-hint" className="text-center text-muted-foreground text-xs">
              Eight letters or digits. Case and the dash don't matter.
            </p>
          </div>
          <Button type="submit" className="w-full" disabled={!normalized}>
            Continue
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

/** `/device`: approve or deny a pending CLI device login. */
export function DevicePage() {
  useDocumentTitle('Approve CLI login');
  const me = useMe();
  const [params, setParams] = useSearchParams();
  const rawCode = params.get('code') ?? '';
  const code = normalizeUserCode(rawCode);
  const [outcome, setOutcome] = useState<Outcome>(null);

  const info = useQuery({
    queryKey: queryKeys.device(code ?? ''),
    queryFn: () => api.getDevice(code ?? ''),
    enabled: code !== null && outcome === null,
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const decide = useMutation({
    mutationFn: (approve: boolean) =>
      approve ? api.approveDevice(code ?? '') : api.denyDevice(code ?? ''),
    onSuccess: (_, approve) => setOutcome(approve ? 'approved' : 'denied'),
    onError: (err) => toast.error('That didn’t work', { description: errorMessage(err) }),
  });

  const chooseCode = (next: string | null) => {
    setOutcome(null);
    const search = new URLSearchParams(params);
    if (next) search.set('code', next);
    else search.delete('code');
    setParams(search, { replace: true });
  };

  if (outcome === 'approved') {
    return (
      <Centered>
        <StatusCard icon={CheckCircle2} tone="success" title="CLI connected">
          <p>
            You can return to your terminal — <code className="font-mono text-xs">parcel</code> is
            now signed in{me ? ` as ${me.email}` : ''}.
          </p>
          <p className="mt-2">You can close this tab.</p>
        </StatusCard>
      </Centered>
    );
  }

  if (outcome === 'denied') {
    return (
      <Centered>
        <StatusCard icon={XCircle} tone="muted" title="Login request denied">
          The CLI was not signed in. You can close this tab.
        </StatusCard>
      </Centered>
    );
  }

  if (!code) {
    return (
      <Centered>
        <CodeEntry initial={rawCode} onSubmit={chooseCode} />
      </Centered>
    );
  }

  if (info.isLoading) {
    return (
      <Centered>
        <Card>
          <CardContent className="space-y-4 p-8">
            <Skeleton className="mx-auto h-12 w-12 rounded-full" />
            <Skeleton className="mx-auto h-6 w-48" />
            <Skeleton className="mx-auto h-4 w-64" />
            <Skeleton className="h-10" />
          </CardContent>
        </Card>
      </Centered>
    );
  }

  if (info.isError) {
    const gone = info.error instanceof ApiError && info.error.status === 404;
    return (
      <Centered>
        <StatusCard
          icon={Clock}
          tone={gone ? 'muted' : 'destructive'}
          title={gone ? 'Code expired or not found' : 'Something went wrong'}
        >
          <p>
            {gone ? (
              <>
                <span className="font-mono">{code}</span> isn't a pending login. Codes expire after
                a few minutes — run <code className="font-mono text-xs">parcel login</code> again
                for a new one.
              </>
            ) : (
              errorMessage(info.error)
            )}
          </p>
          <Button variant="outline" className="mt-4" onClick={() => chooseCode(null)}>
            Enter a different code
          </Button>
        </StatusCard>
      </Centered>
    );
  }

  const device = info.data;
  const expired = device ? new Date(device.expires_at).getTime() <= Date.now() : false;

  return (
    <Centered>
      <Card>
        <CardContent className="flex flex-col items-center gap-5 p-6 text-center sm:p-8">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
            <Terminal className="h-6 w-6" aria-hidden="true" />
          </div>
          <div className="space-y-1.5">
            <h1 className="font-semibold text-xl tracking-tight">Approve CLI login?</h1>
            <p className="text-muted-foreground text-sm">
              <span className="font-medium text-foreground">{device?.client_name}</span> wants to
              sign in to Parcel
              {me ? (
                <>
                  {' '}
                  as <span className="font-medium text-foreground">{me.email}</span>
                </>
              ) : null}
              . It will be able to publish and manage your parcels.
            </p>
          </div>
          <div className="w-full rounded-lg border bg-muted/40 px-4 py-3">
            <p
              className="font-mono font-semibold text-2xl tracking-[0.2em]"
              data-testid="device-code"
            >
              {device?.user_code ?? code}
            </p>
            <p className="mt-1 text-muted-foreground text-xs">
              Check that this matches the code in your terminal.
            </p>
          </div>
          {device && (
            <p className="text-muted-foreground text-xs">
              {expired ? (
                'This code has expired.'
              ) : (
                <>
                  Expires <RelativeTime value={device.expires_at} />
                </>
              )}
            </p>
          )}
          <div className="grid w-full grid-cols-2 gap-3">
            <Button
              variant="outline"
              onClick={() => decide.mutate(false)}
              disabled={decide.isPending || expired}
            >
              Deny
            </Button>
            <Button onClick={() => decide.mutate(true)} disabled={decide.isPending || expired}>
              {decide.isPending && decide.variables && <Loader2 className="animate-spin" />}
              Approve
            </Button>
          </div>
        </CardContent>
      </Card>
    </Centered>
  );
}
