import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { cleanUrl } from '@/lib/refs';
import { checkSlugText } from '@/lib/slugs';
import { useAppOrigin } from '@/stores/server-info.store';

/**
 * A parcel's slug, with the clean URL it gives previewed under it (the
 * preview is the only URL the app spells out itself; once saved, the
 * server's `canonical_url` is what is shown and copied).
 */
export function SlugField({
  id,
  value,
  onChange,
  handle,
  serverError,
  disabled,
  note,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  /** The owner's handle, when known. */
  handle: string | undefined;
  /** What the server said about the last attempt (a 409, say). */
  serverError?: string | null;
  disabled?: boolean;
  /** Extra help under the field. */
  note?: string;
}) {
  const origin = useAppOrigin();
  const checked = checkSlugText(value);
  const error = !checked.ok ? checked.message : (serverError ?? null);
  const preview =
    checked.ok && checked.slug && handle ? cleanUrl(origin, handle, checked.slug) : null;
  const helpId = `${id}-help`;

  return (
    <div className="space-y-2">
      <Label htmlFor={id}>Slug</Label>
      <Input
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder="e.g. roost/q4-report"
        maxLength={120}
        disabled={disabled}
        aria-invalid={error ? true : undefined}
        aria-describedby={helpId}
        spellCheck={false}
        autoCapitalize="none"
        className="font-mono"
      />
      <div id={helpId} className="space-y-1 text-xs" aria-live="polite">
        {error ? (
          <p className="text-destructive" data-testid="slug-error">
            {error}
          </p>
        ) : preview ? (
          <p className="break-all font-mono text-muted-foreground" data-testid="slug-preview">
            {preview}
          </p>
        ) : (
          <p className="text-muted-foreground">
            An optional name for a readable link, unique among your parcels.
          </p>
        )}
        {note && <p className="text-muted-foreground">{note}</p>}
      </div>
    </div>
  );
}
