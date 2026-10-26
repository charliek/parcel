//! `pcl_…` API tokens. Only the SHA-256 of a token is ever stored.

use anyhow::Result;
use parcel_types::ApiToken;
use sqlx::{AssertSqlSafe, PgExecutor, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::error::DbContext;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub prefix_display: String,
    pub created_via: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub expires_at: Option<OffsetDateTime>,
}

impl From<TokenRow> for ApiToken {
    fn from(t: TokenRow) -> Self {
        ApiToken {
            id: t.id.to_string(),
            name: t.name,
            prefix: t.prefix_display,
            created_via: t.created_via,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            expires_at: t.expires_at,
        }
    }
}

const COLUMNS: &str =
    "id, user_id, name, prefix_display, created_via, created_at, last_used_at, expires_at";

pub struct NewToken<'a> {
    pub user_id: Uuid,
    pub name: &'a str,
    pub hash: &'a str,
    pub prefix_display: &'a str,
    pub created_via: &'a str,
    pub expires_at: Option<OffsetDateTime>,
}

pub async fn insert<'e>(db: impl PgExecutor<'e>, t: &NewToken<'_>) -> Result<TokenRow> {
    sqlx::query_as::<_, TokenRow>(AssertSqlSafe(format!(
        "INSERT INTO api_tokens (user_id, name, token_hash, prefix_display, created_via, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {COLUMNS}"
    )))
    .bind(t.user_id)
    .bind(t.name)
    .bind(t.hash)
    .bind(t.prefix_display)
    .bind(t.created_via)
    .bind(t.expires_at)
    .fetch_one(db)
    .await
    .db_context("inserting token")
}

/// A live (not revoked, not expired) token by hash.
pub async fn find_active_by_hash(pool: &PgPool, hash: &str) -> Result<Option<TokenRow>> {
    sqlx::query_as::<_, TokenRow>(AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM api_tokens
         WHERE token_hash = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())"
    )))
    .bind(hash)
    .fetch_optional(pool)
    .await
    .db_context("finding token")
}

pub async fn list_active(pool: &PgPool, user_id: Uuid) -> Result<Vec<TokenRow>> {
    sqlx::query_as::<_, TokenRow>(AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM api_tokens
         WHERE user_id = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
         ORDER BY created_at DESC"
    )))
    .bind(user_id)
    .fetch_all(pool)
    .await
    .db_context("listing tokens")
}

/// Revoke one of the user's tokens. Returns false if it wasn't theirs or was
/// already revoked.
pub async fn revoke(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .db_context("revoking token")?;
    Ok(result.rows_affected() > 0)
}

/// Best-effort last-used stamp, at most once a minute per token.
pub async fn touch_last_used(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE api_tokens SET last_used_at = now()
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
    )
    .bind(id)
    .execute(pool)
    .await
    .db_context("touching token")?;
    Ok(())
}
