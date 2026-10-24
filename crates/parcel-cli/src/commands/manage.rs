//! `update`, `tag`, `archive`/`unarchive`, `restore`, `delete`.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context, Result};
use parcel_types::slugs::normalize_slug;
use parcel_types::{ParcelSummary, PublishResult, UpdateParcelRequest};

use super::tags::{normalize_each, normalize_list, split_tag_list};
use super::{authed_api, fetch_detail, parse_ref, resolve, resolve_id, shown_url};
use crate::cli::{DeleteArgs, RefJsonArgs, RestoreArgs, TagArgs, UpdateArgs};
use crate::client::{Api, Fetched};
use crate::config::Settings;
use crate::error;
use crate::output;

pub fn update(settings: &Settings, args: UpdateArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let tags = match &args.set_tags {
        Some(list) => Some(normalize_list(&split_tag_list(list))?),
        None => None,
    };
    let (add_tags, remove_tags) = tag_edits(
        &args.add_tags,
        &args.remove_tags,
        "--add-tag",
        "--remove-tag",
    )?;
    let slug = match args.slug.as_deref().map(str::trim) {
        Some("") => Some(String::new()),
        Some(slug) => Some(normalize_slug(slug).map_err(|e| error::usage(e.to_string()))?),
        None => None,
    };
    let archived = match (args.archive, args.unarchive) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    };
    let request = UpdateParcelRequest {
        title: args.title,
        description: args.description,
        visibility: args.visibility.visibility(),
        tags,
        add_tags,
        remove_tags,
        archived,
        slug,
    };
    if request == UpdateParcelRequest::default() {
        return Err(error::usage(
            "nothing to change: pass --title, --description, --private, --internal, --public, \
             --add-tag, --remove-tag, --set-tags, --slug, --archive or --unarchive",
        ));
    }
    let api = authed_api(settings)?;
    if request.tags.is_some() || !request.add_tags.is_empty() || !request.remove_tags.is_empty() {
        api.require_feature("tags", "tags")?;
    }
    if request.slug.is_some() {
        api.require_feature("slugs", "slugs")?;
    }
    if request.archived.is_some() {
        api.require_feature("archive", "archiving")?;
    }
    let id = resolve_id(&api, parcel_ref)?;
    let summary = patch(&api, &id, &request)?;
    if args.json.json {
        return output::print_json(&summary.raw);
    }
    let p = &summary.value;
    let mut out = format!(
        "Updated {}: {:?} ({})\n  url:  {}\n",
        p.id,
        p.title,
        p.visibility.as_str(),
        shown_url(p)
    );
    if !p.tags.is_empty() {
        out.push_str(&format!("  tags: {}\n", p.tags.join(", ")));
    }
    if p.archived_at.is_some() {
        out.push_str("  archived (hidden from lists and search; still viewable by URL)\n");
    }
    output::print(&out)
}

/// `--add`/`--remove` (or `--add-tag`/`--remove-tag`) normalized, refusing a
/// tag that is in both (the server would too, after the round trip).
fn tag_edits(
    add: &[String],
    remove: &[String],
    add_flag: &str,
    remove_flag: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let add = normalize_each(add)?;
    let remove = normalize_each(remove)?;
    if let Some(both) = add.iter().find(|t| remove.contains(t)) {
        return Err(error::usage(format!(
            "{both} is in both {add_flag} and {remove_flag}"
        )));
    }
    Ok((add, remove))
}

fn patch(api: &Api, id: &str, request: &UpdateParcelRequest) -> Result<Fetched<ParcelSummary>> {
    api.patch::<_, ParcelSummary>(api.endpoint(["parcels", id]), request)
}

pub fn tag(settings: &Settings, args: TagArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let (add_tags, remove_tags) = tag_edits(&args.add, &args.remove, "--add", "--remove")?;
    let api = authed_api(settings)?;
    api.require_feature("tags", "tags")?;
    let id = resolve_id(&api, parcel_ref)?;
    let request = UpdateParcelRequest {
        add_tags,
        remove_tags,
        ..UpdateParcelRequest::default()
    };
    let summary = patch(&api, &id, &request)?;
    if args.json.json {
        return output::print_json(&summary.raw);
    }
    let p = &summary.value;
    output::print(&if p.tags.is_empty() {
        format!("{} has no tags now\n", p.id)
    } else {
        format!("Tags on {}: {}\n", p.id, p.tags.join(", "))
    })
}

/// `archive` (`archived: true`) and `unarchive`.
pub fn archive(settings: &Settings, args: &RefJsonArgs, archived: bool) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    api.require_feature("archive", "archiving")?;
    let id = resolve_id(&api, parcel_ref)?;
    let request = UpdateParcelRequest {
        archived: Some(archived),
        ..UpdateParcelRequest::default()
    };
    let summary = patch(&api, &id, &request)?;
    if args.json.json {
        return output::print_json(&summary.raw);
    }
    let p = &summary.value;
    output::print(&if archived {
        format!(
            "Archived {} ({:?}): hidden from lists and search, still viewable at {}\n\
             Undo with `parcel unarchive {}`\n",
            p.id,
            p.title,
            shown_url(p),
            p.id
        )
    } else {
        format!(
            "Unarchived {} ({:?}): back in lists and search\n",
            p.id, p.title
        )
    })
}

pub fn restore(settings: &Settings, args: RestoreArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    let version = args.version.to_string();
    let result = api.post_empty::<PublishResult>(api.endpoint([
        "parcels",
        id.as_str(),
        "versions",
        version.as_str(),
        "restore",
    ]))?;
    if args.json.json {
        return output::print_json(&result.raw);
    }
    let PublishResult {
        parcel, version, ..
    } = result.value;
    output::print(&format!(
        "Restored v{} of {} as v{}\n  url: {}\n",
        args.version,
        parcel.id,
        version.summary.version,
        shown_url(&parcel)
    ))
}

pub fn delete(settings: &Settings, args: DeleteArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    let target = resolve(&api, parcel_ref)?;
    let id = target.id;
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            return Err(error::usage(format!(
                "refusing to delete {id} without confirmation; pass --yes"
            )));
        }
        let detail = fetch_detail(&api, &id, target.detail)?.value;
        let versions = detail.versions.len();
        eprint!(
            "Delete {:?} ({id}) and all {versions} version{}? This cannot be undone. [y/N] ",
            detail.parcel.title,
            if versions == 1 { "" } else { "s" }
        );
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .context("reading confirmation")?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            anyhow::bail!("aborted; nothing was deleted");
        }
    }
    api.delete(api.endpoint(["parcels", &id]))?;
    output::print(&format!("Deleted {id}\n"))
}
