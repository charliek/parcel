//! `list`, `info`, `cat`, `pull`, `versions`, `open`, `link`.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use parcel_types::paths::validate_path;
use parcel_types::{
    ArchivedFilter, FileEntry, ListMatch, ListScope, PageEntry, ParcelAccess, ParcelDetail,
    ParcelList, ParcelSummary, SharedParcelList, SourceInfo, VersionDetail, VersionList,
    VersionSummary, Visibility,
};
use serde::Serialize;

use super::tags::normalize_each;
use super::{
    api, authed_api, fetch_detail, links, list_scope, open_in_browser, parse_ref, resolve,
    resolve_id, version_segment, Resolved,
};
use crate::cli::{CatArgs, InfoArgs, LinkArgs, ListArgs, OpenArgs, PullArgs, RefArg};
use crate::client::{Api, Fetched};
use crate::config::Settings;
use crate::error::{self, ApiError};
use crate::files::plural;
use crate::output::{self, age, format_size, printable, table, truncate};

/// Versions shown by `info` before pointing at `parcel versions`.
const INFO_VERSIONS: usize = 10;

/// `<base>/api/v1/<path>` with the paging (and search) options that are set.
fn list_url(api: &Api, path: &str, args: &ListArgs) -> reqwest::Url {
    list_url_with(api, path, args, &[])
}

/// [`list_url`] with these filters first (`scope`, repeated `tag`, `archived`).
fn list_url_with(
    api: &Api,
    path: &str,
    args: &ListArgs,
    filters: &[(&str, String)],
) -> reqwest::Url {
    let mut url = api.endpoint([path]);
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in filters {
            query.append_pair(key, value);
        }
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

/// `list`'s tag filters (`--tag`, `--project`, `--kind`), normalized.
fn filter_tags(args: &ListArgs) -> Result<Vec<String>> {
    let mut raw = args.tags.clone();
    if let Some(project) = &args.project {
        raw.push(format!("project:{project}"));
    }
    if let Some(kind) = &args.kind {
        raw.push(format!("kind:{kind}"));
    }
    normalize_each(&raw)
}

pub fn list(settings: &Settings, args: ListArgs) -> Result<()> {
    let tags = filter_tags(&args)?;
    let archived = if args.archived_only {
        ArchivedFilter::Only
    } else if args.archived {
        ArchivedFilter::Include
    } else {
        ArchivedFilter::Exclude
    };
    let scope = list_scope(args.all, args.shared);
    let api = authed_api(settings)?;
    // A plain `--shared` keeps the endpoint (and the JSON) it always had.
    if scope == ListScope::Shared
        && args.query.is_none()
        && tags.is_empty()
        && archived == ArchivedFilter::Exclude
    {
        return list_shared(&api, &args);
    }
    // An older server ignores parameters it doesn't know: it would list
    // everything rather than fail.
    if !tags.is_empty() {
        api.require_feature("tags", "tags")?;
    }
    if archived != ArchivedFilter::Exclude {
        api.require_feature("archive", "archived parcels")?;
    }
    match scope {
        ListScope::Mine => {}
        ListScope::Shared => api.require_feature("search", "searching shared parcels")?,
        ListScope::All => api.require_feature("search", "--all")?,
    }
    let mut filters: Vec<(&str, String)> = Vec::new();
    if scope != ListScope::Mine {
        filters.push(("scope", scope.as_str().into()));
    }
    filters.extend(tags.iter().map(|t| ("tag", t.clone())));
    if archived != ArchivedFilter::Exclude {
        filters.push(("archived", archived.as_str().into()));
    }
    let list = api.get::<ParcelList>(list_url_with(&api, "parcels", &args, &filters))?;
    if args.json.json {
        return output::print_json(&list.raw);
    }
    let ParcelList {
        parcels,
        total,
        relaxed,
    } = list.value;
    if parcels.is_empty() {
        let message = match (&args.query, total) {
            (Some(q), _) => format!("No parcels match {q:?}.\n"),
            (None, _) if !tags.is_empty() || archived == ArchivedFilter::Only => {
                "No parcels match these filters.\n".to_string()
            }
            (None, 0) => "No parcels yet. Publish one with `parcel publish page.html`.\n".into(),
            (None, _) => format!("No parcels on this page ({total} in total).\n"),
        };
        return output::print(&message);
    }
    if relaxed {
        // On stderr, so the table on stdout stays what it is.
        eprintln!("{RELAXED_NOTICE}");
    }
    let with_owner = scope != ListScope::Mine;
    let with_tags = parcels.iter().any(|p| !p.tags.is_empty());
    let with_slug = parcels.iter().any(|p| p.slug.is_some());
    let mut header = vec!["ID", "TITLE"];
    if with_owner {
        header.extend(["OWNER", "ROLE"]);
    } else {
        header.push("VISIBILITY");
    }
    header.push("VERSION");
    if !with_owner {
        header.push("SIZE");
    }
    header.extend(["UPDATED", "COMMENTS"]);
    if with_tags {
        header.push("TAGS");
    }
    if with_slug {
        header.push("SLUG");
    }
    let rows: Vec<Vec<String>> = parcels
        .iter()
        .map(|item| {
            let p = &item.parcel;
            let mut title = cell(&truncate(&p.title, if with_owner { 40 } else { 48 }));
            if p.archived_at.is_some() {
                title.push_str(" (archived)");
            }
            let mut row = vec![p.id.clone(), title];
            if with_owner {
                match &item.shared {
                    Some(shared) => {
                        row.push(cell(&truncate(&shared.owner_name, 24)));
                        row.push(shared.role.as_str().to_string());
                    }
                    None => row.extend(["you".to_string(), "owner".to_string()]),
                }
            } else {
                row.push(p.visibility.as_str().to_string());
            }
            row.push(format!("v{}", p.current_version));
            if !with_owner {
                row.push(format_size(p.total_bytes));
            }
            row.push(age(p.updated_at));
            row.push(open_comments(p.open_comments));
            if with_tags {
                row.push(cell(&truncate(&p.tags.join(","), 40)));
            }
            if with_slug {
                row.push(cell(&truncate(p.slug.as_deref().unwrap_or(""), 40)));
            }
            row
        })
        .collect();
    let rendered = table(&header, &rows);
    let mut lines = rendered.lines();
    let mut out = format!("{}\n", lines.next().unwrap_or_default());
    // Each row, then where the search matched its page text.
    for (line, item) in lines.zip(&parcels) {
        out.push_str(line);
        out.push('\n');
        if let Some(matched) = &item.matched {
            out.push_str(&format!("    {}\n", snippet(matched)));
        }
    }
    out.push_str(&more_hint(parcels.len(), args.offset, total));
    // Titles, names and snippets can be other people's (or page text).
    output::print(&printable(&out))
}

/// Said (on stderr) above a relaxed list: no parcel matched every word.
const RELAXED_NOTICE: &str = "No exact matches; showing parcels that match any of the words.";

/// A table cell from text someone else may have written: control
/// characters spelled out, kept on one line.
fn cell(text: &str) -> String {
    printable(text).replace(['\n', '\t'], " ")
}

/// A search snippet on one line, matched words in `**…**`. Page text is
/// user content, so it only ever goes through [`printable`].
fn snippet(matched: &ListMatch) -> String {
    let text: String = matched
        .segments
        .iter()
        .map(|s| {
            if s.hit {
                format!("**{}**", s.text)
            } else {
                s.text.clone()
            }
        })
        .collect();
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("… {} …", printable(&truncate(&flat, 200)))
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

pub(crate) fn get_detail(api: &Api, id: &str) -> Result<Fetched<ParcelDetail>> {
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
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let version = args.version.or(target.version);
    let id = target.id;
    let detail = fetch_detail(&api, &id, target.detail)?;
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
        pages,
    } = detail.value;
    let mut out = String::new();
    out.push_str(&format!("{}\n", parcel.title));
    let mut field = |name: &str, value: String| {
        out.push_str(&format!("  {:<12} {value}\n", format!("{name}:")));
    };
    field("id", parcel.id.clone());
    field("url", links(&parcel, version, None, None).1);
    if let Some(slug) = &parcel.slug {
        field("slug", slug.clone());
    }
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
    if !parcel.tags.is_empty() {
        field("tags", parcel.tags.join(", "));
    }
    if let Some(at) = parcel.archived_at {
        let undo = if is_owner {
            format!("; `parcel unarchive {}`", parcel.id)
        } else {
            String::new()
        };
        field(
            "archived",
            format!("{} (left out of lists and search{undo})", age(at)),
        );
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
    // The owner's only: the server leaves it out for everyone else.
    let source = match &pinned_detail {
        Some(v) => v.value.summary.source.clone(),
        None => versions
            .iter()
            .find(|v| v.version == parcel.current_version)
            .and_then(|v| v.source.clone()),
    };
    if let Some(source) = source.as_ref().and_then(describe_source) {
        field("source", source);
    }

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

    let (shown_version, shown_files, shown_pages) = match &pinned_detail {
        Some(v) => (v.value.summary.version, &v.value.files, &v.value.pages),
        None => (parcel.current_version, &files, &pages),
    };
    out.push_str(&format!("\nFiles in v{shown_version}:\n"));
    out.push_str(&indent(&files_table(shown_files)));
    match shown_pages.as_slice() {
        [] => {}
        // One page is the common case; a table would be noise.
        [only] => out.push_str(&format!(
            "\nPages: {}\n",
            match only.title.as_deref() {
                Some(title) => format!("{} — {}", only.path, cell(title)),
                None => only.path.clone(),
            }
        )),
        _ => {
            out.push_str(&format!(
                "\nPages in v{shown_version} (open one with `parcel link {} --page PATH`):\n",
                parcel.id
            ));
            out.push_str(&indent(&pages_table(shown_pages)));
        }
    }
    // Someone else's parcel: its title, description and owner are theirs;
    // page titles come from the pages.
    output::print(&printable(&out))
}

/// `github.com/o/r, branch main, commit 0123456789ab, on laptop, via parcel-cli/0.2.0`.
fn describe_source(source: &SourceInfo) -> Option<String> {
    let parts: Vec<String> = [
        source.repo.clone(),
        source.branch.as_ref().map(|b| format!("branch {b}")),
        source.commit.as_ref().map(|c| format!("commit {c}")),
        source.host.as_ref().map(|h| format!("on {h}")),
        source.client.as_ref().map(|c| format!("via {c}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn pages_table(pages: &[PageEntry]) -> String {
    let rows: Vec<Vec<String>> = pages
        .iter()
        .map(|p| {
            vec![
                p.path.clone(),
                cell(p.title.as_deref().unwrap_or("(no title)")),
            ]
        })
        .collect();
    table(&["PAGE", "TITLE"], &rows)
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
    let parcel_ref = parse_ref(&parcel.parcel)?;
    let api = api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
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
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let id = target.id;
    let version = args.version.or(target.version);
    // A URL's ?page= names the file when no PATH is given.
    let (segment, path) = match (&args.path.clone().or(target.page), version) {
        (Some(path), v) => (version_segment(v), path.clone()),
        // The entry of a specific version.
        (None, Some(v)) => {
            let detail = get_version(&api, &id, Some(v))?.value;
            (v.to_string(), detail.summary.entry)
        }
        // The current entry, pinned to the current version number so the two
        // requests can't straddle a publish.
        (None, None) => {
            let detail = fetch_detail(&api, &id, target.detail)?.value;
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
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let id = target.id;
    let version = args.version.or(target.version);
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

/// A parcel's links at a version and page, checked against the parcel.
struct Link {
    parcel: ParcelSummary,
    /// `/p/<id>[/v/<n>][?page=…][#…]`: works for as long as the parcel exists.
    legacy: String,
    /// The clean URL with `?v=`/`page=` when the parcel has a slug, else `legacy`.
    canonical: String,
}

/// The parcel's links for `target`, with `--page` overriding the
/// reference's page. A page that isn't one of the version's is not found.
fn deep_link(api: &Api, target: Resolved, page_flag: Option<String>) -> Result<Link> {
    let Resolved {
        id,
        version,
        page,
        fragment,
        detail,
    } = target;
    if let Some(flag) = &page_flag {
        validate_path(flag).map_err(|e| error::usage(format!("--page {flag:?}: {e}")))?;
    }
    // A fragment belongs to the page it came with.
    let fragment = match (&page_flag, &page) {
        (Some(flag), ref_page) if ref_page.as_ref() != Some(flag) => None,
        _ => fragment,
    };
    let page = page_flag.or(page);
    if page.is_some() {
        api.require_feature("pages", "pages")?;
    }
    let detail = fetch_detail(api, &id, detail)?.value;
    if let Some(page) = &page {
        let shown = version.unwrap_or(detail.parcel.current_version);
        let pages = if shown == detail.parcel.current_version {
            detail.pages.clone()
        } else {
            get_version(api, &id, Some(shown))?.value.pages
        };
        if !pages.iter().any(|p| &p.path == page) {
            let known: Vec<&str> = pages.iter().map(|p| p.path.as_str()).collect();
            return Err(error::not_found(format!(
                "{page:?} is not a page of {id} v{shown}; its pages: {}",
                known.join(", ")
            )));
        }
    }
    let (legacy, canonical) = links(&detail.parcel, version, page, fragment);
    Ok(Link {
        parcel: detail.parcel,
        legacy,
        canonical,
    })
}

pub fn open(settings: &Settings, args: OpenArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let link = deep_link(&api, target, args.page)?;
    if args.json {
        #[derive(Serialize)]
        struct OpenJson<'a> {
            url: &'a str,
        }
        output::print_json(&serde_json::to_vec(&OpenJson {
            url: &link.canonical,
        })?)?;
    } else {
        // The fragment is raw user input, so it is escaped like `link`'s.
        output::print(&printable(&format!("{}\n", link.canonical)))?;
    }
    open_in_browser(&link.canonical);
    Ok(())
}

/// `link --json`.
#[derive(Serialize)]
struct LinkJson {
    url: String,
    canonical_url: String,
    markdown: String,
}

pub fn link(settings: &Settings, args: LinkArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let link = deep_link(&api, target, args.page)?;
    let markdown = format!(
        "[{}]({})",
        markdown_text(&link.parcel.title),
        markdown_url(&link.canonical)
    );
    if args.json {
        return output::print_json(&serde_json::to_vec(&LinkJson {
            url: link.legacy,
            canonical_url: link.canonical,
            markdown,
        })?);
    }
    let line = if args.markdown {
        markdown
    } else {
        link.canonical
    };
    output::print(&printable(&format!("{line}\n")))
}

/// A title as Markdown link text: one line, control characters dropped,
/// and `\ [ ] ( )` escaped so the title can't end the link early or
/// smuggle in a link of its own.
fn markdown_text(title: &str) -> String {
    let flat = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(flat.len());
    for c in flat.chars().filter(|c| !c.is_control()) {
        if matches!(c, '\\' | '[' | ']' | '(' | ')') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A URL as a Markdown link destination: what would end or split it
/// (spaces, parentheses, angle brackets; only a typed `#fragment` can hold
/// them) percent-encoded.
fn markdown_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            ' ' | '(' | ')' | '<' | '>' | '\\' => out.push_str(&format!("%{:02X}", c as u32)),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
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
    fn markdown_links_escape_titles() {
        assert_eq!(
            markdown_text("A [b](c) \\ d\n\u{1b}[31mred"),
            "A \\[b\\]\\(c\\) \\\\ d \\[31mred"
        );
        assert_eq!(
            markdown_url("https://x.test/p/abcdefghijkm#a (b)<c>"),
            "https://x.test/p/abcdefghijkm#a%20%28b%29%3Cc%3E"
        );
    }

    #[test]
    fn snippets_mark_hits_and_stay_printable() {
        let matched = ListMatch {
            segments: vec![
                parcel_types::MatchSegment {
                    text: "the\nrefresh ".into(),
                    hit: false,
                },
                parcel_types::MatchSegment {
                    text: "token".into(),
                    hit: true,
                },
                parcel_types::MatchSegment {
                    text: " \u{1b}]52;c;aGk=\u{7} rotates".into(),
                    hit: false,
                },
            ],
        };
        assert_eq!(
            snippet(&matched),
            "… the refresh **token** \\u{1b}]52;c;aGk=\\u{7} rotates …"
        );
    }

    #[test]
    fn sources_read_as_one_line() {
        let source = SourceInfo {
            repo: Some("github.com/o/r".into()),
            branch: Some("main".into()),
            commit: None,
            host: Some("laptop".into()),
            client: Some("parcel-cli/0.2.0".into()),
        };
        assert_eq!(
            describe_source(&source).unwrap(),
            "github.com/o/r, branch main, on laptop, via parcel-cli/0.2.0"
        );
        assert_eq!(describe_source(&SourceInfo::default()), None);
    }

    #[test]
    fn signed_out_not_found_suggests_login() {
        let err: anyhow::Error = ApiError::from_response(404, b"").into();
        let hinted = signed_out_hint(err, false);
        assert!(error::format_error(&hinted).contains("run `parcel login`"));
        assert_eq!(error::exit_code(&hinted), error::EXIT_NOT_FOUND);
    }
}
