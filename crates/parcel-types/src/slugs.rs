//! Handles and slugs: the parts of a parcel's clean URL, `/@<handle>/<slug>`.
//!
//! A *handle* names a user (`ada`, `ada-2`); a *slug* names one of their
//! parcels (`parcel/plan-001`). The server checks both with these functions,
//! and the CLI and the web app apply them before sending.
//!
//! A slug never looks like anything a parcel id URL is made of: no segment
//! is shaped like a parcel id, and the segments `p` and `v` are refused, so a
//! parser that predates slugs (the 0.1.2 CLI, an agent's skill) can never
//! read a clean URL as `/p/<id>` or a `/v/<n>` version pin. A clean URL pins
//! a version with `?v=<n>` instead.

use crate::ids::is_valid_parcel_id;
use crate::tags::{is_space, quoted};

/// Longest slug, in characters.
pub const MAX_SLUG_CHARS: usize = 100;
/// Most `/`-separated segments in a slug.
pub const MAX_SLUG_SEGMENTS: usize = 4;
/// Longest handle, in characters.
pub const MAX_HANDLE_CHARS: usize = 32;

/// Why a slug or handle was refused. Safe to show: the value is quoted with
/// `{:?}` (control characters escaped) and cut short.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlugError {
    #[error(
        "slug {0} is not valid: use 1–{MAX_SLUG_SEGMENTS} '/'-separated segments of lowercase \
         letters, digits and single '-' (at most {MAX_SLUG_CHARS} characters); a segment may \
         not be 'p', 'v' or shaped like a parcel id"
    )]
    Slug(String),
    #[error(
        "handle {0} is not valid: use 1–{MAX_HANDLE_CHARS} lowercase letters, digits and '-', \
         starting and ending with a letter or digit"
    )]
    Handle(String),
}

/// ASCII-trimmed and ASCII-lowercased (anything else fails the shape check
/// anyway, and an ASCII-only rule is one the TypeScript twin matches exactly).
fn folded(raw: &str) -> String {
    raw.trim_matches(is_space).to_ascii_lowercase()
}

/// `[a-z0-9]+(-[a-z0-9]+)*`
fn word_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Normalize a slug (trim, lowercase) and check it: 1–100 characters, 1–4
/// `/`-separated segments of `[a-z0-9]+(-[a-z0-9]+)*`, none of them `p`,
/// `v` or shaped like a parcel id.
pub fn normalize_slug(raw: &str) -> Result<String, SlugError> {
    let slug = folded(raw);
    let segments: Vec<&str> = slug.split('/').collect();
    let ok = !slug.is_empty()
        && slug.len() <= MAX_SLUG_CHARS
        && segments.len() <= MAX_SLUG_SEGMENTS
        && segments
            .iter()
            .all(|s| word_segment(s) && *s != "p" && *s != "v" && !is_valid_parcel_id(s));
    if ok {
        Ok(slug)
    } else {
        Err(SlugError::Slug(quoted(raw)))
    }
}

/// Normalize a handle (trim, lowercase) and check it against
/// `^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$`.
pub fn normalize_handle(raw: &str) -> Result<String, SlugError> {
    let handle = folded(raw);
    let bytes = handle.as_bytes();
    let alnum = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let ok = !bytes.is_empty()
        && bytes.len() <= MAX_HANDLE_CHARS
        && bytes.first().is_some_and(alnum)
        && bytes.last().is_some_and(alnum)
        && bytes.iter().all(|b| alnum(b) || *b == b'-');
    if ok {
        Ok(handle)
    } else {
        Err(SlugError::Handle(quoted(raw)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_normalize_and_follow_the_rules() {
        for (raw, expected) in [
            ("plan", "plan"),
            ("  Parcel/Plan-001 ", "parcel/plan-001"),
            ("a/b/c/d", "a/b/c/d"),
            ("q3-2026", "q3-2026"),
            ("x", "x"),
            // Ten or fourteen id-alphabet characters aren't an id.
            ("management", "management"),
        ] {
            assert_eq!(normalize_slug(raw).as_deref(), Ok(expected), "{raw:?}");
        }
        let long = "a".repeat(MAX_SLUG_CHARS);
        assert!(normalize_slug(&long).is_ok());
        for bad in [
            "",
            " ",
            "a/b/c/d/e",
            &"a".repeat(MAX_SLUG_CHARS + 1),
            "/plan",
            "plan/",
            "a//b",
            "-plan",
            "plan-",
            "pl--an",
            "pl_an",
            "pl an",
            "plän",
            "pl.an",
            "plan?v=2",
            "@ada/plan",
            // What a /p/<id> or /v/<n> URL is made of.
            "p",
            "v",
            "p/abcdefghijkm",
            "plans/v/2",
            "abcdefghijkm",
            "plans/abcdefghijkm",
            "ABCDEFGHIJKM",
        ] {
            assert!(
                matches!(normalize_slug(bad), Err(SlugError::Slug(_))),
                "{bad:?} should be refused"
            );
        }
        // Segments that only contain a "p" or "v" are fine.
        assert!(normalize_slug("vp/pv/p2/v-1").is_ok());
    }

    #[test]
    fn handles_normalize_and_follow_the_rules() {
        for (raw, expected) in [
            ("Ada", "ada"),
            ("a", "a"),
            (" ada-2 ", "ada-2"),
            ("a--b", "a--b"),
        ] {
            assert_eq!(normalize_handle(raw).as_deref(), Ok(expected), "{raw:?}");
        }
        assert!(normalize_handle(&"a".repeat(32)).is_ok());
        for bad in [
            "",
            "-ada",
            "ada-",
            "a.b",
            "a_b",
            "a/b",
            "@ada",
            "adà",
            &"a".repeat(33),
        ] {
            assert!(
                matches!(normalize_handle(bad), Err(SlugError::Handle(_))),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn errors_quote_safely() {
        let message = normalize_slug("bad\u{1b}[31m").unwrap_err().to_string();
        assert!(message.contains(r#""bad\u{1b}[31m""#), "{message}");
        let message = normalize_handle(&"!".repeat(500)).unwrap_err().to_string();
        assert!(message.len() < 400, "{message}");
    }
}
