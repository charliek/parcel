//! Users, keyed by their slauth `sub`.

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub sub: String,
    pub email: String,
    pub name: String,
}

/// Find-or-create by `sub`, refreshing email/name from the latest token so a
/// renamed account shows its current details. The update is conditional, so
/// the common case (nothing changed, seen recently) writes nothing.
pub async fn upsert_by_sub(pool: &PgPool, sub: &str, email: &str, name: &str) -> Result<User> {
    let updated = sqlx::query_as::<_, User>(
        "INSERT INTO users (sub, email, name) VALUES ($1, $2, $3)
         ON CONFLICT (sub) DO UPDATE
             SET email = EXCLUDED.email,
                 name = EXCLUDED.name,
                 last_seen_at = now()
             WHERE users.email IS DISTINCT FROM EXCLUDED.email
                OR users.name IS DISTINCT FROM EXCLUDED.name
                OR users.last_seen_at < now() - interval '5 minutes'
         RETURNING id, sub, email, name",
    )
    .bind(sub)
    .bind(email)
    .bind(name)
    .fetch_optional(pool)
    .await
    .context("upserting user")?;
    match updated {
        Some(user) => Ok(user),
        // The conditional DO UPDATE returns no row when nothing changed.
        None => find_by_sub(pool, sub)
            .await?
            .context("user vanished during upsert"),
    }
}

pub async fn find_by_sub(pool: &PgPool, sub: &str) -> Result<Option<User>> {
    sqlx::query_as::<_, User>("SELECT id, sub, email, name FROM users WHERE sub = $1")
        .bind(sub)
        .fetch_optional(pool)
        .await
        .context("finding user by sub")
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    sqlx::query_as::<_, User>("SELECT id, sub, email, name FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("finding user by id")
}
