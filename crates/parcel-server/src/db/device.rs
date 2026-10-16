//! Pending CLI device logins.

use anyhow::{Context, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeviceRow {
    pub id: Uuid,
    pub user_code: String,
    pub client_name: String,
    pub status: String,
    pub user_id: Option<Uuid>,
    pub interval_secs: i32,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub last_polled_at: Option<OffsetDateTime>,
}

const COLUMNS: &str =
    "id, user_code, client_name, status, user_id, interval_secs, created_at, expires_at, last_polled_at";

pub async fn insert(
    pool: &PgPool,
    device_code_hash: &str,
    user_code: &str,
    client_name: &str,
    interval_secs: i32,
    expires_at: OffsetDateTime,
) -> Result<DeviceRow> {
    sqlx::query_as::<_, DeviceRow>(&format!(
        "INSERT INTO device_authorizations (device_code_hash, user_code, client_name, interval_secs, expires_at)
         VALUES ($1, $2, $3, $4, $5) RETURNING {COLUMNS}"
    ))
    .bind(device_code_hash)
    .bind(user_code)
    .bind(client_name)
    .bind(interval_secs)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .context("inserting device authorization")
}

/// A pending, unexpired authorization by its user code (for the approval page).
pub async fn find_pending_by_user_code(
    pool: &PgPool,
    user_code: &str,
) -> Result<Option<DeviceRow>> {
    sqlx::query_as::<_, DeviceRow>(&format!(
        "SELECT {COLUMNS} FROM device_authorizations
         WHERE user_code = $1 AND status = 'pending' AND expires_at > now()"
    ))
    .bind(user_code)
    .fetch_optional(pool)
    .await
    .context("finding device authorization")
}

/// Approve or deny a pending code. Returns false if it was not pending (or expired).
pub async fn decide(pool: &PgPool, user_code: &str, user_id: Uuid, approve: bool) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE device_authorizations SET status = $3, user_id = $2
         WHERE user_code = $1 AND status = 'pending' AND expires_at > now()",
    )
    .bind(user_code)
    .bind(user_id)
    .bind(if approve { "approved" } else { "denied" })
    .execute(pool)
    .await
    .context("deciding device authorization")?;
    Ok(result.rows_affected() > 0)
}

/// Record a poll and return the row as it was *before* this poll (so the
/// caller can enforce the interval against the previous poll time).
pub async fn poll(pool: &PgPool, device_code_hash: &str) -> Result<Option<DeviceRow>> {
    sqlx::query_as::<_, DeviceRow>(
        "UPDATE device_authorizations d SET last_polled_at = now()
         FROM (SELECT id, last_polled_at AS previous_poll FROM device_authorizations
               WHERE device_code_hash = $1) prev
         WHERE d.id = prev.id
         RETURNING d.id, d.user_code, d.client_name, d.status, d.user_id, d.interval_secs,
                   d.created_at, d.expires_at, prev.previous_poll AS last_polled_at",
    )
    .bind(device_code_hash)
    .fetch_optional(pool)
    .await
    .context("polling device authorization")
}

/// Atomically move an approved authorization to `redeemed`. Exactly one
/// poller wins, which is what makes the token deliverable only once.
pub async fn redeem(pool: &PgPool, id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE device_authorizations SET status = 'redeemed'
         WHERE id = $1 AND status = 'approved' AND expires_at > now()",
    )
    .bind(id)
    .execute(pool)
    .await
    .context("redeeming device authorization")?;
    Ok(result.rows_affected() > 0)
}

/// Drop authorizations that expired more than a day ago.
pub async fn purge_expired(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM device_authorizations WHERE expires_at < now() - interval '1 day'",
    )
    .execute(pool)
    .await
    .context("purging device authorizations")?;
    Ok(result.rows_affected())
}
