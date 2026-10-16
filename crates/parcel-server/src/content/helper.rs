//! The page helper (`assets/parcel-helper.js`): a script the content origin
//! adds to every HTML page it serves. It makes storage APIs safe in the
//! sandbox, exposes `window.parcel.data`, and bridges comment mode with the
//! viewer. See the script's header for what it does and doesn't touch.
//!
//! The script is served from a fixed, content-hashed URL on the content
//! origin, so pages fetch it once and cache it; the hash also goes into the
//! ETag of every page it is added to, so upgrading it revalidates pages.

use bytes::Bytes;
use sha2::{Digest, Sha256};

const SOURCE: &str = include_str!("../../assets/parcel-helper.js");
const ORIGIN_PLACEHOLDER: &str = "\"__PARCEL_APP_ORIGIN__\"";

pub struct Helper {
    /// The script, with the app origin filled in.
    pub js: Bytes,
    /// First 12 hex digits of the script's SHA-256.
    pub hash: String,
    /// The `<script>` tag pages get.
    tag: String,
}

impl Helper {
    pub fn new(app_origin: &str) -> Self {
        let origin = serde_json::to_string(app_origin).expect("a string serializes");
        let js = SOURCE.replace(ORIGIN_PLACEHOLDER, &origin);
        let hash = hex::encode(Sha256::digest(js.as_bytes()))[..12].to_string();
        let tag = format!("<script src=\"/_parcel/helper.{hash}.js\"></script>");
        Self {
            js: Bytes::from(js),
            hash,
            tag,
        }
    }

    /// Add the script tag to an HTML document, as early as possible so the
    /// storage stand-ins exist before any page script runs: right after
    /// `<head …>`, else after `<html …>`, else after the doctype and any
    /// leading comments. Never before the doctype, which would drop the page
    /// into quirks mode.
    ///
    /// This is a scan, not a parser: it skips `<!-- … -->` comments and
    /// quoted attribute values, which covers what real pages put before
    /// `<head>`. If it is fooled, the script lands somewhere inert and the
    /// page merely runs without the helper.
    pub fn inject(&self, html: &[u8]) -> Vec<u8> {
        let at = after_open_tag(html, b"head")
            .or_else(|| after_open_tag(html, b"html"))
            .unwrap_or_else(|| prologue_end(html));
        let mut out = Vec::with_capacity(html.len() + self.tag.len());
        out.extend_from_slice(&html[..at]);
        out.extend_from_slice(self.tag.as_bytes());
        out.extend_from_slice(&html[at..]);
        out
    }
}

/// The index just past the first `<name …>` open tag (ASCII
/// case-insensitive) outside comments, where `name` is followed by `>`, `/`
/// or whitespace so `<header>` doesn't count as `<head>`.
fn after_open_tag(html: &[u8], name: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < html.len() {
        if html[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with(b"<!--") {
            // An unterminated comment runs to the end of the document.
            i = find(&html[i + 4..], b"-->").map(|j| i + 4 + j + 3)?;
            continue;
        }
        let tag_end = i + 1 + name.len();
        if html.len() > tag_end
            && html[i + 1..tag_end].eq_ignore_ascii_case(name)
            && matches!(
                html[tag_end],
                b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'
            )
        {
            return end_of_tag(html, tag_end);
        }
        i += 1;
    }
    None
}

/// The index just past the `>` that closes a tag whose attributes start at
/// `from`. A quote opens a value only right after `=`, as in HTML.
fn end_of_tag(html: &[u8], from: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut after_equals = false;
    for (offset, &b) in html[from..].iter().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            continue;
        }
        match b {
            b'>' => return Some(from + offset + 1),
            b'"' | b'\'' if after_equals => {
                quote = Some(b);
                after_equals = false;
            }
            b'=' => after_equals = true,
            b if b.is_ascii_whitespace() => {}
            _ => after_equals = false,
        }
    }
    None
}

/// Past a byte-order mark, whitespace, comments and the doctype: the first
/// place content may go.
fn prologue_end(html: &[u8]) -> usize {
    let mut i = if html.starts_with(b"\xEF\xBB\xBF") {
        3
    } else {
        0
    };
    loop {
        while i < html.len() && html[i].is_ascii_whitespace() {
            i += 1;
        }
        let rest = &html[i..];
        let end = if rest.starts_with(b"<!--") {
            find(&rest[4..], b"-->").map(|j| 4 + j + 3)
        } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case(b"<!doctype") {
            find(rest, b">").map(|j| j + 1)
        } else {
            None
        };
        match end {
            Some(len) => i += len,
            None => return i,
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn injected(html: &str) -> String {
        let helper = Helper::new("https://app.test");
        String::from_utf8(helper.inject(html.as_bytes()))
            .unwrap()
            .replace(&helper.tag, "@")
    }

    #[test]
    fn goes_right_after_head() {
        assert_eq!(
            injected("<!doctype html><html lang=en><HEAD data-x=\"1\"><title>t</title>"),
            "<!doctype html><html lang=en><HEAD data-x=\"1\">@<title>t</title>"
        );
        assert_eq!(
            injected("<html><header>x</header><head>"),
            "<html><header>x</header><head>@"
        );
    }

    #[test]
    fn skips_comments_and_quoted_attributes() {
        assert_eq!(
            injected("<!doctype html><!-- <head> --><html><head>x"),
            "<!doctype html><!-- <head> --><html><head>@x"
        );
        assert_eq!(
            injected("<head data-note=\"a > b\" title='c>d'><title>"),
            "<head data-note=\"a > b\" title='c>d'>@<title>"
        );
        assert_eq!(
            injected("<head data-x=a\"b><title>"),
            "<head data-x=a\"b>@<title>"
        );
        assert_eq!(
            injected("<!-- intro -->\n<!DOCTYPE html><!-- c --><p>hi"),
            "<!-- intro -->\n<!DOCTYPE html><!-- c -->@<p>hi"
        );
        // An unterminated comment hides any tag after it; the script goes
        // first, where it still runs.
        assert_eq!(injected("<!-- <head>"), "@<!-- <head>");
    }

    #[test]
    fn falls_back_to_html_then_doctype_then_start() {
        assert_eq!(
            injected("<!DOCTYPE html>\n<html><body>"),
            "<!DOCTYPE html>\n<html>@<body>"
        );
        assert_eq!(injected("<!DOCTYPE html><p>hi"), "<!DOCTYPE html>@<p>hi");
        assert_eq!(injected("<p>hi"), "@<p>hi");
        assert_eq!(injected("\u{feff}<p>hi"), "\u{feff}@<p>hi");
        assert_eq!(injected(""), "@");
    }

    #[test]
    fn bakes_in_the_app_origin_and_hashes_it() {
        let a = Helper::new("https://a.test");
        let b = Helper::new("https://b.test");
        let js = std::str::from_utf8(&a.js).unwrap();
        assert!(js.contains("const APP_ORIGIN = \"https://a.test\";"));
        assert!(!js.contains("__PARCEL_APP_ORIGIN__"));
        assert_ne!(a.hash, b.hash);
        assert_eq!(a.hash.len(), 12);
    }
}
