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
//!    A create with a slug decides here, under the owner's lock, whether it
//!    creates the parcel or versions the one that has the slug.
//! 4. After the commit, index the new version's entry page for search in a
//!    background task (`search::spawn_index`); publishing doesn't wait on it.
//!
//! A failure after step 2 leaves only unreferenced blobs, which GC collects
//! after the grace period.
//!
//! Lock order, here and in every other write that takes both: the owner's
//! advisory lock (`lock_owner_quota`), then the parcel's row lock.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::extract::multipart::{Multipart, MultipartError};
use bytes::{Bytes, BytesMut};
use parcel_types::media;
use parcel_types::paths::{self, validate_path};
use parcel_types::{slugs, tags};
use parcel_types::{
    CreateParcelMetadata, Limits, NewVersionMetadata, PublishMode, SourceInfo, Visibility,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::{blobs, handles, parcels, parcels::FileRow, slugs as slugs_db};
use crate::error::{slug_error, slug_taken, tag_error, too_large, AppError};
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
    check_metadata: impl Fn(&M) -> Result<(), AppError>,
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
                // Refuse as soon as the metadata says so, before buffering files.
                check_metadata(&parsed)?;
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
                let content_type = check_new_file(&path, &mut seen, limits)?;
                let mut buf = BytesMut::new();
                let mut hasher = Sha256::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    hasher.update(&chunk);
                    buf.extend_from_slice(&chunk);
                    total += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
                    check_sizes(&path, buf.len(), total, per_file_cap, limits)?;
                }
                uploads.push(Upload {
                    path,
                    bytes: buf.freeze(),
                    sha256: hex::encode(hasher.finalize()),
                    content_type,
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

/// Uploads from files already in memory (the MCP tools), held to the same
/// path, type, count and size rules as a multipart publish.
pub fn collect_uploads(
    files: Vec<(String, Bytes)>,
    limits: &Limits,
) -> Result<Vec<Upload>, AppError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut total: i64 = 0;
    let per_file_cap = limits.max_entry_bytes.max(limits.max_file_bytes);
    let mut uploads = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        let content_type = check_new_file(&path, &mut seen, limits)?;
        total = total.saturating_add(i64::try_from(bytes.len()).unwrap_or(i64::MAX));
        check_sizes(&path, bytes.len(), total, per_file_cap, limits)?;
        uploads.push(Upload {
            sha256: hex::encode(Sha256::digest(&bytes)),
            path,
            bytes,
            content_type,
        });
    }
    Ok(uploads)
}

/// A file's path and type are acceptable and it is not one too many; returns
/// its content type. `seen` holds the paths already in this upload.
fn check_new_file(
    path: &str,
    seen: &mut HashSet<String>,
    limits: &Limits,
) -> Result<&'static str, AppError> {
    validate_path(path).map_err(|e| AppError::BadRequest(format!("{path:?}: {e}")))?;
    let media = media::media_type_for(path).ok_or_else(|| {
        AppError::BadRequest(format!(
            "{path:?}: unsupported file type; allowed extensions: {}",
            media::allowed_extensions().join(", ")
        ))
    })?;
    if !seen.insert(path.to_string()) {
        return Err(AppError::BadRequest(format!("{path:?} was uploaded twice")));
    }
    if seen.len() > usize::try_from(limits.max_files_per_upload).unwrap_or(0) {
        return Err(too_large(format!(
            "too many files in one publish (limit {})",
            limits.max_files_per_upload
        )));
    }
    Ok(media.content_type)
}

/// `file_len` bytes of `path` so far, `total` across the upload.
fn check_sizes(
    path: &str,
    file_len: usize,
    total: i64,
    per_file_cap: i64,
    limits: &Limits,
) -> Result<(), AppError> {
    if i64::try_from(file_len).unwrap_or(i64::MAX) > per_file_cap {
        return Err(too_large(format!(
            "{path:?} is larger than {per_file_cap} bytes"
        )));
    }
    if total > limits.max_upload_bytes {
        return Err(too_large(format!(
            "upload is larger than {} bytes",
            limits.max_upload_bytes
        )));
    }
    Ok(())
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
        page_title: page_title(&u.path, &u.bytes),
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
    found_title(entry, bytes).unwrap_or_else(|| paths::file_name(entry).to_string())
}

/// The title a page (an HTML or Markdown file) gives itself, kept with its
/// file row so the viewer can list the version's pages by name; `None` for
/// other files and for a page without one.
pub fn page_title(path: &str, bytes: &[u8]) -> Option<String> {
    media::is_entry_candidate(path)
        .then(|| found_title(path, bytes))
        .flatten()
}

/// A page's `<title>` or first `# heading`, as plain text: control
/// characters stripped (the comment rule; a title is shown in lists and
/// terminals) and cut to the title limit.
fn found_title(path: &str, bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(256 * 1024)];
    let text = String::from_utf8_lossy(head);
    let found = if media::is_markdown(path) {
        crate::content::markdown::first_heading(&text)
    } else {
        html_title(&text)
    }?;
    let clean: String = found.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    (!clean.is_empty()).then(|| truncate_chars(clean, MAX_TITLE_CHARS))
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

/// Commit a publish's transaction, then index the new version's entry page
/// in the background (step 4), from these uploads when they carry it.
async fn commit_and_index(
    state: &AppState,
    tx: Transaction<'_, Postgres>,
    parcel_id: &str,
    version: i32,
    uploads: &[Upload],
    entry: &str,
) -> Result<(), AppError> {
    tx.commit()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let entry_upload = uploads
        .iter()
        .find(|u| u.path == entry)
        .map(|u| (u.path.clone(), u.bytes.clone()));
    crate::search::spawn_index(state.clone(), parcel_id.to_string(), version, entry_upload);
    Ok(())
}

fn new_parcel_id() -> String {
    let bytes: [u8; parcel_types::ids::ID_LEN] = rand::random();
    parcel_types::ids::parcel_id_from_bytes(bytes)
}

/// What a publish did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub id: String,
    pub version: i32,
    /// False when a create with a slug versioned an existing parcel.
    pub created: bool,
}

/// What a new parcel's first version needs beyond the upload itself.
struct NewParcelVersion {
    entry: String,
    title: String,
    total: i64,
}

/// Entry, limits and title for a publish that creates a parcel. `title` is
/// the one sent (cleaned), if any; else it is derived from the entry page.
fn new_parcel_version(
    manifest: &Manifest,
    uploads: &[Upload],
    entry: Option<&str>,
    title: Option<&str>,
    limits: &Limits,
) -> Result<NewParcelVersion, AppError> {
    let entry = resolve_entry(entry, manifest, None)?;
    let total = check_version_limits(manifest, &entry, limits)?;
    let title = match title {
        Some(t) => t.to_string(),
        None => {
            let entry_bytes = &uploads
                .iter()
                .find(|u| u.path == entry)
                .expect("entry uploaded")
                .bytes;
            derive_title(&entry, entry_bytes)
        }
    };
    Ok(NewParcelVersion {
        entry,
        title,
        total,
    })
}

/// A version's `source`, checked: a value-free 400 naming the field when
/// one isn't acceptable (the value is never stored, echoed or logged).
fn checked_source(source: Option<&SourceInfo>) -> Result<Option<SourceInfo>, AppError> {
    match source {
        None => Ok(None),
        Some(s) => s
            .validated()
            .map_err(|e| AppError::BadRequest(e.to_string())),
    }
}

/// Create a parcel with version 1, or, when `meta.slug` is one the owner
/// already uses (as a slug or a redirect), add a version to that parcel.
pub async fn create(
    state: &AppState,
    owner_id: Uuid,
    meta: CreateParcelMetadata,
    uploads: Vec<Upload>,
) -> Result<Published, AppError> {
    if uploads.is_empty() {
        return Err(AppError::BadRequest("upload at least one file".into()));
    }
    let limits = &state.config.limits;
    let slug = meta
        .slug
        .as_deref()
        .map(slugs::normalize_slug)
        .transpose()
        .map_err(slug_error)?;
    if meta.base_version.is_some() && slug.is_none() {
        return Err(AppError::BadRequest(
            "base_version needs a slug here; to version a parcel by id, publish to \
             /parcels/{id}/versions"
                .into(),
        ));
    }
    let source = checked_source(meta.source.as_ref())?;
    let manifest: Manifest = uploads
        .iter()
        .map(|u| (u.path.clone(), upload_row(u)))
        .collect();
    let title = clean_text(meta.title, MAX_TITLE_CHARS, "title")?.filter(|t| !t.is_empty());
    let description = clean_text(meta.description, MAX_DESCRIPTION_CHARS, "description")?;
    let label = clean_text(meta.label, MAX_LABEL_CHARS, "label")?.filter(|l| !l.is_empty());
    // The tags a created parcel starts with. Auto tags count only when this
    // call creates the parcel (an upsert ignores them, invalid or not, and
    // only adds `tags`), so this is checked only for a create.
    let all_tags: Vec<&String> = meta.tags.iter().chain(&meta.auto_tags).collect();
    let create_tags = || tags::normalize_tags(&all_tags).map_err(tag_error);
    // Without a slug this publish can only create, so check all of it before
    // any bytes are written. With one, which it is is decided under the
    // owner's lock, and the creating branch is checked there.
    let checked_new = slug
        .is_none()
        .then(|| {
            Ok::<_, AppError>((
                new_parcel_version(
                    &manifest,
                    &uploads,
                    meta.entry.as_deref(),
                    title.as_deref(),
                    limits,
                )?,
                create_tags()?,
            ))
        })
        .transpose()?;
    // The publish as a version: what an upsert does to the parcel that has
    // the slug (replace its files, change only what was sent, add the tags).
    // A create reads its fields from here too.
    let prepared = PreparedVersion {
        mode: PublishMode::Replace,
        remove: Vec::new(),
        entry: meta.entry,
        label,
        title,
        description,
        visibility: meta.visibility,
        base_version: meta.base_version,
        add_tags: tags::normalize_set(&meta.tags).map_err(tag_error)?,
        source,
        uploads,
        restore_files: None,
        created_via: "api",
    };

    // Cheap pre-check so a request that will be refused never writes bytes;
    // the authoritative check runs again under the quota lock below.
    let upload_bytes: i64 = manifest.values().map(|f| f.size_bytes).sum();
    precheck_quota(state, owner_id, upload_bytes).await?;
    store_blobs(state, &prepared.uploads).await?;

    let files: Vec<FileRow> = manifest.values().cloned().collect();
    // A 60-bit id collides essentially never; retry anyway rather than 500.
    for attempt in 0..3 {
        let id = new_parcel_id();
        let mut tx = state
            .db
            .begin()
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        parcels::lock_owner_quota(&mut tx, owner_id).await?;
        if let Some(slug) = slug.as_deref() {
            // Under the owner's lock, so two first publishes of one slug
            // can't both create.
            if let Some(existing) = slugs_db::find(&mut *tx, owner_id, slug).await? {
                let (version, entry) =
                    version_in_tx(state, &mut tx, &existing, owner_id, &prepared).await?;
                commit_and_index(state, tx, &existing, version, &prepared.uploads, &entry).await?;
                return Ok(Published {
                    id: existing,
                    version,
                    created: false,
                });
            }
            if meta.base_version.is_some() {
                return Err(AppError::Conflict(format!(
                    "no parcel has the slug {slug:?} yet, so there is no base_version to check"
                )));
            }
        }
        let computed;
        let (new, new_tags) = match &checked_new {
            Some(new) => new,
            None => {
                computed = (
                    new_parcel_version(
                        &manifest,
                        &prepared.uploads,
                        prepared.entry.as_deref(),
                        prepared.title.as_deref(),
                        limits,
                    )?,
                    create_tags()?,
                );
                &computed
            }
        };
        let stored = parcels::stored_bytes_tx(&mut tx, owner_id).await?;
        check_quota(limits, stored, new.total)?;
        let inserted = parcels::insert_parcel(
            &mut tx,
            &parcels::NewParcel {
                id: &id,
                owner_id,
                title: &new.title,
                description: prepared.description.as_deref().unwrap_or_default(),
                visibility: meta.visibility.unwrap_or_default(),
                tags: new_tags,
                slug: slug.as_deref(),
            },
        )
        .await;
        if let Err(e) = inserted {
            match parcels::violated_constraint(&e) {
                Some(parcels::PARCEL_PKEY) if attempt < 2 => continue,
                // Only a write that skipped the owner's lock could get here.
                Some(parcels::PARCEL_SLUG_INDEX) => return Err(slug_taken()),
                _ => return Err(AppError::Internal(e)),
            }
        }
        if slug.is_some() {
            // The clean URL needs the owner's handle.
            handles::ensure(&mut *tx, owner_id).await?;
        }
        parcels::insert_version(
            &mut tx,
            &parcels::NewVersion {
                parcel_id: &id,
                version: 1,
                entry: &new.entry,
                label: prepared.label.as_deref(),
                created_via: "api",
                files: &files,
                source: prepared.source.as_ref(),
            },
        )
        .await?;
        commit_and_index(state, tx, &id, 1, &prepared.uploads, &new.entry).await?;
        return Ok(Published {
            id,
            version: 1,
            created: true,
        });
    }
    Err(AppError::Internal(anyhow::anyhow!(
        "could not allocate a parcel id"
    )))
}

pub(crate) fn is_unique_violation(e: &anyhow::Error) -> bool {
    parcels::violated_constraint(e).is_some()
}

/// What a new version is built from. Built once per publish and consumed
/// at once, so the size difference between the variants costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum VersionSource {
    /// An upload, merged into or replacing the current version.
    Upload {
        meta: NewVersionMetadata,
        uploads: Vec<Upload>,
    },
    /// Copy an earlier version's files (a restore).
    Restore { from_version: i32 },
}

/// A new version, checked and with its blobs written, ready to be applied
/// under the locks ([`version_in_tx`]).
struct PreparedVersion {
    mode: PublishMode,
    remove: Vec<String>,
    entry: Option<String>,
    label: Option<String>,
    title: Option<String>,
    description: Option<String>,
    visibility: Option<Visibility>,
    base_version: Option<i32>,
    add_tags: BTreeSet<String>,
    source: Option<SourceInfo>,
    uploads: Vec<Upload>,
    /// A restore's files (instead of uploads).
    restore_files: Option<Vec<FileRow>>,
    created_via: &'static str,
}

/// Check a new version and write its blobs, before taking any lock.
async fn prepare_version(
    state: &AppState,
    parcel_id: &str,
    source: VersionSource,
) -> Result<PreparedVersion, AppError> {
    let (meta, uploads, restore_files, version_source) = match source {
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
            let source = checked_source(meta.source.as_ref())?;
            store_blobs(state, &uploads).await?;
            (meta, uploads, None, source)
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
            let restored = parcels::get_version(&state.db, parcel_id, from_version)
                .await
                .map_err(AppError::Internal)?;
            let meta = NewVersionMetadata {
                mode: Some(PublishMode::Replace),
                entry: restored.as_ref().map(|v| v.entry.clone()),
                label: Some(format!("Restored from v{from_version}")),
                ..NewVersionMetadata::default()
            };
            // A restore is the restored version again, provenance included.
            let source = restored.and_then(|v| v.source);
            (meta, Vec::new(), Some(files), source)
        }
    };
    Ok(PreparedVersion {
        mode: meta.mode.unwrap_or_default(),
        label: clean_text(meta.label, MAX_LABEL_CHARS, "label")?.filter(|l| !l.is_empty()),
        title: clean_text(meta.title, MAX_TITLE_CHARS, "title")?.filter(|t| !t.is_empty()),
        description: clean_text(meta.description, MAX_DESCRIPTION_CHARS, "description")?,
        add_tags: tags::normalize_set(&meta.add_tags).map_err(tag_error)?,
        remove: meta.remove,
        entry: meta.entry,
        visibility: meta.visibility,
        base_version: meta.base_version,
        source: version_source,
        uploads,
        created_via: if restore_files.is_some() {
            "restore"
        } else {
            "api"
        },
        restore_files,
    })
}

/// Append a version to a parcel the caller owns. Returns the new version number.
pub async fn add_version(
    state: &AppState,
    parcel_id: &str,
    owner_id: Uuid,
    source: VersionSource,
) -> Result<i32, AppError> {
    let prepared = prepare_version(state, parcel_id, source).await?;
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    // The owner's lock before the row lock (the one order; see the module
    // docs). Taken for the caller: a non-owner only serializes with itself
    // and is refused below.
    parcels::lock_owner_quota(&mut tx, owner_id).await?;
    let (version, entry) = version_in_tx(state, &mut tx, parcel_id, owner_id, &prepared).await?;
    commit_and_index(state, tx, parcel_id, version, &prepared.uploads, &entry).await?;
    Ok(version)
}

/// Apply a prepared version inside a transaction that already holds the
/// owner's lock: lock the parcel's row, check ownership and `base_version`,
/// build the file set against the locked current version (so concurrent
/// merges never drop each other's files), insert it. Returns the new
/// version and its entry. The caller commits.
async fn version_in_tx(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    parcel_id: &str,
    owner_id: Uuid,
    v: &PreparedVersion,
) -> Result<(i32, String), AppError> {
    let limits = &state.config.limits;
    let locked = parcels::lock_for_update(tx, parcel_id)
        .await?
        .ok_or_else(crate::error::not_found)?;
    if locked.owner_id != owner_id {
        return Err(crate::error::not_found());
    }
    let current = locked.current_version;
    if let Some(base) = v.base_version {
        if base != current {
            return Err(stale_base(current, base));
        }
    }

    let previous = parcels::get_version(&mut **tx, parcel_id, current)
        .await
        .map_err(AppError::Internal)?;
    let mut manifest: Manifest = match (&v.restore_files, v.mode) {
        (Some(files), _) => files.iter().map(|f| (f.path.clone(), f.clone())).collect(),
        (None, PublishMode::Replace) => Manifest::new(),
        (None, PublishMode::Merge) => parcels::version_files(&mut **tx, parcel_id, current)
            .await
            .map_err(AppError::Internal)?
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect(),
    };
    for path in &v.remove {
        if manifest.remove(path).is_none() {
            return Err(AppError::BadRequest(format!(
                "cannot remove {path:?}: it is not in version {current}"
            )));
        }
    }
    for upload in &v.uploads {
        manifest.insert(upload.path.clone(), upload_row(upload));
    }
    if manifest.is_empty() {
        return Err(AppError::BadRequest(
            "a version must contain at least one file".into(),
        ));
    }
    let previous_entry = previous.as_ref().map(|p| p.entry.as_str());
    let entry = resolve_entry(v.entry.as_deref(), &manifest, previous_entry)?;
    let total = check_version_limits(&manifest, &entry, limits)?;
    let new_tags = if v.add_tags.is_empty() {
        None
    } else {
        Some(tags::edit_tags(&locked.tags, &v.add_tags, &Default::default()).map_err(tag_error)?)
    };

    let stored = parcels::stored_bytes_tx(tx, owner_id).await?;
    check_quota(limits, stored, total)?;

    let version = current + 1;
    let files: Vec<FileRow> = manifest.into_values().collect();
    parcels::insert_version(
        tx,
        &parcels::NewVersion {
            parcel_id,
            version,
            entry: &entry,
            label: v.label.as_deref(),
            created_via: v.created_via,
            files: &files,
            source: v.source.as_ref(),
        },
    )
    .await?;
    parcels::update_metadata_tx(
        tx,
        parcel_id,
        v.title.as_deref(),
        v.description.as_deref(),
        v.visibility,
        false,
    )
    .await?;
    // A new version also un-archives: the version-change trigger (migration
    // 0006) clears `archived_at`, for older revisions' publishes too.
    if new_tags.is_some() {
        parcels::update_organization(tx, parcel_id, new_tags.as_deref(), None).await?;
    }
    Ok((version, entry))
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
                        page_title: None,
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
    fn page_titles() {
        assert_eq!(page_title("dir/page.html", b"<p>no title</p>"), None);
        assert_eq!(page_title("a.htm", b"<title>A</title>"), Some("A".into()));
        assert_eq!(
            page_title("docs/Notes.MARKDOWN", b"# Notes `v2`\n"),
            Some("Notes v2".into())
        );
        // Only pages have titles, whatever their bytes say.
        assert_eq!(page_title("x.txt", b"<title>T</title>"), None);
        assert_eq!(page_title("x.svg", b"<title>T</title>"), None);
        // Control characters go (escape sequences, bells, C1 controls); a
        // title of nothing else is no title.
        assert_eq!(
            page_title(
                "a.html",
                "<title>\u{1b}[31mRed\u{7}\u{9b}</title>".as_bytes()
            ),
            Some("[31mRed".into())
        );
        assert_eq!(page_title("a.md", "# \u{1}\u{2}\n".as_bytes()), None);
        let long = format!("<title>{}</title>", "é".repeat(300));
        assert_eq!(
            page_title("a.html", long.as_bytes()).map(|t| t.chars().count()),
            Some(200)
        );
        // The parcel title falls back to the file name; a page title doesn't.
        assert_eq!(derive_title("a.md", b"no heading"), "a.md");
        assert_eq!(page_title("a.md", b"no heading"), None);
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
            max_data_bytes: 0,
            max_data_value_bytes: 0,
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
