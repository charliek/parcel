//! Tags: free-form labels on a parcel (`plan`, `project:parcel`, `kind:report`).
//!
//! The server normalizes every tag it stores with these functions, and the
//! CLI and the web app (`frontend/src/lib/tags.ts`, a twin tested against the
//! same cases in `tests/fixtures/tag-cases.json`) apply them before sending,
//! so all three agree on what a tag is. A tag that doesn't normalize is
//! refused, never silently dropped.
//!
//! Only ASCII is folded and trimmed: anything else fails the shape check
//! anyway, and an exact-ASCII rule is one the TypeScript twin can match
//! byte for byte.

use std::collections::BTreeSet;

/// Most tags one parcel may carry.
pub const MAX_TAGS: usize = 20;
/// Longest tag, after normalization.
pub const MAX_TAG_CHARS: usize = 64;

/// Why a tag (or a set of tags) was refused. The message is safe to show:
/// the offending tag is quoted with `{:?}` (control characters escaped) and
/// cut short.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    #[error(
        "tag {0} is not valid: use lowercase letters, digits, '.', '+' and '-', optionally \
         as namespace:value (the value may also hold '/'), at most {MAX_TAG_CHARS} characters"
    )]
    Invalid(String),
    #[error("a parcel may have at most {MAX_TAGS} tags (this would be {0})")]
    TooMany(usize),
}

/// `raw` quoted with `{:?}` (control characters escaped) and cut short, for
/// error messages (slugs and handles use it too).
pub(crate) fn quoted(raw: &str) -> String {
    let shown: String = raw.chars().take(80).collect();
    if shown.len() < raw.len() {
        format!("{shown:?}…")
    } else {
        format!("{shown:?}")
    }
}

/// The whitespace a tag (or slug, or handle) may be padded with (ASCII only,
/// so the TypeScript twins match exactly).
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Normalize one tag: trim, lowercase, turn each run of whitespace and `_`
/// into `-`, then require
/// `^[a-z0-9][a-z0-9.+-]*(:[a-z0-9][a-z0-9.+/-]*)?$` and at most 64 characters.
pub fn normalize_tag(raw: &str) -> Result<String, TagError> {
    let mut out = String::with_capacity(raw.len());
    let mut in_run = false;
    for c in raw.trim_matches(is_space).chars() {
        if is_space(c) || c == '_' {
            if !in_run {
                out.push('-');
            }
            in_run = true;
        } else {
            out.push(c.to_ascii_lowercase());
            in_run = false;
        }
    }
    if out.len() > MAX_TAG_CHARS || !well_formed(&out) {
        return Err(TagError::Invalid(quoted(raw)));
    }
    Ok(out)
}

fn well_formed(tag: &str) -> bool {
    let (ns, value) = match tag.split_once(':') {
        Some((ns, value)) => (ns, Some(value)),
        None => (tag, None),
    };
    let part = |s: &str, slash: bool| {
        let mut chars = s.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && chars.all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, '.' | '+' | '-')
                    || (slash && c == '/')
            })
    };
    part(ns, false) && value.is_none_or(|v| part(v, true))
}

/// Normalize a set of tags: each one as [`normalize_tag`], duplicates
/// dropped, sorted by bytes (the order Postgres's `COLLATE "C"` gives, so
/// arrays rewritten in SQL come out identical), then held to [`MAX_TAGS`].
pub fn normalize_tags<S: AsRef<str>>(raw: &[S]) -> Result<Vec<String>, TagError> {
    let set = normalize_set(raw)?;
    capped(set)
}

/// Normalized and deduplicated, without the count cap: for the parts of an
/// edit (`add_tags`, `remove_tags`), where only the result is capped.
pub fn normalize_set<S: AsRef<str>>(raw: &[S]) -> Result<BTreeSet<String>, TagError> {
    raw.iter().map(|t| normalize_tag(t.as_ref())).collect()
}

fn capped(set: BTreeSet<String>) -> Result<Vec<String>, TagError> {
    if set.len() > MAX_TAGS {
        return Err(TagError::TooMany(set.len()));
    }
    // A BTreeSet of Strings iterates in byte order.
    Ok(set.into_iter().collect())
}

/// `(current ∪ add) − remove`, sorted and capped. `current` is what is
/// stored (already normalized); `add` and `remove` come from
/// [`normalize_set`].
pub fn edit_tags(
    current: &[String],
    add: &BTreeSet<String>,
    remove: &BTreeSet<String>,
) -> Result<Vec<String>, TagError> {
    let set: BTreeSet<String> = current
        .iter()
        .chain(add)
        .filter(|t| !remove.contains(*t))
        .cloned()
        .collect();
    capped(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/tag-cases.json")).unwrap()
    }

    #[test]
    fn single_tags_follow_the_shared_cases() {
        let cases = cases();
        for pair in cases["valid"].as_array().unwrap() {
            let (input, expected) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
            assert_eq!(normalize_tag(input).as_deref(), Ok(expected), "{input:?}");
        }
        for input in cases["invalid"].as_array().unwrap() {
            let input = input.as_str().unwrap();
            assert!(
                matches!(normalize_tag(input), Err(TagError::Invalid(_))),
                "{input:?} should be refused"
            );
        }
    }

    #[test]
    fn tag_sets_follow_the_shared_cases() {
        for case in cases()["sets"].as_array().unwrap() {
            let input: Vec<String> = serde_json::from_value(case["input"].clone()).unwrap();
            let got = normalize_tags(&input);
            match case["error"].as_str() {
                Some("too_many") => assert!(matches!(got, Err(TagError::TooMany(_))), "{input:?}"),
                Some("invalid") => assert!(matches!(got, Err(TagError::Invalid(_))), "{input:?}"),
                Some(other) => panic!("unknown error kind {other}"),
                None => {
                    let expected: Vec<String> =
                        serde_json::from_value(case["expected"].clone()).unwrap();
                    assert_eq!(got.unwrap(), expected, "{input:?}");
                }
            }
        }
    }

    #[test]
    fn errors_name_the_tag_safely() {
        let message = normalize_tag("bad\u{1b}[31m!").unwrap_err().to_string();
        assert!(message.contains(r#""bad\u{1b}[31m!""#), "{message}");
        let long = "x".repeat(500) + "!";
        let message = normalize_tag(&long).unwrap_err().to_string();
        assert!(message.len() < 400, "{message}");
    }

    #[test]
    fn edits_union_then_remove_then_cap() {
        let current = vec!["a".to_string(), "b".to_string()];
        let add = normalize_set(&["C", "a"]).unwrap();
        let remove = normalize_set(&["b"]).unwrap();
        assert_eq!(edit_tags(&current, &add, &remove).unwrap(), ["a", "c"]);
        let many: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
        let add = normalize_set(&["extra"]).unwrap();
        assert_eq!(
            edit_tags(&many, &add, &BTreeSet::new()),
            Err(TagError::TooMany(21))
        );
        // Removing one makes room for the addition.
        let remove = normalize_set(&["t0"]).unwrap();
        assert_eq!(edit_tags(&many, &add, &remove).unwrap().len(), 20);
    }
}
