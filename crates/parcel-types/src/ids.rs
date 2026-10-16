//! Parcel identifiers.
//!
//! A parcel id is 12 characters from a 32-letter lowercase alphabet with the
//! easily-confused glyphs (`0`, `o`, `1`, `l`) removed: short enough to read
//! aloud, URL- and DNS-label-safe (so a later per-parcel subdomain needs no new
//! id scheme), and ~60 bits of entropy so ids are not guessable.

/// The id alphabet. 32 symbols, so each random byte maps without bias via `% 32`.
pub const ID_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// Length of every parcel id.
pub const ID_LEN: usize = 12;

/// Whether `s` is shaped like a parcel id. Says nothing about existence.
pub fn is_valid_parcel_id(s: &str) -> bool {
    s.len() == ID_LEN && s.bytes().all(|b| ID_ALPHABET.contains(&b))
}

/// Build an id from `ID_LEN` random bytes (the caller owns the RNG, which
/// keeps this crate free of one).
pub fn parcel_id_from_bytes(bytes: [u8; ID_LEN]) -> String {
    bytes
        .iter()
        .map(|b| ID_ALPHABET[(*b as usize) % ID_ALPHABET.len()] as char)
        .collect()
}

/// Extract a parcel id from either a bare id or a viewer URL such as
/// `https://parcel.stridelabs.ai/p/abcd…` (optionally followed by
/// `/v/<n>`). Returns the id and the version, if the URL pinned one.
pub fn parse_parcel_ref(input: &str) -> Option<(String, Option<i32>)> {
    let trimmed = input.trim().trim_end_matches('/');
    if is_valid_parcel_id(trimmed) {
        return Some((trimmed.to_string(), None));
    }
    // Drop scheme + authority if present, plus any query or fragment.
    let without_query = trimmed.split(['?', '#']).next().unwrap_or("");
    let path = match without_query.find("://") {
        Some(idx) => {
            let rest = &without_query[idx + 3..];
            rest.find('/').map(|i| &rest[i..]).unwrap_or("")
        }
        None => without_query,
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let pos = segments.iter().position(|s| *s == "p")?;
    let id = segments.get(pos + 1)?;
    if !is_valid_parcel_id(id) {
        return None;
    }
    let version = match (segments.get(pos + 2), segments.get(pos + 3)) {
        (Some(&"v"), Some(v)) => v.parse::<i32>().ok(),
        _ => None,
    };
    Some((id.to_string(), version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_bytes_are_valid() {
        let id = parcel_id_from_bytes([0, 31, 32, 255, 7, 8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(id.len(), ID_LEN);
        assert!(is_valid_parcel_id(&id));
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(!is_valid_parcel_id("short"));
        assert!(!is_valid_parcel_id("abcdefghijk0")); // '0' is not in the alphabet
        assert!(!is_valid_parcel_id("ABCDEFGHIJKM"));
    }

    #[test]
    fn parses_refs() {
        assert_eq!(
            parse_parcel_ref("abcdefghijkm"),
            Some(("abcdefghijkm".into(), None))
        );
        assert_eq!(
            parse_parcel_ref("https://parcel.stridelabs.ai/p/abcdefghijkm"),
            Some(("abcdefghijkm".into(), None))
        );
        assert_eq!(
            parse_parcel_ref("https://parcel.stridelabs.ai/p/abcdefghijkm/v/3?x=1"),
            Some(("abcdefghijkm".into(), Some(3)))
        );
        assert_eq!(
            parse_parcel_ref("/p/abcdefghijkm/"),
            Some(("abcdefghijkm".into(), None))
        );
        assert_eq!(parse_parcel_ref("https://example.com/x/abcdefghijkm"), None);
    }
}
