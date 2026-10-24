import { type KeyboardEvent, useCallback, useId, useState } from 'react';
import { comboboxKey } from '@/lib/combobox';

interface ComboboxOptions<T> {
  options: readonly T[];
  onPick: (option: T) => void;
  /** Backspace in an empty input: drop the last chip. */
  onRemoveLast?: () => void;
  /** How many chips sit before the input. */
  chips: number;
  /** The input's text. */
  text: string;
}

/**
 * State and props for the ARIA combobox pattern (lib/combobox.ts has the
 * key rules). Focus stays in the input; the active option is announced
 * through `aria-activedescendant`.
 */
export function useCombobox<T>({ options, onPick, onRemoveLast, chips, text }: ComboboxOptions<T>) {
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  // Typing chose the active option; the user hasn't moved yet.
  const [fresh, setFresh] = useState(false);
  const shown = open && options.length > 0;
  const current = Math.min(active, options.length - 1);
  const optionId = (index: number) => `${listId}-option-${index}`;

  /** Typing: show the list with its first option active. */
  const show = useCallback(() => {
    setOpen(true);
    setActive(0);
    setFresh(true);
  }, []);
  const close = useCallback(() => setOpen(false), []);

  const pick = (index: number) => {
    const option = options[index];
    if (option === undefined) return;
    onPick(option);
    setActive(0);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    const action = comboboxKey(event.key, {
      open: shown,
      active: current,
      count: options.length,
      text,
      chips,
      fresh,
    });
    switch (action.type) {
      case 'move':
        event.preventDefault();
        setOpen(true);
        setActive(action.active);
        setFresh(false);
        break;
      case 'pick':
        event.preventDefault();
        pick(action.index);
        break;
      case 'close':
        // A dialog around the input stays open (see `keepDialogOpen`).
        event.preventDefault();
        setOpen(false);
        break;
      case 'remove-last':
        event.preventDefault();
        onRemoveLast?.();
        break;
      case 'none':
        break;
    }
  };

  return {
    shown,
    active: current,
    show,
    close,
    pick,
    optionId,
    listId,
    inputProps: {
      role: 'combobox',
      'aria-expanded': shown,
      'aria-controls': listId,
      'aria-autocomplete': 'list' as const,
      'aria-activedescendant': shown && current >= 0 ? optionId(current) : undefined,
      autoComplete: 'off',
      onKeyDown,
      onBlur: close,
      // Back in the input (a dialog's focus trap returns it there, say):
      // its suggestions again, if the text has any.
      onFocus: () => setOpen(true),
    },
  };
}

/**
 * For a Radix dialog's `onEscapeKeyDown`: Escape in an open combobox closes
 * its list, not the dialog. (Radix sees the key first, in the capture phase,
 * while the list is still open.)
 */
export function keepDialogOpen(event: globalThis.KeyboardEvent): void {
  const target = event.target;
  if (
    target instanceof HTMLElement &&
    target.getAttribute('role') === 'combobox' &&
    target.getAttribute('aria-expanded') === 'true'
  ) {
    event.preventDefault();
  }
}
