import { Loader2 } from 'lucide-react';
import type { FormEvent, KeyboardEvent, ReactNode, Ref } from 'react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { isValidBody, MAX_COMMENT_CHARS } from '@/lib/comments';
import { cn } from '@/lib/utils';

/** Show the character count once a comment gets this close to the limit. */
const COUNT_FROM = MAX_COMMENT_CHARS - 500;

interface CommentComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  /** Shown as a Cancel button, and run on Escape. */
  onCancel?: () => void;
  pending: boolean;
  /** Accessible name of the text box. */
  label: string;
  placeholder: string;
  submitLabel: string;
  autoFocus?: boolean;
  textareaRef?: Ref<HTMLTextAreaElement>;
  /** Shown above the text box (the anchored spot, for a new thread). */
  context?: ReactNode;
  className?: string;
}

/** A small form for a comment, reply or edit. Ctrl/⌘+Enter sends. */
export function CommentComposer({
  value,
  onChange,
  onSubmit,
  onCancel,
  pending,
  label,
  placeholder,
  submitLabel,
  autoFocus,
  textareaRef,
  context,
  className,
}: CommentComposerProps) {
  const sendable = isValidBody(value) && !pending;
  const length = Array.from(value.trim()).length;

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (sendable) onSubmit();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      submit();
    } else if (event.key === 'Escape' && onCancel) {
      // Handled here, so the viewer doesn't also leave comment mode.
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    }
  };

  return (
    <form onSubmit={submit} className={cn('space-y-2', className)}>
      {context}
      <Textarea
        ref={textareaRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        aria-label={label}
        placeholder={placeholder}
        autoFocus={autoFocus}
        rows={2}
        className="max-h-60 min-h-[64px] resize-y"
      />
      <div className="flex items-center gap-2">
        {length >= COUNT_FROM && (
          <span
            className={cn(
              'text-xs tabular-nums',
              length > MAX_COMMENT_CHARS ? 'text-destructive' : 'text-muted-foreground',
            )}
          >
            {length}/{MAX_COMMENT_CHARS}
          </span>
        )}
        <div className="ml-auto flex gap-2">
          {onCancel && (
            <Button type="button" variant="ghost" size="sm" onClick={onCancel} disabled={pending}>
              Cancel
            </Button>
          )}
          <Button type="submit" size="sm" disabled={!sendable}>
            {pending && <Loader2 className="animate-spin" />}
            {submitLabel}
          </Button>
        </div>
      </div>
    </form>
  );
}
