//! `update`, `restore`, `delete`.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context, Result};
use parcel_types::{ParcelDetail, ParcelSummary, PublishResult, UpdateParcelRequest};

use super::{authed_api, parse_ref};
use crate::cli::{DeleteArgs, RestoreArgs, UpdateArgs};
use crate::config::Settings;
use crate::error;
use crate::output;

pub fn update(settings: &Settings, args: UpdateArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let request = UpdateParcelRequest {
        title: args.title,
        description: args.description,
        visibility: args.visibility.visibility(),
        ..UpdateParcelRequest::default()
    };
    if request == UpdateParcelRequest::default() {
        return Err(error::usage(
            "nothing to change: pass --title, --description, --private, --internal or --public",
        ));
    }
    let api = authed_api(settings)?;
    let summary = api.patch::<_, ParcelSummary>(api.endpoint(["parcels", &id]), &request)?;
    if args.json.json {
        return output::print_json(&summary.raw);
    }
    let p = summary.value;
    output::print(&format!(
        "Updated {}: {:?} ({})\n  url: {}\n",
        p.id,
        p.title,
        p.visibility.as_str(),
        p.url
    ))
}

pub fn restore(settings: &Settings, args: RestoreArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
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
    let PublishResult { parcel, version } = result.value;
    output::print(&format!(
        "Restored v{} of {} as v{}\n  url: {}\n",
        args.version, parcel.id, version.summary.version, parcel.url
    ))
}

pub fn delete(settings: &Settings, args: DeleteArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            return Err(error::usage(format!(
                "refusing to delete {id} without confirmation; pass --yes"
            )));
        }
        let detail = api
            .get::<ParcelDetail>(api.endpoint(["parcels", &id]))?
            .value;
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
