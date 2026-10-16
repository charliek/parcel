//! Which files a parcel may contain, and the media type each is served with.
//!
//! The table is an allow-list keyed by extension: anything not listed is
//! rejected at upload. Serving a stored file with a type the server chose —
//! never one the uploader sent — is what keeps `nosniff` meaningful.

/// How a file can participate in a parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Can be the page a parcel opens on (HTML or Markdown).
    Entry,
    /// Supporting file only (scripts, styles, images, data, …).
    Asset,
}

/// One allowed extension.
#[derive(Debug, Clone, Copy)]
pub struct MediaType {
    pub extension: &'static str,
    pub content_type: &'static str,
    pub role: Role,
}

const fn entry(extension: &'static str, content_type: &'static str) -> MediaType {
    MediaType {
        extension,
        content_type,
        role: Role::Entry,
    }
}

const fn asset(extension: &'static str, content_type: &'static str) -> MediaType {
    MediaType {
        extension,
        content_type,
        role: Role::Asset,
    }
}

/// Every extension a parcel may contain. Text types carry an explicit UTF-8
/// charset so a page without a `<meta charset>` still renders correctly.
pub const MEDIA_TYPES: &[MediaType] = &[
    // Pages
    entry("html", "text/html; charset=utf-8"),
    entry("htm", "text/html; charset=utf-8"),
    entry("md", "text/markdown; charset=utf-8"),
    entry("markdown", "text/markdown; charset=utf-8"),
    // Code and styles
    asset("css", "text/css; charset=utf-8"),
    asset("js", "text/javascript; charset=utf-8"),
    asset("mjs", "text/javascript; charset=utf-8"),
    asset("map", "application/json; charset=utf-8"),
    asset("wasm", "application/wasm"),
    // Data
    asset("json", "application/json; charset=utf-8"),
    asset("geojson", "application/geo+json; charset=utf-8"),
    asset("txt", "text/plain; charset=utf-8"),
    asset("csv", "text/csv; charset=utf-8"),
    asset("tsv", "text/tab-separated-values; charset=utf-8"),
    asset("xml", "application/xml; charset=utf-8"),
    asset("yaml", "text/yaml; charset=utf-8"),
    asset("yml", "text/yaml; charset=utf-8"),
    // Images
    asset("svg", "image/svg+xml"),
    asset("png", "image/png"),
    asset("jpg", "image/jpeg"),
    asset("jpeg", "image/jpeg"),
    asset("gif", "image/gif"),
    asset("webp", "image/webp"),
    asset("avif", "image/avif"),
    asset("ico", "image/x-icon"),
    asset("bmp", "image/bmp"),
    // Fonts
    asset("woff", "font/woff"),
    asset("woff2", "font/woff2"),
    asset("ttf", "font/ttf"),
    asset("otf", "font/otf"),
    // Audio / video
    asset("mp3", "audio/mpeg"),
    asset("wav", "audio/wav"),
    asset("ogg", "audio/ogg"),
    asset("m4a", "audio/mp4"),
    asset("mp4", "video/mp4"),
    asset("webm", "video/webm"),
    // Documents and 3D
    asset("pdf", "application/pdf"),
    asset("glb", "model/gltf-binary"),
    asset("gltf", "model/gltf+json"),
];

/// The extension of a path, lowercased (`Assets/App.JS` -> `js`).
pub fn extension(path: &str) -> Option<String> {
    let name = crate::paths::file_name(path);
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Look up the media type for a path, or `None` if its extension is not allowed.
pub fn media_type_for(path: &str) -> Option<&'static MediaType> {
    let ext = extension(path)?;
    MEDIA_TYPES.iter().find(|m| m.extension == ext)
}

/// Whether a path may be a parcel's entry page.
pub fn is_entry_candidate(path: &str) -> bool {
    media_type_for(path).is_some_and(|m| m.role == Role::Entry)
}

/// Whether a path is Markdown (rendered to HTML when it is the entry).
pub fn is_markdown(path: &str) -> bool {
    matches!(extension(path).as_deref(), Some("md" | "markdown"))
}

/// Every allowed extension, for error messages and `/api/v1/info`.
pub fn allowed_extensions() -> Vec<&'static str> {
    MEDIA_TYPES.iter().map(|m| m.extension).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_types() {
        assert_eq!(
            media_type_for("index.html").unwrap().content_type,
            "text/html; charset=utf-8"
        );
        assert_eq!(media_type_for("a/B.PNG").unwrap().content_type, "image/png");
        assert!(media_type_for("run.exe").is_none());
        assert!(media_type_for("Makefile").is_none());
        assert!(media_type_for(".html").is_none());
    }

    #[test]
    fn entry_roles() {
        assert!(is_entry_candidate("index.html"));
        assert!(is_entry_candidate("README.md"));
        assert!(!is_entry_candidate("app.js"));
        assert!(is_markdown("notes.markdown"));
        assert!(!is_markdown("index.html"));
    }
}
