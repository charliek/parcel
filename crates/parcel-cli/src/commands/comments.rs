//! `comments list|add|reply|resolve|reopen|delete`.
//!
//! The human output of `list` is written for an agent acting on feedback:
//! each thread says which page and which element it is about, who asked,
//! and what they asked, with the replies beneath.

use anyhow::{Context, Result};
use parcel_types::comments::validate_body;
use parcel_types::{
    Comment, CommentAnchor, CommentAuthor, CommentList, CommentStatus, CommentVia, NewComment,
    UpdateComment,
};
use time::OffsetDateTime;

use super::{authed_api, parse_ref, read_stdin};
use crate::cli::{
    CommentNumberArgs, CommentNumbersArgs, CommentsAddArgs, CommentsCommand, CommentsListArgs,
    CommentsReplyArgs, CommentsResolveArgs,
};
use crate::client::Api;
use crate::config::Settings;
use crate::error;
use crate::files::plural;
use crate::output::{self, format_age, truncate};

/// How much of an anchored element's text `list` shows.
const ANCHOR_TEXT_CHARS: usize = 60;
/// How much of a selector `list` shows (its most specific end).
const SELECTOR_CHARS: usize = 80;

pub fn run(settings: &Settings, command: CommentsCommand) -> Result<()> {
    match command {
        CommentsCommand::List(args) => list(settings, args),
        CommentsCommand::Add(args) => add(settings, args),
        CommentsCommand::Reply(args) => reply(settings, args),
        CommentsCommand::Resolve(args) => resolve(settings, args),
        CommentsCommand::Reopen(args) => reopen(settings, args),
        CommentsCommand::Delete(args) => delete(settings, args),
    }
}

/// A comment's text from its argument (`-` reads stdin), checked against
/// the server's rules so a bad one fails before any request. `flag` names
/// the option it came from, if any, for the error message.
fn comment_text(arg: &str, flag: Option<&str>) -> Result<String> {
    let text = if arg == "-" {
        read_stdin("the comment")?
    } else {
        arg.to_string()
    };
    validate_body(&text).map(str::to_string).map_err(|e| {
        error::usage(match flag {
            Some(flag) => format!("{flag}: {e}"),
            None => e,
        })
    })
}

fn comments_url(api: &Api, id: &str) -> reqwest::Url {
    api.endpoint(["parcels", id, "comments"])
}

fn comment_url(api: &Api, id: &str, number: i32) -> reqwest::Url {
    let number = number.to_string();
    api.endpoint(["parcels", id, "comments", number.as_str()])
}

/// Thread numbers in the order given, without repeats.
fn unique(numbers: &[i32]) -> Vec<i32> {
    let mut out = Vec::with_capacity(numbers.len());
    for n in numbers {
        if !out.contains(n) {
            out.push(*n);
        }
    }
    out
}

pub fn list(settings: &Settings, args: CommentsListArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let status = CommentStatus::from(args.status);
    let api = authed_api(settings)?;
    let mut url = comments_url(&api, &id);
    url.query_pairs_mut().append_pair(
        "status",
        match status {
            CommentStatus::Open => "open",
            CommentStatus::Resolved => "resolved",
            CommentStatus::All => "all",
        },
    );
    let list = api.get::<CommentList>(url)?;
    if args.json.json {
        return output::print_json(&list.raw);
    }
    output::print(&render(&list.value, &id, status, OffsetDateTime::now_utc()))
}

/// The `list` text: a header with the counts, then each thread.
fn render(list: &CommentList, id: &str, status: CommentStatus, now: OffsetDateTime) -> String {
    let shown = list.comments.len();
    let open = list.open;
    let mut out = match (status, shown) {
        (CommentStatus::Open, 0) => format!("No open comments on {id}.\n"),
        (CommentStatus::Open, n) => format!("{} on {id}\n", plural(n, "open thread")),
        (CommentStatus::Resolved, 0) => format!("No resolved comments on {id} ({open} open).\n"),
        (CommentStatus::Resolved, n) => {
            format!("{} on {id} ({open} open)\n", plural(n, "resolved thread"))
        }
        (CommentStatus::All, 0) => format!("No comments on {id}.\n"),
        (CommentStatus::All, n) => format!("{} on {id} ({open} open)\n", plural(n, "thread")),
    };
    for thread in &list.comments {
        out.push('\n');
        out.push_str(&render_thread(thread, now));
    }
    out
}

/// ```text
/// #1 · v2 · index.html › "Revenue by region" · Ada (owner) · 3h ago
///     Make the bars blue.
///     #2 · you via cli · 2h ago
///         Done in v3.
/// ```
fn render_thread(thread: &Comment, now: OffsetDateTime) -> String {
    let mut head = vec![
        format!("#{}", thread.number),
        format!("v{}", thread.version),
        anchor_summary(thread.anchor.as_ref()),
        author_label(&thread.author, thread.via),
        format_age(thread.created_at, now),
    ];
    if thread.resolved {
        head.push(match thread.resolved_at {
            Some(at) => format!("resolved {}", format_age(at, now)),
            None => "resolved".to_string(),
        });
    }
    let mut out = format!("{}\n", head.join(" · "));
    out.push_str(&indent(&thread.body, 4));
    for reply in &thread.replies {
        out.push_str(&format!(
            "    #{} · {} · {}\n",
            reply.number,
            author_label(&reply.author, reply.via),
            format_age(reply.created_at, now)
        ));
        out.push_str(&indent(&reply.body, 8));
    }
    out
}

/// `you (owner) via cli`, `Ada (owner)`, `Bob`.
fn author_label(author: &CommentAuthor, via: CommentVia) -> String {
    let mut label = if author.is_you {
        "you".to_string()
    } else {
        author.name.clone()
    };
    if author.is_owner {
        label.push_str(" (owner)");
    }
    if via == CommentVia::Cli {
        label.push_str(" via cli");
    }
    label
}

/// Where a thread points: the page, then the element's text when it had
/// any (what an agent searches the source for), else its selector.
/// `general` for a comment about the whole parcel.
fn anchor_summary(anchor: Option<&CommentAnchor>) -> String {
    let Some(anchor) = anchor else {
        return "general".to_string();
    };
    let text = anchor
        .text
        .as_deref()
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty());
    match (text, anchor.selector.as_deref()) {
        (Some(text), _) => format!("{} › {:?}", anchor.path, truncate(&text, ANCHOR_TEXT_CHARS)),
        (None, Some(selector)) => format!(
            "{} › {}",
            anchor.path,
            truncate_start(selector, SELECTOR_CHARS)
        ),
        (None, None) => anchor.path.clone(),
    }
}

/// Keep the last `max` characters behind a leading `…`: a selector's end
/// names the element, its start only the document's outer layers.
fn truncate_start(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(count - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                "\n".to_string()
            } else {
                format!("{pad}{}\n", line.trim_end())
            }
        })
        .collect()
}

pub fn add(settings: &Settings, args: CommentsAddArgs) -> Result<()> {
    let (id, pinned) = parse_ref(&args.parcel.parcel)?;
    let body = comment_text(&args.text.text, None)?;
    let api = authed_api(settings)?;
    let request = NewComment {
        body,
        version: args.version.or(pinned),
        ..Default::default()
    };
    let thread = api.post::<_, Comment>(comments_url(&api, &id), &request)?;
    if args.json.json {
        return output::print_json(&thread.raw);
    }
    output::print(&format!(
        "Added comment #{} on {id} (v{})\n",
        thread.value.number, thread.value.version
    ))
}

/// Post a reply; returns the thread as the server sent it (for `--json`)
/// and the new reply's number.
fn post_reply(api: &Api, id: &str, thread: i32, body: &str) -> Result<(Vec<u8>, i32)> {
    let request = NewComment {
        body: body.to_string(),
        reply_to: Some(thread),
        ..Default::default()
    };
    let fetched = api.post::<_, Comment>(comments_url(api, id), &request)?;
    // Numbers only grow, so the caller's newest reply is the one just made.
    let number = fetched
        .value
        .replies
        .iter()
        .filter(|r| r.author.is_you)
        .map(|r| r.number)
        .max()
        .context("the server's reply did not include the new comment")?;
    Ok((fetched.raw, number))
}

pub fn reply(settings: &Settings, args: CommentsReplyArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let body = comment_text(&args.text.text, None)?;
    let api = authed_api(settings)?;
    let (raw, number) = post_reply(&api, &id, args.number, &body)?;
    if args.json.json {
        return output::print_json(&raw);
    }
    output::print(&format!("Replied to #{} as #{number}\n", args.number))
}

fn set_resolved(api: &Api, id: &str, number: i32, resolved: bool) -> Result<Comment> {
    let request = UpdateComment {
        resolved: Some(resolved),
        ..Default::default()
    };
    Ok(api
        .patch::<_, Comment>(comment_url(api, id, number), &request)?
        .value)
}

/// Each thread in turn: reply (with `--reply`), then resolve. Stops at the
/// first failure; threads already done stay done and are reported.
pub fn resolve(settings: &Settings, args: CommentsResolveArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let reply = match &args.reply {
        Some(text) => Some(comment_text(text, Some("--reply"))?),
        None => None,
    };
    let api = authed_api(settings)?;
    for number in unique(&args.numbers) {
        let replied = match &reply {
            Some(body) => {
                let (_, reply_number) = post_reply(&api, &id, number, body)
                    .with_context(|| format!("replying to #{number}"))?;
                Some(reply_number)
            }
            None => None,
        };
        set_resolved(&api, &id, number, true).with_context(|| format!("resolving #{number}"))?;
        output::print(&match replied {
            Some(r) => format!("Resolved #{number} (replied as #{r})\n"),
            None => format!("Resolved #{number}\n"),
        })?;
    }
    Ok(())
}

pub fn reopen(settings: &Settings, args: CommentNumbersArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    for number in unique(&args.numbers) {
        set_resolved(&api, &id, number, false).with_context(|| format!("reopening #{number}"))?;
        output::print(&format!("Reopened #{number}\n"))?;
    }
    Ok(())
}

pub fn delete(settings: &Settings, args: CommentNumberArgs) -> Result<()> {
    let (id, _) = parse_ref(&args.parcel.parcel)?;
    let api = authed_api(settings)?;
    api.delete(comment_url(&api, &id, args.number))?;
    output::print(&format!("Deleted comment #{} from {id}\n", args.number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use parcel_types::CommentReply;
    use time::Duration;

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::days(400)
    }

    fn who(name: &str, is_owner: bool, is_you: bool) -> CommentAuthor {
        CommentAuthor {
            name: name.into(),
            is_owner,
            is_you,
        }
    }

    fn thread(number: i32, anchor: Option<CommentAnchor>) -> Comment {
        Comment {
            number,
            version: 2,
            author: who("Ada", true, false),
            body: "Make the bars blue.\n\nAnd label the axis.".into(),
            anchor,
            via: CommentVia::Web,
            resolved: false,
            resolved_at: None,
            created_at: now() - Duration::hours(3),
            updated_at: now() - Duration::hours(3),
            replies: vec![],
        }
    }

    fn anchor(selector: Option<&str>, text: Option<&str>) -> CommentAnchor {
        CommentAnchor {
            path: "index.html".into(),
            selector: selector.map(str::to_string),
            text: text.map(str::to_string),
            x: None,
            y: None,
        }
    }

    #[test]
    fn anchors_name_the_page_and_element() {
        assert_eq!(anchor_summary(None), "general");
        assert_eq!(
            anchor_summary(Some(&anchor(
                Some("h2"),
                Some("  Revenue \n by \"region\" ")
            ))),
            r#"index.html › "Revenue by \"region\"""#
        );
        assert_eq!(
            anchor_summary(Some(&anchor(Some("html > body > p"), Some("   ")))),
            "index.html › html > body > p"
        );
        assert_eq!(anchor_summary(Some(&anchor(None, None))), "index.html");
        let long = format!("html > {}p:nth-of-type(3)", "div > ".repeat(30));
        let shown = anchor_summary(Some(&anchor(Some(&long), None)));
        assert!(shown.starts_with("index.html › …"), "{shown}");
        assert!(shown.ends_with("div > p:nth-of-type(3)"), "{shown}");
    }

    #[test]
    fn authors_say_who_and_how() {
        assert_eq!(
            author_label(&who("Ada", true, true), CommentVia::Cli),
            "you (owner) via cli"
        );
        assert_eq!(
            author_label(&who("Ada", true, false), CommentVia::Web),
            "Ada (owner)"
        );
        assert_eq!(
            author_label(&who("Bob", false, false), CommentVia::Web),
            "Bob"
        );
    }

    #[test]
    fn renders_threads_with_replies() {
        let mut first = thread(
            1,
            Some(anchor(Some("main > h2"), Some("Revenue by region"))),
        );
        first.replies.push(CommentReply {
            number: 4,
            author: who("Ada", true, true),
            body: "Done in v3.".into(),
            via: CommentVia::Cli,
            created_at: now() - Duration::minutes(5),
            updated_at: now() - Duration::minutes(5),
        });
        let mut general = thread(3, None);
        general.author = who("bob@example.com", false, false);
        general.body = "Looks good overall.".into();
        general.resolved = true;
        general.resolved_at = Some(now() - Duration::hours(1));
        let list = CommentList {
            comments: vec![first, general],
            open: 1,
        };
        assert_eq!(
            render(&list, "abcdefghijkm", CommentStatus::All, now()),
            "2 threads on abcdefghijkm (1 open)\n\
             \n\
             #1 · v2 · index.html › \"Revenue by region\" · Ada (owner) · 3h ago\n\
             \x20   Make the bars blue.\n\
             \n\
             \x20   And label the axis.\n\
             \x20   #4 · you (owner) via cli · 5m ago\n\
             \x20       Done in v3.\n\
             \n\
             #3 · v2 · general · bob@example.com · 3h ago · resolved 1h ago\n\
             \x20   Looks good overall.\n"
        );
    }

    #[test]
    fn headers_count_threads() {
        let empty = CommentList {
            comments: vec![],
            open: 2,
        };
        assert_eq!(
            render(&empty, "abcdefghijkm", CommentStatus::Resolved, now()),
            "No resolved comments on abcdefghijkm (2 open).\n"
        );
        let one = CommentList {
            comments: vec![thread(1, None)],
            open: 1,
        };
        assert!(render(&one, "abcdefghijkm", CommentStatus::Open, now())
            .starts_with("1 open thread on abcdefghijkm\n"));
    }

    #[test]
    fn numbers_are_deduplicated_in_order() {
        assert_eq!(unique(&[3, 1, 3, 2, 1]), vec![3, 1, 2]);
    }
}
