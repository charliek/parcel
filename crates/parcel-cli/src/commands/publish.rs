//! `parcel publish`.

use anyhow::{bail, Context, Result};
use parcel_types::ids::ParcelRef;
use parcel_types::media::is_entry_candidate;
use parcel_types::paths::validate_path;
use parcel_types::slugs::normalize_slug;
use parcel_types::tags::MAX_TAGS;
use parcel_types::{
    CreateParcelMetadata, NewVersionMetadata, PublishMode, PublishResult, SourceInfo, Visibility,
};
use reqwest::blocking::multipart::{Form, Part};

use super::tags::{normalize_arg, normalize_list};
use super::{authed_api, open_in_browser, parse_ref, resolve_id, shown_url};
use crate::cli::PublishArgs;
use crate::config::{self, Settings};
use crate::error;
use crate::files::{self, plural, SourceKind, UploadFile};
use crate::git::{self, Repo};
use crate::output::{self, format_bytes, format_size};

/// Whether the upload creates a parcel (or, with a slug, may version the
/// one that has it) or adds a version, with the metadata part to send.
#[derive(Debug, PartialEq)]
enum Target {
    Create(CreateParcelMetadata),
    Update {
        parcel: ParcelRef,
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
    let mut target = plan(&args, &upload, source)?;
    // --project names a new parcel's project; a bad one is a usage error
    // before anything is sent. PARCEL_PROJECT does the same but is ambient:
    // only the flag is a request an older server must refuse (below).
    let project = match &target {
        Target::Create(_) if !args.no_auto_tags => args
            .project
            .clone()
            .or_else(|| config::non_empty_env("PARCEL_PROJECT"))
            .map(|name| normalize_arg(&format!("project:{name}")))
            .transpose()?,
        _ => None,
    };
    let explicit_project = project.is_some() && args.project.is_some();

    let api = authed_api(settings)?;
    let info = api.server_info()?;
    if !upload.is_empty() {
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
    let explicit_tags = match &target {
        Target::Create(m) => !m.tags.is_empty(),
        Target::Update { metadata, .. } => !metadata.add_tags.is_empty(),
    };
    if explicit_tags || explicit_project {
        api.require_feature("tags", "tags")?;
    }
    if matches!(&target, Target::Create(m) if m.slug.is_some()) {
        api.require_feature("slugs", "slugs")?;
    }

    // What the CLI adds by itself goes only to a server that knows it (an
    // older one refuses unknown metadata fields): the project tag on a new
    // parcel, and where every version came from.
    let creating = matches!(target, Target::Create(_));
    let auto = creating && !args.no_auto_tags && api.has_feature("tags")?;
    let with_source = !args.no_source && api.has_feature("slugs")?;
    let repo = if (auto && project.is_none()) || with_source {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| git::probe(&cwd))
    } else {
        None
    };
    let source_info = with_source.then(|| version_source(repo.as_ref())).flatten();
    match &mut target {
        Target::Create(metadata) => {
            if auto {
                let derived = project.or_else(|| {
                    repo.as_ref()
                        .and_then(|r| r.name.as_deref())
                        .and_then(git::project_tag)
                });
                metadata.auto_tags = auto_tags(&metadata.tags, derived);
            }
            metadata.source = source_info;
        }
        Target::Update { metadata, .. } => metadata.source = source_info,
    }

    let (url, metadata_json) = match &target {
        Target::Create(metadata) => (api.endpoint(["parcels"]), serde_json::to_string(metadata)?),
        Target::Update { parcel, metadata } => {
            let id = resolve_id(&api, parcel.clone())?;
            (
                api.endpoint(["parcels", &id, "versions"]),
                serde_json::to_string(metadata)?,
            )
        }
    };
    let total: u64 = upload.iter().map(|f| f.size).sum();
    if !upload.is_empty() {
        eprintln!(
            "Uploading {} ({})…",
            plural(upload.len(), "file"),
            format_bytes(total)
        );
    }
    let form = build_form(&metadata_json, &upload)?;
    let result = api.upload::<PublishResult>(url, form)?;

    if args.json.json {
        output::print_json(&result.raw)?;
    } else {
        let PublishResult {
            parcel,
            version,
            created,
            ..
        } = &result.value;
        // `created` only matters (and is only sent by a server that knows
        // slugs) when a slug was sent: a plain create always creates.
        let created = match &target {
            Target::Create(m) => m.slug.is_none() || *created,
            Target::Update { .. } => false,
        };
        let verb = if created { "Published" } else { "Updated" };
        let mut out = format!(
            "{verb} {:?} ({})\n  url:     {}\n  id:      {}\n  version: {} ({}, {})\n",
            parcel.title,
            parcel.visibility.as_str(),
            shown_url(parcel),
            parcel.id,
            version.summary.version,
            plural(
                usize::try_from(version.summary.file_count).unwrap_or(0),
                "file"
            ),
            format_size(version.summary.total_bytes),
        );
        if !parcel.tags.is_empty() {
            out.push_str(&format!("  tags:    {}\n", parcel.tags.join(", ")));
        }
        output::print(&out)?;
    }
    if args.open {
        open_in_browser(shown_url(&result.value.parcel));
    }
    Ok(())
}

/// The automatic tags to send beside the explicit ones. Explicit tags win
/// the 20-tag cap: when the two together would pass it, the automatic ones
/// are dropped (and one already given explicitly isn't sent twice).
fn auto_tags(explicit: &[String], derived: Option<String>) -> Vec<String> {
    let auto: Vec<String> = derived
        .into_iter()
        .filter(|t| !explicit.contains(t))
        .collect();
    if explicit.len() + auto.len() > MAX_TAGS {
        Vec::new()
    } else {
        auto
    }
}

/// Where this version comes from: the repository (sanitized: a credential
/// in the remote URL never leaves the machine), branch and commit when
/// there is one, plus the machine and this client. Fields the server would
/// refuse are dropped rather than failing the publish.
fn version_source(repo: Option<&Repo>) -> Option<SourceInfo> {
    let host = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    SourceInfo {
        repo: repo.and_then(|r| r.origin.clone()),
        branch: repo.and_then(|r| r.branch.clone()),
        commit: repo.and_then(|r| r.commit.clone()),
        host: (!host.is_empty()).then_some(host),
        client: Some(concat!("parcel-cli/", env!("CARGO_PKG_VERSION")).to_string()),
    }
    .sanitized()
}

/// Decide create vs. update and build the metadata, validating the flag
/// combinations, tags, slug and entry page before any network call.
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
    let mut tags = args.tags.clone();
    if let Some(kind) = &args.kind {
        tags.push(format!("kind:{kind}"));
    }
    let tags = normalize_list(&tags)?;

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
        let slug = match &args.slug {
            Some(slug) => Some(normalize_slug(slug).map_err(|e| error::usage(e.to_string()))?),
            None => None,
        };
        return Ok(Target::Create(CreateParcelMetadata {
            title: args.title.clone(),
            description: args.description.clone(),
            visibility,
            entry,
            label: args.label.clone(),
            tags,
            slug,
            // clap only takes --base-version with --parcel or --slug.
            base_version: args.base_version,
            ..CreateParcelMetadata::default()
        }));
    };

    let parcel = parse_ref(parcel_ref)?;
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
        parcel,
        metadata: NewVersionMetadata {
            mode: Some(mode),
            remove: args.remove.clone(),
            entry: args.entry.clone(),
            label: args.label.clone(),
            title: args.title.clone(),
            description: args.description.clone(),
            visibility,
            base_version: args.base_version,
            add_tags: tags,
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
    use parcel_types::ids::RefTarget;
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
        let Target::Update { parcel, metadata } =
            plan(&a, &[file("index.html")], Some(SourceKind::Directory)).unwrap()
        else {
            panic!()
        };
        assert_eq!(parcel.target, RefTarget::Id("abcdefghijkm".into()));
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

        // A bare word is a slug now; a near-id is no reference at all.
        let a = args(&["--parcel", "abcdefghijk", "--file", "x.html"]);
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

    #[test]
    fn tags_kind_and_slug_are_normalized_up_front() {
        let a = args(&[
            "plan.md",
            "--tag",
            "Auth",
            "--tag",
            "auth",
            "--kind",
            "Plan",
            "--slug",
            " Parcel/Plan-1 ",
            "--base-version",
            "3",
        ]);
        let Target::Create(m) = plan(&a, &[file("plan.md")], Some(SourceKind::File)).unwrap()
        else {
            panic!()
        };
        assert_eq!(m.tags, ["auth", "kind:plan"]);
        assert_eq!(m.slug.as_deref(), Some("parcel/plan-1"));
        assert_eq!(m.base_version, Some(3));
        assert!(m.auto_tags.is_empty() && m.source.is_none(), "added later");

        // On a new version, --tag/--kind are added to the parcel's tags.
        let a = args(&["x.html", "--parcel", "@ada/plan", "--kind", "report"]);
        let Target::Update { parcel, metadata } =
            plan(&a, &[file("x.html")], Some(SourceKind::File)).unwrap()
        else {
            panic!()
        };
        assert!(matches!(parcel.target, RefTarget::Slug { .. }));
        assert_eq!(metadata.add_tags, ["kind:report"]);

        for bad in [
            &["x.html", "--tag", "no way!"][..],
            &["x.html", "--slug", "abcdefghijkm"],
            &["x.html", "--kind", "a:b"],
        ] {
            let err = plan(&args(bad), &[file("x.html")], Some(SourceKind::File)).unwrap_err();
            assert_eq!(error::exit_code(&err), error::EXIT_USAGE, "{bad:?}");
        }
        let many: Vec<String> = (0..21)
            .flat_map(|i| ["--tag".into(), format!("t{i}")])
            .collect();
        let mut argv = vec!["x.html"];
        argv.extend(many.iter().map(String::as_str));
        let err = plan(&args(&argv), &[file("x.html")], Some(SourceKind::File)).unwrap_err();
        assert!(err.to_string().contains("at most 20 tags"), "{err}");
    }

    #[test]
    fn slug_flags_conflict_where_they_must() {
        for bad in [
            &[
                "parcel",
                "publish",
                "x.html",
                "--slug",
                "s",
                "--parcel",
                "abcdefghijkm",
            ][..],
            &[
                "parcel", "publish", "x.html", "--slug", "s", "--mode", "merge",
            ],
            &[
                "parcel", "publish", "x.html", "--slug", "s", "--remove", "a.css",
            ],
            &["parcel", "publish", "x.html", "--base-version", "2"],
        ] {
            assert_eq!(
                Cli::try_parse_from(bad).unwrap_err().exit_code(),
                2,
                "{bad:?}"
            );
        }
        assert!(Cli::try_parse_from([
            "parcel",
            "publish",
            "x.html",
            "--slug",
            "s",
            "--base-version",
            "2"
        ])
        .is_ok());
    }

    #[test]
    fn auto_tags_give_way_to_explicit_ones() {
        let project = || Some("project:parcel".to_string());
        assert_eq!(auto_tags(&[], project()), ["project:parcel"]);
        assert!(auto_tags(&[], None).is_empty());
        // Already explicit: not sent twice.
        assert!(auto_tags(&["project:parcel".into()], project()).is_empty());
        let nineteen: Vec<String> = (0..19).map(|i| format!("t{i}")).collect();
        assert_eq!(auto_tags(&nineteen, project()), ["project:parcel"]);
        let twenty: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
        assert!(
            auto_tags(&twenty, project()).is_empty(),
            "explicit tags win the cap"
        );
    }

    #[test]
    fn sources_are_sanitized_before_they_are_sent() {
        let repo = Repo {
            name: Some("r".into()),
            origin: Some("https://ada:ghp_CANARY@github.com/o/r.git".into()),
            branch: Some("main".into()),
            commit: Some("0123456789ab".into()),
        };
        let sent = version_source(Some(&repo)).unwrap();
        assert_eq!(sent.repo.as_deref(), Some("https://github.com/o/r"));
        assert_eq!(sent.branch.as_deref(), Some("main"));
        assert!(sent.client.as_deref().unwrap().starts_with("parcel-cli/"));
        assert!(!serde_json::to_string(&sent).unwrap().contains("CANARY"));

        let refused = Repo {
            origin: Some("git://host/o/r".into()),
            ..repo
        };
        assert_eq!(version_source(Some(&refused)).unwrap().repo, None);
        let bare = version_source(None).unwrap();
        assert!(bare.repo.is_none() && bare.branch.is_none() && bare.client.is_some());
    }
}
