import { type ReactNode, useEffect, useRef } from 'react';
import { cn } from '@/lib/utils';

/**
 * The listbox under a combobox input. Options are picked with the mouse
 * (mousedown is cancelled so the input keeps focus) or from the keyboard
 * through the input.
 */
export function SuggestionList<T>({
  id,
  label,
  options,
  active,
  optionId,
  onPick,
  render,
  footer,
  inline,
  className,
}: {
  id: string;
  label: string;
  options: readonly T[];
  active: number;
  optionId: (index: number) => string;
  onPick: (index: number) => void;
  render: (option: T) => ReactNode;
  /** A hint under the options (not an option). */
  footer?: ReactNode;
  /** In the page's flow under the input (in dialogs, which clip overflow) instead of floating. */
  inline?: boolean;
  className?: string;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${active}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  return (
    <div
      className={cn(
        'overflow-hidden rounded-lg border bg-popover text-popover-foreground',
        inline ? 'mt-1' : 'absolute top-[calc(100%+4px)] right-0 left-0 z-50 shadow-lg',
        className,
      )}
    >
      <div
        ref={listRef}
        id={id}
        role="listbox"
        aria-label={label}
        className="max-h-64 overflow-y-auto p-1"
      >
        {options.map((option, index) => (
          // biome-ignore lint/a11y/useKeyWithClickEvents: the input owns the keyboard (aria-activedescendant).
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: options are positional; the index is the id.
            key={index}
            id={optionId(index)}
            role="option"
            // Focus stays in the input; options are reached with ↑/↓.
            tabIndex={-1}
            aria-selected={index === active}
            data-index={index}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => onPick(index)}
            className={cn(
              'flex cursor-pointer items-center justify-between gap-3 rounded-md px-2 py-1.5 text-sm',
              index === active ? 'bg-accent' : 'hover:bg-accent/60',
            )}
          >
            {render(option)}
          </div>
        ))}
      </div>
      {footer && <div className="border-t px-3 py-1.5 text-muted-foreground text-xs">{footer}</div>}
    </div>
  );
}
