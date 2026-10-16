//! Publishing: turn an upload into a new parcel or a new version.
//!
//! Order of operations, chosen so a crash at any point leaves no dangling
//! reference:
//!
//! 1. Read and validate the multipart upload in memory (bounded by the
//!    upload limit and by the concurrent-upload semaphore).
//! 2. Register blob rows (bumping `touched_at`, which shields them from GC)
//!    and write any object the store might not have yet.
//! 3. In one transaction: lock, check limits and quota against the locked
//!    state, insert the version and its file rows, advance `current_version`.
//!
//! A failure after step 2 leaves only unreferenced blobs, which GC collects
//! after the grace period.

use std::collections::{BTreeMap, HashSet};

use axum::extract::multipart::{Multipart, MultipartError};
use bytes::{Bytes, BytesMut};
use parcel_types::media;
use parcel_types::paths::{self, validate_path};
use parcel_types::{CreateParcelMetadata, Limits, NewVersionMetadata, PublishMode};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::{blobs, parcels, parcels::FileRow};
use crate::error::{too_large, AppError};
use crate::state::AppState;
use crate::storage::blob_key;

const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 4000;
const MAX_LABEL_CHARS: usize = 200;

/// One file received in an upload.
#[derive(Debug, Clone)]
pub struct Upload {
    pub path: String,
    pub bytes: Bytes,
    pub sha256: String,
    pub content_type: &'static str,
}

/// Read a publish upload: an optional `metadata` JSON part and one `file`
/// part per file, whose filename is the published path.
pub async fn read_multipart<M: DeserializeOwned + Default>(
    mut multipart: Multipart,
    limits: &Limits,
) -> Result<(M, Vec<Upload>), AppError> {
    let mut metadata: Option<M> = None;
    let mut uploads: Vec<Upload> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total: i64 = 0;
    let per_file_cap = limits.max_entry_bytes.max(limits.max_file_bytes);

    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "metadata" => {
                if metadata.is_some() {
                    return Err(AppError::BadRequest("more than one metadata part".into()));
                }
                let mut buf = BytesMut::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    buf.extend_from_slice(&chunk);
                    if buf.len() > MAX_METADATA_BYTES {
                        return Err(too_large("metadata part is too large"));
                    }
                }
                let parsed = if buf.iter().all(u8::is_ascii_whitespace) {
                    M::default()
                } else {
                    serde_json::from_slice::<M>(&buf)
                        .map_err(|e| AppError::BadRequest(format!("invalid metadata JSON: {e}")))?
                };
                metadata = Some(parsed);
            }
            "file" => {
                let path = field
                    .file_name()
                    .map(str::to_string)
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| {
                        AppError::BadRequest(
                            "each file part needs a filename (the published path)".into(),
                        )
                    })?;
                validate_path(&path).map_err(|e| AppError::BadRequest(format!("{path:?}: {e}")))?;
                let media = media::media_type_for(&path).ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "{path:?}: unsupported file type; allowed extensions: {}",
                        media::allowed_extensions().join(", ")
                    ))
                })?;
                if !seen.insert(path.clone()) {
                    return Err(AppError::BadRequest(format!("{path:?} was uploaded twice")));
                }
                if uploads.len() >= usize::try_from(limits.max_files_per_upload).unwrap_or(0) {
                    return Err(too_large(format!(
                        "too many files in one publish (limit {})",
                        limits.max_files_per_upload
                    )));
                }
                let mut buf = BytesMut::new();
                let mut hasher = Sha256::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    hasher.update(&chunk);
                    buf.extend_from_slice(&chunk);
                    total += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
                    if i64::try_from(buf.len()).unwrap_or(i64::MAX) > per_file_cap {
                        return Err(too_large(format!(
                            "{path:?} is larger than {} bytes",
                            per_file_cap
                        )));
                    }
                    if total > limits.max_upload_bytes {
                        return Err(too_large(format!(
                            "upload is larger than {} bytes",
                            limits.max_upload_bytes
                        )));
                    }
                }
                uploads.push(Upload {
                    path,
                    bytes: buf.freeze(),
                    sha256: hex::encode(hasher.finalize()),
                    content_type: media.content_type,
                });
            }
            other => {
                return Err(AppError::BadRequest(format!(
                    "unexpected multipart part {other:?} (expected \"metadata\" or \"file\")"
                )))
            }
        }
    }
    Ok((metadata.unwrap_or_default(), uploads))
}

fn multipart_error(e: MultipartError) -> AppError {
    let status = e.status();
    let message = e.body_text();
    if status == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        too_large(message)
    } else {
        AppError::BadRequest(format!("invalid multipart upload: {message}"))
    }
}

/// The file set of a version under construction, keyed by path.
type Manifest = BTreeMap<String, FileRow>;

fn upload_row(u: &Upload) -> FileRow {
    FileRow {
        path: u.path.clone(),
        blob_sha256: u.sha256.clone(),
        size_bytes: i64::try_from(u.bytes.len()).unwrap_or(i64::MAX),
        content_type: u.content_type.to_string(),
    }
}

/// Pick the entry page: the requested one, else the previous version's (if
/// still present), else the only page, else `index.html`/`index.htm`/`index.md`.
pub fn resolve_entry(
    requested: Option<&str>,
    manifest: &Manifest,
    previous: Option<&str>,
) -> Result<String, AppError> {
    if let Some(entry) = requested {
        if !manifest.contains_key(entry) {
            return Err(AppError::BadRequest(format!(
                "entry {entry:?} is not one of the parcel's files"
            )));
        }
        if !media::is_entry_candidate(entry) {
            return Err(AppError::BadRequest(format!(
                "entry {entry:?} must be an .html, .htm or .md file"
            )));
        }
        return Ok(entry.to_string());
    }
    if let Some(prev) = previous.filter(|p| manifest.contains_key(*p)) {
        return Ok(prev.to_string());
    }
    let pages: Vec<&String> = manifest
        .keys()
        .filter(|p| media::is_entry_candidate(p))
        .collect();
    match pages.as_slice() {
        [] => Err(AppError::BadRequest(
            "a parcel needs an entry page (.html, .htm or .md)".into(),
        )),
        [only] => Ok((*only).clone()),
        _ => ["index.html", "index.htm", "index.md"]
            .into_iter()
            .find(|p| manifest.contains_key(*p))
            .map(str::to_string)
            .ok_or_else(|| {
                AppError::BadRequest(
                    "several pages were uploaded and none is index.html; pass `entry` in metadata"
                        .into(),
                )
            }),
    }
}

/// Check a complete version against the per-version limits.
fn check_version_limits(
    manifest: &Manifest,
    entry: &str,
    limits: &Limits,
) -> Result<i64, AppError> {
    if manifest.len() > usize::try_from(limits.max_files_per_version).unwrap_or(0) {
        return Err(too_large(format!(
            "a version may hold at most {} files",
            limits.max_files_per_version
        )));
    }
    let mut total = 0i64;
    for (path, file) in manifest {
        let cap = if path == entry {
            limits.max_entry_bytes
        } else {
            limits.max_file_bytes
        };
        if file.size_bytes > cap {
            let what = if path == entry { "entry page" } else { "file" };
            return Err(too_large(format!(
                "{path:?} is {} bytes; the {what} limit is {cap}",
                file.size_bytes
            )));
        }
        total += file.size_bytes;
    }
    if total > limits.max_version_bytes {
        return Err(too_large(format!(
            "this version would be {total} bytes; the limit is {}",
            limits.max_version_bytes
        )));
    }
    Ok(total)
}

/// Default title: the entry's `<title>` or first `# heading`, else its file name.
pub fn derive_title(entry: &str, bytes: &[u8]) -> String {
    let head = &bytes[..bytes.len().min(256 * 1024)];
    let text = String::from_utf8_lossy(head);
    let found = if media::is_markdown(entry) {
        crate::content::markdown::first_heading(&text)
    } else {
        html_title(&text)
    };
    found
        .map(|t| truncate_chars(&t, MAX_TITLE_CHARS))
        .unwrap_or_else(|| paths::file_name(entry).to_string())
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let raw = &html[start..end];
    let decoded = raw
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn clean_text(value: Option<String>, max: usize, field: &str) -> Result<Option<String>, AppError> {
    match value {
        None => Ok(None),
        Some(v) => {
            let trimmed = v.trim();
            if trimmed.chars().count() > max {
                return Err(AppError::BadRequest(format!(
                    "{field} must be at most {max} characters"
                )));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// Register blobs and write objects the store may not have.
async fn store_blobs(state: &AppState, uploads: &[Upload]) -> Result<(), AppError> {
    if uploads.is_empty() {
        return Ok(());
    }
    // One row per distinct blob: identical files in one upload share a hash,
    // and a single INSERT … ON CONFLICT may not touch the same row twice.
    let mut distinct: BTreeMap<&str, i64> = BTreeMap::new();
    for u in uploads {
        distinct.insert(&u.sha256, i64::try_from(u.bytes.len()).unwrap_or(i64::MAX));
    }
    let shas: Vec<String> = distinct.keys().map(|s| s.to_string()).collect();
    let sizes: Vec<i64> = distinct.values().copied().collect();
    let touched = blobs::touch(&state.db, &shas, &sizes)
        .await
        .map_err(AppError::Internal)?;
    // A blob some committed version references was written before that
    // version committed, so normally it is already in the store. Check
    // anyway (one metadata request per such file) rather than trust it: if
    // the store ever lost an object, the next publish of those bytes heals it
    // instead of committing another reference to nothing.
    let referenced: HashSet<&str> = touched
        .iter()
        .filter(|t| t.referenced)
        .map(|t| t.sha256.as_str())
        .collect();
    let mut handled: HashSet<&str> = HashSet::new();
    for upload in uploads {
        let sha = upload.sha256.as_str();
        if !handled.insert(sha) {
            continue;
        }
        let key = blob_key(sha);
        let present = referenced.contains(sha)
            && state
                .blobs
                .exists(&key)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
        if !present {
            state
                .blobs
                .put(&key, upload.bytes.clone(), upload.content_type)
                .await
                .map_err(|e| AppError::Internal(e.into()))?;
        }
    }
    Ok(())
}

fn check_quota(limits: &Limits, stored: i64, adding: i64) -> Result<(), AppError> {
    if limits.user_quota_bytes > 0 && stored + adding > limits.user_quota_bytes {
        return Err(too_large(format!(
            "storage quota exceeded: {stored} of {} bytes used, this version needs {adding}",
            limits.user_quota_bytes
        )));
    }
    Ok(())
}

fn stale_base(current: i32, base: i32) -> AppError {
    AppError::Conflict(format!(
        "parcel is at version {current}, not {base}; refetch and retry"
    ))
}

/// Quota check outside any transaction, used to refuse a publish before its
/// bytes are written. Only a lower bound on what the version will need.
async fn precheck_quota(state: &AppState, owner_id: Uuid, adding: i64) -> Result<(), AppError> {
    let limits = &state.config.limits;
    if limits.user_quota_bytes == 0 {
        return Ok(());
    }
    let usage = parcels::usage(&state.db, owner_id)
        .await
        .map_err(AppError::Internal)?;
    check_quota(limits, usage.stored_bytes, adding)
}

fn new_parcel_id() -> String {
    let bytes: [u8; parcel_types::ids::ID_LEN] = rand::random();
    parcel_types::ids::parcel_id_from_bytes(bytes)
}

/// Create a parcel with version 1. Returns its id.
pub async fn create(
    state: &AppState,
    owner_id: Uuid,
    meta: CreateParcelMetadata,
    uploads: Vec<Upload>,
) -> Result<String, AppError> {
    if uploads.is_empty() {
        return Err(AppError::BadRequest("upload at least one file".into()));
    }
    let limits = &state.config.limits;
    let manifest: Manifest = uploads
        .iter()
        .map(|u| (u.path.clone(), upload_row(u)))
        .collect();
    let entry = resolve_entry(meta.entry.as_deref(), &manifest, None)?;
    let total = check_version_limits(&manifest, &entry, limits)?;

    let title = match clean_text(meta.title, MAX_TITLE_CHARS, "title")?.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            let entry_bytes = &uploads
                .iter()
                .find(|u| u.path == entry)
                .expect("entry uploaded")
                .bytes;
            derive_title(&entry, entry_bytes)
        }
    };
    let description =
        clean_text(meta.description, MAX_DESCRIPTION_CHARS, "description")?.unwrap_or_default();
    let label = clean_text(meta.label, MAX_LABEL_CHARS, "label")?.filter(|l| !l.is_empty());
    let visibility = meta.visibility.unwrap_or_default();

    // Cheap pre-check so a request that will be refused never writes bytes;
    // the authoritative check runs again under the quota lock below.
    precheck_quota(state, owner_id, total).await?;
    store_blobs(state, &uploads).await?;

    let files: Vec<FileRow> = manifest.into_values().collect();
    // A 60-bit id collides essentially never; retry anyway rather than 500.
    for attempt in 0..3 {
        let id = new_parcel_id();
        let mut tx = state
            .db
            .begin()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        parcels::lock_owner_quota(&mut tx, owner_id).await?;
        let stored = parcels::stored_bytes_tx(&mut tx, owner_id).await?;
        check_quota(limits, stored, total)?;
        match parcels::insert_parcel(&mut tx, &id, owner_id, &title, &description, visibility).await
        {
            Ok(()) => {}
            Err(e) if attempt < 2 && is_unique_violation(&e) => continue,
            Err(e) => return Err(AppError::Internal(e)),
        }
        parcels::insert_version(
            &mut tx,
            &parcels::NewVersion {
                parcel_id: &id,
                version: 1,
                entry: &entry,
                label: label.as_deref(),
                created_via: "api",
                files: &files,
            },
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        return Ok(id);
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "could not allocate a parcel id"
    )))
}

pub(crate) fn is_unique_violation(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .and_then(|s| s.as_database_error())
            .is_some_and(|d| d.is_unique_violation())
    })
}

/// What a new version is built from.
pub enum VersionSource {
    /// An upload, merged into or replacing the current version.
    Upload {
        meta: NewVersionMetadata,
        uploads: Vec<Upload>,
    },
    /// Copy an earlier version's files (a restore).
    Restore { from_version: i32 },
}

/// Append a version to a parcel the caller owns. Returns the new version number.
pub async fn add_version(
    state: &AppState,
    parcel_id: &str,
    owner_id: Uuid,
    source: VersionSource,
) -> Result<i32, AppError> {
    let limits = &state.config.limits;

    // Validate and write blobs before taking any lock.
    let (meta, uploads, restore_files) = match source {
        VersionSource::Upload { meta, uploads } => {
            if uploads.is_empty() && meta.remove.is_empty() && meta.entry.is_none() {
                return Err(AppError::BadRequest(
                    "a new version needs files to upload, paths to remove, or a new entry".into(),
                ));
            }
            if meta.mode == Some(PublishMode::Replace) && uploads.is_empty() {
                return Err(AppError::BadRequest(
                    "replace mode needs at least one file".into(),
                ));
            }
            if meta.mode == Some(PublishMode::Replace) && !meta.remove.is_empty() {
                return Err(AppError::BadRequest(
                    "`remove` only applies in merge mode".into(),
                ));
            }
            store_blobs(state, &uploads).await?;
            (meta, uploads, None)
        }
        VersionSource::Restore { from_version } => {
            let files = parcels::version_files(&state.db, parcel_id, from_version)
                .await
                .map_err(AppError::Internal)?;
            if files.is_empty() {
                return Err(AppError::NotFound(format!(
                    "version {from_version} not found"
                )));
            }
            let entry = parcels::get_version(&state.db, parcel_id, from_version)
                .await
                .map_err(AppError::Internal)?
                .map(|v| v.entry);
            let meta = NewVersionMetadata {
                mode: Some(PublishMode::Replace),
                entry,
                label: Some(format!("Restored from v{from_version}")),
                ..NewVersionMetadata::default()
            };
            (meta, Vec::new(), Some(files))
        }
    };
    let created_via = if restore_files.is_some() {
        "restore"
    } else {
        "api"
    };
    let label = clean_text(meta.label.clone(), MAX_LABEL_CHARS, "label")?.filter(|l| !l.is_empty());
    let title = clean_text(meta.title.clone(), MAX_TITLE_CHARS, "title")?.filter(|t| !t.is_empty());
    let description = clean_text(
        meta.description.clone(),
        MAX_DESCRIPTION_CHARS,
        "description",
    )?;

    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let (owner, current) = parcels::lock_for_update(&mut tx, parcel_id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    if owner != owner_id {
        return Err(crate::error::not_found());
    }
    if let Some(base) = meta.base_version {
        if base != current {
            return Err(stale_base(current, base));
        }
    }

    // Build the manifest against the locked current version, so concurrent
    // merges never drop each other's files.
    let previous = parcels::get_version(&mut *tx, parcel_id, current)
        .await
        .map_err(AppError::Internal)?;
    let mut manifest: Manifest = match (&restore_files, meta.mode.unwrap_or_default()) {
        (Some(files), _) => files.iter().map(|f| (f.path.clone(), f.clone())).collect(),
        (None, PublishMode::Replace) => Manifest::new(),
        (None, PublishMode::Merge) => parcels::version_files(&mut *tx, parcel_id, current)
            .await
            .map_err(AppError::Internal)?
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect(),
    };
    for path in &meta.remove {
        if manifest.remove(path).is_none() {
            return Err(AppError::BadRequest(format!(
                "cannot remove {path:?}: it is not in version {current}"
            )));
        }
    }
    for upload in &uploads {
        manifest.insert(upload.path.clone(), upload_row(upload));
    }
    if manifest.is_empty() {
        return Err(AppError::BadRequest(
            "a version must contain at least one file".into(),
        ));
    }
    let previous_entry = previous.as_ref().map(|v| v.entry.as_str());
    let entry = resolve_entry(meta.entry.as_deref(), &manifest, previous_entry)?;
    let total = check_version_limits(&manifest, &entry, limits)?;

    parcels::lock_owner_quota(&mut tx, owner_id).await?;
    let stored = parcels::stored_bytes_tx(&mut tx, owner_id).await?;
    check_quota(limits, stored, total)?;

    let version = current + 1;
    let files: Vec<FileRow> = manifest.into_values().collect();
    parcels::insert_version(
        &mut tx,
        &parcels::NewVersion {
            parcel_id,
            version,
            entry: &entry,
            label: label.as_deref(),
            created_via,
            files: &files,
        },
    )
    .await?;
    parcels::update_metadata_tx(
        &mut tx,
        parcel_id,
        title.as_deref(),
        description.as_deref(),
        meta.visibility,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(paths: &[&str]) -> Manifest {
        paths
            .iter()
            .map(|p| {
                (
                    p.to_string(),
                    FileRow {
                        path: p.to_string(),
                        blob_sha256: "x".into(),
                        size_bytes: 10,
                        content_type: "text/html".into(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn entry_resolution() {
        assert_eq!(
            resolve_entry(None, &manifest(&["a.html", "x.css"]), None).unwrap(),
            "a.html"
        );
        assert_eq!(
            resolve_entry(None, &manifest(&["a.html", "index.html"]), None).unwrap(),
            "index.html"
        );
        assert!(resolve_entry(None, &manifest(&["a.html", "b.html"]), None).is_err());
        assert!(resolve_entry(None, &manifest(&["x.css"]), None).is_err());
        assert_eq!(
            resolve_entry(None, &manifest(&["a.html", "b.html"]), Some("b.html")).unwrap(),
            "b.html"
        );
        assert!(resolve_entry(Some("x.css"), &manifest(&["x.css", "a.html"]), None).is_err());
        assert!(resolve_entry(Some("nope.html"), &manifest(&["a.html"]), None).is_err());
    }

    #[test]
    fn titles() {
        assert_eq!(
            derive_title("index.html", b"<html><head><TITLE>\n  Q3 &amp; Q4 </title>"),
            "Q3 & Q4"
        );
        assert_eq!(derive_title("notes.md", b"intro\n\n# Notes\n"), "Notes");
        assert_eq!(
            derive_title("dir/page.html", b"<p>no title</p>"),
            "page.html"
        );
    }

    #[test]
    fn limits_apply_per_role() {
        let limits = Limits {
            max_entry_bytes: 15,
            max_file_bytes: 5,
            max_upload_bytes: 100,
            max_files_per_upload: 10,
            max_files_per_version: 10,
            max_version_bytes: 100,
            user_quota_bytes: 0,
        };
        let m = manifest(&["index.html"]);
        assert_eq!(check_version_limits(&m, "index.html", &limits).unwrap(), 10);
        let m = manifest(&["index.html", "a.css"]);
        assert!(check_version_limits(&m, "index.html", &limits).is_err());
        assert!(check_quota(
            &Limits {
                user_quota_bytes: 50,
                ..limits.clone()
            },
            45,
            10
        )
        .is_err());
        assert!(check_quota(&limits, 1 << 40, 10).is_ok());
    }
}
