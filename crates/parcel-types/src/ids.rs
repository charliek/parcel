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

/// What a parcel reference names: a parcel id, or a slug under a handle
/// (`None` = the caller's own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    Id(String),
    Slug {
        handle: Option<String>,
        slug: String,
    },
}

/// A parcel reference as people and agents write it, with the version,
/// page and fragment it pins (if any):
///
/// * a parcel id: `abcdefghijkm`;
/// * a viewer URL: `https://…/p/<id>[/v/<n>][?page=<path>][#fragment]`;
/// * a clean URL: `https://…/@<handle>/<slug>[?v=<n>][&page=<path>][#fragment]`;
/// * `@<handle>/<slug>`, or a bare slug (the caller's own).
///
/// A bare string of 10–14 id-alphabet characters that isn't an id is
/// refused rather than read as a slug: it is far more likely a mistyped id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParcelRef {
    pub target: RefTarget,
    pub version: Option<i32>,
    /// A page of the version (a published path), from `?page=`.
    pub page: Option<String>,
    /// The URL's `#fragment`, without the `#`.
    pub fragment: Option<String>,
}

impl ParcelRef {
    /// Parse a reference; `None` when it is none of the forms above.
    pub fn parse(input: &str) -> Option<ParcelRef> {
        let trimmed = input.trim();
        let (rest, fragment) = match trimmed.split_once('#') {
            Some((rest, f)) => (rest, (!f.is_empty()).then(|| f.to_string())),
            None => (trimmed, None),
        };
        let (location, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut query_version = None;
        let mut page = None;
        for (key, value) in query.split('&').filter_map(|kv| kv.split_once('=')) {
            match key {
                "v" => query_version = Some(value.parse::<i32>().ok().filter(|v| *v > 0)?),
                "page" => {
                    let path = percent_decode(value)?;
                    crate::paths::validate_path(&path).ok()?;
                    page = Some(path);
                }
                _ => {}
            }
        }
        let located = location.contains("://") || location.starts_with('/');
        // Drop the scheme and authority, if any.
        let path = match location.find("://") {
            Some(i) => {
                let after = &location[i + 3..];
                after.find('/').map_or("", |j| &after[j..])
            }
            None => location,
        };
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let found = |target| {
            Some(ParcelRef {
                target,
                version: query_version,
                page: page.clone(),
                fragment: fragment.clone(),
            })
        };

        // A viewer URL. No slug contains a `p` segment, so this is unambiguous.
        if let Some(pos) = segments.iter().position(|s| *s == "p") {
            let id = segments.get(pos + 1).filter(|id| is_valid_parcel_id(id))?;
            let pinned = match (segments.get(pos + 2), segments.get(pos + 3)) {
                (Some(&"v"), Some(v)) => v.parse::<i32>().ok(),
                _ => None,
            };
            let mut found = found(RefTarget::Id(id.to_string()))?;
            found.version = pinned.or(found.version);
            return Some(found);
        }
        // A clean URL, or `@handle/slug`.
        if let Some(pos) = segments.iter().position(|s| s.starts_with('@')) {
            let handle = crate::slugs::normalize_handle(&segments[pos][1..]).ok()?;
            let slug = crate::slugs::normalize_slug(&segments[pos + 1..].join("/")).ok()?;
            return found(RefTarget::Slug {
                handle: Some(handle),
                slug,
            });
        }
        if located {
            return None;
        }
        let bare = path.trim_end_matches('/');
        if is_valid_parcel_id(bare) {
            return found(RefTarget::Id(bare.to_string()));
        }
        if (ID_LEN - 2..=ID_LEN + 2).contains(&bare.len())
            && bare.bytes().all(|b| ID_ALPHABET.contains(&b))
        {
            return None;
        }
        let slug = crate::slugs::normalize_slug(bare).ok()?;
        found(RefTarget::Slug { handle: None, slug })
    }

    /// What a URL for this reference carries after its path: `?v=<n>`,
    /// `page=<path>` and `#fragment`, so canonicalizing a ref keeps them.
    pub fn url_suffix(&self) -> String {
        let mut params = Vec::new();
        if let Some(v) = self.version {
            params.push(format!("v={v}"));
        }
        if let Some(page) = &self.page {
            params.push(format!("page={}", percent_encode(page)));
        }
        let mut out = String::new();
        if !params.is_empty() {
            out.push('?');
            out.push_str(&params.join("&"));
        }
        if let Some(fragment) = &self.fragment {
            out.push('#');
            out.push_str(fragment);
        }
        out
    }
}

/// `%XX` escapes and `+` (a space, as browsers' `URLSearchParams` writes
/// it); `None` for a broken escape or bytes that aren't UTF-8.
fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Escape everything but unreserved characters and `/`.
fn percent_encode(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Extract a parcel id from either a bare id or a viewer URL such as
/// `https://parcel.stridelabs.ai/p/abcd…` (optionally followed by
/// `/v/<n>`). Returns the id and the version, if the URL pinned one.
/// Slug references are `None` here: new code uses [`ParcelRef::parse`].
///
/// Kept exactly as it was before slugs (query and fragment ignored, a
/// version only from `/v/<n>`) for the callers that still use it, so it
/// doesn't share [`ParcelRef::parse`]'s stricter query handling.
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
        assert_eq!(parse_parcel_ref("my-plan"), None, "slugs aren't ids");
        assert_eq!(parse_parcel_ref("@ada/my-plan"), None);
    }

    /// The wrapper reads what it read before slugs: query text is ignored
    /// (never validated), and a version comes only from `/v/<n>`.
    #[test]
    fn the_wrapper_keeps_its_old_reading() {
        let id = "abcdefghijkm";
        for (input, version) in [
            ("/p/abcdefghijkm?v=garbage", None),
            ("/p/abcdefghijkm?v=3", None),
            ("/p/abcdefghijkm?page=%zz", None),
            ("/p/abcdefghijkm/v/2?page=../x", Some(2)),
            ("https://x.test/p/abcdefghijkm/v/2#frag", Some(2)),
            ("abcdefghijkm/", None),
            ("p/abcdefghijkm", None),
        ] {
            assert_eq!(
                parse_parcel_ref(input),
                Some((id.to_string(), version)),
                "{input:?}"
            );
        }
        assert_eq!(parse_parcel_ref("abcdefghijkm?v=3"), None, "as before");
    }

    fn id_ref(id: &str, version: Option<i32>) -> ParcelRef {
        ParcelRef {
            target: RefTarget::Id(id.into()),
            version,
            page: None,
            fragment: None,
        }
    }

    fn slug_ref(handle: Option<&str>, slug: &str) -> ParcelRef {
        ParcelRef {
            target: RefTarget::Slug {
                handle: handle.map(String::from),
                slug: slug.into(),
            },
            version: None,
            page: None,
            fragment: None,
        }
    }

    #[test]
    fn parses_every_ref_form() {
        let id = "abcdefghijkm";
        assert_eq!(ParcelRef::parse(id), Some(id_ref(id, None)));
        assert_eq!(
            ParcelRef::parse(&format!("https://x.test/p/{id}/v/3")),
            Some(id_ref(id, Some(3)))
        );
        assert_eq!(
            ParcelRef::parse(&format!("p/{id}")),
            Some(id_ref(id, None)),
            "as the 0.1.2 parser read it"
        );
        let with_page =
            ParcelRef::parse(&format!("https://x.test/p/{id}?page=docs%2Fa+b.html#s-2")).unwrap();
        assert_eq!(with_page.page.as_deref(), Some("docs/a b.html"));
        assert_eq!(with_page.fragment.as_deref(), Some("s-2"));

        assert_eq!(
            ParcelRef::parse("https://x.test/@Ada/parcel/plan-001"),
            Some(slug_ref(Some("ada"), "parcel/plan-001"))
        );
        assert_eq!(
            ParcelRef::parse("@ada/plan"),
            Some(slug_ref(Some("ada"), "plan"))
        );
        assert_eq!(
            ParcelRef::parse("parcel/plan-001"),
            Some(slug_ref(None, "parcel/plan-001"))
        );
        assert_eq!(ParcelRef::parse(" Plan "), Some(slug_ref(None, "plan")));
        let pinned = ParcelRef::parse("/@ada/plan?v=2&page=b.md&x=1#top").unwrap();
        assert_eq!(pinned.version, Some(2));
        assert_eq!(pinned.page.as_deref(), Some("b.md"));
        assert_eq!(pinned.fragment.as_deref(), Some("top"));
        assert_eq!(pinned.url_suffix(), "?v=2&page=b.md#top");
        assert_eq!(
            ParcelRef::parse(&format!("{id}?page=a%20b/c%26d.html"))
                .unwrap()
                .url_suffix(),
            "?page=a%20b/c%26d.html"
        );
        assert_eq!(ParcelRef::parse(id).unwrap().url_suffix(), "");
    }

    #[test]
    fn refuses_what_is_no_ref() {
        for bad in [
            "",
            "https://example.com/x/abcdefghijkm",
            "https://example.com/some/slug",
            "/plan",
            "@ada",
            "@ada/",
            "@-ada/plan",
            "@ada/p/abcdefghijkm2",
            "@ada/abcdefghijkm",
            "/p/short",
            "plan?v=0",
            "plan?v=x",
            "plan?page=%zz",
            "plan?page=../x",
            "plan?page=%2Fetc",
            "two words",
            // Near-ids: a mistyped id, not a slug.
            "abcdefghijk",
            "abcdefghijkmn",
            "abcdefghij",
            "abcdefghijkmnp",
        ] {
            assert_eq!(ParcelRef::parse(bad), None, "{bad:?}");
        }
        // Outside the near-id lengths, words of the id alphabet are slugs.
        assert!(ParcelRef::parse("abcdefghi").is_some());
        assert!(ParcelRef::parse("abcdefghijkmnpq").is_some());
    }
}
