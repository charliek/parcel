import { AlertCircle, Loader2, UserPlus, X } from 'lucide-react';
import { type FormEvent, useId, useRef, useState } from 'react';
import { toast } from 'sonner';
import { CopyButton } from '@/components/common/CopyButton';
import { IconButton } from '@/components/common/IconButton';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { NativeSelect } from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { VisibilityPicker } from '@/components/upload/VisibilityPicker';
import { useMe } from '@/hooks/useSession';
import { useDeleteShare, usePutShare, useShares } from '@/hooks/useShares';
import { useChangeVisibility } from '@/hooks/useVisibility';
import { ROLE_LABEL } from '@/lib/access';
import { asSentence, initials } from '@/lib/format';
import { errorMessage } from '@/lib/http';
import { normalizeEmail, shareDisplay } from '@/lib/shares';
import type { ParcelSummary, Share, ShareRole, Visibility } from '@/lib/types';
import { useAllowPublic } from '@/stores/server-info.store';

const ROLES: ShareRole[] = ['viewer', 'commenter'];

const SECTION_HEADING = 'font-medium text-sm';

/** Owner sharing: people by email, who else can open it, and the link. */
export function ShareDialog({
  parcel,
  link,
  open,
  onOpenChange,
}: {
  parcel: ParcelSummary;
  link: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const allowPublic = useAllowPublic();
  const visibility = useChangeVisibility(parcel, link);
  const generalId = useId();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {/* One shrinkable column: long emails truncate instead of widening the dialog. */}
      <DialogContent className="grid-cols-[minmax(0,1fr)] gap-6">
        <DialogHeader>
          <DialogTitle className="pr-6 leading-snug">Share “{parcel.title}”</DialogTitle>
          <DialogDescription>
            Add people by email, or open it to everyone signed in. People sign in with the email you
            add.
          </DialogDescription>
        </DialogHeader>

        {/* Mounted only while the dialog is (Radix unmounts closed content). */}
        <PeopleWithAccess parcelId={parcel.id} visibility={visibility.shown} />

        <section className="space-y-2" aria-labelledby={generalId}>
          <h3 id={generalId} className={SECTION_HEADING}>
            General access
          </h3>
          <VisibilityPicker
            legend="General access"
            layout="stack"
            value={visibility.shown}
            onChange={visibility.change}
            allowPublic={allowPublic}
            disabled={visibility.pending}
          />
        </section>

        <div className="space-y-2">
          <Label htmlFor="share-link">Link</Label>
          <div className="flex gap-2">
            <Input
              id="share-link"
              value={link}
              readOnly
              className="font-mono text-xs"
              onFocus={(e) => e.currentTarget.select()}
            />
            <CopyButton
              value={link}
              label="Copy link"
              size="default"
              toastMessage="Link copied"
              className="shrink-0"
            />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Avatar({ name, email }: { name: string | null | undefined; email: string }) {
  return (
    <span
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/12 font-semibold text-primary text-xs"
      aria-hidden="true"
    >
      {initials(name, email)}
    </span>
  );
}

/** The add-by-email form and everyone the parcel is shared with. */
function PeopleWithAccess({ parcelId, visibility }: { parcelId: string; visibility: Visibility }) {
  const me = useMe();
  const shares = useShares(parcelId, true);
  const add = usePutShare(parcelId);
  // Removal lives here, not in the row: the row is gone by the time it settles.
  const remove = useDeleteShare(parcelId);
  const [removeError, setRemoveError] = useState<{ email: string; message: string } | null>(null);
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<ShareRole>('viewer');
  const [addError, setAddError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const ids = useId();
  const headingId = `${ids}-people`;
  const errorId = `${ids}-error`;

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const address = normalizeEmail(email);
    if (!address || add.isPending) return;
    setAddError(null);
    add.mutate(
      { email: address, role },
      {
        onSuccess: (share) => {
          setEmail('');
          toast.success(`Shared with ${share.email}`, {
            description: share.name ? undefined : 'They get access when they sign in.',
          });
          inputRef.current?.focus();
        },
        onError: (err) => {
          setAddError(asSentence(errorMessage(err, 'Could not share')));
          inputRef.current?.focus();
        },
      },
    );
  };

  const onRemove = (address: string) => {
    setRemoveError(null);
    remove.mutate(address, {
      onSuccess: () => {
        toast.success(`Removed ${address}`, { description: 'Pages they have open stop loading.' });
        inputRef.current?.focus();
      },
      onError: (err) =>
        setRemoveError({
          email: address,
          message: asSentence(errorMessage(err, 'Could not remove them')),
        }),
    });
  };

  const list = shares.data?.shares ?? null;

  return (
    <section className="space-y-3" aria-labelledby={headingId}>
      <h3 id={headingId} className={SECTION_HEADING}>
        People with access
      </h3>

      <form onSubmit={submit} noValidate className="space-y-1.5">
        <Label htmlFor={`${ids}-email`} className="sr-only">
          Add people by email
        </Label>
        <div className="flex flex-col gap-2 sm:flex-row">
          <Input
            ref={inputRef}
            id={`${ids}-email`}
            type="email"
            inputMode="email"
            autoComplete="off"
            spellCheck={false}
            placeholder="Add people by email"
            value={email}
            onChange={(e) => {
              setEmail(e.target.value);
              if (addError) setAddError(null);
            }}
            aria-invalid={addError ? true : undefined}
            aria-describedby={addError ? errorId : undefined}
            className="sm:flex-1"
          />
          <div className="flex gap-2">
            <div className="w-36 shrink-0">
              <NativeSelect
                aria-label="Role for the person you add"
                value={role}
                onChange={(e) => setRole(e.target.value as ShareRole)}
              >
                {ROLES.map((r) => (
                  <option key={r} value={r}>
                    {ROLE_LABEL[r]}
                  </option>
                ))}
              </NativeSelect>
            </div>
            <Button
              type="submit"
              className="flex-1 sm:flex-none"
              disabled={!email.trim() || add.isPending}
            >
              {add.isPending ? <Loader2 className="animate-spin" /> : <UserPlus />}
              Share
            </Button>
          </div>
        </div>
        {addError && (
          <p
            id={errorId}
            role="alert"
            className="flex items-start gap-1.5 text-destructive text-sm"
          >
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            {addError}
          </p>
        )}
      </form>

      {shares.isError ? (
        <div className="space-y-2 rounded-lg border border-destructive/30 p-3 text-sm">
          <p className="text-destructive">
            {asSentence(errorMessage(shares.error, 'Could not load who it’s shared with.'))}
          </p>
          <Button variant="outline" size="sm" onClick={() => void shares.refetch()}>
            Try again
          </Button>
        </div>
      ) : list === null ? (
        <div className="space-y-2" role="status" aria-label="Loading people with access">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : (
        <ul className="divide-y rounded-lg border" aria-label="People with access">
          {me && (
            <li className="flex items-center gap-3 px-3 py-2">
              <Avatar name={me.name} email={me.email} />
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium text-sm">{me.name || me.email} (you)</p>
                {me.name && <p className="truncate text-muted-foreground text-xs">{me.email}</p>}
              </div>
              <span className="shrink-0 pr-1 text-muted-foreground text-sm">Owner</span>
            </li>
          )}
          {list.map((share) => (
            <ShareRow
              key={share.email}
              parcelId={parcelId}
              share={share}
              onRemove={() => onRemove(share.email)}
              removing={remove.isPending && remove.variables === share.email}
              removeError={removeError?.email === share.email ? removeError.message : null}
            />
          ))}
        </ul>
      )}

      {list !== null && list.length === 0 && visibility === 'private' && (
        <p className="text-muted-foreground text-xs">
          Not shared with anyone yet: only you can open it.
        </p>
      )}
      {list !== null && list.length > 0 && visibility !== 'private' && (
        <p className="text-muted-foreground text-xs">
          While general access is open to everyone signed in, everyone here can comment, whatever
          their role.
        </p>
      )}
    </section>
  );
}

/** One person: who they are, their role, and removing them. */
function ShareRow({
  parcelId,
  share,
  onRemove,
  removing,
  removeError,
}: {
  parcelId: string;
  share: Share;
  onRemove: () => void;
  removing: boolean;
  removeError: string | null;
}) {
  const change = usePutShare(parcelId);
  const [changeError, setChangeError] = useState<string | null>(null);
  const display = shareDisplay(share);
  const errorId = useId();
  const busy = change.isPending || removing;
  const error = changeError ?? removeError;

  const setRole = (role: ShareRole) => {
    if (role === share.role) return;
    setChangeError(null);
    change.mutate(
      { email: share.email, role },
      {
        onSuccess: (updated) =>
          toast.success(`${shareDisplay(updated).primary}: ${ROLE_LABEL[updated.role]}`),
        onError: (err) =>
          setChangeError(asSentence(errorMessage(err, 'Could not change their role'))),
      },
    );
  };

  return (
    <li className="px-3 py-2" data-testid="share-row">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Avatar name={share.name} email={share.email} />
        {/* Wide enough to read; on phones the controls wrap below instead. */}
        <div className="min-w-0 flex-1 basis-40">
          <p className="truncate font-medium text-sm" title={display.primary}>
            {display.primary}
          </p>
          {display.secondary && (
            <p className="truncate text-muted-foreground text-xs">{display.secondary}</p>
          )}
          {display.pending && <p className="text-muted-foreground text-xs">Hasn’t signed in yet</p>}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-1">
          <div className="w-36">
            <NativeSelect
              aria-label={`Role for ${share.email}`}
              aria-describedby={error ? errorId : undefined}
              value={share.role}
              onChange={(e) => setRole(e.target.value as ShareRole)}
              disabled={busy}
              className="h-8"
            >
              {ROLES.map((r) => (
                <option key={r} value={r}>
                  {ROLE_LABEL[r]}
                </option>
              ))}
            </NativeSelect>
          </div>
          <IconButton
            label={`Remove ${share.email}`}
            onClick={() => {
              setChangeError(null);
              onRemove();
            }}
            disabled={busy}
            className="text-muted-foreground hover:text-destructive"
          >
            {removing ? <Loader2 className="animate-spin" /> : <X />}
          </IconButton>
        </div>
      </div>
      {error && (
        <p id={errorId} role="alert" className="mt-1.5 text-destructive text-xs">
          {error}
        </p>
      )}
    </li>
  );
}
