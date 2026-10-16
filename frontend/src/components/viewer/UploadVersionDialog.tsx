import { Loader2, Upload } from 'lucide-react';
import { type FormEvent, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { NativeSelect } from '@/components/ui/select';
import { FileDropZone } from '@/components/upload/FileDropZone';
import { FileSelectionList } from '@/components/upload/FileSelectionList';
import { useFileSelection } from '@/hooks/useFileSelection';
import { invalidateParcel } from '@/hooks/useParcels';
import { api } from '@/lib/api';
import { formatBytes } from '@/lib/format';
import { ApiError, errorMessage } from '@/lib/http';
import { queryClient } from '@/lib/query-client';
import type { FileEntry, ParcelSummary, PublishMode } from '@/lib/types';
import { buildPublishForm } from '@/lib/upload';
import { cn } from '@/lib/utils';

const MODES: { value: PublishMode; label: string; hint: string }[] = [
  {
    value: 'merge',
    label: 'Update files',
    hint: 'Add or replace the files you upload; keep the rest.',
  },
  {
    value: 'replace',
    label: 'Replace everything',
    hint: 'The new version is exactly the files you upload.',
  },
];

interface UploadVersionDialogProps {
  parcel: ParcelSummary;
  /** Files of the current version (for merge-mode removals and entry choice). */
  currentFiles: FileEntry[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Publish a new version: `POST /api/v1/parcels/{id}/versions`. */
export function UploadVersionDialog(props: UploadVersionDialogProps) {
  // Remount the body each time it opens so state starts fresh.
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent className="max-w-2xl">
        {props.open && <UploadVersionForm {...props} />}
      </DialogContent>
    </Dialog>
  );
}

function UploadVersionForm({ parcel, currentFiles, onOpenChange }: UploadVersionDialogProps) {
  const navigate = useNavigate();
  const [mode, setMode] = useState<PublishMode>('merge');
  const [label, setLabel] = useState('');
  const [removed, setRemoved] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<number | null>(null);
  const submitting = progress !== null;

  const keptPaths = useMemo(
    () => (mode === 'merge' ? currentFiles.map((f) => f.path).filter((p) => !removed.has(p)) : []),
    [mode, currentFiles, removed],
  );
  const selection = useFileSelection({
    requireEntry: mode === 'replace',
    existingPaths: keptPaths,
    currentEntry: mode === 'merge' && !removed.has(parcel.entry) ? parcel.entry : undefined,
  });

  const removingEntry = mode === 'merge' && removed.has(parcel.entry) && !selection.chosenEntry;
  const nothingToDo = selection.files.length === 0 && (mode === 'replace' || removed.size === 0);
  const canSubmit =
    !submitting &&
    !removingEntry &&
    !nothingToDo &&
    (selection.files.length === 0 ? mode === 'merge' : selection.check.ok);

  const toggleRemove = (path: string) => {
    setRemoved((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const onSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canSubmit) return;
    setProgress(0);
    try {
      const form = buildPublishForm(
        {
          mode,
          label,
          remove: mode === 'merge' ? [...removed] : undefined,
          entry:
            selection.chosenEntry ??
            (mode === 'replace' ? (selection.entry ?? undefined) : undefined),
          base_version: parcel.current_version,
        },
        selection.files,
      );
      const result = await api.publishVersion(parcel.id, form, setProgress);
      await invalidateParcel(queryClient, parcel.id);
      toast.success(`Published version ${result.version.version}`);
      onOpenChange(false);
      navigate(`/p/${parcel.id}`);
    } catch (err) {
      setProgress(null);
      const conflict = err instanceof ApiError && err.status === 409;
      toast.error(conflict ? 'A newer version was published meanwhile' : 'Publish failed', {
        description: conflict
          ? 'Reload to see the latest version, then try again.'
          : errorMessage(err),
      });
    }
  };

  return (
    <form onSubmit={onSubmit} className="grid gap-5">
      <DialogHeader>
        <DialogTitle>Upload a new version</DialogTitle>
        <DialogDescription>
          The link stays the same. Version {parcel.current_version} stays viewable in the history.
        </DialogDescription>
      </DialogHeader>

      <fieldset className="grid gap-2 sm:grid-cols-2" disabled={submitting}>
        <legend className="sr-only">How to build the new version</legend>
        {MODES.map((option) => (
          <label
            key={option.value}
            className={cn(
              'flex cursor-pointer flex-col gap-0.5 rounded-lg border p-3 transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring',
              mode === option.value ? 'border-primary bg-primary/5' : 'hover:bg-muted/50',
            )}
          >
            <input
              type="radio"
              name="publish-mode"
              value={option.value}
              checked={mode === option.value}
              onChange={() => setMode(option.value)}
              className="sr-only"
            />
            <span className="font-medium text-sm">{option.label}</span>
            <span className="text-muted-foreground text-xs">{option.hint}</span>
          </label>
        ))}
      </fieldset>

      <div className="space-y-3">
        <FileDropZone onItems={selection.add} compact disabled={submitting} />
        <FileSelectionList selection={selection} disabled={submitting} />
      </div>

      {mode === 'merge' && currentFiles.length > 0 && (
        <details className="group rounded-lg border">
          <summary className="cursor-pointer select-none px-3 py-2 text-sm hover:bg-muted/50">
            Remove files from version {parcel.current_version}
            {removed.size > 0 && (
              <span className="ml-1 text-muted-foreground">({removed.size} selected)</span>
            )}
          </summary>
          <ul className="max-h-48 divide-y overflow-y-auto border-t">
            {currentFiles.map((file) => (
              <li key={file.path}>
                <label className="flex cursor-pointer items-center gap-3 px-3 py-1.5 text-sm hover:bg-muted/40">
                  <input
                    type="checkbox"
                    checked={removed.has(file.path)}
                    onChange={() => toggleRemove(file.path)}
                    disabled={submitting}
                    className="h-4 w-4 accent-[var(--color-primary)]"
                  />
                  <span
                    className={cn(
                      'min-w-0 flex-1 truncate font-mono text-[13px]',
                      removed.has(file.path) && 'text-muted-foreground line-through',
                    )}
                  >
                    {file.path}
                  </span>
                  <span className="text-muted-foreground text-xs">
                    {formatBytes(file.size_bytes)}
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </details>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="version-entry">Opens on</Label>
          <NativeSelect
            id="version-entry"
            value={selection.chosenEntry ?? ''}
            onChange={(e) => selection.setEntryChoice(e.target.value || null)}
            disabled={submitting || selection.candidates.length === 0}
          >
            <option value="">
              {mode === 'merge' && !removed.has(parcel.entry)
                ? `Keep ${parcel.entry}`
                : selection.entry
                  ? `Automatic (${selection.entry})`
                  : 'Choose a page…'}
            </option>
            {selection.candidates.map((path) => (
              <option key={path} value={path}>
                {path}
              </option>
            ))}
          </NativeSelect>
          {removingEntry && (
            <p className="text-destructive text-xs">
              You're removing the entry page; choose another page to open on.
            </p>
          )}
        </div>
        <div className="space-y-2">
          <Label htmlFor="version-label">Version note</Label>
          <Input
            id="version-label"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. Fixed the chart colors"
            maxLength={60}
            disabled={submitting}
          />
        </div>
      </div>

      <DialogFooter className="items-center">
        {submitting && (
          <span className="text-muted-foreground text-sm sm:mr-auto" aria-live="polite">
            {progress !== null && progress < 1
              ? `Uploading… ${Math.round(progress * 100)}%`
              : 'Processing…'}
          </span>
        )}
        <Button
          type="button"
          variant="outline"
          onClick={() => onOpenChange(false)}
          disabled={submitting}
        >
          Cancel
        </Button>
        <Button type="submit" disabled={!canSubmit}>
          {submitting ? <Loader2 className="animate-spin" /> : <Upload />}
          Publish version
        </Button>
      </DialogFooter>
    </form>
  );
}
