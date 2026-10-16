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
    /// `<head …>`, else after `<html …>`, else after the doctype, else first.
    /// Never before the doctype, which would drop the page into quirks mode.
    pub fn inject(&self, html: &[u8]) -> Vec<u8> {
        let at = after_open_tag(html, b"head")
            .or_else(|| after_open_tag(html, b"html"))
            .or_else(|| after_doctype(html))
            .unwrap_or_else(|| {
                if html.starts_with(b"\xEF\xBB\xBF") {
                    3
                } else {
                    0
                }
            });
        let mut out = Vec::with_capacity(html.len() + self.tag.len());
        out.extend_from_slice(&html[..at]);
        out.extend_from_slice(self.tag.as_bytes());
        out.extend_from_slice(&html[at..]);
        out
    }
}

/// The index just past the first `<name …>` open tag (ASCII
/// case-insensitive), where `name` is followed by `>`, `/` or whitespace so
/// `<header>` doesn't count as `<head>`.
fn after_open_tag(html: &[u8], name: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(lt) = find(&html[from..], b"<").map(|i| from + i) {
        let tag_start = lt + 1;
        let tag_end = tag_start + name.len();
        if html.len() > tag_end
            && html[tag_start..tag_end].eq_ignore_ascii_case(name)
            && matches!(
                html[tag_end],
                b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'
            )
        {
            return find(&html[tag_end..], b">").map(|i| tag_end + i + 1);
        }
        from = tag_start;
    }
    None
}

fn after_doctype(html: &[u8]) -> Option<usize> {
    let start = html
        .iter()
        .position(|b| !b.is_ascii_whitespace() && !matches!(b, 0xEF | 0xBB | 0xBF))?;
    let rest = &html[start..];
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case(b"<!doctype") {
        find(rest, b">").map(|i| start + i + 1)
    } else {
        None
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
