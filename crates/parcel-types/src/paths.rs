//! Published-path rules.
//!
//! A published path is what a parcel's HTML references (`assets/app.js`), so it
//! must be relative, normalized and unambiguous: no leading slash, no `.`/`..`
//! segments, no backslashes, no empty segments, no control characters. The
//! server enforces these on every upload; the CLI checks them first so a bad
//! publish fails before any bytes move.

use thiserror::Error;

/// Longest accepted published path, in bytes.
pub const MAX_PATH_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    #[error("path is empty")]
    Empty,
    #[error("path is longer than {MAX_PATH_LEN} bytes")]
    TooLong,
    #[error("path must be relative (no leading '/')")]
    Absolute,
    #[error("path must use '/' separators, not '\\'")]
    Backslash,
    #[error("path contains a control character")]
    ControlChar,
    #[error("path contains an empty segment ('//' or a trailing '/')")]
    EmptySegment,
    #[error("path contains a '.' or '..' segment")]
    DotSegment,
}

/// Validate a published path. Returns `Ok(())` when it may be stored as-is.
pub fn validate_path(path: &str) -> Result<(), PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH_LEN {
        return Err(PathError::TooLong);
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    if path.chars().any(char::is_control) {
        return Err(PathError::ControlChar);
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(PathError::EmptySegment);
        }
        if segment == "." || segment == ".." {
            return Err(PathError::DotSegment);
        }
    }
    Ok(())
}

/// The final segment of a path (`assets/app.js` -> `app.js`).
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_paths() {
        for p in ["index.html", "assets/app.js", "a/b/c/d.png", "My File (1).html"] {
            assert_eq!(validate_path(p), Ok(()), "{p}");
        }
    }

    #[test]
    fn rejects_bad_paths() {
        assert_eq!(validate_path(""), Err(PathError::Empty));
        assert_eq!(validate_path("/etc/passwd"), Err(PathError::Absolute));
        assert_eq!(validate_path("a\\b"), Err(PathError::Backslash));
        assert_eq!(validate_path("a//b"), Err(PathError::EmptySegment));
        assert_eq!(validate_path("a/"), Err(PathError::EmptySegment));
        assert_eq!(validate_path("../x"), Err(PathError::DotSegment));
        assert_eq!(validate_path("a/./b"), Err(PathError::DotSegment));
        assert_eq!(validate_path("a\nb"), Err(PathError::ControlChar));
        assert_eq!(validate_path(&"a".repeat(513)), Err(PathError::TooLong));
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("assets/app.js"), "app.js");
        assert_eq!(file_name("index.html"), "index.html");
    }
}
