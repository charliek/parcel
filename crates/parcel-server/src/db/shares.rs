//! Who a parcel is shared with, by email.

use anyhow::{Context, Result};
use parcel_types::{ShareRole, Visibility};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::parcels::{ParcelRow, PARCEL_COLUMNS, PARCEL_FROM};

/// Most people one parcel may be shared with.
pub const MAX_SHARES_PER_PARCEL: i64 = 200;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ShareRow {
    pub email: String,
    pub role: String,
    pub created_at: OffsetDateTime,
    /// Set once someone with this email has signed in.
    pub name: Option<String>,
}

impl ShareRow {
    pub fn role(&self) -> ShareRole {
        ShareRole::parse(&self.role).unwrap_or_default()
    }
}

/// The role `email` (normalized) has on a parcel, if it is shared with them.
pub async fn role_for(pool: &PgPool, parcel_id: &str, email: &str) -> Result<Option<ShareRole>> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM parcel_shares WHERE parcel_id = $1 AND email = lower($2)",
    )
    .bind(parcel_id)
    .bind(email)
    .fetch_optional(pool)
    .await
    .context("loading share")?;
    Ok(role.and_then(|r| ShareRole::parse(&r)))
}

/// What a content capability bound to `user_id` needs re-checked: the
/// parcel's visibility and owner, the user's share (if any), and whether
/// the user can still sign in at all. `None` if either is gone.
pub struct MemberCheck {
    pub visibility: Visibility,
    pub owner_id: Uuid,
    pub role: Option<ShareRole>,
    pub disabled: bool,
}

pub async fn member_check(
    pool: &PgPool,
    parcel_id: &str,
    user_id: Uuid,
) -> Result<Option<MemberCheck>> {
    let row: Option<(String, Uuid, Option<String>, bool)> = sqlx::query_as(
        "SELECT p.visibility, p.owner_id, s.role, u.disabled_at IS NOT NULL
         FROM parcels p
         JOIN users u ON u.id = $2
         LEFT JOIN parcel_shares s ON s.parcel_id = p.id AND s.email = lower(u.email)
         WHERE p.id = $1",
    )
    .bind(parcel_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("checking member access")?;
    Ok(
        row.map(|(visibility, owner_id, role, disabled)| MemberCheck {
            visibility: Visibility::parse(&visibility).unwrap_or_default(),
            owner_id,
            role: role.and_then(|r| ShareRole::parse(&r)),
            disabled,
        }),
    )
}

const SHARE_SELECT: &str = "SELECT s.email, s.role, s.created_at,
            (SELECT NULLIF(u.name, '') FROM users u WHERE lower(u.email) = s.email
              ORDER BY u.created_at LIMIT 1) AS name
     FROM parcel_shares s";

pub async fn list(pool: &PgPool, parcel_id: &str) -> Result<Vec<ShareRow>> {
    sqlx::query_as::<_, ShareRow>(&format!(
        "{SHARE_SELECT} WHERE s.parcel_id = $1 ORDER BY s.email COLLATE \"C\""
    ))
    .bind(parcel_id)
    .fetch_all(pool)
    .await
    .context("listing shares")
}

#[derive(Debug, PartialEq, Eq)]
pub enum PutRefused {
    ParcelGone,
    TooMany,
}

/// Share with `email` (normalized) as `role`, or change their role.
pub async fn put(
    pool: &PgPool,
    parcel_id: &str,
    email: &str,
    role: ShareRole,
) -> Result<Result<ShareRow, PutRefused>> {
    let mut tx = pool.begin().await.context("beginning share")?;
    // The parcel row lock serializes shares, so two can't both squeeze
    // under the cap.
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM parcels WHERE id = $1 FOR UPDATE")
        .bind(parcel_id)
        .fetch_optional(&mut *tx)
        .await
        .context("locking parcel for share")?;
    if exists.is_none() {
        return Ok(Err(PutRefused::ParcelGone));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM parcel_shares WHERE parcel_id = $1 AND email <> $2",
    )
    .bind(parcel_id)
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .context("counting shares")?;
    if count >= MAX_SHARES_PER_PARCEL {
        return Ok(Err(PutRefused::TooMany));
    }
    sqlx::query(
        "INSERT INTO parcel_shares (parcel_id, email, role) VALUES ($1, $2, $3)
         ON CONFLICT (parcel_id, email) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(parcel_id)
    .bind(email)
    .bind(role.as_str())
    .execute(&mut *tx)
    .await
    .context("saving share")?;
    let row = sqlx::query_as::<_, ShareRow>(&format!(
        "{SHARE_SELECT} WHERE s.parcel_id = $1 AND s.email = $2"
    ))
    .bind(parcel_id)
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .context("loading share")?;
    tx.commit().await.context("committing share")?;
    Ok(Ok(row))
}

/// Stop sharing with `email`; false if it wasn't shared with them.
pub async fn delete(pool: &PgPool, parcel_id: &str, email: &str) -> Result<bool> {
    let deleted = sqlx::query("DELETE FROM parcel_shares WHERE parcel_id = $1 AND email = $2")
        .bind(parcel_id)
        .bind(email)
        .execute(pool)
        .await
        .context("deleting share")?
        .rows_affected();
    Ok(deleted > 0)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SharedRow {
    #[sqlx(flatten)]
    pub parcel: ParcelRow,
    pub owner_name: String,
    pub owner_email: String,
    pub role: String,
    pub shared_at: OffsetDateTime,
}

/// Parcels shared with `email` by name (not ones the user owns), most
/// recently updated first.
pub async fn shared_with(
    pool: &PgPool,
    email: &str,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<(Vec<SharedRow>, i64)> {
    let rows = sqlx::query_as::<_, SharedRow>(&format!(
        "SELECT {PARCEL_COLUMNS},
                o.name AS owner_name, o.email AS owner_email, s.role, s.created_at AS shared_at
         {PARCEL_FROM}
         JOIN parcel_shares s ON s.parcel_id = p.id AND s.email = lower($1)
         JOIN users o ON o.id = p.owner_id
         WHERE p.owner_id <> $2
         ORDER BY p.updated_at DESC, p.id LIMIT $3 OFFSET $4"
    ))
    .bind(email)
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .context("listing parcels shared with me")?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM parcel_shares s JOIN parcels p ON p.id = s.parcel_id
         WHERE s.email = lower($1) AND p.owner_id <> $2",
    )
    .bind(email)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("counting parcels shared with me")?;
    Ok((rows, total))
}
