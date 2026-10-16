import { AlertCircle, FileIcon, X } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { FileSelection } from '@/hooks/useFileSelection';
import { formatBytes, percentOf, pluralize } from '@/lib/format';
import { cn } from '@/lib/utils';

/** The selected files with sizes, the entry marker, per-file problems and the size meter. */
export function FileSelectionList({
  selection,
  disabled,
}: {
  selection: FileSelection;
  disabled?: boolean;
}) {
  const { files, check, limits, entry, remove, clear } = selection;
  if (files.length === 0) return null;
  const pct = percentOf(check.totalBytes, limits.max_upload_bytes);
  const over = check.totalBytes > limits.max_upload_bytes;

  return (
    <div className="overflow-hidden rounded-xl border bg-card">
      <div className="flex items-center gap-3 border-b px-4 py-2.5">
        <span className="font-medium text-sm">{pluralize(files.length, 'file')}</span>
        <span className="text-muted-foreground text-xs">
          {formatBytes(check.totalBytes)} of {formatBytes(limits.max_upload_bytes)}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="ml-auto h-7 text-muted-foreground"
          onClick={clear}
          disabled={disabled}
        >
          Clear all
        </Button>
      </div>
      {/* Decorative: the header text already states the size against the limit. */}
      <div className="h-1 bg-muted" aria-hidden="true">
        <div
          className={cn('h-full transition-all', over ? 'bg-destructive' : 'bg-primary')}
          style={{ width: `${Math.max(pct, 1)}%` }}
        />
      </div>
      <ul className="max-h-80 divide-y overflow-y-auto" aria-label="Selected files">
        {files.map(({ path, file }) => {
          const problem = check.fileErrors[path];
          return (
            <li key={path} className="flex items-center gap-3 px-4 py-2">
              <FileIcon className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate font-mono text-[13px]" title={path}>
                    {path}
                  </span>
                  {path === entry && (
                    <Badge variant="accent" className="shrink-0">
                      Entry
                    </Badge>
                  )}
                </div>
                {problem && (
                  <p className="mt-0.5 flex items-center gap-1 text-destructive text-xs">
                    <AlertCircle className="h-3 w-3" aria-hidden="true" />
                    {problem}
                  </p>
                )}
              </div>
              <span className="shrink-0 text-muted-foreground text-xs tabular-nums">
                {formatBytes(file.size)}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="h-7 w-7 shrink-0 text-muted-foreground"
                onClick={() => remove(path)}
                disabled={disabled}
                aria-label={`Remove ${path}`}
              >
                <X />
              </Button>
            </li>
          );
        })}
      </ul>
      {check.errors.length > 0 && (
        <div className="space-y-1 border-t bg-destructive/5 px-4 py-2.5" role="alert">
          {check.errors.map((message) => (
            <p key={message} className="flex items-center gap-1.5 text-destructive text-xs">
              <AlertCircle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              {message}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}
