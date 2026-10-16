/**
 * Turn browser file inputs and drag-and-drop payloads into {@link RawUploadItem}s.
 *
 * Folder drops have no `webkitRelativePath`, so directories are walked with the
 * (non-standard but universally supported) `webkitGetAsEntry()` API, using each
 * entry's `fullPath` as the relative path.
 */

import type { RawUploadItem } from './upload';

/** Items from an `<input type="file">` (with or without `webkitdirectory`). */
export function itemsFromFileList(list: FileList | File[]): RawUploadItem[] {
  return Array.from(list).map((file) => ({
    file,
    name: file.name,
    relativePath: file.webkitRelativePath || '',
  }));
}

function readFile(entry: FileSystemFileEntry): Promise<File> {
  return new Promise((resolve, reject) => entry.file(resolve, reject));
}

function readBatch(reader: FileSystemDirectoryReader): Promise<FileSystemEntry[]> {
  return new Promise((resolve, reject) => reader.readEntries(resolve, reject));
}

async function walk(entry: FileSystemEntry, out: RawUploadItem[]): Promise<void> {
  if (entry.isFile) {
    const file = await readFile(entry as FileSystemFileEntry);
    out.push({ file, name: file.name, relativePath: entry.fullPath || file.name });
    return;
  }
  if (entry.isDirectory) {
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    // readEntries returns results in batches (~100 in Chrome) until empty.
    for (;;) {
      const batch = await readBatch(reader);
      if (batch.length === 0) break;
      for (const child of batch) await walk(child, out);
    }
  }
}

/** Items from a drop event, recursing into dropped folders. */
export async function itemsFromDataTransfer(dt: DataTransfer): Promise<RawUploadItem[]> {
  const entries: FileSystemEntry[] = [];
  const looseFiles: File[] = [];
  // Entries must be captured synchronously, before the first await.
  for (const item of Array.from(dt.items ?? [])) {
    if (item.kind !== 'file') continue;
    const entry = typeof item.webkitGetAsEntry === 'function' ? item.webkitGetAsEntry() : null;
    if (entry) {
      entries.push(entry);
    } else {
      const file = item.getAsFile();
      if (file) looseFiles.push(file);
    }
  }
  if (entries.length === 0 && looseFiles.length === 0) {
    return itemsFromFileList(dt.files);
  }
  const out: RawUploadItem[] = [];
  for (const entry of entries) await walk(entry, out);
  for (const file of looseFiles) out.push({ file, name: file.name, relativePath: '' });
  return out;
}
