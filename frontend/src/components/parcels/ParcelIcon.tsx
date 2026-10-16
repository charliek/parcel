import { FileCode2, FileText } from 'lucide-react';
import { isMarkdown } from '@/lib/upload';
import { cn } from '@/lib/utils';

/** A small tile hinting at the entry type (HTML page or Markdown document). */
export function ParcelIcon({ entry, className }: { entry: string; className?: string }) {
  const Icon = isMarkdown(entry) ? FileText : FileCode2;
  return (
    <span
      className={cn(
        'flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border bg-muted/60 text-muted-foreground',
        className,
      )}
      aria-hidden="true"
    >
      <Icon className="h-[18px] w-[18px]" />
    </span>
  );
}
