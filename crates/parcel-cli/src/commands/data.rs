//! `data list|get|set|update|delete|clear`.
//!
//! Reads work signed out on public parcels, like `info` and `cat`; writes
//! need the owner's login. Values are sent as the JSON the user wrote
//! (minified, keys in their order) and printed with the server's key order.

use std::fs;

use anyhow::{bail, Context, Result};
use parcel_types::ids::ParcelRef;
use parcel_types::{validate_data_name, DataItem, DataSummary};
use reqwest::{Method, Url};
use serde::Deserialize;
use serde_json::value::RawValue;

use super::read::signed_out_hint;
use super::{api, authed_api, parse_ref, read_stdin, resolve_id};
use crate::cli::{
    DataCollectionArgs, DataCommand, DataGetArgs, DataItemArgs, DataListArgs, DataSetArgs,
    DataUpdateArgs,
};
use crate::client::{Api, Fetched};
use crate::config::Settings;
use crate::error;
use crate::output::{self, age, format_size, table, truncate};

/// Items per request when listing a collection (the server's maximum).
const PAGE_SIZE: &str = "1000";
/// How much of each value `data list <collection>` shows.
const VALUE_PREVIEW_CHARS: usize = 60;

pub fn run(settings: &Settings, command: DataCommand) -> Result<()> {
    match command {
        DataCommand::List(args) => list(settings, args),
        DataCommand::Get(args) => get(settings, args),
        DataCommand::Set(args) => set(settings, args),
        DataCommand::Update(args) => update(settings, args),
        DataCommand::Delete(args) => delete(settings, args),
        DataCommand::Clear(args) => clear(settings, args),
    }
}

fn check_name(kind: &str, name: &str) -> Result<()> {
    validate_data_name(kind, name).map_err(|e| error::usage(format!("{name:?}: {e}")))
}

/// The parcel reference, once the collection and key names are known to be valid.
fn item_ref(args: &DataItemArgs) -> Result<ParcelRef> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    check_name("collection", &args.collection)?;
    check_name("key", &args.key)?;
    Ok(parcel_ref)
}

fn data_url(api: &Api, id: &str, rest: &[&str]) -> Url {
    api.endpoint(
        ["parcels", id, "data"]
            .into_iter()
            .chain(rest.iter().copied()),
    )
}

// ------------------------------------------------------------------ reading

/// One page of a collection, items kept as the server sent them.
#[derive(Deserialize)]
struct RawPage {
    items: Vec<Box<RawValue>>,
    #[serde(default)]
    next_after: Option<String>,
}

#[derive(Deserialize)]
struct RawItem {
    value: Box<RawValue>,
}

pub fn list(settings: &Settings, args: DataListArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    if let Some(collection) = &args.collection {
        check_name("collection", collection)?;
    }
    let api = api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    match &args.collection {
        None => list_collections(&api, &id, args.json),
        Some(collection) => list_items(&api, &id, collection, args.json),
    }
}

fn list_collections(api: &Api, id: &str, json: bool) -> Result<()> {
    let summary = api
        .get::<DataSummary>(data_url(api, id, &[]))
        .map_err(|e| signed_out_hint(e, api.has_token()))?;
    if json {
        return output::print_json(&summary.raw);
    }
    let DataSummary {
        usage_bytes,
        limit_bytes,
        writable,
        collections,
        ..
    } = summary.value;
    let mut out = if collections.is_empty() {
        format!("No data on {id} yet.\n")
    } else {
        let rows: Vec<Vec<String>> = collections
            .iter()
            .map(|c| {
                vec![
                    c.name.clone(),
                    c.count.to_string(),
                    format_size(c.usage_bytes),
                    age(c.updated_at),
                ]
            })
            .collect();
        table(&["COLLECTION", "ITEMS", "SIZE", "UPDATED"], &rows)
    };
    out.push_str(&format!(
        "\nUsing {} of the parcel's {} data limit.",
        format_size(usage_bytes),
        format_size(limit_bytes)
    ));
    if !writable {
        out.push_str(" Read-only: only the parcel's owner can change it.");
    }
    out.push('\n');
    output::print(&out)
}

/// Every item of a collection, following `next_after` page by page.
fn fetch_all(api: &Api, id: &str, collection: &str) -> Result<Vec<Box<RawValue>>> {
    let mut items = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut url = data_url(api, id, &[collection]);
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", PAGE_SIZE);
            if let Some(after) = &after {
                query.append_pair("after", after);
            }
        }
        let page = api
            .get::<RawPage>(url)
            .map_err(|e| signed_out_hint(e, api.has_token()))?
            .value;
        items.extend(page.items);
        match page.next_after {
            // A cursor that doesn't move would loop forever.
            Some(next) if after.as_deref() == Some(next.as_str()) => {
                bail!("the server repeated the page after {next:?}")
            }
            Some(next) => after = Some(next),
            None => return Ok(items),
        }
    }
}

fn list_items(api: &Api, id: &str, collection: &str, json: bool) -> Result<()> {
    let items = fetch_all(api, id, collection)?;
    if json {
        // One DataItemList holding every page, items as the server sent them.
        let joined: Vec<&str> = items.iter().map(|i| i.get()).collect();
        return output::print_json(format!("{{\"items\":[{}]}}", joined.join(",")).as_bytes());
    }
    if items.is_empty() {
        return output::print(&format!("No items in {collection} on {id}.\n"));
    }
    let mut rows = Vec::with_capacity(items.len());
    for raw in &items {
        let item: DataItem =
            serde_json::from_str(raw.get()).context("unexpected item from the server")?;
        let value: RawItem = serde_json::from_str(raw.get())?;
        rows.push(vec![
            item.key,
            format_size(item.size_bytes),
            age(item.updated_at),
            truncate(value.value.get(), VALUE_PREVIEW_CHARS),
        ]);
    }
    output::print(&table(&["KEY", "SIZE", "UPDATED", "VALUE"], &rows))
}

pub fn get(settings: &Settings, args: DataGetArgs) -> Result<()> {
    let parcel_ref = item_ref(&args.item)?;
    let api = api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    let item = api
        .get::<RawItem>(data_url(
            &api,
            &id,
            &[&args.item.collection, &args.item.key],
        ))
        .map_err(|e| signed_out_hint(e, api.has_token()))?;
    if args.json {
        return output::print_json(&item.raw);
    }
    output::print_json(item.value.value.get().as_bytes())
}

// ------------------------------------------------------------------ writing

/// The text of a VALUE/PATCH argument: inline, `@FILE`, or `-` for stdin.
/// The flag says whether it came from a file or stdin.
fn value_source(arg: &str) -> Result<(String, bool)> {
    if arg == "-" {
        return Ok((read_stdin("the value")?, true));
    }
    if let Some(path) = arg.strip_prefix('@') {
        let text = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        return Ok((text, true));
    }
    Ok((arg.to_string(), false))
}

/// Compact JSON with the keys in the order written; an error if not JSON.
fn minify(text: &str) -> Result<Vec<u8>, serde_json::Error> {
    let mut out = Vec::with_capacity(text.len());
    let mut de = serde_json::Deserializer::from_str(text);
    serde_transcode::transcode(&mut de, &mut serde_json::Serializer::new(&mut out))?;
    de.end()?;
    Ok(out)
}

/// The JSON body for `set`.
fn set_body(text: &str, from_stream: bool, as_string: bool) -> Result<Vec<u8>> {
    if as_string {
        // `echo hi | parcel data set … - --string` means "hi", not "hi\n".
        let text = if from_stream {
            text.strip_suffix('\n')
                .map(|t| t.strip_suffix('\r').unwrap_or(t))
                .unwrap_or(text)
        } else {
            text
        };
        return Ok(serde_json::to_vec(text)?);
    }
    minify(text).map_err(|e| {
        error::usage(format!(
            "the value is not valid JSON ({e}); quote strings ('\"text\"') or pass --string"
        ))
    })
}

/// The JSON body for `update`: a JSON object.
fn patch_body(text: &str) -> Result<Vec<u8>> {
    let body =
        minify(text).map_err(|e| error::usage(format!("the patch is not valid JSON ({e})")))?;
    if body.first() != Some(&b'{') {
        return Err(error::usage(
            "the patch must be a JSON object; use `parcel data set` to replace a value",
        ));
    }
    Ok(body)
}

fn write(
    api: &Api,
    id: &str,
    args: &DataItemArgs,
    method: Method,
    content_type: &'static str,
    body: Vec<u8>,
) -> Result<Fetched<DataItem>> {
    api.send_json_text::<DataItem>(
        method,
        data_url(api, id, &[&args.collection, &args.key]),
        content_type,
        body,
    )
}

pub fn set(settings: &Settings, args: DataSetArgs) -> Result<()> {
    let parcel_ref = item_ref(&args.item)?;
    let (text, from_stream) = value_source(&args.value)?;
    let body = set_body(&text, from_stream, args.string)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    let item = write(&api, &id, &args.item, Method::PUT, "application/json", body)?;
    if args.json.json {
        return output::print_json(&item.raw);
    }
    output::print(&format!(
        "Set {}/{} on {id} ({})\n",
        item.value.collection,
        item.value.key,
        format_size(item.value.size_bytes)
    ))
}

pub fn update(settings: &Settings, args: DataUpdateArgs) -> Result<()> {
    let parcel_ref = item_ref(&args.item)?;
    let (text, _) = value_source(&args.patch)?;
    let body = patch_body(&text)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    let item = write(
        &api,
        &id,
        &args.item,
        Method::PATCH,
        "application/merge-patch+json",
        body,
    )?;
    if args.json.json {
        return output::print_json(&item.raw);
    }
    output::print(&format!(
        "Updated {}/{} on {id} ({})\n",
        item.value.collection,
        item.value.key,
        format_size(item.value.size_bytes)
    ))
}

pub fn delete(settings: &Settings, args: DataItemArgs) -> Result<()> {
    let parcel_ref = item_ref(&args)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    api.delete(data_url(&api, &id, &[&args.collection, &args.key]))?;
    output::print(&format!(
        "Deleted {}/{} from {id}\n",
        args.collection, args.key
    ))
}

pub fn clear(settings: &Settings, args: DataCollectionArgs) -> Result<()> {
    let parcel_ref = parse_ref(&args.parcel.parcel)?;
    check_name("collection", &args.collection)?;
    let api = authed_api(settings)?;
    let id = resolve_id(&api, parcel_ref)?;
    api.delete(data_url(&api, &id, &[&args.collection]))?;
    output::print(&format!("Cleared {} on {id}\n", args.collection))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_keep_their_key_order() {
        assert_eq!(
            minify("{ \"b\": 1,\n  \"a\": [true, null] }\n").unwrap(),
            br#"{"b":1,"a":[true,null]}"#
        );
        assert!(minify("{\"a\": 1} trailing").is_err());
        assert!(minify("").is_err());
    }

    #[test]
    fn set_bodies() {
        assert_eq!(set_body("42", false, false).unwrap(), b"42");
        let err = set_body("hello", false, false).unwrap_err();
        assert_eq!(error::exit_code(&err), error::EXIT_USAGE);
        assert!(err.to_string().contains("--string"), "{err}");
        assert_eq!(set_body("hello", false, true).unwrap(), br#""hello""#);
        assert_eq!(set_body("hi\r\n", true, true).unwrap(), br#""hi""#);
        assert_eq!(set_body("a\n\n", true, true).unwrap(), br#""a\n""#);
        assert_eq!(set_body("hi\n", false, true).unwrap(), br#""hi\n""#);
    }

    #[test]
    fn patches_must_be_objects() {
        assert_eq!(patch_body(" {\"a\": null} ").unwrap(), br#"{"a":null}"#);
        for bad in ["[1]", "3", "\"x\"", "nope"] {
            let err = patch_body(bad).unwrap_err();
            assert_eq!(error::exit_code(&err), error::EXIT_USAGE, "{bad}");
        }
    }
}
