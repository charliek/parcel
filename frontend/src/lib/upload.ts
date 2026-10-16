/**
 * Upload preparation: published-path derivation, entry detection and limit
 * checks, mirroring the server rules in `crates/parcel-types/src/{paths,media}.rs`
 * so a bad publish fails in the browser before any bytes move.
 *
 * Everything here is pure (no DOM), so it is unit-tested under bun.
 */

import { formatBytes } from './format';
import type { CreateParcelMetadata, Limits, NewVersionMetadata } from './types';

/** Longest accepted published path, in UTF-8 bytes (`paths::MAX_PATH_LEN`). */
export const MAX_PATH_LEN = 512;

/** Extensions that can be a parcel's entry page (`media::Role::Entry`). */
export const ENTRY_EXTENSIONS = ['html', 'htm', 'md', 'markdown'] as const;

/**
 * The server's allow-list (`media::MEDIA_TYPES`), used only until
 * `/api/v1/info` provides the authoritative `allowed_extensions`.
 */
export const DEFAULT_ALLOWED_EXTENSIONS = [
  'html',
  'htm',
  'md',
  'markdown',
  'css',
  'js',
  'mjs',
  'map',
  'wasm',
  'json',
  'geojson',
  'txt',
  'csv',
  'tsv',
  'xml',
  'yaml',
  'yml',
  'svg',
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'avif',
  'ico',
  'bmp',
  'woff',
  'woff2',
  'ttf',
  'otf',
  'mp3',
  'wav',
  'ogg',
  'm4a',
  'mp4',
  'webm',
  'pdf',
  'glb',
  'gltf',
];

/** OS clutter that is never worth publishing. Compared case-insensitively. */
const JUNK_FILE_NAMES = new Set(['thumbs.db', 'desktop.ini']);
const JUNK_DIR_NAMES = new Set(['__macosx', 'node_modules']);

/** One file as picked or dropped, before a published path is decided. */
export interface RawUploadItem<F = File> {
  file: F;
  /** The file's own name (`app.js`). */
  name: string;
  /**
   * Path relative to what the user picked: `webkitRelativePath` from a folder
   * picker (`site/assets/app.js`) or a drop entry's `fullPath`
   * (`/site/assets/app.js`). Empty for individually picked files.
   */
  relativePath?: string;
}

/** A file with the path it will be published at. */
export interface PreparedFile<F = File> {
  file: F;
  path: string;
}

export interface DerivedPaths<F = File> {
  files: PreparedFile<F>[];
  /** Paths skipped as hidden (dot-files/dirs) or OS clutter. */
  ignored: string[];
}

function splitSegments(raw: string): string[] {
  return raw
    .replace(/\\/g, '/')
    .split('/')
    .filter((segment) => segment !== '' && segment !== '.');
}

/**
 * Decide published paths for one pick/drop batch.
 *
 * - Each file's path comes from its relative path, else its name.
 * - When every file sits under the same top folder (a folder upload), that
 *   folder is stripped: picking `site/` publishes `index.html`, not
 *   `site/index.html`, so the site's relative links keep working.
 * - Hidden files and folders (`.DS_Store`, `.git/…`) and OS clutter are skipped.
 */
export function derivePublishedPaths<F>(items: RawUploadItem<F>[]): DerivedPaths<F> {
  const split = items.map((item) => ({
    item,
    segments: splitSegments(item.relativePath ? item.relativePath : item.name),
  }));

  const first = split[0]?.segments[0];
  const sharedTopFolder =
    split.length > 0 &&
    first !== undefined &&
    split.every(({ segments }) => segments.length >= 2 && segments[0] === first);

  const files: PreparedFile<F>[] = [];
  const ignored: string[] = [];
  for (const { item, segments } of split) {
    const kept = sharedTopFolder ? segments.slice(1) : segments;
    const path = kept.join('/');
    if (kept.length === 0) continue;
    const fileName = kept[kept.length - 1].toLowerCase();
    const dirs = kept.slice(0, -1).map((segment) => segment.toLowerCase());
    const hidden = kept.some((segment) => segment.startsWith('.'));
    const junk = JUNK_FILE_NAMES.has(fileName) || dirs.some((dir) => JUNK_DIR_NAMES.has(dir));
    if (hidden || junk) {
      ignored.push(path);
      continue;
    }
    files.push({ file: item.file, path });
  }
  return { files, ignored };
}

/** Add a new batch to the current selection; a re-added path replaces the old file. */
export function mergeSelections<F>(
  current: PreparedFile<F>[],
  incoming: PreparedFile<F>[],
): PreparedFile<F>[] {
  const byPath = new Map<string, PreparedFile<F>>();
  for (const entry of current) byPath.set(entry.path, entry);
  for (const entry of incoming) byPath.set(entry.path, entry);
  return [...byPath.values()].sort((a, b) => comparePaths(a.path, b.path));
}

/** Sort root files before nested ones, then alphabetically. */
export function comparePaths(a: string, b: string): number {
  const depthA = a.split('/').length;
  const depthB = b.split('/').length;
  if (depthA !== depthB) return depthA - depthB;
  return a.localeCompare(b);
}

/** Validate a published path (`paths::validate_path`). Returns an error message or null. */
export function validatePublishedPath(path: string): string | null {
  if (path.length === 0) return 'Path is empty';
  if (new TextEncoder().encode(path).length > MAX_PATH_LEN) {
    return `Path is longer than ${MAX_PATH_LEN} bytes`;
  }
  if (path.startsWith('/')) return "Path must be relative (no leading '/')";
  if (path.includes('\\')) return "Path must use '/' separators";
  // biome-ignore lint/suspicious/noControlCharactersInRegex: matching control characters is the point.
  if (/[\u0000-\u001f\u007f-\u009f]/.test(path)) return 'Path contains a control character';
  for (const segment of path.split('/')) {
    if (segment === '') return 'Path contains an empty segment';
    if (segment === '.' || segment === '..') return "Path contains a '.' or '..' segment";
  }
  return null;
}

/** The lowercased extension of a path's file name (`media::extension`), or null. */
export function extensionOf(path: string): string | null {
  const name = path.split('/').pop() ?? path;
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return null;
  return name.slice(dot + 1).toLowerCase();
}

/** Whether a path can be the page a parcel opens on. */
export function isEntryCandidate(path: string): boolean {
  const ext = extensionOf(path);
  return ext !== null && (ENTRY_EXTENSIONS as readonly string[]).includes(ext);
}

/** Whether a path is Markdown (rendered to HTML by the server when it is the entry). */
export function isMarkdown(path: string): boolean {
  const ext = extensionOf(path);
  return ext === 'md' || ext === 'markdown';
}

/**
 * The server's default entry choice: the only HTML/Markdown file, else a
 * root `index.html`. Returns null when there is no candidate or the choice is
 * ambiguous (the user must pick one).
 */
export function detectEntry(paths: string[]): string | null {
  const candidates = paths.filter(isEntryCandidate);
  if (candidates.length === 1) return candidates[0];
  if (paths.includes('index.html')) return 'index.html';
  return null;
}

export interface UploadCheck {
  /** Per-path problems, keyed by published path. */
  fileErrors: Record<string, string>;
  /** Problems with the upload as a whole. */
  errors: string[];
  totalBytes: number;
  /** True when there is at least one file and nothing blocks the publish. */
  ok: boolean;
}

export interface UploadCheckOptions {
  limits: Limits;
  allowedExtensions?: string[];
  /** The entry page path, when known (gets the entry size limit). */
  entry?: string | null;
  /** Creates need an entry page; merge-mode versions keep the previous one. */
  requireEntry: boolean;
}

/** Check a selection against the server's path rules, allow-list and limits. */
export function checkUpload(
  files: { path: string; size: number }[],
  { limits, allowedExtensions, entry, requireEntry }: UploadCheckOptions,
): UploadCheck {
  const allowed = new Set(
    (allowedExtensions && allowedExtensions.length > 0
      ? allowedExtensions
      : DEFAULT_ALLOWED_EXTENSIONS
    ).map((ext) => ext.toLowerCase()),
  );
  const fileErrors: Record<string, string> = {};
  const errors: string[] = [];
  let totalBytes = 0;

  for (const { path, size } of files) {
    totalBytes += size;
    const pathError = validatePublishedPath(path);
    if (pathError) {
      fileErrors[path] = pathError;
      continue;
    }
    const ext = extensionOf(path);
    if (!ext || !allowed.has(ext)) {
      fileErrors[path] = ext ? `.${ext} files aren't supported` : 'Files need an extension';
      continue;
    }
    if (path === entry) {
      if (size > limits.max_entry_bytes) {
        fileErrors[path] =
          `Entry page is ${formatBytes(size)}; the limit is ${formatBytes(limits.max_entry_bytes)}`;
      }
    } else if (size > limits.max_file_bytes) {
      fileErrors[path] =
        `${formatBytes(size)} is over the ${formatBytes(limits.max_file_bytes)} per-file limit`;
    }
  }

  if (files.length > limits.max_files_per_upload) {
    errors.push(
      `${files.length} files is more than the ${limits.max_files_per_upload} allowed in one upload`,
    );
  }
  if (totalBytes > limits.max_upload_bytes) {
    errors.push(
      `${formatBytes(totalBytes)} is more than the ${formatBytes(limits.max_upload_bytes)} upload limit`,
    );
  }
  if (requireEntry && files.length > 0) {
    if (!files.some((f) => isEntryCandidate(f.path))) {
      errors.push('Add an HTML or Markdown page for the parcel to open on');
    } else if (!entry) {
      errors.push('Choose which page the parcel opens on');
    }
  }

  const ok = files.length > 0 && errors.length === 0 && Object.keys(fileErrors).length === 0;
  return { fileErrors, errors, totalBytes, ok };
}

/** Drop empty strings and undefined so optional fields are omitted on the wire. */
export function compactMetadata<T extends CreateParcelMetadata | NewVersionMetadata>(
  meta: T,
): Partial<T> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(meta)) {
    if (value === undefined || value === null) continue;
    if (typeof value === 'string' && value.trim() === '') continue;
    if (Array.isArray(value) && value.length === 0) continue;
    out[key] = typeof value === 'string' ? value.trim() : value;
  }
  return out as Partial<T>;
}

/**
 * Build the multipart body for `POST /api/v1/parcels` and
 * `POST /api/v1/parcels/{id}/versions`: the `metadata` JSON part first (when
 * there is any), then one `file` part per file whose *filename* is the
 * published path.
 */
export function buildPublishForm(
  metadata: CreateParcelMetadata | NewVersionMetadata,
  files: PreparedFile<Blob>[],
): FormData {
  const form = new FormData();
  const meta = compactMetadata(metadata);
  if (Object.keys(meta).length > 0) {
    form.append('metadata', JSON.stringify(meta));
  }
  for (const { file, path } of files) {
    form.append('file', file, path);
  }
  return form;
}
