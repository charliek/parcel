import { FolderOpen, Upload } from 'lucide-react';
import { type DragEvent, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { itemsFromDataTransfer, itemsFromFileList } from '@/lib/drop';
import { formatBytes } from '@/lib/format';
import type { RawUploadItem } from '@/lib/upload';
import { cn } from '@/lib/utils';
import { useServerInfo } from '@/stores/server-info.store';

interface FileDropZoneProps {
  onItems: (items: RawUploadItem[]) => void;
  compact?: boolean;
  disabled?: boolean;
  className?: string;
}

/**
 * Drag & drop target plus file and folder pickers. A dropped or picked folder
 * keeps its structure (the top folder is stripped later by
 * `derivePublishedPaths`).
 */
export function FileDropZone({ onItems, compact, disabled, className }: FileDropZoneProps) {
  const info = useServerInfo();
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const accept = info.allowed_extensions.map((ext) => `.${ext}`).join(',');

  const onDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setDragging(false);
    if (disabled) return;
    try {
      const items = await itemsFromDataTransfer(event.dataTransfer);
      if (items.length > 0) onItems(items);
    } catch {
      toast.error('Could not read the dropped files');
    }
  };

  const onPicked = (input: HTMLInputElement | null) => {
    if (!input?.files || input.files.length === 0) return;
    onItems(itemsFromFileList(input.files));
    // Reset so picking the same file again still fires `change`.
    input.value = '';
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: drag target only; the buttons inside are the keyboard path.
    <div
      onDragEnter={(e) => {
        e.preventDefault();
        if (!disabled) setDragging(true);
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = disabled ? 'none' : 'copy';
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={onDrop}
      className={cn(
        'flex flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed px-6 text-center transition-colors',
        compact ? 'py-6' : 'py-12',
        dragging ? 'border-primary bg-primary/5' : 'border-border bg-muted/30',
        disabled && 'opacity-60',
        className,
      )}
      data-testid="drop-zone"
    >
      <div
        className={cn(
          'flex items-center justify-center rounded-full bg-background text-muted-foreground shadow-xs ring-1 ring-border',
          compact ? 'h-10 w-10' : 'h-12 w-12',
          dragging && 'text-primary',
        )}
      >
        <Upload className="h-5 w-5" aria-hidden="true" />
      </div>
      <div className="space-y-1">
        <p className="font-medium text-sm">
          {dragging ? 'Drop to add' : 'Drag and drop files or a folder'}
        </p>
        {!compact && (
          <p className="mx-auto max-w-md text-muted-foreground text-xs">
            An HTML or Markdown entry page plus any CSS, JavaScript, images, fonts or data it uses.
            Up to {formatBytes(info.limits.max_upload_bytes)} and {info.limits.max_files_per_upload}{' '}
            files per upload.
          </p>
        )}
      </div>
      <div className="flex flex-wrap justify-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => fileInput.current?.click()}
        >
          <Upload />
          Choose files
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => folderInput.current?.click()}
        >
          <FolderOpen />
          Choose folder
        </Button>
      </div>
      <input
        ref={fileInput}
        type="file"
        multiple
        accept={accept}
        className="sr-only"
        tabIndex={-1}
        aria-label="Choose files"
        data-testid="file-input"
        onChange={(e) => onPicked(e.currentTarget)}
      />
      <input
        ref={(el) => {
          folderInput.current = el;
          // Not in React's typings; set as a plain attribute.
          el?.setAttribute('webkitdirectory', '');
          el?.setAttribute('directory', '');
        }}
        type="file"
        multiple
        className="sr-only"
        tabIndex={-1}
        aria-label="Choose folder"
        data-testid="folder-input"
        onChange={(e) => onPicked(e.currentTarget)}
      />
    </div>
  );
}
