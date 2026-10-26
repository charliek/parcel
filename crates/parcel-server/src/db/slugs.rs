//! Slugs: a parcel's name under its owner's handle, and the redirects a
//! rename leaves behind.
//!
//! A name is in exactly one place at a time: the current slug of one of the
//! owner's parcels (`parcels.slug`), or a redirect row pointing at the parcel
//! that had it. Setting a slug deletes any redirect row for it; renaming or
//! clearing one turns the old name into a redirect row.

use anyhow::Result;
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::error::DbContext;

/// The owner's parcel `slug` names: the one that has it, else the one a
/// redirect row for it points at. `$owner` is the SQL for the owner's id;
/// the slug is `$2`.
macro_rules! slug_lookup {
    ($owner:literal) => {
        concat!(
            "SELECT id FROM (
                 SELECT id, 0 AS rank FROM parcels WHERE owner_id = ",
            $owner,
            " AND slug = $2
                 UNION ALL
                 SELECT parcel_id, 1 FROM parcel_slug_redirects
                 WHERE owner_id = ",
            $owner,
            " AND slug = $2
             ) found ORDER BY rank LIMIT 1"
        )
    };
}

/// The parcel `/@<handle>/<slug>` names: by the handle's user (current or
/// retired handle), then their parcel with that slug or the redirect row for
/// it. No access check: callers apply `readable()` to what this returns.
pub async fn resolve(pool: &PgPool, handle: &str, slug: &str) -> Result<Option<String>> {
    sqlx::query_scalar(slug_lookup!(
        "(SELECT user_id FROM user_handles WHERE handle = $1)"
    ))
    .bind(handle)
    .bind(slug)
    .fetch_optional(pool)
    .await
    .db_context("resolving slug")
}

/// [`resolve`] for an owner known by id: the parcel a publish to `slug`
/// versions (under the owner's lock), or a caller's own slug. `None` when
/// the name is free.
pub async fn find<'e>(
    db: impl PgExecutor<'e>,
    owner_id: Uuid,
    slug: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar(slug_lookup!("$1"))
        .bind(owner_id)
        .bind(slug)
        .fetch_optional(db)
        .await
        .db_context("finding parcel by slug")
}

/// Whether another of the owner's parcels has `slug` as its current slug.
pub async fn taken_by_other(
    tx: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    slug: &str,
    parcel_id: &str,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM parcels WHERE owner_id = $1 AND slug = $2 AND id <> $3)",
    )
    .bind(owner_id)
    .bind(slug)
    .bind(parcel_id)
    .fetch_one(&mut **tx)
    .await
    .db_context("checking slug")
}

/// Change a locked parcel's slug from `old` to `new` (either may be `None`),
/// leaving `updated_at` alone. The caller holds the owner's lock and the
/// parcel's row lock, and has checked that no other parcel has `new`.
pub async fn set(
    tx: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    parcel_id: &str,
    old: Option<&str>,
    new: Option<&str>,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    if let Some(new) = new {
        // The clean URL needs the owner's handle.
        super::handles::ensure(&mut **tx, owner_id).await?;
        // Whoever the name redirected to, it is this parcel's now.
        sqlx::query("DELETE FROM parcel_slug_redirects WHERE owner_id = $1 AND slug = $2")
            .bind(owner_id)
            .bind(new)
            .execute(&mut **tx)
            .await
            .db_context("taking over slug redirect")?;
    }
    sqlx::query("UPDATE parcels SET slug = $2 WHERE id = $1")
        .bind(parcel_id)
        .bind(new)
        .execute(&mut **tx)
        .await
        .db_context("setting slug")?;
    if let Some(old) = old {
        sqlx::query(
            "INSERT INTO parcel_slug_redirects (owner_id, slug, parcel_id) VALUES ($1, $2, $3)
             ON CONFLICT (owner_id, slug) DO UPDATE SET parcel_id = EXCLUDED.parcel_id",
        )
        .bind(owner_id)
        .bind(old)
        .bind(parcel_id)
        .execute(&mut **tx)
        .await
        .db_context("recording slug redirect")?;
    }
    Ok(())
}
