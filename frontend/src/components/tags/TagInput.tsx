import { useState } from 'react';
import { useCombobox } from '@/hooks/useCombobox';
import { OWN_TAGS, useTagCounts } from '@/hooks/useTags';
import { type TagOption, tagOptions } from '@/lib/combobox';
import { pluralize } from '@/lib/format';
import { MAX_TAGS, normalizeTag, TAG_RULES } from '@/lib/tags';
import { cn } from '@/lib/utils';
import { SuggestionList } from './SuggestionList';
import { RemovableChip, TagChip } from './TagChip';

/**
 * A chip input for a parcel's tags, with autocomplete from the caller's own
 * vocabulary: existing tags first (with counts), then "Create …" for a new
 * one, shown as it will be saved (normalized as you type).
 */
export function TagInput({
  id,
  value,
  onChange,
  disabled,
  describedBy,
}: {
  id: string;
  value: string[];
  onChange: (tags: string[]) => void;
  disabled?: boolean;
  /** Id of the field's help text. */
  describedBy?: string;
}) {
  const [text, setText] = useState('');
  const vocabulary = useTagCounts(OWN_TAGS, !disabled);
  const full = value.length >= MAX_TAGS;
  const options = full ? [] : tagOptions(vocabulary.data ?? [], text, value);
  const normalized = text.trim() ? normalizeTag(text) : null;

  const add = (tag: string) => {
    if (!value.includes(tag) && value.length < MAX_TAGS) onChange([...value, tag]);
    setText('');
  };

  const combobox = useCombobox<TagOption>({
    options,
    onPick: (option) => add(option.tag),
    onRemoveLast: () => onChange(value.slice(0, -1)),
    chips: value.length,
    text,
  });

  let hint: string | null = null;
  if (full) hint = `At most ${MAX_TAGS} tags.`;
  else if (text.trim() && normalized === null) {
    hint = `Tags are ${TAG_RULES}`;
  } else if (normalized && normalized !== text.trim()) hint = `Saved as “${normalized}”.`;

  return (
    <div className="space-y-1.5">
      <div className="relative">
        <div
          className={cn(
            'flex min-h-9 w-full flex-wrap items-center gap-1 rounded-md border border-input bg-background px-2 py-1 shadow-xs focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 focus-within:ring-offset-background',
            disabled && 'cursor-not-allowed opacity-50',
          )}
          data-testid="tag-input"
        >
          {value.map((tag) => (
            <RemovableChip
              key={tag}
              tag={tag}
              disabled={disabled}
              onRemove={() => onChange(value.filter((t) => t !== tag))}
            />
          ))}
          <input
            id={id}
            {...combobox.inputProps}
            aria-describedby={describedBy}
            value={text}
            disabled={disabled}
            placeholder={value.length === 0 ? 'Add tags, e.g. project:roost or kind:plan' : ''}
            onChange={(event) => {
              setText(event.target.value);
              combobox.show();
            }}
            onKeyDown={(event) => {
              combobox.inputProps.onKeyDown(event);
              if (event.defaultPrevented) return;
              // Enter with text adds it (never submits the form); a comma too.
              if ((event.key === 'Enter' || event.key === ',') && text.trim()) {
                event.preventDefault();
                if (normalized) add(normalized);
              }
            }}
            className="min-w-[8rem] flex-1 bg-transparent py-0.5 text-sm outline-none placeholder:text-muted-foreground"
          />
        </div>
        {combobox.shown && (
          <SuggestionList
            id={combobox.listId}
            label="Tag suggestions"
            inline
            options={options}
            active={combobox.active}
            optionId={combobox.optionId}
            onPick={combobox.pick}
            render={(option) =>
              option.kind === 'existing' ? (
                <>
                  <TagChip tag={option.tag} />
                  <span className="shrink-0 text-muted-foreground text-xs">
                    {pluralize(option.count, 'parcel')}
                  </span>
                </>
              ) : (
                <>
                  <span className="min-w-0 truncate">
                    Create <TagChip tag={option.tag} />
                  </span>
                  <span className="shrink-0 text-muted-foreground text-xs">new</span>
                </>
              )
            }
          />
        )}
      </div>
      {hint && (
        <p className="text-muted-foreground text-xs" aria-live="polite">
          {hint}
        </p>
      )}
    </div>
  );
}
