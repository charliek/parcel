import { Globe, Lock } from 'lucide-react';
import type { Visibility } from '@/lib/types';
import { cn } from '@/lib/utils';

const OPTIONS: { value: Visibility; label: string; hint: string; icon: typeof Lock }[] = [
  { value: 'private', label: 'Private', hint: 'Only you can view it', icon: Lock },
  { value: 'public', label: 'Public', hint: 'Anyone with the link, no sign-in', icon: Globe },
];

/** Two radio cards for parcel visibility. */
export function VisibilityPicker({
  value,
  onChange,
  disabled,
}: {
  value: Visibility;
  onChange: (value: Visibility) => void;
  disabled?: boolean;
}) {
  return (
    <fieldset className="grid gap-2 sm:grid-cols-2" disabled={disabled}>
      <legend className="sr-only">Visibility</legend>
      {OPTIONS.map(({ value: option, label, hint, icon: Icon }) => (
        <label
          key={option}
          className={cn(
            'flex cursor-pointer items-start gap-3 rounded-lg border p-3 transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring',
            value === option ? 'border-primary bg-primary/5' : 'hover:bg-muted/50',
          )}
        >
          <input
            type="radio"
            name="visibility"
            value={option}
            checked={value === option}
            onChange={() => onChange(option)}
            className="sr-only"
          />
          <Icon
            className={cn(
              'mt-0.5 h-4 w-4 shrink-0',
              value === option ? 'text-primary' : 'text-muted-foreground',
            )}
            aria-hidden="true"
          />
          <span className="space-y-0.5">
            <span className="block font-medium text-sm">{label}</span>
            <span className="block text-muted-foreground text-xs">{hint}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}
