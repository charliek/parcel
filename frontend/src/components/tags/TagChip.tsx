import { X } from 'lucide-react';
import { Link } from 'react-router-dom';
import { tagNamespace, tagValue } from '@/lib/tags';
import { cn } from '@/lib/utils';

/** Colours by namespace: projects and kinds stand out, other tags stay neutral. */
function chipTone(tag: string): string {
  switch (tagNamespace(tag)) {
    case 'project':
      return 'bg-cyan-600/10 text-cyan-800 dark:bg-cyan-400/15 dark:text-cyan-200';
    case 'kind':
      return 'bg-indigo-600/10 text-indigo-800 dark:bg-indigo-400/15 dark:text-indigo-200';
    default:
      return 'bg-muted text-foreground';
  }
}

const CHIP =
  'inline-flex max-w-full items-center gap-0.5 whitespace-nowrap rounded-full border border-transparent px-2 py-0.5 font-medium font-mono text-[11px] leading-4';
const FOCUS =
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background';

/** `project:` dimmed, then the value. */
function TagLabel({ tag }: { tag: string }) {
  const ns = tagNamespace(tag);
  return (
    <span className="truncate">
      {ns && <span className="opacity-60">{ns}:</span>}
      {tagValue(tag)}
    </span>
  );
}

/**
 * A tag as a chip; with `to`, a link that opens that page (the dashboard
 * filtered to it).
 */
export function TagChip({ tag, to }: { tag: string; to?: string }) {
  if (to === undefined) {
    return (
      <span className={cn(CHIP, chipTone(tag))}>
        <TagLabel tag={tag} />
      </span>
    );
  }
  return (
    <Link
      to={to}
      title={`Parcels tagged ${tag}`}
      className={cn(CHIP, chipTone(tag), FOCUS, 'hover:brightness-95')}
    >
      <TagLabel tag={tag} />
    </Link>
  );
}

/** A tag chip that adds or removes a filter when clicked. */
export function TagChipButton({
  tag,
  on,
  onClick,
}: {
  tag: string;
  /** A selected filter: outlined. */
  on: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      title={on ? `Remove the ${tag} filter` : `Filter by ${tag}`}
      className={cn(CHIP, chipTone(tag), FOCUS, 'hover:brightness-95', on && 'border-current')}
      data-tag={tag}
    >
      <TagLabel tag={tag} />
    </button>
  );
}

/** A chip in an input, with a button that removes it. */
export function RemovableChip({
  tag,
  onRemove,
  disabled,
  removeLabel,
}: {
  tag: string;
  onRemove: () => void;
  disabled?: boolean;
  removeLabel?: string;
}) {
  return (
    <span className={cn(CHIP, chipTone(tag), 'pr-0.5')} data-tag={tag}>
      <TagLabel tag={tag} />
      <button
        type="button"
        onClick={onRemove}
        disabled={disabled}
        aria-label={removeLabel ?? `Remove ${tag}`}
        className="ml-0.5 rounded-full p-0.5 opacity-70 hover:bg-foreground/10 hover:opacity-100 disabled:opacity-40"
      >
        <X className="h-3 w-3" aria-hidden="true" />
      </button>
    </span>
  );
}
