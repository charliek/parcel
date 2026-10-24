import { Search, X } from 'lucide-react';
import { SuggestionList } from '@/components/tags/SuggestionList';
import { TagChip } from '@/components/tags/TagChip';
import { useCombobox } from '@/hooks/useCombobox';
import { matchingTags } from '@/lib/combobox';
import { formatNumber } from '@/lib/format';
import { splitLastWord } from '@/lib/search-query';
import type { TagCount } from '@/lib/types';

/**
 * The dashboard's search: free text (titles, descriptions, ids, slugs and
 * page text) with tag autocomplete on the word being typed. Picking a
 * suggestion turns that word into a tag filter; the rest stays text.
 * Backspace in an empty box removes the last tag filter.
 */
export function SearchBox({
  text,
  onTextChange,
  tags,
  onPickTag,
  onRemoveLastTag,
  vocabulary,
}: {
  text: string;
  onTextChange: (text: string) => void;
  /** Tag filters in force (not suggested again). */
  tags: readonly string[];
  /** A suggestion was picked; `rest` is the text without the word it completed. */
  onPickTag: (tag: string, rest: string) => void;
  onRemoveLastTag: () => void;
  vocabulary: TagCount[];
}) {
  const { rest, last } = splitLastWord(text);
  const options = matchingTags(vocabulary, last, tags, 7);
  const combobox = useCombobox<TagCount>({
    options,
    onPick: (option) => onPickTag(option.tag, rest),
    onRemoveLast: onRemoveLastTag,
    chips: tags.length,
    text,
  });

  return (
    <div className="relative min-w-0 flex-1 basis-64">
      <Search
        className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
      <input
        type="search"
        {...combobox.inputProps}
        value={text}
        onChange={(event) => {
          onTextChange(event.target.value);
          combobox.show();
        }}
        placeholder="Search text, or type a tag"
        aria-label="Search parcels"
        className="flex h-9 w-full rounded-md border border-input bg-background py-1.5 pr-9 pl-9 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background [&::-webkit-search-cancel-button]:hidden"
      />
      {text && (
        <button
          type="button"
          onClick={() => onTextChange('')}
          className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Clear search"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
      {combobox.shown && (
        <SuggestionList
          id={combobox.listId}
          label="Tag suggestions"
          options={options}
          active={combobox.active}
          optionId={combobox.optionId}
          onPick={combobox.pick}
          render={(option) => (
            <>
              <TagChip tag={option.tag} />
              <span className="shrink-0 text-muted-foreground text-xs tabular-nums">
                {formatNumber(option.count)}
              </span>
            </>
          )}
          footer="Enter adds the tag · text you leave searches titles and page content"
        />
      )}
    </div>
  );
}
