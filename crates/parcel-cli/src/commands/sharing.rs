//! `share`, `unshare`, `shares`.
//!
//! Shares are by email and owner only. Both changing commands end by
//! printing who now has access, so an agent sees the result of what it did
//! without a second call.

use anyhow::{Context, Result};
use parcel_types::{normalize_email, PutShareRequest, Share, ShareList, ShareRole};

use super::{authed_api, parse_ref, resolve_id};
use crate::cli::{RefArg, ShareArgs, UnshareArgs};
use crate::client::{Api, Fetched};
use crate::config::Settings;
use crate::error::{self, ApiError};
use crate::output::{self, printable, table};

/// The emails given, normalized the way the server matches them, without
/// repeats. A malformed one is a usage error before any request.
fn emails(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    for email in raw {
        let email = normalize_email(email).map_err(error::usage)?;
        if !out.contains(&email) {
            out.push(email);
        }
    }
    Ok(out)
}

fn shares_url(api: &Api, id: &str) -> reqwest::Url {
    api.endpoint(["parcels", id, "shares"])
}

fn share_url(api: &Api, id: &str, email: &str) -> reqwest::Url {
    api.endpoint(["parcels", id, "shares", email])
}

fn get_shares(api: &Api, id: &str) -> Result<Fetched<ShareList>> {
    api.get::<ShareList>(shares_url(api, id))
}

pub fn share(settings: &Settings, args: ShareArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let emails = emails(&args.emails)?;
    let role = ShareRole::from(args.role);
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    // One at a time; a share is idempotent, so after a failure the command
    // can simply be run again.
    for email in &emails {
        api.put::<_, Share>(share_url(&api, &id, email), &PutShareRequest { role })
            .with_context(|| format!("sharing with {email}"))?;
        if !args.json {
            output::print(&format!("Shared {id} with {email} ({})\n", role.as_str()))?;
        }
    }
    finish(&api, &id, args.json, true)
}

pub fn unshare(settings: &Settings, args: UnshareArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    let emails = emails(&args.emails)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    // Reading the list first tells "not shared with them" (nothing to do)
    // apart from "no such parcel" (an error), which share a 404 otherwise.
    let current = get_shares(&api, &id)?.value;
    let mut removed_any = false;
    for email in &emails {
        let shared = current.shares.iter().any(|s| &s.email == email);
        let removed = shared
            && match api.delete(share_url(&api, &id, email)) {
                Ok(()) => true,
                // Removed by someone else in the meantime.
                Err(e)
                    if e.downcast_ref::<ApiError>()
                        .is_some_and(|a| a.status == 404) =>
                {
                    false
                }
                Err(e) => return Err(e.context(format!("unsharing with {email}"))),
            };
        if removed {
            removed_any = true;
            if !args.json {
                output::print(&format!("Unshared {id} with {email}\n"))?;
            }
        } else {
            eprintln!("note: {id} was not shared with {email}");
        }
    }
    finish(&api, &id, args.json, removed_any)
}

pub fn shares(settings: &Settings, parcel: &RefArg, json: bool) -> Result<()> {
    let parcel_ref = parse_ref(&parcel.parcel)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    finish(&api, &id, json, false)
}

/// Print the parcel's shares as they now stand, after a blank line when
/// `after_lines` (lines saying what changed were printed above).
fn finish(api: &Api, id: &str, json: bool, after_lines: bool) -> Result<()> {
    let list = get_shares(api, id)?;
    if json {
        return output::print_json(&list.raw);
    }
    let mut out = String::new();
    if after_lines {
        out.push('\n');
    }
    out.push_str(&render(id, &list.value.shares));
    // Names come from the people themselves (their provider profile).
    output::print(&printable(&out))
}

/// ```text
/// Shared with 2 people:
/// EMAIL            ROLE       NAME
/// ada@example.com  commenter  Ada Lovelace
/// bob@example.com  viewer     not signed in yet
/// ```
fn render(id: &str, shares: &[Share]) -> String {
    if shares.is_empty() {
        return format!(
            "{id} is not shared with anyone. Share it with `parcel share {id} <email>`.\n"
        );
    }
    let people = if shares.len() == 1 {
        "1 person".to_string()
    } else {
        format!("{} people", shares.len())
    };
    let rows: Vec<Vec<String>> = shares
        .iter()
        .map(|s| {
            let name = s
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or("not signed in yet");
            vec![
                s.email.clone(),
                s.role.as_str().to_string(),
                name.to_string(),
            ]
        })
        .collect();
    format!(
        "Shared with {people}:\n{}",
        table(&["EMAIL", "ROLE", "NAME"], &rows)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    fn share(email: &str, role: ShareRole, name: Option<&str>) -> Share {
        Share {
            email: email.into(),
            role,
            name: name.map(str::to_string),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn emails_are_normalized_and_deduplicated() {
        let given = ["Ada@Example.com ", "bob@example.com", "ada@example.com"].map(String::from);
        assert_eq!(
            emails(&given).unwrap(),
            vec!["ada@example.com", "bob@example.com"]
        );
        let err = emails(&["not-an-email".to_string()]).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
        assert!(err.to_string().contains("is not an email address"), "{err}");
    }

    #[test]
    fn renders_who_has_access() {
        assert_eq!(
            render("abcdefghijkm", &[]),
            "abcdefghijkm is not shared with anyone. \
             Share it with `parcel share abcdefghijkm <email>`.\n"
        );
        let shares = [
            share(
                "ada@example.com",
                ShareRole::Commenter,
                Some("Ada Lovelace"),
            ),
            share("bob@example.com", ShareRole::Viewer, None),
            share("cy@example.com", ShareRole::Viewer, Some("  ")),
        ];
        assert_eq!(
            render("abcdefghijkm", &shares),
            "Shared with 3 people:\n\
             EMAIL            ROLE       NAME\n\
             ada@example.com  commenter  Ada Lovelace\n\
             bob@example.com  viewer     not signed in yet\n\
             cy@example.com   viewer     not signed in yet\n"
        );
        assert!(render("abcdefghijkm", &shares[..1]).starts_with("Shared with 1 person:\n"));
    }
}
