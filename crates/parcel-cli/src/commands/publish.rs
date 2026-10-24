//! `parcel publish`.

use anyhow::{bail, Context, Result};
use parcel_types::media::is_entry_candidate;
use parcel_types::paths::validate_path;
use parcel_types::{
    CreateParcelMetadata, NewVersionMetadata, PublishMode, PublishResult, ServerInfo, Visibility,
};
use reqwest::blocking::multipart::{Form, Part};

use super::{authed_api, open_in_browser, parse_ref};
use crate::cli::PublishArgs;
use crate::config::Settings;
use crate::error;
use crate::files::{self, plural, SourceKind, UploadFile};
use crate::output::{self, format_bytes, format_size};

/// Whether the upload creates a parcel or adds a version, with the metadata
/// part to send.
#[derive(Debug, PartialEq)]
enum Target {
    Create(CreateParcelMetadata),
    Update {
        id: String,
        metadata: NewVersionMetadata,
    },
}

pub fn publish(settings: &Settings, args: PublishArgs) -> Result<()> {
    if args.path.is_none() && args.files.is_empty() && args.remove.is_empty() {
        return Err(error::usage(
            "nothing to publish: give a file or directory (or --file / --remove with --parcel)",
        ));
    }
    let (upload, source) = files::collect(args.path.as_deref(), &args.files)?;
    let target = plan(&args, &upload, source)?;

    let api = authed_api(settings)?;
    if !upload.is_empty() {
        let info = api
            .get::<ServerInfo>(api.endpoint(["info"]))
            .context("reading the server's limits")?
            .value;
        let entry = match &target {
            Target::Create(m) => m.entry.as_deref(),
            Target::Update { metadata, .. } => metadata.entry.as_deref(),
        };
        files::check_limits(&upload, files::likely_entry(&upload, entry), &info.limits)?;
        // The server refuses this too, but only after the whole upload.
        if args.visibility.visibility() == Some(Visibility::Public) && !info.allow_public {
            bail!(
                "public links are turned off on {}; publish with --internal (everyone \
                 signed in can view) or keep it private and share it with `parcel share`",
                settings.server_display()
            );
        }
    }

    let total: u64 = upload.iter().map(|f| f.size).sum();
    if !upload.is_empty() {
        eprintln!(
            "Uploading {} ({})…",
            plural(upload.len(), "file"),
            format_bytes(total)
        );
    }
    let (url, metadata_json) = match &target {
        Target::Create(metadata) => (api.endpoint(["parcels"]), serde_json::to_string(metadata)?),
        Target::Update { id, metadata } => (
            api.endpoint(["parcels", id, "versions"]),
            serde_json::to_string(metadata)?,
        ),
    };
    let form = build_form(&metadata_json, &upload)?;
    let result = api.upload::<PublishResult>(url, form)?;

    if args.json.json {
        output::print_json(&result.raw)?;
    } else {
        let PublishResult { parcel, version } = &result.value;
        let verb = if matches!(target, Target::Create(_)) {
            "Published"
        } else {
            "Updated"
        };
        output::print(&format!(
            "{verb} {:?} ({})\n  url:     {}\n  id:      {}\n  version: {} ({}, {})\n",
            parcel.title,
            parcel.visibility.as_str(),
            parcel.url,
            parcel.id,
            version.summary.version,
            plural(
                usize::try_from(version.summary.file_count).unwrap_or(0),
                "file"
            ),
            format_size(version.summary.total_bytes),
        ))?;
    }
    if args.open {
        open_in_browser(&result.value.parcel.url);
    }
    Ok(())
}

/// Decide create vs. update and build the metadata, validating the flag
/// combinations and the entry page before any network call.
fn plan(args: &PublishArgs, upload: &[UploadFile], source: Option<SourceKind>) -> Result<Target> {
    if let Some(entry) = &args.entry {
        validate_path(entry).map_err(|e| error::usage(format!("--entry {entry:?}: {e}")))?;
        if !is_entry_candidate(entry) {
            return Err(error::usage(format!(
                "--entry {entry:?} must be an .html, .htm, .md or .markdown file"
            )));
        }
    }
    let visibility = args.visibility.visibility();

    let Some(parcel_ref) = &args.parcel else {
        if upload.is_empty() {
            return Err(error::usage("a new parcel needs at least one file"));
        }
        let entry = args.entry.clone().or_else(|| match (source, upload) {
            // A single file is its own entry page.
            (Some(SourceKind::File), [only]) if is_entry_candidate(&only.published) => {
                Some(only.published.clone())
            }
            _ => None,
        });
        check_entry_present(upload, entry.as_deref())?;
        return Ok(Target::Create(CreateParcelMetadata {
            title: args.title.clone(),
            description: args.description.clone(),
            visibility,
            entry,
            label: args.label.clone(),
            ..CreateParcelMetadata::default()
        }));
    };

    let (id, _pinned_version) = parse_ref(parcel_ref)?;
    let mode = args
        .mode
        .map(PublishMode::from)
        .unwrap_or_else(|| files::default_mode(source));
    if mode == PublishMode::Replace {
        if !args.remove.is_empty() {
            return Err(error::usage(
                "--remove only applies in merge mode (replace already drops every file not sent); \
                 pass --mode merge",
            ));
        }
        check_entry_present(upload, args.entry.as_deref())?;
    }
    for path in &args.remove {
        validate_path(path).map_err(|e| error::usage(format!("--remove {path:?}: {e}")))?;
    }
    Ok(Target::Update {
        id,
        metadata: NewVersionMetadata {
            mode: Some(mode),
            remove: args.remove.clone(),
            entry: args.entry.clone(),
            label: args.label.clone(),
            title: args.title.clone(),
            description: args.description.clone(),
            visibility,
            base_version: args.base_version,
            ..NewVersionMetadata::default()
        },
    })
}

/// A version made only of the files sent must contain its entry page.
fn check_entry_present(upload: &[UploadFile], entry: Option<&str>) -> Result<()> {
    match entry {
        Some(entry) if !upload.iter().any(|f| f.published == entry) => Err(error::usage(format!(
            "--entry {entry:?} is not one of the files being published"
        ))),
        Some(_) => Ok(()),
        None if upload.iter().any(|f| is_entry_candidate(&f.published)) => Ok(()),
        None => Err(error::usage(
            "no entry page among the files: include an index.html (or another .html/.md \
             file), or pass --entry",
        )),
    }
}

/// `metadata` (JSON text) first, then one `file` part per upload whose
/// filename is the published path and whose type comes from the media table.
fn build_form(metadata_json: &str, upload: &[UploadFile]) -> Result<Form> {
    let mut form = Form::new().part(
        "metadata",
        Part::text(metadata_json.to_string()).mime_str("application/json")?,
    );
    for file in upload {
        let handle = std::fs::File::open(&file.local)
            .with_context(|| format!("opening {}", file.local.display()))?;
        let part = Part::reader_with_length(handle, file.size)
            .file_name(file.published.clone())
            .mime_str(file.content_type)?;
        form = form.part("file", part);
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;
    use parcel_types::Visibility;
    use std::path::PathBuf;

    fn args(argv: &[&str]) -> PublishArgs {
        let mut full = vec!["parcel", "publish"];
        full.extend_from_slice(argv);
        match Cli::try_parse_from(full).unwrap().command {
            Command::Publish(a) => a,
            _ => unreachable!(),
        }
    }

    fn file(published: &str) -> UploadFile {
        UploadFile {
            published: published.into(),
            local: PathBuf::from(published),
            size: 1,
            content_type: "text/html; charset=utf-8",
        }
    }

    #[test]
    fn a_single_file_is_its_own_entry() {
        let a = args(&["report.html", "--title", "Q3", "--public"]);
        let target = plan(&a, &[file("report.html")], Some(SourceKind::File)).unwrap();
        assert_eq!(
            target,
            Target::Create(CreateParcelMetadata {
                title: Some("Q3".into()),
                visibility: Some(Visibility::Public),
                entry: Some("report.html".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn update_mode_defaults() {
        let a = args(&["site", "--parcel", "abcdefghijkm"]);
        let Target::Update { id, metadata } =
            plan(&a, &[file("index.html")], Some(SourceKind::Directory)).unwrap()
        else {
            panic!()
        };
        assert_eq!(id, "abcdefghijkm");
        assert_eq!(metadata.mode, Some(PublishMode::Replace));
        assert_eq!(metadata.entry, None);

        let a = args(&[
            "about.html",
            "--parcel",
            "https://x.test/p/abcdefghijkm/v/2",
        ]);
        let Target::Update { metadata, .. } =
            plan(&a, &[file("about.html")], Some(SourceKind::File)).unwrap()
        else {
            panic!()
        };
        // Merging one page must not silently move the entry to it.
        assert_eq!(metadata.mode, Some(PublishMode::Merge));
        assert_eq!(metadata.entry, None);
    }

    #[test]
    fn rejects_inconsistent_requests() {
        let a = args(&["site", "--parcel", "abcdefghijkm", "--remove", "old.css"]);
        let err = plan(&a, &[file("index.html")], Some(SourceKind::Directory)).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
        assert!(err.to_string().contains("--mode merge"));

        let a = args(&["data.json"]);
        let mut json = file("data.json");
        json.content_type = "application/json";
        let err = plan(&a, &[json], Some(SourceKind::File)).unwrap_err();
        assert!(err.to_string().contains("no entry page"), "{err}");

        let a = args(&["site", "--entry", "app.js"]);
        let err = plan(&a, &[file("index.html")], Some(SourceKind::Directory)).unwrap_err();
        assert!(err.to_string().contains("must be an .html"), "{err}");

        let a = args(&["site", "--entry", "missing.html"]);
        let err = plan(&a, &[file("index.html")], Some(SourceKind::Directory)).unwrap_err();
        assert!(err.to_string().contains("not one of the files"), "{err}");

        let a = args(&["--parcel", "nope", "--file", "x.html"]);
        let err = plan(&a, &[file("x.html")], None).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
    }

    #[test]
    fn remove_only_merges() {
        let a = args(&[
            "--parcel",
            "abcdefghijkm",
            "--remove",
            "old.css",
            "--base-version",
            "4",
        ]);
        let Target::Update { metadata, .. } = plan(&a, &[], None).unwrap() else {
            panic!()
        };
        assert_eq!(metadata.mode, Some(PublishMode::Merge));
        assert_eq!(metadata.remove, vec!["old.css"]);
        assert_eq!(metadata.base_version, Some(4));
    }
}
