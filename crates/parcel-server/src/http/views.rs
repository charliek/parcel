//! Row → wire-type conversions and the shared access checks.

use parcel_types::{
    FileEntry, ParcelAccess, ParcelSummary, ShareRole, VersionDetail, VersionSummary, Visibility,
};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::config::Config;
use crate::db::parcels::{self, FileRow, ParcelRow, VersionRow};
use crate::db::{shares, users};
use crate::error::{not_found, AppError};
use crate::state::AppState;

pub fn summary(config: &Config, row: &ParcelRow) -> ParcelSummary {
    ParcelSummary {
        id: row.id.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        visibility: effective_visibility(config, row.visibility()),
        current_version: row.current_version,
        entry: row.entry.clone(),
        file_count: row.file_count,
        total_bytes: row.total_bytes,
        created_at: row.created_at,
        updated_at: row.updated_at,
        url: config.viewer_url(&row.id),
        open_comments: row.open_comments,
        tags: row.tags.clone(),
        archived_at: row.archived_at,
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

/// What a stored visibility means on this server: with public links
/// turned off (`PARCEL_ALLOW_PUBLIC=false`), public parcels are internal.
pub fn effective_visibility(config: &Config, stored: Visibility) -> Visibility {
    if stored == Visibility::Public && !config.allow_public {
        Visibility::Internal
    } else {
        stored
    }
}

/// The access rule (PLAN.md, "Sign-in providers and sharing"), given what
/// is known about the caller: `None` = anonymous, else their id and the
/// role the parcel is shared with them as. `None` means no access (a 404).
pub fn resolve_access(
    config: &Config,
    owner_id: Uuid,
    stored: Visibility,
    caller: Option<(Uuid, Option<ShareRole>)>,
) -> Option<ParcelAccess> {
    let visibility = effective_visibility(config, stored);
    match caller {
        Some((id, _)) if id == owner_id => Some(ParcelAccess::Owner),
        // Everyone signed in may comment on what everyone signed in may see.
        Some(_) if visibility != Visibility::Private => Some(ParcelAccess::Commenter),
        Some((_, Some(ShareRole::Commenter))) => Some(ParcelAccess::Commenter),
        Some((_, Some(ShareRole::Viewer))) => Some(ParcelAccess::Viewer),
        Some((_, None)) => None,
        None if visibility == Visibility::Public => Some(ParcelAccess::Public),
        None => None,
    }
}

/// The caller's access to a loaded parcel. Looks up a share only when it
/// could matter (a signed-in non-owner of a private parcel).
pub async fn access_to(
    state: &AppState,
    row: &ParcelRow,
    caller: Option<&AuthUser>,
) -> Result<Option<ParcelAccess>, AppError> {
    let caller = match caller {
        Some(c)
            if c.user.id != row.owner_id
                && effective_visibility(&state.config, row.visibility()) == Visibility::Private =>
        {
            let role = shares::role_for(&state.db, &row.id, &c.user.email)
                .await
                .map_err(AppError::Internal)?;
            Some((c.user.id, role))
        }
        Some(c) => Some((c.user.id, None)),
        None => None,
    };
    Ok(resolve_access(
        &state.config,
        row.owner_id,
        row.visibility(),
        caller,
    ))
}

/// Load a parcel the caller may see, with what they may do. Anyone without
/// access gets a 404, so ids can't be probed.
pub async fn readable(
    state: &AppState,
    id: &str,
    caller: Option<&AuthUser>,
) -> Result<(ParcelRow, ParcelAccess), AppError> {
    let row = load(state, id).await?;
    let access = access_to(state, &row, caller)
        .await?
        .ok_or_else(not_found)?;
    Ok((row, access))
}

/// Load a parcel the caller owns. Others get 403 if they can see it, else 404.
pub async fn owned(state: &AppState, id: &str, caller: &AuthUser) -> Result<ParcelRow, AppError> {
    let row = load(state, id).await?;
    if row.owner_id == caller.user.id {
        return Ok(row);
    }
    match access_to(state, &row, Some(caller)).await? {
        Some(_) => Err(AppError::Forbidden(
            "only the owner can change this parcel".into(),
        )),
        None => Err(not_found()),
    }
}

/// The owner's display name, for signed-in callers (anonymous viewers of
/// a public parcel aren't told who published it).
pub async fn owner_name(
    state: &AppState,
    row: &ParcelRow,
    caller: Option<&AuthUser>,
) -> Result<Option<String>, AppError> {
    let Some(caller) = caller else {
        return Ok(None);
    };
    if caller.user.id == row.owner_id {
        return Ok(Some(users::display_name(
            &caller.user.name,
            &caller.user.email,
        )));
    }
    Ok(users::find_by_id(&state.db, row.owner_id)
        .await
        .map_err(AppError::Internal)?
        .map(|u| users::display_name(&u.name, &u.email)))
}

/// Refuse a visibility this server doesn't allow.
pub fn check_visibility(config: &Config, visibility: Option<Visibility>) -> Result<(), AppError> {
    if visibility == Some(Visibility::Public) && !config.allow_public {
        return Err(AppError::BadRequest(
            "public links are turned off on this server; use \"internal\" (everyone signed in) \
             or share with people by email"
                .into(),
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use ParcelAccess::{Commenter, Owner, Public, Viewer};
    use Visibility::{Internal, Private};

    #[test]
    fn the_access_table() {
        let config = Config::default();
        let owner = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let access = |v, caller| resolve_access(&config, owner, v, caller);
        for v in [Private, Internal, Visibility::Public] {
            assert_eq!(access(v, Some((owner, None))), Some(Owner));
        }
        assert_eq!(access(Private, Some((other, None))), None);
        assert_eq!(
            access(Private, Some((other, Some(ShareRole::Viewer)))),
            Some(Viewer)
        );
        assert_eq!(
            access(Private, Some((other, Some(ShareRole::Commenter)))),
            Some(Commenter)
        );
        assert_eq!(access(Internal, Some((other, None))), Some(Commenter));
        assert_eq!(
            access(Internal, Some((other, Some(ShareRole::Viewer)))),
            Some(Commenter)
        );
        assert_eq!(
            access(Visibility::Public, Some((other, None))),
            Some(Commenter)
        );
        assert_eq!(access(Private, None), None);
        assert_eq!(access(Internal, None), None);
        assert_eq!(access(Visibility::Public, None), Some(Public));
    }

    #[test]
    fn public_off_makes_public_internal() {
        let config = Config {
            allow_public: false,
            ..Config::default()
        };
        assert_eq!(effective_visibility(&config, Visibility::Public), Internal);
        assert_eq!(
            resolve_access(&config, Uuid::nil(), Visibility::Public, None),
            None
        );
        assert!(check_visibility(&config, Some(Visibility::Public)).is_err());
        assert!(check_visibility(&config, Some(Internal)).is_ok());
    }
}
