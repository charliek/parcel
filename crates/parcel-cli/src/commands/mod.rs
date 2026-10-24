//! Command implementations. Results go to stdout; progress, prompts and
//! warnings to stderr; errors are returned and printed once by `main`.

mod auth;
mod comments;
mod data;
mod local;
mod manage;
mod publish;
mod read;
mod sharing;
mod tags;

use std::io::Read;

use anyhow::{Context, Result};
use parcel_types::ids::{ParcelRef, RefTarget};
use parcel_types::{ParcelDetail, ParcelSummary};

use crate::cli::{Cli, Command};
use crate::client::{Api, Fetched};
use crate::config::{self, ConfigStore, Settings};
use crate::error;

pub fn run(cli: Cli) -> Result<()> {
    let Cli {
        server,
        profile,
        command,
    } = cli;
    match command {
        // Never touches the config or the network, so a broken config.toml
        // can't stop an agent from installing the skill.
        Command::Skill(cmd) => local::skill(cmd),
        command => {
            let settings = load_settings(server.as_deref(), &profile)?;
            dispatch(settings, command)
        }
    }
}

fn load_settings(server: Option<&str>, profile: &str) -> Result<Settings> {
    let store = ConfigStore::new(&config::config_dir()?);
    Settings::resolve(
        server,
        profile,
        config::non_empty_env("PARCEL_TOKEN"),
        store,
    )
}

fn dispatch(settings: Settings, command: Command) -> Result<()> {
    match command {
        Command::Login(args) => auth::login(settings, args),
        Command::Logout => auth::logout(settings),
        Command::Whoami(args) => auth::whoami(&settings, args.json),
        Command::Publish(args) => publish::publish(&settings, args),
        Command::List(args) => read::list(&settings, args),
        Command::Info(args) => read::info(&settings, args),
        Command::Cat(args) => read::cat(&settings, args),
        Command::Pull(args) => read::pull(&settings, args),
        Command::Versions(args) => read::versions(&settings, &args.parcel, args.json.json),
        Command::Open(args) => read::open(&settings, args),
        Command::Link(args) => read::link(&settings, args),
        Command::Update(args) => manage::update(&settings, args),
        Command::Tag(args) => manage::tag(&settings, args),
        Command::Archive(args) => manage::archive(&settings, &args, true),
        Command::Unarchive(args) => manage::archive(&settings, &args, false),
        Command::Tags(args) => tags::run(&settings, args),
        Command::Handle(args) => auth::handle(&settings, args),
        Command::Restore(args) => manage::restore(&settings, args),
        Command::Delete(args) => manage::delete(&settings, args),
        Command::Share(args) => sharing::share(&settings, args),
        Command::Unshare(args) => sharing::unshare(&settings, args),
        Command::Shares(args) => sharing::shares(&settings, &args.parcel, args.json.json),
        Command::Comments(cmd) => comments::run(&settings, cmd),
        Command::Data(cmd) => data::run(&settings, cmd),
        Command::Config(cmd) => local::config(settings, cmd),
        Command::Skill(_) => unreachable!("handled before loading settings"),
    }
}

/// A client that sends the token when there is one (reads of public parcels
/// work signed out).
fn api(settings: &Settings) -> Result<Api> {
    Api::new(&settings.server, settings.token.as_deref())
}

/// A client for calls that need a signed-in user; exit 3 without a token.
fn authed_api(settings: &Settings) -> Result<Api> {
    let token = settings.require_token()?;
    Api::new(&settings.server, Some(token))
}

/// A REF argument, checked before any request (so a typo is a usage error
/// even when signed out).
pub fn parse_ref(input: &str) -> Result<ParcelRef> {
    ParcelRef::parse(input).ok_or_else(|| {
        error::usage(format!(
            "{input:?} is not a parcel id or URL (expected a 12-character id, …/p/<id>, \
             @<handle>/<slug> or one of your slugs)"
        ))
    })
}

/// A reference resolved to a parcel id, with what it pinned.
pub struct Resolved {
    pub id: String,
    pub version: Option<i32>,
    pub page: Option<String>,
    pub fragment: Option<String>,
    /// The parcel, when resolving had to fetch it (a slug reference), so
    /// commands that need it anyway don't ask twice.
    pub detail: Option<Fetched<ParcelDetail>>,
}

/// An id needs no request; a slug costs one `by-slug` lookup, after which
/// everything proceeds by id.
pub fn resolve(api: &Api, parcel_ref: ParcelRef) -> Result<Resolved> {
    let ParcelRef {
        target,
        version,
        page,
        fragment,
    } = parcel_ref;
    let (id, detail) = match target {
        RefTarget::Id(id) => (id, None),
        RefTarget::Slug { handle, slug } => {
            api.require_feature("slugs", "slugs")?;
            let mut url = api.endpoint(["parcels", "by-slug"]);
            {
                let mut query = url.query_pairs_mut();
                if let Some(handle) = &handle {
                    query.append_pair("handle", handle);
                }
                query.append_pair("slug", &slug);
            }
            let detail = api
                .get::<ParcelDetail>(url)
                .map_err(|e| read::signed_out_hint(e, api.has_token()))?;
            (detail.value.parcel.id.clone(), Some(detail))
        }
    };
    Ok(Resolved {
        id,
        version,
        page,
        fragment,
        detail,
    })
}

/// [`resolve`], for commands that only need the id.
pub fn resolve_id(api: &Api, parcel_ref: ParcelRef) -> Result<String> {
    Ok(resolve(api, parcel_ref)?.id)
}

/// The link to show people: the clean URL when the parcel has a slug (an
/// older server sends no `canonical_url`: then the viewer URL).
pub fn shown_url(parcel: &ParcelSummary) -> &str {
    if parcel.canonical_url.is_empty() {
        &parcel.url
    } else {
        &parcel.canonical_url
    }
}

/// A parcel's viewer links at a version, page and fragment: the legacy
/// `/p/<id>[/v/<n>]` form and the canonical one (`/@<handle>/<slug>?v=<n>`
/// when it has a slug, else the same).
pub fn links(
    parcel: &ParcelSummary,
    version: Option<i32>,
    page: Option<String>,
    fragment: Option<String>,
) -> (String, String) {
    let mut suffix = ParcelRef {
        target: RefTarget::Id(parcel.id.clone()),
        version: None,
        page,
        fragment,
    };
    let pin = version.map(|v| format!("/v/{v}")).unwrap_or_default();
    let legacy = format!("{}{pin}{}", parcel.url, suffix.url_suffix());
    let canonical = if parcel.slug.is_some() && !parcel.canonical_url.is_empty() {
        suffix.version = version;
        format!("{}{}", parcel.canonical_url, suffix.url_suffix())
    } else {
        legacy.clone()
    };
    (legacy, canonical)
}

/// The `{version}` path segment: a number, or `latest`.
fn version_segment(version: Option<i32>) -> String {
    version.map_or_else(|| "latest".to_string(), |v| v.to_string())
}

/// All of stdin as text, for arguments given as `-`.
fn read_stdin(what: &str) -> Result<String> {
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .with_context(|| format!("reading {what} from stdin"))?;
    Ok(text)
}

/// Try to open a URL in the default browser without waiting for it.
fn open_in_browser(url: &str) {
    if let Err(e) = open::that_detached(url) {
        eprintln!("warning: could not open a browser ({e}); open the URL above yourself");
    }
}

/// Whether to launch a browser for interactive flows: not over SSH and only
/// when a person is plausibly watching stdout.
fn can_launch_browser() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("SSH_CONNECTION").is_none() && std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_refs_for_commands() {
        let id = parse_ref("abcdefghijkm").unwrap();
        assert_eq!(id.target, RefTarget::Id("abcdefghijkm".into()));
        let pinned = parse_ref("http://localhost:5178/p/abcdefghijkm/v/7").unwrap();
        assert_eq!(pinned.version, Some(7));
        let slug = parse_ref("@ada/parcel/plan-001?page=b.html").unwrap();
        assert_eq!(
            slug.target,
            RefTarget::Slug {
                handle: Some("ada".into()),
                slug: "parcel/plan-001".into()
            }
        );
        assert_eq!(slug.page.as_deref(), Some("b.html"));
        for bad in ["not an id", "abcdefghijk", "https://x.test/elsewhere"] {
            let err = parse_ref(bad).unwrap_err();
            assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
            assert!(err.to_string().contains("not a parcel id or URL"), "{err}");
        }
    }

    fn summary(slug: Option<&str>) -> ParcelSummary {
        let at = time::OffsetDateTime::UNIX_EPOCH;
        ParcelSummary {
            id: "abcdefghijkm".into(),
            title: "T".into(),
            description: String::new(),
            visibility: parcel_types::Visibility::Private,
            current_version: 3,
            entry: "index.html".into(),
            file_count: 1,
            total_bytes: 1,
            created_at: at,
            updated_at: at,
            url: "https://x.test/p/abcdefghijkm".into(),
            open_comments: 0,
            tags: vec![],
            archived_at: None,
            slug: slug.map(String::from),
            owner_handle: Some("ada".into()),
            canonical_url: match slug {
                Some(s) => format!("https://x.test/@ada/{s}"),
                None => "https://x.test/p/abcdefghijkm".into(),
            },
        }
    }

    #[test]
    fn links_pin_versions_and_pages() {
        let plain = summary(None);
        assert_eq!(
            links(&plain, None, None, None),
            (plain.url.clone(), plain.url.clone())
        );
        let (legacy, canonical) = links(&plain, Some(2), Some("a b.html".into()), None);
        assert_eq!(legacy, "https://x.test/p/abcdefghijkm/v/2?page=a%20b.html");
        assert_eq!(canonical, legacy);

        let slugged = summary(Some("parcel/plan"));
        let (legacy, canonical) = links(&slugged, Some(2), Some("b.md".into()), Some("s".into()));
        assert_eq!(legacy, "https://x.test/p/abcdefghijkm/v/2?page=b.md#s");
        assert_eq!(canonical, "https://x.test/@ada/parcel/plan?v=2&page=b.md#s");
        assert_eq!(shown_url(&slugged), "https://x.test/@ada/parcel/plan");

        // An older server sends no canonical URL.
        let mut old = summary(None);
        old.canonical_url = String::new();
        assert_eq!(shown_url(&old), old.url);
        assert_eq!(links(&old, None, None, None).1, old.url);
    }

    #[test]
    fn version_segments() {
        assert_eq!(version_segment(None), "latest");
        assert_eq!(version_segment(Some(3)), "3");
    }
}
