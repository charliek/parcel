//! Row → wire-type conversions and the shared access checks.

use parcel_types::{FileEntry, ParcelSummary, VersionDetail, VersionSummary, Visibility};

use crate::auth::AuthUser;
use crate::config::Config;
use crate::db::parcels::{self, FileRow, ParcelRow, VersionRow};
use crate::error::{not_found, AppError};
use crate::state::AppState;

pub fn summary(config: &Config, row: &ParcelRow) -> ParcelSummary {
    ParcelSummary {
        id: row.id.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        visibility: row.visibility(),
        current_version: row.current_version,
        entry: row.entry.clone(),
        file_count: row.file_count,
        total_bytes: row.total_bytes,
        created_at: row.created_at,
        updated_at: row.updated_at,
        url: config.viewer_url(&row.id),
        open_comments: row.open_comments,
    }
}

pub fn version_summary(v: &VersionRow) -> VersionSummary {
    VersionSummary {
        version: v.version,
        entry: v.entry.clone(),
        label: v.label.clone(),
        file_count: v.file_count,
        total_bytes: v.total_bytes,
        created_at: v.created_at,
    }
}

pub fn version_detail(v: &VersionRow, files: Vec<FileRow>) -> VersionDetail {
    VersionDetail {
        summary: version_summary(v),
        files: files.into_iter().map(FileEntry::from).collect(),
    }
}

/// Load a parcel the caller may read: its owner, or anyone when public.
/// A private parcel is a 404 to everyone else, so ids can't be probed.
/// Returns the row and whether the caller owns it.
pub async fn readable(
    state: &AppState,
    id: &str,
    caller: Option<&AuthUser>,
) -> Result<(ParcelRow, bool), AppError> {
    let row = load(state, id).await?;
    let is_owner = caller.is_some_and(|c| c.user.id == row.owner_id);
    if is_owner || row.visibility() == Visibility::Public {
        Ok((row, is_owner))
    } else {
        Err(not_found())
    }
}

/// Load a parcel the caller owns. Non-owners get 404 (private) or 403 (public).
pub async fn owned(state: &AppState, id: &str, caller: &AuthUser) -> Result<ParcelRow, AppError> {
    let row = load(state, id).await?;
    if row.owner_id == caller.user.id {
        Ok(row)
    } else if row.visibility() == Visibility::Public {
        Err(AppError::Forbidden(
            "only the owner can change this parcel".into(),
        ))
    } else {
        Err(not_found())
    }
}

async fn load(state: &AppState, id: &str) -> Result<ParcelRow, AppError> {
    if !parcel_types::ids::is_valid_parcel_id(id) {
        return Err(not_found());
    }
    parcels::get(&state.db, id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(not_found)
}

/// Parse a `{version}` path segment: a positive number or `latest`.
pub fn parse_version(raw: &str, current: i32) -> Result<i32, AppError> {
    if raw == "latest" {
        return Ok(current);
    }
    raw.parse::<i32>().ok().filter(|v| *v > 0).ok_or_else(|| {
        AppError::BadRequest("version must be a positive number or \"latest\"".into())
    })
}
