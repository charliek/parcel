import { useCallback, useMemo, useState } from 'react';
import { toast } from 'sonner';
import { pluralize } from '@/lib/format';
import {
  checkUpload,
  derivePublishedPaths,
  detectEntry,
  isEntryCandidate,
  mergeSelections,
  type PreparedFile,
  type RawUploadItem,
} from '@/lib/upload';
import { useServerInfo } from '@/stores/server-info.store';

/**
 * Selected files for a publish: path derivation per pick/drop batch, entry
 * choice, and the limit check against `/api/v1/info`.
 */
export function useFileSelection(options: {
  /** Creates (and replace-mode versions) need an entry page. */
  requireEntry: boolean;
  /** Paths that already exist (merge mode), for entry candidates. */
  existingPaths?: string[];
  /** The previous version's entry (merge mode keeps it unless another is chosen). */
  currentEntry?: string;
}) {
  const info = useServerInfo();
  const [files, setFiles] = useState<PreparedFile[]>([]);
  const [entryChoice, setEntryChoice] = useState<string | null>(null);

  const add = useCallback((items: RawUploadItem[]) => {
    const { files: incoming, ignored } = derivePublishedPaths(items);
    if (ignored.length > 0) {
      toast.info(`Skipped ${pluralize(ignored.length, 'hidden or system file')}`, {
        description: ignored.slice(0, 3).join(', ') + (ignored.length > 3 ? '…' : ''),
      });
    }
    if (incoming.length > 0) setFiles((current) => mergeSelections(current, incoming));
  }, []);

  const remove = useCallback((path: string) => {
    setFiles((current) => current.filter((f) => f.path !== path));
  }, []);

  const clear = useCallback(() => {
    setFiles([]);
    setEntryChoice(null);
  }, []);

  const paths = useMemo(() => files.map((f) => f.path), [files]);
  const candidates = useMemo(() => {
    const all = new Set([...(options.existingPaths ?? []), ...paths].filter(isEntryCandidate));
    return [...all].sort();
  }, [paths, options.existingPaths]);

  // A choice that no longer exists falls back to automatic detection.
  const chosen = entryChoice && candidates.includes(entryChoice) ? entryChoice : null;
  const detected = options.requireEntry ? detectEntry(paths) : null;
  const entry = chosen ?? detected ?? options.currentEntry ?? null;

  const check = useMemo(
    () =>
      checkUpload(
        files.map((f) => ({ path: f.path, size: f.file.size })),
        {
          limits: info.limits,
          allowedExtensions: info.allowed_extensions,
          entry,
          requireEntry: options.requireEntry,
        },
      ),
    [files, info, entry, options.requireEntry],
  );

  return {
    files,
    add,
    remove,
    clear,
    candidates,
    /** The entry the user picked explicitly, if any. */
    chosenEntry: chosen,
    /** The effective entry (explicit choice or detected default). */
    entry,
    setEntryChoice,
    check,
    limits: info.limits,
  };
}

export type FileSelection = ReturnType<typeof useFileSelection>;
