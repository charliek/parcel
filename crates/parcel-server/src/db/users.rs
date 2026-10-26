//! Users, and the provider identities they sign in with.

use anyhow::Result;
use sqlx::{AssertSqlSafe, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::error::DbContext;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub session_epoch: i32,
    pub last_login_at: OffsetDateTime,
    pub disabled_at: Option<OffsetDateTime>,
}

const USER_COLUMNS: &str = "id, email, name, session_epoch, last_login_at, disabled_at";

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(AssertSqlSafe(format!(
        "SELECT {USER_COLUMNS} FROM users WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await
    .db_context("finding user by id")
}

pub async fn find_by_identity(
    pool: &PgPool,
    provider: &str,
    subject: &str,
) -> Result<Option<User>> {
    sqlx::query_as::<_, User>(AssertSqlSafe(format!(
        "SELECT {USER_COLUMNS} FROM users
         WHERE id = (SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2)"
    )))
    .bind(provider)
    .bind(subject)
    .fetch_optional(pool)
    .await
    .db_context("finding user by identity")
}

/// An interactive sign-in through `provider`, whose email the caller has
/// already checked is trusted and allowed. Returns the user, which is
/// (in order) the one this identity signed in as before, the one with the
/// same email (linking a second provider to the same person), or a new one.
/// Refreshes email and name from the provider and stamps the sign-in.
pub async fn sign_in(
    pool: &PgPool,
    provider: &str,
    subject: &str,
    email: &str,
    name: &str,
) -> Result<User> {
    let mut tx = pool.begin().await.db_context("beginning sign-in")?;
    // Serialize sign-ins per email, so two first sign-ins (through two
    // providers, or the same one twice) can't both create a user.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(lower($1), 0))")
        .bind(email)
        .execute(&mut *tx)
        .await
        .db_context("locking email for sign-in")?;
    let known: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2 FOR UPDATE",
    )
    .bind(provider)
    .bind(subject)
    .fetch_optional(&mut *tx)
    .await
    .db_context("finding identity")?;

    let user_id = match known {
        Some(id) => id,
        None => {
            let same_email: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM users WHERE lower(email) = lower($1)
                 ORDER BY created_at LIMIT 1",
            )
            .bind(email)
            .fetch_optional(&mut *tx)
            .await
            .db_context("finding user by email")?;
            let id = match same_email {
                Some(id) => id,
                None => sqlx::query_scalar(
                    "INSERT INTO users (email, name) VALUES ($1, $2) RETURNING id",
                )
                .bind(email)
                .bind(name)
                .fetch_one(&mut *tx)
                .await
                .db_context("creating user")?,
            };
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO user_identities (provider, subject, user_id, email)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (provider, subject) DO UPDATE SET email = EXCLUDED.email
                 RETURNING user_id",
            )
            .bind(provider)
            .bind(subject)
            .bind(id)
            .bind(email)
            .fetch_one(&mut *tx)
            .await
            .db_context("linking identity")?
        }
    };

    sqlx::query(
        "UPDATE user_identities SET email = $3, last_login_at = now()
         WHERE provider = $1 AND subject = $2",
    )
    .bind(provider)
    .bind(subject)
    .bind(email)
    .execute(&mut *tx)
    .await
    .db_context("stamping identity")?;
    // Keep a name the provider didn't send rather than blanking it.
    let user = sqlx::query_as::<_, User>(AssertSqlSafe(format!(
        "UPDATE users SET email = $2, name = CASE WHEN $3 = '' THEN name ELSE $3 END,
                last_login_at = now(), last_seen_at = now()
         WHERE id = $1 RETURNING {USER_COLUMNS}"
    )))
    .bind(user_id)
    .bind(email)
    .bind(name)
    .fetch_one(&mut *tx)
    .await
    .db_context("updating user at sign-in")?;
    // A new user gets their handle here, in the transaction that creates
    // them; one an older revision created gets theirs now (a no-op for
    // everyone else).
    super::handles::allocate(&mut *tx, user.id, &user.email).await?;
    tx.commit().await.db_context("committing sign-in")?;
    Ok(user)
}

/// End every session of a user ("sign out everywhere"), if `epoch` is
/// still theirs and they are enabled; false otherwise.
pub async fn bump_session_epoch(pool: &PgPool, id: Uuid, epoch: i32) -> Result<bool> {
    let bumped = sqlx::query(
        "UPDATE users SET session_epoch = session_epoch + 1
         WHERE id = $1 AND session_epoch = $2 AND disabled_at IS NULL",
    )
    .bind(id)
    .bind(epoch)
    .execute(pool)
    .await
    .db_context("bumping session epoch")?
    .rows_affected();
    Ok(bumped > 0)
}

/// Display name: the name, else the email.
pub fn display_name(name: &str, email: &str) -> String {
    if name.trim().is_empty() {
        email.to_string()
    } else {
        name.to_string()
    }
}
