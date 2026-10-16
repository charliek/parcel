import { useId } from 'react';
import { VISIBILITY_ICON } from '@/components/common/VisibilityBadge';
import type { Visibility } from '@/lib/types';
import { cn } from '@/lib/utils';
import { visibilityOptions } from '@/lib/visibility';

/**
 * Radio cards for who can open a parcel. `public` is disabled, with the
 * reason, when the server turned public links off.
 */
export function VisibilityPicker({
  value,
  onChange,
  allowPublic,
  disabled,
  layout = 'grid',
  legend = 'Visibility',
}: {
  value: Visibility;
  onChange: (value: Visibility) => void;
  allowPublic: boolean;
  disabled?: boolean;
  /** Side by side from `sm` up (forms), or stacked (dialogs). */
  layout?: 'grid' | 'stack';
  legend?: string;
}) {
  const id = useId();
  return (
    <fieldset
      className={cn('grid gap-2', layout === 'grid' && 'sm:grid-cols-3')}
      disabled={disabled}
    >
      <legend className="sr-only">{legend}</legend>
      {visibilityOptions(allowPublic).map(({ value: option, label, hint, disabledReason }) => {
        const Icon = VISIBILITY_ICON[option];
        const checked = value === option;
        const off = disabledReason !== null;
        return (
          <label
            key={option}
            className={cn(
              'flex items-start gap-3 rounded-lg border p-3 transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring',
              checked && 'border-primary bg-primary/5',
              !checked && !off && 'cursor-pointer hover:bg-muted/50',
              off && 'cursor-not-allowed bg-muted/30',
            )}
          >
            <input
              type="radio"
              name={`visibility-${id}`}
              value={option}
              checked={checked}
              disabled={off}
              onChange={() => onChange(option)}
              aria-labelledby={`${id}-${option}-label`}
              aria-describedby={`${id}-${option}-hint`}
              className="sr-only"
            />
            <Icon
              className={cn(
                'mt-0.5 h-4 w-4 shrink-0',
                checked ? 'text-primary' : 'text-muted-foreground',
              )}
              aria-hidden="true"
            />
            <span className="space-y-0.5">
              <span
                id={`${id}-${option}-label`}
                className={cn('block font-medium text-sm', off && 'text-muted-foreground')}
              >
                {label}
              </span>
              <span id={`${id}-${option}-hint`} className="block text-muted-foreground text-xs">
                {disabledReason ?? hint}
              </span>
            </span>
          </label>
        );
      })}
    </fieldset>
  );
}
