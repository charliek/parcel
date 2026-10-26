//! `tags` (the vocabulary with counts), `tags rename`, `tags delete`, and
//! the tag-argument helpers the other commands share.

use anyhow::Result;
use parcel_types::tags::{normalize_set, normalize_tag, normalize_tags};
use parcel_types::{DeleteTagRequest, ListScope, RenameTagRequest, TagCount, TagList, TagsUpdated};

use super::{authed_api, list_scope};
use crate::cli::{TagsArgs, TagsCommand, TagsDeleteArgs, TagsListArgs, TagsRenameArgs};
use crate::client::Api;
use crate::config::Settings;
use crate::error;
use crate::files::plural;
use crate::output::{self, printable, table};

/// The server's largest `GET /tags` page.
const TAGS_PAGE: usize = 1000;

/// One tag argument, normalized the way the server will store it; a bad one
/// is a usage error naming it.
pub fn normalize_arg(raw: &str) -> Result<String> {
    normalize_tag(raw).map_err(|e| error::usage(e.to_string()))
}

/// Tag arguments normalized and deduplicated (sorted), without the 20-cap:
/// the parts of an edit, where only the result is capped (by the server).
pub fn normalize_each(raw: &[String]) -> Result<Vec<String>> {
    Ok(normalize_set(raw)
        .map_err(|e| error::usage(e.to_string()))?
        .into_iter()
        .collect())
}

/// A whole tag set: normalized, deduplicated, at most 20.
pub fn normalize_list(raw: &[String]) -> Result<Vec<String>> {
    normalize_tags(raw).map_err(|e| error::usage(e.to_string()))
}

/// `a, b,c` → `["a", "b", "c"]`; `""` → no tags.
pub fn split_tag_list(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn run(settings: &Settings, args: TagsArgs) -> Result<()> {
    match args.command {
        None => list(settings, args.list),
        Some(TagsCommand::Rename(args)) => rename(settings, args),
        Some(TagsCommand::Delete(args)) => delete(settings, args),
    }
}

fn list(settings: &Settings, args: TagsListArgs) -> Result<()> {
    let api = authed_api(settings)?;
    api.require_feature("tags", "tags")?;
    let scope = list_scope(args.all, args.shared);
    let tags = fetch_all(&api, scope, args.prefix.as_deref())?;
    if args.json {
        // Every page merged into one body of the server's shape.
        let body = TagList {
            total: i64::try_from(tags.len()).unwrap_or(i64::MAX),
            tags,
            // No query: nothing to relax.
            relaxed: false,
        };
        return output::print_json(&serde_json::to_vec(&body)?);
    }
    if tags.is_empty() {
        return output::print(&match &args.prefix {
            Some(prefix) => format!("No tags start with {prefix:?}.\n"),
            None => "No tags yet. Add some with `parcel tag <ref> --add <tag>` or \
                     `parcel publish --tag <tag>`.\n"
                .to_string(),
        });
    }
    let rows: Vec<Vec<String>> = tags
        .iter()
        .map(|t| vec![t.tag.clone(), t.count.to_string()])
        .collect();
    // Tags on parcels shared with you are other people's.
    output::print(&printable(&table(&["TAG", "PARCELS"], &rows)))
}

/// Every page of `GET /tags` (most used first, then by name).
fn fetch_all(api: &Api, scope: ListScope, prefix: Option<&str>) -> Result<Vec<TagCount>> {
    let mut all: Vec<TagCount> = Vec::new();
    loop {
        let mut url = api.endpoint(["tags"]);
        {
            let mut query = url.query_pairs_mut();
            if scope != ListScope::Mine {
                query.append_pair("scope", scope.as_str());
            }
            if let Some(prefix) = prefix {
                query.append_pair("prefix", prefix);
            }
            query.append_pair("limit", &TAGS_PAGE.to_string());
            if !all.is_empty() {
                query.append_pair("offset", &all.len().to_string());
            }
        }
        let page = api.get::<TagList>(url)?.value;
        let fetched = page.tags.len();
        all.extend(page.tags);
        let total = usize::try_from(page.total).unwrap_or(0);
        if fetched < TAGS_PAGE || all.len() >= total {
            return Ok(all);
        }
    }
}

fn rename(settings: &Settings, args: TagsRenameArgs) -> Result<()> {
    let request = RenameTagRequest {
        from: normalize_arg(&args.from)?,
        to: normalize_arg(&args.to)?,
    };
    let api = authed_api(settings)?;
    api.require_feature("tags", "tags")?;
    let done = api.post::<_, TagsUpdated>(api.endpoint(["tags", "rename"]), &request)?;
    if args.json {
        return output::print_json(&done.raw);
    }
    output::print(&format!(
        "Renamed {} to {} on {}\n",
        request.from,
        request.to,
        parcels(done.value.updated)
    ))
}

fn delete(settings: &Settings, args: TagsDeleteArgs) -> Result<()> {
    let request = DeleteTagRequest {
        tag: normalize_arg(&args.tag)?,
    };
    let api = authed_api(settings)?;
    api.require_feature("tags", "tags")?;
    let done = api.post::<_, TagsUpdated>(api.endpoint(["tags", "delete"]), &request)?;
    if args.json {
        return output::print_json(&done.raw);
    }
    output::print(&format!(
        "Removed {} from {}\n",
        request.tag,
        parcels(done.value.updated)
    ))
}

fn parcels(n: i64) -> String {
    plural(usize::try_from(n).unwrap_or(0), "parcel")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_lists_split_on_commas() {
        assert_eq!(split_tag_list("a, b,,c "), ["a", "b", "c"]);
        assert!(split_tag_list("").is_empty());
        assert!(split_tag_list(" , ").is_empty());
    }

    #[test]
    fn bad_tags_are_usage_errors() {
        let err = normalize_each(&["ok".into(), "no way!".into()]).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
        assert!(err.to_string().contains("\"no way!\""), "{err}");
        assert_eq!(
            normalize_each(&["B".into(), "a".into(), "b".into()]).unwrap(),
            ["a", "b"]
        );
        let many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
        assert!(normalize_list(&many).is_err());
        assert_eq!(normalize_each(&many).unwrap().len(), 21);
    }
}
