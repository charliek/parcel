import { describe, expect, it } from 'bun:test';
import type { Limits } from './types';
import {
  buildPublishForm,
  checkUpload,
  compactMetadata,
  derivePublishedPaths,
  detectEntry,
  extensionOf,
  isEntryCandidate,
  mergeSelections,
  type RawUploadItem,
  validatePublishedPath,
} from './upload';

const MB = 1024 * 1024;
const LIMITS: Limits = {
  max_entry_bytes: 16 * MB,
  max_file_bytes: 15 * MB,
  max_upload_bytes: 64 * MB,
  max_files_per_upload: 256,
  max_files_per_version: 512,
  max_version_bytes: 256 * MB,
  user_quota_bytes: 2 * 1024 * MB,
  max_data_bytes: 10 * MB,
  max_data_value_bytes: 256 * 1024,
};

/** A fake upload item; `file` is just the path so assertions stay readable. */
function item(name: string, relativePath = ''): RawUploadItem<string> {
  return { file: relativePath || name, name, relativePath };
}

function paths(items: RawUploadItem<string>[]) {
  return derivePublishedPaths(items).files.map((f) => f.path);
}

describe('derivePublishedPaths', () => {
  it('uses the file name for individually picked files', () => {
    expect(paths([item('index.html'), item('style.css')])).toEqual(['index.html', 'style.css']);
  });

  it('strips the top folder of a folder upload (webkitRelativePath)', () => {
    expect(
      paths([
        item('index.html', 'site/index.html'),
        item('app.js', 'site/assets/app.js'),
        item('logo.png', 'site/assets/img/logo.png'),
      ]),
    ).toEqual(['index.html', 'assets/app.js', 'assets/img/logo.png']);
  });

  it('strips the top folder of a dropped folder (fullPath with a leading slash)', () => {
    expect(
      paths([item('index.html', '/report/index.html'), item('data.json', '/report/data.json')]),
    ).toEqual(['index.html', 'data.json']);
  });

  it('keeps folders when files come from different roots', () => {
    expect(paths([item('index.html', '/index.html'), item('app.js', '/assets/app.js')])).toEqual([
      'index.html',
      'assets/app.js',
    ]);
    expect(paths([item('a.html', 'one/a.html'), item('b.html', 'two/b.html')])).toEqual([
      'one/a.html',
      'two/b.html',
    ]);
  });

  it('only strips one level for a single nested folder', () => {
    expect(paths([item('index.html', 'outer/inner/index.html')])).toEqual(['inner/index.html']);
  });

  it('normalizes backslashes and dot segments', () => {
    expect(paths([item('a.js', 'site\\js\\a.js'), item('b.js', 'site/./js/b.js')])).toEqual([
      'js/a.js',
      'js/b.js',
    ]);
  });

  it('skips hidden files, hidden folders and OS clutter', () => {
    const result = derivePublishedPaths([
      item('index.html', 'site/index.html'),
      item('.DS_Store', 'site/.DS_Store'),
      item('config', 'site/.git/config'),
      item('Thumbs.db', 'site/img/Thumbs.db'),
      item('x.js', 'site/node_modules/x/x.js'),
      item('._index.html', 'site/__MACOSX/._index.html'),
    ]);
    expect(result.files.map((f) => f.path)).toEqual(['index.html']);
    expect(result.ignored).toEqual([
      '.DS_Store',
      '.git/config',
      'img/Thumbs.db',
      'node_modules/x/x.js',
      '__MACOSX/._index.html',
    ]);
  });

  it('handles an empty batch', () => {
    expect(derivePublishedPaths([])).toEqual({ files: [], ignored: [] });
  });
});

describe('mergeSelections', () => {
  it('replaces re-added paths and sorts root files first', () => {
    const merged = mergeSelections(
      [
        { file: 'old', path: 'index.html' },
        { file: 'a', path: 'assets/a.js' },
      ],
      [
        { file: 'new', path: 'index.html' },
        { file: 'b', path: 'b.css' },
      ],
    );
    expect(merged).toEqual([
      { file: 'b', path: 'b.css' },
      { file: 'new', path: 'index.html' },
      { file: 'a', path: 'assets/a.js' },
    ]);
  });
});

describe('validatePublishedPath (mirrors paths::validate_path)', () => {
  it('accepts normal paths', () => {
    for (const p of ['index.html', 'assets/app.js', 'a/b/c/d.png', 'My File (1).html']) {
      expect(validatePublishedPath(p)).toBeNull();
    }
  });

  it('rejects bad paths', () => {
    expect(validatePublishedPath('')).toContain('empty');
    expect(validatePublishedPath('/etc/passwd')).toContain('relative');
    expect(validatePublishedPath('a\\b')).toContain("'/'");
    expect(validatePublishedPath('a//b')).toContain('empty segment');
    expect(validatePublishedPath('a/')).toContain('empty segment');
    expect(validatePublishedPath('../x')).toContain("'..'");
    expect(validatePublishedPath('a/./b')).toContain("'.'");
    expect(validatePublishedPath('a\nb')).toContain('control');
    expect(validatePublishedPath('a'.repeat(513))).toContain('512');
  });

  it('measures length in UTF-8 bytes', () => {
    // 171 three-byte characters = 513 bytes.
    expect(validatePublishedPath('€'.repeat(171))).toContain('512');
    expect(validatePublishedPath('€'.repeat(170))).toBeNull();
  });
});

describe('extensions and entries (mirrors media.rs)', () => {
  it('extracts lowercase extensions', () => {
    expect(extensionOf('a/B.PNG')).toBe('png');
    expect(extensionOf('index.html')).toBe('html');
    expect(extensionOf('Makefile')).toBeNull();
    expect(extensionOf('.html')).toBeNull();
    expect(extensionOf('file.')).toBeNull();
  });

  it('knows entry candidates', () => {
    expect(isEntryCandidate('index.html')).toBe(true);
    expect(isEntryCandidate('page.HTM')).toBe(true);
    expect(isEntryCandidate('README.md')).toBe(true);
    expect(isEntryCandidate('notes.markdown')).toBe(true);
    expect(isEntryCandidate('app.js')).toBe(false);
  });

  it('detects the entry like the server', () => {
    expect(detectEntry(['report.html', 'style.css'])).toBe('report.html');
    expect(detectEntry(['notes.md'])).toBe('notes.md');
    expect(detectEntry(['index.html', 'about.html', 'app.js'])).toBe('index.html');
    expect(detectEntry(['a.html', 'b.html'])).toBeNull();
    expect(detectEntry(['docs/index.html', 'docs/other.html'])).toBeNull();
    expect(detectEntry(['app.js'])).toBeNull();
    expect(detectEntry([])).toBeNull();
  });
});

describe('checkUpload', () => {
  const opts = { limits: LIMITS, entry: 'index.html', requireEntry: true };

  it('passes a normal upload', () => {
    const check = checkUpload(
      [
        { path: 'index.html', size: 1000 },
        { path: 'app.js', size: 2000 },
      ],
      opts,
    );
    expect(check.ok).toBe(true);
    expect(check.totalBytes).toBe(3000);
    expect(check.errors).toEqual([]);
  });

  it('is not ok when nothing is selected', () => {
    expect(checkUpload([], opts).ok).toBe(false);
  });

  it('flags unsupported extensions and bad paths per file', () => {
    const check = checkUpload(
      [
        { path: 'index.html', size: 10 },
        { path: 'run.exe', size: 10 },
        { path: 'Makefile', size: 10 },
      ],
      opts,
    );
    expect(check.ok).toBe(false);
    expect(check.fileErrors['run.exe']).toContain('.exe');
    expect(check.fileErrors.Makefile).toContain('extension');
  });

  it('uses the server allow-list when given', () => {
    const check = checkUpload(
      [
        { path: 'index.html', size: 1 },
        { path: 'x.css', size: 1 },
      ],
      {
        ...opts,
        allowedExtensions: ['html'],
      },
    );
    expect(check.fileErrors['x.css']).toBeDefined();
  });

  it('applies the entry limit to the entry and the file limit to the rest', () => {
    const check = checkUpload(
      [
        { path: 'index.html', size: 15.5 * MB },
        { path: 'big.png', size: 15.5 * MB },
      ],
      opts,
    );
    expect(check.fileErrors['index.html']).toBeUndefined();
    expect(check.fileErrors['big.png']).toContain('15 MB');
    const tooBigEntry = checkUpload([{ path: 'index.html', size: 17 * MB }], opts);
    expect(tooBigEntry.fileErrors['index.html']).toContain('16 MB');
  });

  it('enforces the upload size and file count limits', () => {
    const many = Array.from({ length: 257 }, (_, i) => ({ path: `f${i}.js`, size: 1 }));
    many.push({ path: 'index.html', size: 1 });
    expect(checkUpload(many, opts).errors[0]).toContain('256');

    const huge = Array.from({ length: 5 }, (_, i) => ({ path: `f${i}.png`, size: 14 * MB }));
    huge.push({ path: 'index.html', size: 1 });
    expect(checkUpload(huge, opts).errors[0]).toContain('64 MB');
  });

  it('requires an entry page for creates', () => {
    const none = checkUpload([{ path: 'app.js', size: 1 }], { ...opts, entry: null });
    expect(none.errors[0]).toContain('HTML or Markdown');
    const ambiguous = checkUpload(
      [
        { path: 'a.html', size: 1 },
        { path: 'b.html', size: 1 },
      ],
      { ...opts, entry: null },
    );
    expect(ambiguous.errors[0]).toContain('Choose');
  });

  it('does not require an entry for merge-mode versions', () => {
    const check = checkUpload([{ path: 'app.js', size: 1 }], {
      ...opts,
      entry: null,
      requireEntry: false,
    });
    expect(check.ok).toBe(true);
  });
});

describe('compactMetadata', () => {
  it('drops empty and undefined fields and trims strings', () => {
    expect(
      compactMetadata({
        title: '  Report  ',
        description: '',
        visibility: 'public',
        label: '   ',
        entry: undefined,
      }),
    ).toEqual({ title: 'Report', visibility: 'public' });
    expect(compactMetadata({ mode: 'merge', remove: [], base_version: 3 })).toEqual({
      mode: 'merge',
      base_version: 3,
    });
  });
});

describe('buildPublishForm', () => {
  it('sends metadata first, then one file part per file named by its published path', async () => {
    const form = buildPublishForm({ title: 'Hi', visibility: 'private' }, [
      { file: new Blob(['<h1>hi</h1>'], { type: 'text/html' }), path: 'index.html' },
      { file: new Blob(['body{}'], { type: 'text/css' }), path: 'assets/site.css' },
    ]);
    const entries = [...form.entries()];
    expect(entries.map(([name]) => name)).toEqual(['metadata', 'file', 'file']);
    expect(JSON.parse(entries[0][1] as string)).toEqual({ title: 'Hi', visibility: 'private' });
    const files = form.getAll('file') as File[];
    expect(files.map((f) => f.name)).toEqual(['index.html', 'assets/site.css']);
    expect(await files[1].text()).toBe('body{}');
  });

  it('omits the metadata part when there is nothing to say', () => {
    const form = buildPublishForm({ title: '' }, [{ file: new Blob(['x']), path: 'a.html' }]);
    expect(form.has('metadata')).toBe(false);
    expect(form.getAll('file')).toHaveLength(1);
  });
});
