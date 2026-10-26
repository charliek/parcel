//! Full-text search: the plain text of each parcel's current entry page.
//!
//! `parcels.search_text` holds it (≤ 128 KiB); a trigger folds it into
//! `search_vec` with the title, description and tags, and another clears it
//! whenever `current_version` changes (migration 0006). This module fills it
//! back in:
//!
//! - after a publish commits, in a spawned task ([`spawn_index`]), so
//!   publishing never waits on extraction;
//! - from a loop that sweeps up whatever is still missing every minute
//!   ([`spawn_backfill`]): parcels from before search existed, versions an
//!   older revision published mid-rollout, and transient failures (each
//!   parcel gets [`MAX_ATTEMPTS`] tries per version);
//! - on demand, `parcel-server reindex` ([`drain`] without a budget).
//!
//! Every write ([`crate::db::search`]) is guarded by `current_version` (and
//! `search_text IS NULL`), so a slow extraction can never store an older
//! version's text over a newer one, and several instances racing write the
//! same thing at most once.
//!
//! Extraction is a small hand-written stripper, not an HTML parser: its
//! output only ever reaches the tsvector and search snippets, which every
//! consumer renders as text. It drops control characters *after* decoding
//! entities, so `\x02`/`\x03` (the snippet markers, [`HEADLINE_OPTIONS`])
//! can only come from `ts_headline`.
//!
//! Failures are logged value-free (invariant 5): page text, titles and
//! database messages (which can quote row values) never reach a log line.
//!
//! [`HEADLINE_OPTIONS`]: crate::db::parcels::HEADLINE_OPTIONS

use std::time::Duration;

use bytes::Bytes;
use parcel_types::{media, ListMatch, MatchSegment};
use pulldown_cmark::{Event, Parser, TagEnd};
use sqlx::PgPool;
use stridelabs_postgres::error_summary;
use tokio::time::Instant;

use crate::db::search::{self, Pending};
use crate::state::AppState;

/// Most page text kept per parcel (bytes, cut at a character boundary).
pub const MAX_SEARCH_TEXT_BYTES: usize = 128 * 1024;
/// Extractions the backfill tries per version before giving up on it.
pub const MAX_ATTEMPTS: i16 = 5;
const BACKFILL_INTERVAL: Duration = Duration::from_secs(60);
const BACKFILL_BUDGET: Duration = Duration::from_secs(20);
const BATCH: i64 = 50;

// ---- extraction -----------------------------------------------------------

/// The searchable text of an entry page (HTML or Markdown, by its path).
pub fn extract_text(entry: &str, bytes: &[u8]) -> String {
    let source = String::from_utf8_lossy(bytes);
    if media::is_markdown(entry) {
        markdown_text(&source)
    } else {
        html_text(&source)
    }
}

/// Elements whose contents are never page text.
const SKIPPED: [&str; 5] = ["script", "style", "template", "noscript", "svg"];
/// The [`SKIPPED`] elements whose contents are raw text, ending at the first
/// matching end tag whatever comes before it (`noscript` too: that's how a
/// browser with scripting on parses it). The others nest, so skipping one
/// counts its own start and end tags.
const RAW_TEXT: [&str; 3] = ["script", "style", "noscript"];
/// Elements that end a line of text, so words either side don't run together.
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "caption",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "ol",
    "option",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

/// Text of an HTML page: tags and comments dropped, the contents of
/// [`SKIPPED`] elements skipped, entities decoded, whitespace collapsed.
pub fn html_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = PlainText::with_capacity(html.len());
    let mut text_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            out.push_html(&html[text_start..i]);
            i = comment_end(html, i);
            text_start = i;
            continue;
        }
        if !starts_tag(bytes, i) {
            // A bare `<` is text ("a < b").
            i += 1;
            continue;
        }
        out.push_html(&html[text_start..i]);
        let end = tag_end(bytes, i + 1);
        let tag = &html[i + 1..end];
        let name = tag_name(tag);
        i = if !tag.starts_with('/') && SKIPPED.contains(&name.as_str()) {
            skip_element(html, end, &name, tag)
        } else {
            end
        };
        if BLOCKS.contains(&name.as_str()) {
            out.line_break();
        }
        text_start = i;
        if out.is_full() {
            // Enough: the rest would be cut anyway.
            return out.into_string();
        }
    }
    out.push_html(&html[text_start..]);
    out.into_string()
}

/// Whether the `<` at `i` starts a tag (a bare `<` is text).
fn starts_tag(bytes: &[u8], i: usize) -> bool {
    bytes
        .get(i + 1)
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'/' | b'!' | b'?'))
}

/// Index just past the `-->` closing a comment that starts at `i`; the end
/// of input if unterminated.
fn comment_end(html: &str, i: usize) -> usize {
    html[i + 4..]
        .find("-->")
        .map_or(html.len(), |e| i + 4 + e + 3)
}

/// Index just past the `>` closing a tag that starts at `from`, ignoring a
/// `>` inside a quoted attribute value; the end of input if unterminated.
fn tag_end(bytes: &[u8], from: usize) -> usize {
    let mut quote: Option<u8> = None;
    for (offset, &b) in bytes[from..].iter().enumerate() {
        match (quote, b) {
            (None, b'"' | b'\'') => quote = Some(b),
            (Some(q), _) if b == q => quote = None,
            (None, b'>') => return from + offset + 1,
            _ => {}
        }
    }
    bytes.len()
}

/// Lowercased element name of a tag's inside (`/P class=x>` → `p`).
fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Index just past the end of a [`SKIPPED`] element whose start tag `tag`
/// ended at `from`.
fn skip_element(html: &str, from: usize, name: &str, tag: &str) -> usize {
    if RAW_TEXT.contains(&name) {
        return raw_text_end(html, from, name);
    }
    // `<svg/>` is empty (foreign elements honour a self-closing tag); an
    // HTML element like `<template/>` ignores the slash and still opens.
    if name == "svg" && tag.ends_with("/>") {
        return from;
    }
    skip_nested(html, from, name)
}

/// Index just past the end tag closing a nestable skipped element (`template`
/// or `svg`), counting its own nested start and end tags. Comments are
/// opaque, as are CDATA sections in SVG and raw-text children in a template,
/// so an end tag written inside one doesn't count.
fn skip_nested(html: &str, from: usize, name: &str) -> usize {
    let bytes = html.as_bytes();
    let mut depth = 1usize;
    let mut at = from;
    while let Some(found) = html[at..].find('<') {
        let i = at + found;
        if html[i..].starts_with("<!--") {
            at = comment_end(html, i);
            continue;
        }
        if name == "svg" && html[i..].starts_with("<![CDATA[") {
            at = html[i..].find("]]>").map_or(bytes.len(), |e| i + e + 3);
            continue;
        }
        if !starts_tag(bytes, i) {
            at = i + 1;
            continue;
        }
        let end = tag_end(bytes, i + 1);
        let tag = &html[i + 1..end];
        let tag_name = tag_name(tag);
        at = end;
        if tag.starts_with('/') {
            if tag_name == name {
                depth -= 1;
                if depth == 0 {
                    return end;
                }
            }
        } else if tag_name == name {
            if !(name == "svg" && tag.ends_with("/>")) {
                depth += 1;
            }
        } else if name == "template" && RAW_TEXT.contains(&tag_name.as_str()) {
            at = raw_text_end(html, end, &tag_name);
        }
    }
    bytes.len()
}

/// Index just past the first `</name …>` after `from`: the end of a raw-text
/// element whose start tag ended there.
fn raw_text_end(html: &str, from: usize, name: &str) -> usize {
    let bytes = html.as_bytes();
    let mut at = from;
    while let Some(found) = html[at..].find("</") {
        let start = at + found + 2;
        let candidate = &bytes[start..];
        if candidate.len() >= name.len()
            && candidate[..name.len()].eq_ignore_ascii_case(name.as_bytes())
            && !candidate
                .get(name.len())
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
        {
            return tag_end(bytes, start);
        }
        at = start;
    }
    bytes.len()
}

/// Extracted text under construction: control characters dropped and
/// whitespace collapsed as it arrives (a run holding a line break becomes one
/// `\n`, any other run one space; none leading or trailing), and capped at
/// [`MAX_SEARCH_TEXT_BYTES`] at a character boundary. The cap applies to this
/// normalized text, so markup or whitespace padding ahead of the words can't
/// use it up.
struct PlainText {
    out: String,
    pending: Option<char>,
    full: bool,
}

impl PlainText {
    fn with_capacity(source_len: usize) -> Self {
        PlainText {
            out: String::with_capacity(source_len.min(MAX_SEARCH_TEXT_BYTES)),
            pending: None,
            full: false,
        }
    }

    /// Append text, decoding HTML entities first.
    fn push_html(&mut self, raw: &str) {
        if self.full {
            return;
        }
        if raw.contains('&') {
            self.push(&decode_entities(raw));
        } else {
            self.push(raw);
        }
    }

    fn push(&mut self, text: &str) {
        for c in text.chars() {
            if self.full {
                return;
            }
            if c == '\n' || c == '\r' {
                self.line_break();
            } else if c.is_whitespace() {
                self.pending = self.pending.or(Some(' '));
            } else if !c.is_control() {
                self.push_char(c);
            }
        }
    }

    fn push_char(&mut self, c: char) {
        let space = self.pending.take().filter(|_| !self.out.is_empty());
        let needed = space.map_or(0, char::len_utf8) + c.len_utf8();
        if self.out.len() + needed > MAX_SEARCH_TEXT_BYTES {
            self.full = true;
            return;
        }
        if let Some(space) = space {
            self.out.push(space);
        }
        self.out.push(c);
    }

    fn line_break(&mut self) {
        self.pending = Some('\n');
    }

    fn is_full(&self) -> bool {
        self.full
    }

    fn into_string(self) -> String {
        self.out
    }
}

/// Decode `&name;`, `&#n;` and `&#xh;`. Unknown or unterminated references
/// stay as written; a number that isn't a character becomes U+FFFD.
pub(crate) fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after.char_indices().take(32).find(|(_, c)| *c == ';');
        let decoded = semi.and_then(|(at, _)| entity(&after[..at]).map(|c| (c, at)));
        match decoded {
            Some((c, at)) => {
                out.push(c);
                rest = &after[at + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse::<u32>().ok()?,
        };
        return Some(
            char::from_u32(code)
                .filter(|c| *c != '\0')
                .unwrap_or('\u{fffd}'),
        );
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "middot" => '·',
        "bull" => '•',
        "deg" => '°',
        "times" => '×',
        "divide" => '÷',
        "plusmn" => '±',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "sect" => '§',
        "para" => '¶',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        _ => return None,
    })
}

/// Text of a Markdown page: its text and code spans, joined with spaces.
pub fn markdown_text(markdown: &str) -> String {
    let mut out = PlainText::with_capacity(markdown.len());
    for event in Parser::new_ext(markdown, crate::content::markdown::OPTIONS) {
        match event {
            Event::Text(t) | Event::Code(t) => {
                out.push(&t);
                out.push(" ");
            }
            Event::SoftBreak | Event::HardBreak => out.push(" "),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::TableRow
                | TagEnd::BlockQuote(_),
            ) => out.line_break(),
            _ => {}
        }
        if out.is_full() {
            break;
        }
    }
    out.into_string()
}

// ---- snippets -------------------------------------------------------------

/// Turn `ts_headline` output into snippet segments. `None` unless it marked
/// at least one word (the query matched the title or tags but not the
/// page text). Whitespace is collapsed and any other control character
/// dropped, so a segment is plain printable text.
pub fn segments(headline: &str) -> Option<ListMatch> {
    if !headline.contains('\u{2}') {
        return None;
    }
    let printable: String = headline
        .chars()
        .filter(|&c| !c.is_control() || c.is_whitespace() || matches!(c, '\u{2}' | '\u{3}'))
        .collect();
    let collapsed = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut segments: Vec<MatchSegment> = Vec::new();
    let mut push = |text: &str, hit: bool| {
        // A stray end marker (outside a hit) is dropped like any control.
        let text = text.replace('\u{3}', "");
        if text.is_empty() {
            return;
        }
        match segments.last_mut() {
            Some(last) if last.hit == hit => last.text.push_str(&text),
            _ => segments.push(MatchSegment { text, hit }),
        }
    };
    let mut pieces = collapsed.split('\u{2}');
    push(pieces.next().unwrap_or_default(), false);
    for piece in pieces {
        let (hit, rest) = piece.split_once('\u{3}').unwrap_or((piece, ""));
        push(hit, true);
        push(rest, false);
    }
    segments
        .iter()
        .any(|s| s.hit)
        .then_some(ListMatch { segments })
}

// ---- indexing -------------------------------------------------------------

/// Why indexing a parcel failed. `Display` is value-free: it never carries
/// page text, a title, a blob key or a database message.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    // `From` is written out below rather than `#[from]`, which would also
    // make the sqlx error the `source()`: a chain printed in full can quote
    // row values.
    #[error("{}", error_summary(.0))]
    Database(sqlx::Error),
    /// The blob store failed (the error itself is dropped: it can name
    /// objects and buckets, and says nothing actionable here).
    #[error("blob store error")]
    Store,
    /// The entry's blob isn't in the store.
    #[error("the entry page's blob is missing")]
    MissingBlob,
    /// Extraction itself failed (panicked).
    #[error("extracting the page text failed")]
    Extraction,
}

impl From<sqlx::Error> for IndexError {
    fn from(e: sqlx::Error) -> Self {
        IndexError::Database(e)
    }
}

/// Extract `bytes` (the entry page `entry` of `version`) and store the text.
pub async fn index_bytes(
    db: &PgPool,
    id: &str,
    version: i32,
    entry: &str,
    bytes: Bytes,
) -> Result<bool, IndexError> {
    let entry = entry.to_string();
    // Extraction is CPU work over up to a whole entry page: off the runtime.
    let text = tokio::task::spawn_blocking(move || extract_text(&entry, &bytes))
        .await
        .map_err(|_| IndexError::Extraction)?;
    Ok(search::store_text(db, id, version, &text).await?)
}

/// Index `version` of a parcel from what is stored: its entry's blob. Returns
/// `false` when there was nothing to do (the parcel or version is gone, or
/// it has moved on, or the text is already there).
pub async fn index_version(state: &AppState, id: &str, version: i32) -> Result<bool, IndexError> {
    match search::pending(&state.db, id, version).await? {
        Some(pending) => index_pending(state, &pending).await,
        None => Ok(false),
    }
}

async fn index_pending(state: &AppState, p: &Pending) -> Result<bool, IndexError> {
    let bytes = crate::content::peek_blob(state, &p.blob_sha256)
        .await
        .map_err(|_| IndexError::Store)?
        .ok_or(IndexError::MissingBlob)?;
    index_bytes(&state.db, &p.id, p.version, &p.entry, bytes).await
}

/// Log a failed indexing and count it against `version` (the backfill stops
/// trying after [`MAX_ATTEMPTS`]); passes `result` through.
async fn counted(
    state: &AppState,
    id: &str,
    version: i32,
    result: Result<bool, IndexError>,
) -> Result<bool, IndexError> {
    if let Err(e) = &result {
        tracing::warn!(parcel = %id, version, error = %e, "indexing a parcel's page text failed");
        if let Err(e) = search::record_failure(&state.db, id, version).await {
            tracing::warn!(parcel = %id, version, error = %error_summary(&e), "recording an indexing failure failed");
        }
    }
    result
}

/// After a publish commits: index the new version in the background.
/// `entry` is the entry page's path and bytes when the upload carried it;
/// otherwise (a merge that kept it, a restore) it is read from the store.
pub fn spawn_index(state: AppState, id: String, version: i32, entry: Option<(String, Bytes)>) {
    // One per committed version: tests count these to prove a retried
    // publish dispatches once.
    tracing::debug!(parcel = %id, version, "indexing a published version");
    let tasks = state.index_tasks.clone();
    tasks.spawn(async move {
        let result = match entry {
            Some((entry, bytes)) => index_bytes(&state.db, &id, version, &entry, bytes).await,
            None => index_version(&state, &id, version).await,
        };
        // Logged inside; the backfill retries a failure.
        let _ = counted(&state, &id, version, result).await;
    });
}

/// Wait until every index task [`spawn_index`] started has finished. Tests
/// call it before they inspect or rewrite search state: a task still in
/// flight could otherwise store its text after they look.
pub async fn wait_for_index_tasks(state: &AppState) {
    // `wait` returns only once the tracker is closed and empty; closing
    // doesn't stop new spawns, and reopening restores the idle state.
    state.index_tasks.close();
    state.index_tasks.wait().await;
    state.index_tasks.reopen();
}

/// What a [`drain`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DrainReport {
    pub indexed: u64,
    pub failed: u64,
}

/// Index every parcel still missing its page text and tried fewer than
/// `max_attempts` times, in id order, until done or `deadline`. Each parcel
/// is tried at most once per call.
pub async fn drain(
    state: &AppState,
    deadline: Option<Instant>,
    max_attempts: i16,
) -> Result<DrainReport, IndexError> {
    let mut report = DrainReport::default();
    let mut after = String::new();
    loop {
        let batch = search::pending_batch(&state.db, max_attempts, &after, BATCH).await?;
        let full = batch.len() as i64 == BATCH;
        for pending in batch {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Ok(report);
            }
            let result = index_pending(state, &pending).await;
            match counted(state, &pending.id, pending.version, result).await {
                Ok(true) => report.indexed += 1,
                Ok(false) => {}
                Err(_) => report.failed += 1,
            }
            after = pending.id;
        }
        if !full {
            return Ok(report);
        }
    }
}

/// The background backfill: every minute, index what is missing for up to
/// 20 seconds. Independent of GC's (hourly) cadence, so a parcel published
/// by an older revision mid-rollout becomes searchable within a minute or so.
pub fn spawn_backfill(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(BACKFILL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately; skip it, so a starting instance
        // isn't fetching and extracting pages while it takes its first traffic.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match drain(&state, Some(Instant::now() + BACKFILL_BUDGET), MAX_ATTEMPTS).await {
                Ok(r) if r.indexed > 0 || r.failed > 0 => {
                    tracing::info!(indexed = r.indexed, failed = r.failed, "search backfill");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "search backfill failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(m: &ListMatch) -> String {
        m.segments.iter().map(|s| s.text.as_str()).collect()
    }

    fn hits(m: &ListMatch) -> Vec<&str> {
        m.segments
            .iter()
            .filter(|s| s.hit)
            .map(|s| s.text.as_str())
            .collect()
    }

    #[test]
    fn html_drops_markup_and_hidden_content() {
        let page = "<!doctype html><html><head><title>Q3 &amp; Q4</title>\
            <style>.x{color:red}</style><script>var secret = 'nope';</script></head>\
            <body><h1>Revenue</h1><p>Grew <b>12%</b> in&nbsp;Q3.</p><!-- hidden note -->\
            <template><p>template text</p></template><noscript>enable js</noscript>\
            <svg><text>chart label</text></svg><ul><li>one</li><li>two</li></ul>\
            <SCRIPT type=module>alert(1)</SCRIPT >after</body></html>";
        let t = html_text(page);
        assert_eq!(t, "Q3 & Q4\nRevenue\nGrew 12% in Q3.\none\ntwo\nafter");
        for hidden in [
            "secret", "color", "hidden", "template", "enable", "chart", "alert",
        ] {
            assert!(!t.contains(hidden), "{hidden} leaked into {t:?}");
        }
    }

    #[test]
    fn html_edge_cases() {
        assert_eq!(html_text("a < b and c > d"), "a < b and c > d");
        assert_eq!(html_text(r#"<a title="x > y">link</a>"#), "link");
        assert_eq!(html_text("<p>unterminated <!-- comment"), "unterminated");
        assert_eq!(html_text("<script>never closed"), "");
        assert_eq!(html_text("<svg/>after"), "after");
        assert_eq!(html_text("<scripted>kept</scripted>"), "kept");
        assert_eq!(html_text("<script>x</scriptx>y</script>z"), "z");
        assert_eq!(
            html_text("&#x1F600; &#128512; &bogus; &amp &#0; &#xD800;"),
            "😀 😀 &bogus; &amp \u{fffd} \u{fffd}"
        );
        assert_eq!(
            html_text("e\u{301}t\u{e9} caf\u{e9}"),
            "e\u{301}t\u{e9} caf\u{e9}"
        );
        assert_eq!(
            html_text(&String::from_utf8_lossy(b"ok \xff\xfe bytes")),
            "ok \u{fffd}\u{fffd} bytes"
        );
    }

    #[test]
    fn control_characters_are_stripped_after_entity_decoding() {
        // Encoded snippet markers and an ANSI escape: decoded, then dropped.
        let t = html_text("<p>a&#2;fake&#3;b \u{2}raw\u{3} \u{1b}[31mred&#x1b;</p>");
        assert_eq!(t, "afakeb raw [31mred");
        assert!(!t.chars().any(|c| c.is_control() && c != '\n'), "{t:?}");
        let md = markdown_text("# T\u{2}itle\n\nbody &#3; `co\u{7}de`");
        assert!(!md.chars().any(|c| c.is_control() && c != '\n'), "{md:?}");
    }

    #[test]
    fn markdown_keeps_text_and_code() {
        let t = markdown_text(
            "# Plan\n\nShip **tags** and `search`.\n\n- one\n- two\n\n<script>x()</script>\n",
        );
        assert_eq!(t, "Plan\nShip tags and search .\none\ntwo");
        assert_eq!(extract_text("notes.md", b"# Hi &amp; bye"), "Hi & bye");
        assert_eq!(
            extract_text("index.html", b"<h1>Hi &amp; bye</h1>"),
            "Hi & bye"
        );
    }

    #[test]
    fn text_is_capped_at_a_character_boundary() {
        let big = "é".repeat(MAX_SEARCH_TEXT_BYTES);
        let t = html_text(&format!("<p>{big}</p>"));
        assert!(t.len() <= MAX_SEARCH_TEXT_BYTES && t.len() > MAX_SEARCH_TEXT_BYTES - 4);
        assert!(t.chars().all(|c| c == 'é'));
        let words = "word ".repeat(MAX_SEARCH_TEXT_BYTES);
        assert!(markdown_text(&words).len() <= MAX_SEARCH_TEXT_BYTES);
    }

    #[test]
    fn the_cap_applies_to_normalized_text() {
        // Whitespace padding past twice the cap still leaves the words.
        let padded = format!("{}<p>needle</p>", " ".repeat(262_145));
        assert_eq!(html_text(&padded), "needle");
        let markup = format!("{}<p>needle</p>", "<b></b>\n".repeat(MAX_SEARCH_TEXT_BYTES));
        assert_eq!(html_text(&markup), "needle");
        let md = format!("```\n{}\n```\n\nneedle\n", " ".repeat(262_145));
        assert_eq!(markdown_text(&md), "needle");
        // A separator that doesn't fit isn't left dangling at the end.
        let exact = format!("{} b", "a".repeat(MAX_SEARCH_TEXT_BYTES));
        assert_eq!(html_text(&exact).len(), MAX_SEARCH_TEXT_BYTES);
        let near = format!("{} b", "a".repeat(MAX_SEARCH_TEXT_BYTES - 1));
        assert!(!html_text(&near).ends_with(' '));
    }

    #[test]
    fn nested_skipped_elements_stay_hidden() {
        assert_eq!(
            html_text("<template><template>x</template>hiddenneedle</template><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_text("<svg><svg><text>x</text></svg>hiddenneedle</svg><p>visible</p>"),
            "visible"
        );
        assert_eq!(
            html_text("<SVG><g><Svg>a</sVg>b</g></svg >visible"),
            "visible"
        );
        // A nested self-closing <svg/> opens nothing; <template/> still opens.
        assert_eq!(html_text("<svg><svg/>hidden</svg>visible"), "visible");
        assert_eq!(html_text("<template/>hidden</template>visible"), "visible");
        // End tags inside comments, CDATA or a template's script don't count.
        assert_eq!(
            html_text("<template><!-- </template> -->hidden</template>visible"),
            "visible"
        );
        assert_eq!(
            html_text("<template><script>'</template>'</script>hidden</template>visible"),
            "visible"
        );
        assert_eq!(
            html_text("<svg><![CDATA[</svg>]]>hidden</svg>visible"),
            "visible"
        );
        // Raw-text elements end at the first matching end tag, as a browser
        // parses them.
        assert_eq!(
            html_text("<script><script>x</script>shown</script>"),
            "shown"
        );
        assert_eq!(
            html_text("<noscript><noscript>x</noscript>shown</noscript>"),
            "shown"
        );
        assert_eq!(html_text("<template><template>x</template>"), "");
    }

    #[test]
    fn segments_mark_hits_and_stay_plain_text() {
        let m = segments("the quick \u{2}brown\u{3} fox jumps over \u{2}brown\u{3}  dogs").unwrap();
        assert_eq!(hits(&m), ["brown", "brown"]);
        assert_eq!(text(&m), "the quick brown fox jumps over brown dogs");
        assert!(!m.segments[0].hit);

        let m = segments("\u{2}Start\u{3} of\n\tthe page").unwrap();
        assert_eq!(
            m.segments[0],
            MatchSegment {
                text: "Start".into(),
                hit: true
            }
        );
        assert_eq!(text(&m), "Start of the page");

        // Hostile text stays text: no HTML is produced or interpreted.
        let m = segments("<img src=x onerror=alert(1)> \u{2}needle\u{3} \u{1b}[2J 👩‍💻 e\u{301}")
            .unwrap();
        assert_eq!(hits(&m), ["needle"]);
        assert_eq!(
            text(&m),
            "<img src=x onerror=alert(1)> needle [2J 👩‍💻 e\u{301}"
        );
        assert!(m
            .segments
            .iter()
            .all(|s| !s.text.chars().any(char::is_control)));

        assert_eq!(segments("no markers here"), None);
        assert_eq!(segments("\u{2}\u{3} empty mark"), None);
    }
}
