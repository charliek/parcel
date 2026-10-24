//! `list`, `info`, `cat`, `pull`, `versions`, `open`.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use parcel_types::paths::validate_path;
use parcel_types::{
    FileEntry, ParcelAccess, ParcelDetail, ParcelList, SharedParcelList, VersionDetail,
    VersionList, VersionSummary, Visibility,
};

use super::{api, authed_api, open_in_browser, parse_ref, version_segment};
use crate::cli::{CatArgs, InfoArgs, ListArgs, PullArgs, RefArg};
use crate::client::{Api, Fetched};
use crate::config::Settings;
use crate::error::{self, ApiError};
use crate::files::plural;
use crate::output::{self, age, format_size, printable, table, truncate};

/// Versions shown by `info` before pointing at `parcel versions`.
const INFO_VERSIONS: usize = 10;

/// `<base>/api/v1/<path>` with the paging (and search) options that are set.
fn list_url(api: &Api, path: &str, args: &ListArgs) -> reqwest::Url {
    let mut url = api.endpoint([path]);
    {
        let mut query = url.query_pairs_mut();
        if let Some(q) = &args.query {
            query.append_pair("q", q);
        }
        if let Some(limit) = args.limit {
            query.append_pair("limit", &limit.to_string());
        }
        if let Some(offset) = args.offset {
            query.append_pair("offset", &offset.to_string());
        }
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    url
}

/// `Showing 3–4 of 9; …` when the page doesn't hold everything.
fn more_hint(shown: usize, offset: Option<u32>, total: i64) -> String {
    let shown = i64::try_from(shown).unwrap_or(i64::MAX);
    let skipped = i64::from(offset.unwrap_or(0));
    if total > shown + skipped {
        format!(
            "Showing {}–{} of {total}; use --offset/--limit for more.\n",
            skipped + 1,
            skipped + shown
        )
    } else {
        String::new()
    }
}

pub fn list(settings: &Settings, args: ListArgs) -> Result<()> {
    let api = authed_api(settings)?;
    if args.shared {
        return list_shared(&api, &args);
    }
    let list = api.get::<ParcelList>(list_url(&api, "parcels", &args))?;
    if args.json.json {
        return output::print_json(&list.raw);
    }
    let ParcelList { parcels, total } = list.value;
    if parcels.is_empty() {
        let message = match (&args.query, total) {
            (Some(q), _) => format!("No parcels match {q:?}.\n"),
            (None, 0) => "No parcels yet. Publish one with `parcel publish page.html`.\n".into(),
            (None, _) => format!("No parcels on this page ({total} in total).\n"),
        };
        return output::print(&message);
    }
    let rows: Vec<Vec<String>> = parcels
        .iter()
        .map(|p| {
            vec![
                p.id.clone(),
                truncate(&p.title, 48),
                p.visibility.as_str().to_string(),
                format!("v{}", p.current_version),
                format_size(p.total_bytes),
                age(p.updated_at),
                open_comments(p.open_comments),
            ]
        })
        .collect();
    let mut out = table(
        &[
            "ID",
            "TITLE",
            "VISIBILITY",
            "VERSION",
            "SIZE",
            "UPDATED",
            "COMMENTS",
        ],
        &rows,
    );
    out.push_str(&more_hint(parcels.len(), args.offset, total));
    output::print(&out)
}

/// `list --shared`: parcels other people shared with the caller by email.
fn list_shared(api: &Api, args: &ListArgs) -> Result<()> {
    let list = api.get::<SharedParcelList>(list_url(api, "shared", args))?;
    if args.json.json {
        return output::print_json(&list.raw);
    }
    let SharedParcelList { parcels, total } = list.value;
    if parcels.is_empty() {
        let message = if total == 0 {
            "Nothing has been shared with you yet.\n".to_string()
        } else {
            format!("No parcels on this page ({total} in total).\n")
        };
        return output::print(&message);
    }
    let rows: Vec<Vec<String>> = parcels
        .iter()
        .map(|s| {
            let p = &s.parcel;
            vec![
                p.id.clone(),
                truncate(&p.title, 40),
                truncate(&s.owner_name, 24),
                s.role.as_str().to_string(),
                format!("v{}", p.current_version),
                age(p.updated_at),
                open_comments(p.open_comments),
            ]
        })
        .collect();
    let mut out = table(
        &[
            "ID", "TITLE", "OWNER", "ROLE", "VERSION", "UPDATED", "COMMENTS",
        ],
        &rows,
    );
    out.push_str(&more_hint(parcels.len(), args.offset, total));
    // Titles and names are other people's.
    output::print(&printable(&out))
}

/// `2 open`, or blank when there are none (so the column stays quiet).
fn open_comments(n: i64) -> String {
    if n > 0 {
        format!("{n} open")
    } else {
        String::new()
    }
}

fn get_detail(api: &Api, id: &str) -> Result<Fetched<ParcelDetail>> {
    api.get::<ParcelDetail>(api.endpoint(["parcels", id]))
        .map_err(|e| signed_out_hint(e, api.has_token()))
}

fn get_version(api: &Api, id: &str, version: Option<i32>) -> Result<Fetched<VersionDetail>> {
    let segment = version_segment(version);
    api.get::<VersionDetail>(api.endpoint(["parcels", id, "versions", &segment]))
        .map_err(|e| signed_out_hint(e, api.has_token()))
}

/// A 404 while signed out is usually a private parcel; say so.
pub(super) fn signed_out_hint(err: anyhow::Error, has_token: bool) -> anyhow::Error {
    match err.downcast_ref::<ApiError>() {
        Some(api) if !has_token && api.status == 404 => ApiError {
            status: 404,
            message: format!(
                "{} (you are not logged in; if the parcel is private, run `parcel login`)",
                api.message
            ),
        }
        .into(),
        _ => err,
    }
}

pub fn info(settings: &Settings, args: InfoArgs) -> Result<()> {
    let (id, pinned) = parse_ref(&args.parcel.parcel)?;
    let version = args.version.or(pinned);
    let api = api(settings)?;
    let detail = get_detail(&api, &id)?;
    let pinned_detail = match version {
        Some(v) => Some(get_version(&api, &id, Some(v))?),
        None => None,
    };
    if args.json.json {
        // A pinned version prints that version (VersionDetail); otherwise the
        // parcel (ParcelDetail).
        return match &pinned_detail {
            Some(v) => output::print_json(&v.raw),
            None => output::print_json(&detail.raw),
        };
    }

    let ParcelDetail {
        parcel,
        is_owner,
        access,
        owner_name,
        versions,
        files,
        pages: _,
    } = detail.value;
    let mut out = String::new();
    out.push_str(&format!("{}\n", parcel.title));
    let mut field = |name: &str, value: String| {
        out.push_str(&format!("  {:<12} {value}\n", format!("{name}:")));
    };
    field("id", parcel.id.clone());
    let url = match version {
        Some(v) => format!("{}/v/{v}", parcel.url),
        None => parcel.url.clone(),
    };
    field("url", url);
    field("visibility", visibility_note(parcel.visibility).to_string());
    if !is_owner {
        field(
            "owner",
            owner_name.unwrap_or_else(|| "someone else".to_string()),
        );
        field("access", access_note(access).to_string());
    }
    if !parcel.description.trim().is_empty() {
        field("description", parcel.description.trim().to_string());
    }
    field("version", format!("v{}", parcel.current_version));
    field("entry", parcel.entry.clone());
    field(
        "size",
        format!(
            "{} ({})",
            format_size(parcel.total_bytes),
            plural(usize::try_from(parcel.file_count).unwrap_or(0), "file")
        ),
    );
    field("created", age(parcel.created_at));
    field("updated", age(parcel.updated_at));
    let comments = match parcel.open_comments {
        0 => "none open".to_string(),
        n => format!("{n} open; see `parcel comments list {}`", parcel.id),
    };
    field("comments", comments);

    out.push_str("\nVersions (newest first):\n");
    out.push_str(&indent(&versions_table(
        &versions[..versions.len().min(INFO_VERSIONS)],
        parcel.current_version,
    )));
    if versions.len() > INFO_VERSIONS {
        out.push_str(&format!(
            "  … {} older; see `parcel versions {}`\n",
            versions.len() - INFO_VERSIONS,
            parcel.id
        ));
    }

    let (shown_version, shown_files) = match &pinned_detail {
        Some(v) => (v.value.summary.version, &v.value.files),
        None => (parcel.current_version, &files),
    };
    out.push_str(&format!("\nFiles in v{shown_version}:\n"));
    out.push_str(&indent(&files_table(shown_files)));
    // Someone else's parcel: its title, description and owner are theirs.
    output::print(&printable(&out))
}

/// The visibility, and whom it reaches.
fn visibility_note(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private (the owner and the people it's shared with)",
        Visibility::Internal => "internal (anyone signed in to this server)",
        Visibility::Public => "public (anyone with the link)",
    }
}

/// What the caller may do with someone else's parcel.
fn access_note(access: ParcelAccess) -> &'static str {
    match access {
        ParcelAccess::Owner => "owner",
        ParcelAccess::Commenter => "commenter (you can view and comment)",
        ParcelAccess::Viewer => "viewer (you can view and read comments, not comment)",
        ParcelAccess::Public => "public link (view only; sign in to see or add comments)",
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|l| format!("  {l}\n")).collect()
}

fn versions_table(versions: &[VersionSummary], current: i32) -> String {
    let rows: Vec<Vec<String>> = versions
        .iter()
        .map(|v| {
            let mut label = v.label.clone().unwrap_or_default();
            if v.version == current {
                label = if label.is_empty() {
                    "(current)".into()
                } else {
                    format!("{label} (current)")
                };
            }
            vec![
                format!("v{}", v.version),
                age(v.created_at),
                v.file_count.to_string(),
                format_size(v.total_bytes),
                v.entry.clone(),
                label,
            ]
        })
        .collect();
    table(
        &["VERSION", "CREATED", "FILES", "SIZE", "ENTRY", "LABEL"],
        &rows,
    )
}

fn files_table(files: &[FileEntry]) -> String {
    let rows: Vec<Vec<String>> = files
        .iter()
        .map(|f| {
            let kind = f
                .content_type
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            vec![f.path.clone(), format_size(f.size_bytes), kind]
        })
        .collect();
    table(&["PATH", "SIZE", "TYPE"], &rows)
}

pub fn versions(settings: &Settings, parcel: &RefArg, json: bool) -> Result<()> {
    let (id, _) = parse_ref(&parcel.parcel)?;
    let api = api(settings)?;
    let list = api
        .get::<VersionList>(api.endpoint(["parcels", &id, "versions"]))
        .map_err(|e| signed_out_hint(e, api.has_token()))?;
    if json {
        return output::print_json(&list.raw);
    }
    // Versions are immutable and restore appends, so the newest is current.
    let current = list.value.versions.first().map_or(0, |v| v.version);
    output::print(&versions_table(&list.value.versions, current))
}

/// The URL for a parcel file, each path segment encoded separately.
fn file_url(api: &Api, id: &str, version: &str, path: &str) -> reqwest::Url {
    api.endpoint(
        ["parcels", id, "versions", version, "files"]
            .into_iter()
            .chain(path.split('/')),
    )
}

pub fn cat(settings: &Settings, args: CatArgs) -> Result<()> {
    let (id, pinned) = parse_ref(&args.parcel.parcel)?;
    let version = args.version.or(pinned);
    let api = api(settings)?;
    let (segment, path) = match (&args.path, version) {
        (Some(path), v) => (version_segment(v), path.clone()),
        // The entry of a specific version.
        (None, Some(v)) => {
            let detail = get_version(&api, &id, Some(v))?.value;
            (v.to_string(), detail.summary.entry)
        }
        // The current entry, pinned to the current version number so the two
        // requests can't straddle a publish.
        (None, None) => {
            let detail = get_detail(&api, &id)?.value;
            (
                detail.parcel.current_version.to_string(),
                detail.parcel.entry,
            )
        }
    };
    validate_path(&path).map_err(|e| error::usage(format!("{path:?}: {e}")))?;
    let mut response = api
        .download(file_url(&api, &id, &segment, &path))
        .map_err(|e| signed_out_hint(e, api.has_token()))?;
    let mut stdout = std::io::stdout().lock();
    // A closed stdout surfaces as an io::Error in the chain, which main ignores.
    std::io::copy(&mut response, &mut stdout).with_context(|| format!("downloading {path}"))?;
    stdout.flush()?;
    Ok(())
}

/// Where a published path lands under `dir`, refusing anything that could
/// escape it (the server validates paths too; this does not trust it).
fn local_target(dir: &Path, published: &str) -> Result<PathBuf> {
    validate_path(published)
        .map_err(|e| anyhow::anyhow!("server sent an unsafe path {published:?}: {e}"))?;
    let relative = Path::new(published);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("server sent an unsafe path {published:?}");
    }
    Ok(dir.join(relative))
}

/// Create `rel` under `base` one component at a time, refusing to traverse a
/// symlink: `create_dir_all` would follow a planted `assets -> /elsewhere`
/// link and write outside the destination.
fn create_dirs_inside(base: &Path, rel: &Path) -> Result<()> {
    let mut current = base.to_path_buf();
    for component in rel.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => bail!(
                "{} is a symlink; refusing to write through it",
                current.display()
            ),
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => bail!("{} exists and is not a directory", current.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .with_context(|| format!("creating {}", current.display()))?;
            }
            Err(e) => return Err(e).with_context(|| format!("checking {}", current.display())),
        }
    }
    Ok(())
}

pub fn pull(settings: &Settings, args: PullArgs) -> Result<()> {
    let (id, pinned) = parse_ref(&args.parcel.parcel)?;
    let version = args.version.or(pinned);
    let api = api(settings)?;
    let detail = get_version(&api, &id, version)?.value;
    let dir = args.dir.clone().unwrap_or_else(|| PathBuf::from(&id));
    if dir.exists() && !dir.is_dir() {
        bail!("{} exists and is not a directory", dir.display());
    }

    let mut targets = Vec::new();
    let mut conflicts = Vec::new();
    for file in &detail.files {
        let target = local_target(&dir, &file.path)?;
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!(
                    "{} is a symlink; refusing to write through it",
                    target.display()
                )
            }
            Ok(meta) if meta.is_dir() => bail!("{} is a directory", target.display()),
            Ok(_) if !args.force => conflicts.push(target.display().to_string()),
            _ => {}
        }
        targets.push((file, target));
    }
    if !conflicts.is_empty() {
        let shown: Vec<String> = conflicts
            .iter()
            .take(10)
            .map(|c| format!("  {c}"))
            .collect();
        let more = if conflicts.len() > 10 {
            format!("\n  … and {} more", conflicts.len() - 10)
        } else {
            String::new()
        };
        bail!(
            "refusing to overwrite existing files (pass --force):\n{}{more}",
            shown.join("\n")
        );
    }

    let segment = detail.summary.version.to_string();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for (n, (file, target)) in targets.iter().enumerate() {
        if let Some(parent) = Path::new(&file.path).parent() {
            create_dirs_inside(&dir, parent)?;
        }
        let mut response = api.download(file_url(&api, &id, &segment, &file.path))?;
        // Unique and created exclusively (O_EXCL): a planted file or symlink
        // at a predictable temp name can never be followed or truncated.
        let partial = target.with_file_name(format!(
            ".{}.{}-{n}.parcel-partial",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file"),
            std::process::id()
        ));
        let written = (|| -> Result<u64> {
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)?;
            let n = std::io::copy(&mut response, &mut out)?;
            out.sync_all()?;
            Ok(n)
        })()
        .with_context(|| format!("downloading {}", file.path));
        let written = match written {
            Ok(n) => n,
            Err(e) => {
                let _ = fs::remove_file(&partial);
                return Err(e);
            }
        };
        if i64::try_from(written).ok() != Some(file.size_bytes) {
            let _ = fs::remove_file(&partial);
            bail!(
                "{}: expected {} bytes, received {written}",
                file.path,
                file.size_bytes
            );
        }
        fs::rename(&partial, target).with_context(|| format!("writing {}", target.display()))?;
    }
    output::print(&format!(
        "Pulled {} ({}) of {id} v{} into {}\n",
        plural(targets.len(), "file"),
        format_size(detail.summary.total_bytes),
        detail.summary.version,
        dir.display()
    ))
}

pub fn open(settings: &Settings, parcel: &RefArg) -> Result<()> {
    let (id, pinned) = parse_ref(&parcel.parcel)?;
    let api = api(settings)?;
    let detail = get_detail(&api, &id)?.value;
    let url = match pinned {
        Some(v) => format!("{}/v/{v}", detail.parcel.url),
        None => detail.parcel.url,
    };
    output::print(&format!("{url}\n"))?;
    open_in_browser(&url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_targets_stay_inside_the_directory() {
        let dir = Path::new("out");
        assert_eq!(
            local_target(dir, "assets/app.js").unwrap(),
            Path::new("out/assets/app.js")
        );
        for bad in ["../x", "/etc/passwd", "a/../../x", "a\\..\\x", ""] {
            assert!(local_target(dir, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn signed_out_not_found_suggests_login() {
        let err: anyhow::Error = ApiError::from_response(404, b"").into();
        let hinted = signed_out_hint(err, false);
        assert!(error::format_error(&hinted).contains("run `parcel login`"));
        assert_eq!(error::exit_code(&hinted), error::EXIT_NOT_FOUND);
    }
}
