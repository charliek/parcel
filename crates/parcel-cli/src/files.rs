//! Turning `parcel publish` arguments into a validated list of uploads.
//!
//! Everything the server would reject for shape — bad paths, disallowed file
//! types, files or totals over the limits — is caught here, before a byte is
//! sent, using the same rules (`parcel_types::{paths, media}`) the server runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use parcel_types::media::{self, media_type_for};
use parcel_types::paths::validate_path;
use parcel_types::{Limits, PublishMode};

use crate::error;
use crate::output::format_bytes;

/// What the positional PATH of `parcel publish` was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    File,
    Directory,
}

/// One file to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadFile {
    /// Path inside the parcel (`assets/app.js`); also the multipart filename.
    pub published: String,
    pub local: PathBuf,
    pub size: u64,
    pub content_type: &'static str,
}

/// Default for `--mode` when updating: a directory is a whole site (replace),
/// a single file or bare `--file`/`--remove` edits patch the previous version
/// (merge).
pub fn default_mode(source: Option<SourceKind>) -> PublishMode {
    match source {
        Some(SourceKind::Directory) => PublishMode::Replace,
        Some(SourceKind::File) | None => PublishMode::Merge,
    }
}

/// Parse a `--file` value: `published=local`, or just `local` to publish it
/// under its file name.
pub fn parse_file_arg(value: &str) -> std::result::Result<(String, PathBuf), String> {
    match value.split_once('=') {
        Some((published, local)) => {
            if local.is_empty() {
                return Err(format!("{value:?}: missing the local path after '='"));
            }
            Ok((published.to_string(), PathBuf::from(local)))
        }
        None => {
            let local = PathBuf::from(value);
            let name = local
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| {
                    format!("{value:?}: cannot derive a published path; use PUBLISHED=LOCAL")
                })?
                .to_string();
            Ok((name, local))
        }
    }
}

/// Walk a directory for publishing: every regular file, keyed by its path
/// relative to `root` with `/` separators. Skips dotfiles and dot-directories
/// (`.git`, `.env`, `.DS_Store`) and `node_modules`. Symlinks are skipped
/// with a warning rather than followed, so a link can't smuggle a file from
/// outside the folder into a publish.
pub fn walk_dir(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped_name(entry.file_name()));
    for entry in walker {
        let entry = entry.with_context(|| format!("reading {}", root.display()))?;
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walkdir yields paths under its root");
        let published = published_path(relative)?;
        if file_type.is_symlink() {
            eprintln!(
                "warning: skipping symlink {published} (publish its target with --file {published}=<path>)"
            );
            continue;
        }
        if file_type.is_file() {
            out.push((published, entry.into_path()));
        }
    }
    Ok(out)
}

fn is_skipped_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || name == "node_modules"
}

/// `a/b/c.js` from a relative filesystem path, whatever the OS separator.
fn published_path(relative: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(
                part.to_str()
                    .with_context(|| format!("{} is not valid UTF-8", relative.display()))?
                    .to_string(),
            ),
            other => bail!(
                "unexpected path component {other:?} in {}",
                relative.display()
            ),
        }
    }
    Ok(parts.join("/"))
}

/// Gather the files for a publish: PATH (a file or a directory) plus every
/// `--file`. A `--file` overrides a walked file with the same published path;
/// two `--file`s for one path is an error.
pub fn collect(
    path: Option<&Path>,
    extra: &[(String, PathBuf)],
) -> Result<(Vec<UploadFile>, Option<SourceKind>)> {
    let mut chosen: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut kind = None;

    if let Some(path) = path {
        let meta =
            std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
        if meta.is_dir() {
            kind = Some(SourceKind::Directory);
            for (published, local) in walk_dir(path)? {
                chosen.insert(published, local);
            }
            if chosen.is_empty() && extra.is_empty() {
                bail!("{} contains no files to publish", path.display());
            }
        } else if meta.is_file() {
            kind = Some(SourceKind::File);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("{} has no usable file name", path.display()))?;
            chosen.insert(name.to_string(), path.to_path_buf());
        } else {
            bail!("{} is neither a file nor a directory", path.display());
        }
    }

    let mut explicit = std::collections::BTreeSet::new();
    for (published, local) in extra {
        if !explicit.insert(published.clone()) {
            return Err(error::usage(format!(
                "--file gives the published path {published:?} more than once"
            )));
        }
        chosen.insert(published.clone(), local.clone());
    }

    let mut problems = Vec::new();
    let mut files = Vec::new();
    for (published, local) in chosen {
        if let Err(e) = validate_path(&published) {
            problems.push(format!("{published:?}: {e}"));
            continue;
        }
        let Some(media) = media_type_for(&published) else {
            problems.push(format!("{published}: file type not allowed"));
            continue;
        };
        let meta = match std::fs::metadata(&local) {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                problems.push(format!("{}: not a regular file", local.display()));
                continue;
            }
            Err(e) => {
                problems.push(format!("{}: {e}", local.display()));
                continue;
            }
        };
        files.push(UploadFile {
            published,
            local,
            size: meta.len(),
            content_type: media.content_type,
        });
    }
    if !problems.is_empty() {
        let mut message = format!("cannot publish {}:", plural(problems.len(), "file"));
        for p in problems.iter().take(20) {
            message.push_str("\n  ");
            message.push_str(p);
        }
        if problems.len() > 20 {
            message.push_str(&format!("\n  … and {} more", problems.len() - 20));
        }
        if problems
            .iter()
            .any(|p| p.ends_with("file type not allowed"))
        {
            message.push_str(&format!(
                "\nallowed extensions: {}",
                media::allowed_extensions().join(", ")
            ));
        }
        bail!(message);
    }
    Ok((files, kind))
}

/// The entry page this upload most likely has, for the size check: the
/// explicit one, else `index.html`, else the only HTML/Markdown file.
pub fn likely_entry<'a>(files: &'a [UploadFile], explicit: Option<&'a str>) -> Option<&'a str> {
    if explicit.is_some() {
        return explicit;
    }
    if files.iter().any(|f| f.published == "index.html") {
        return Some("index.html");
    }
    let mut candidates = files
        .iter()
        .filter(|f| media::is_entry_candidate(&f.published));
    match (candidates.next(), candidates.next()) {
        (Some(only), None) => Some(&only.published),
        _ => None,
    }
}

/// Reject uploads the server's limits would refuse, listing every problem.
pub fn check_limits(files: &[UploadFile], entry: Option<&str>, limits: &Limits) -> Result<()> {
    let mut problems = Vec::new();
    let max_files = usize::try_from(limits.max_files_per_upload).unwrap_or(usize::MAX);
    if files.len() > max_files {
        problems.push(format!(
            "this publish has {} files; the server accepts at most {} per publish",
            files.len(),
            limits.max_files_per_upload
        ));
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total > as_u64(limits.max_upload_bytes) {
        problems.push(format!(
            "this publish totals {}; the server accepts at most {} per publish",
            format_bytes(total),
            format_bytes(as_u64(limits.max_upload_bytes))
        ));
    }
    for f in files {
        let (limit, what) = if Some(f.published.as_str()) == entry {
            (as_u64(limits.max_entry_bytes), "entry page")
        } else {
            (as_u64(limits.max_file_bytes), "per-file")
        };
        if f.size > limit {
            problems.push(format!(
                "{} is {}; the {what} limit is {}",
                f.published,
                format_bytes(f.size),
                format_bytes(limit)
            ));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!("over the server's limits:\n  {}", problems.join("\n  "))
}

fn as_u64(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

pub fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(root: &Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn limits() -> Limits {
        Limits {
            max_entry_bytes: 100,
            max_file_bytes: 50,
            max_upload_bytes: 120,
            max_files_per_upload: 3,
            max_files_per_version: 10,
            max_version_bytes: 1000,
            user_quota_bytes: 0,
            max_data_bytes: 0,
            max_data_value_bytes: 0,
        }
    }

    #[test]
    fn walks_directories_into_published_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(root, "index.html", b"<h1>hi</h1>");
        touch(root, "assets/app.js", b"1");
        touch(root, "assets/img/logo.png", b"png");
        touch(root, ".env", b"SECRET=1");
        touch(root, ".git/config", b"x");
        touch(root, "assets/.cache/x.js", b"x");
        touch(root, "node_modules/lib/index.js", b"x");

        let walked: Vec<String> = walk_dir(root)
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(
            walked,
            vec!["assets/app.js", "assets/img/logo.png", "index.html"]
        );

        let (files, kind) = collect(Some(root), &[]).unwrap();
        assert_eq!(kind, Some(SourceKind::Directory));
        let index = files.iter().find(|f| f.published == "index.html").unwrap();
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert_eq!(index.size, 11);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        touch(outside.path(), "secret.txt", b"key");
        touch(dir.path(), "index.html", b"x");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("data.txt"),
        )
        .unwrap();
        let walked: Vec<String> = walk_dir(dir.path())
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(walked, vec!["index.html"]);
    }

    #[test]
    fn single_files_publish_under_their_name() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "My Report.html", b"<p>x</p>");
        let (files, kind) = collect(Some(&dir.path().join("My Report.html")), &[]).unwrap();
        assert_eq!(kind, Some(SourceKind::File));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].published, "My Report.html");
    }

    #[test]
    fn extra_files_override_and_validate() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "site/index.html", b"old");
        touch(dir.path(), "other.html", b"new");
        touch(dir.path(), "data.json", b"{}");
        let extra = vec![
            ("index.html".to_string(), dir.path().join("other.html")),
            ("data/d.json".to_string(), dir.path().join("data.json")),
        ];
        let (files, _) = collect(Some(&dir.path().join("site")), &extra).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.published.as_str()).collect();
        assert_eq!(paths, vec!["data/d.json", "index.html"]);
        assert_eq!(files[1].local, dir.path().join("other.html"));

        let dup = vec![
            ("a.html".to_string(), dir.path().join("other.html")),
            ("a.html".to_string(), dir.path().join("other.html")),
        ];
        let err = collect(None, &dup).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
    }

    #[test]
    fn rejects_bad_paths_and_types_together() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "site/index.html", b"x");
        touch(dir.path(), "site/run.exe", b"x");
        touch(dir.path(), "site/Makefile", b"x");
        let extra = vec![(
            "../escape.html".to_string(),
            dir.path().join("site/index.html"),
        )];
        let err = collect(Some(&dir.path().join("site")), &extra)
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("cannot publish 3 files:"), "{err}");
        assert!(err.contains("run.exe: file type not allowed"), "{err}");
        assert!(err.contains("Makefile: file type not allowed"), "{err}");
        assert!(
            err.contains("\"../escape.html\": path contains a '.' or '..' segment"),
            "{err}"
        );
        assert!(err.contains("allowed extensions: html, htm"), "{err}");
    }

    #[test]
    fn parses_file_args() {
        assert_eq!(
            parse_file_arg("assets/a.js=build/a.js").unwrap(),
            ("assets/a.js".to_string(), PathBuf::from("build/a.js"))
        );
        assert_eq!(
            parse_file_arg("build/chart.png").unwrap(),
            ("chart.png".to_string(), PathBuf::from("build/chart.png"))
        );
        assert!(parse_file_arg("a.js=").is_err());
    }

    #[test]
    fn mode_defaults_follow_the_source() {
        assert_eq!(
            default_mode(Some(SourceKind::Directory)),
            PublishMode::Replace
        );
        assert_eq!(default_mode(Some(SourceKind::File)), PublishMode::Merge);
        assert_eq!(default_mode(None), PublishMode::Merge);
    }

    fn file(published: &str, size: u64) -> UploadFile {
        UploadFile {
            published: published.into(),
            local: PathBuf::from(published),
            size,
            content_type: "text/plain",
        }
    }

    #[test]
    fn finds_the_likely_entry() {
        let files = vec![file("a.html", 1), file("index.html", 1)];
        assert_eq!(likely_entry(&files, None), Some("index.html"));
        assert_eq!(likely_entry(&files, Some("a.html")), Some("a.html"));
        let files = vec![file("report.md", 1), file("app.js", 1)];
        assert_eq!(likely_entry(&files, None), Some("report.md"));
        let files = vec![file("a.html", 1), file("b.html", 1)];
        assert_eq!(likely_entry(&files, None), None);
    }

    #[test]
    fn enforces_limits() {
        let ok = vec![file("index.html", 90), file("a.js", 20)];
        check_limits(&ok, Some("index.html"), &limits()).unwrap();

        // The entry gets the larger limit; other files do not.
        let err = check_limits(&ok, None, &limits()).unwrap_err().to_string();
        assert!(
            err.contains("index.html is 90 B; the per-file limit is 50 B"),
            "{err}"
        );

        let too_many = vec![
            file("a.js", 1),
            file("b.js", 1),
            file("c.js", 1),
            file("d.js", 1),
        ];
        let err = check_limits(&too_many, None, &limits())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("4 files; the server accepts at most 3"),
            "{err}"
        );

        let too_big = vec![file("index.html", 100), file("a.js", 30)];
        let err = check_limits(&too_big, Some("index.html"), &limits())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("totals 130 B; the server accepts at most 120 B"),
            "{err}"
        );
    }
}
