import { cn } from '@/lib/utils';
import { CopyButton } from './CopyButton';

interface CodeBlockProps {
  /** The command(s) to show. Copied verbatim. */
  code: string;
  /** Show a `$` prompt before each line (not copied). */
  prompt?: boolean;
  className?: string;
  label?: string;
}

/** A terminal-style code block with a copy button. */
export function CodeBlock({ code, prompt = true, className, label }: CodeBlockProps) {
  // Stable keys without array indexes: a line's text plus its occurrence count.
  const seen = new Map<string, number>();
  const lines = code.split('\n').map((line) => {
    const n = (seen.get(line) ?? 0) + 1;
    seen.set(line, n);
    return { key: `${n}:${line}`, line };
  });
  return (
    <div
      className={cn(
        'group relative rounded-lg border bg-muted/50 font-mono text-[13px] leading-relaxed',
        className,
      )}
    >
      <pre className="overflow-x-auto py-3 pr-12 pl-4">
        <code>
          {lines.map(({ key, line }) => (
            <span key={key} className="block whitespace-pre">
              {prompt && line && !line.startsWith(' ') && (
                <span className="select-none text-muted-foreground">$ </span>
              )}
              {line}
            </span>
          ))}
        </code>
      </pre>
      <CopyButton
        value={code}
        variant="ghost"
        className="absolute top-1.5 right-1.5 text-muted-foreground"
        aria-label={label ? `Copy ${label}` : 'Copy command'}
        toastMessage="Copied"
      />
    </div>
  );
}
