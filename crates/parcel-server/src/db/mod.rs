//! Database access: pool, migrations, and per-table repositories.
//!
//! Queries use sqlx's runtime API (no compile-time database), keeping `cargo
//! build` and CI free of a live Postgres. Repository functions return
//! `anyhow::Result`; handlers convert at the `AppError::Internal` boundary.
//! That boundary logs the whole chain, so every sqlx error becomes a
//! value-free [`error::DbError`] first: `.db_context(..)`, never
//! `.context(..)` or a bare `?` (`search` is the one exception, see there).
//!
//! Some queries splice a shared `const` column list or `SELECT` in with
//! `format!`, which sqlx 0.9 accepts only as `AssertSqlSafe(..)`. Each one
//! is spliced from `const` fragments and literals alone; every value goes
//! through `.bind`. Keep it that way.

pub mod blobs;
pub mod comments;
pub mod data;
pub mod device;
pub mod error;
pub mod handles;
pub mod parcels;
pub mod retry;
pub mod search;
pub mod shares;
pub mod slugs;
pub mod tags;
pub mod tokens;
pub mod users;

use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use stridelabs_postgres::{error_summary, PoolSettings, UrlRefusal};

use self::error::DbContext;

/// Why `DATABASE_URL` can't be used. Value-free by construction: every text
/// here names a parameter, a field or a host label from stridelabs-postgres's
/// own lists, never the URL or anything in it (invariant 5).
#[derive(Debug, thiserror::Error)]
pub enum DatabaseUrlError {
    /// stridelabs-postgres's allow-list refused it (an unknown parameter,
    /// `channel_binding`, a `search_path` in the startup options, …).
    #[error("DATABASE_URL was refused: {0}")]
    Refused(UrlRefusal),
    /// Migrations need a session: sqlx's migrator holds a session-level
    /// advisory lock, which a transaction pooler neither holds nor releases
    /// where the client thinks.
    /// The text is fixed: `PoolerRef` names the matched host label, which
    /// is still text from the URL, so it is never printed.
    #[error(
        "DATABASE_URL reaches a Neon pooled (`-pooler`) endpoint, and migrations need a direct \
         connection: use Neon's direct URL (the same host without `-pooler`) for \
         `parcel-server migrate`, or set RUN_MIGRATIONS=false on a server that uses the pooled URL"
    )]
    Pooler,
}

/// `DATABASE_URL` checked and parsed: the one place it reaches sqlx, so
/// sqlx never logs or quotes a parameter's value.
pub fn connect_options(database_url: &str) -> Result<PgConnectOptions, DatabaseUrlError> {
    stridelabs_postgres::connect_options(database_url).map_err(DatabaseUrlError::Refused)
}

/// Refuse a `DATABASE_URL` that reaches (or names) Neon's pooled endpoint,
/// or that [`connect_options`] refuses. Checked before `migrate` and before
/// `serve` with `RUN_MIGRATIONS`.
pub fn check_migration_url(database_url: &str) -> Result<(), DatabaseUrlError> {
    let options = connect_options(database_url)?;
    match stridelabs_postgres::neon_pooler_url(database_url, &options) {
        Some(_) => Err(DatabaseUrlError::Pooler),
        None => Ok(()),
    }
}

/// The service pool's settings: 15 connections, 30 s to wait for one.
fn pool_settings() -> PoolSettings {
    PoolSettings::default()
        .with_max_connections(15)
        .with_acquire_timeout(Duration::from_secs(30))
}

/// A pool that connects on first use, so building state (and every
/// non-database route) works before Postgres answers.
pub fn connect_lazy(database_url: &str) -> Result<PgPool> {
    let options = connect_options(database_url).context("creating Postgres connection pool")?;
    Ok(stridelabs_postgres::lazy_pool(options, pool_settings()))
}

/// Apply embedded migrations (idempotent). sqlx takes an advisory lock, so
/// concurrently booting instances serialize here rather than racing.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| anyhow::anyhow!("running database migrations: {}", migrate_error_summary(&e)))
}

/// A migration failure without the server's message text, which can echo a
/// value from the URL's startup options (invariant 5). sqlx's own variants
/// (version mismatches) carry only migration numbers and stay as they are.
fn migrate_error_summary(error: &sqlx::migrate::MigrateError) -> String {
    use sqlx::migrate::MigrateError;
    match error {
        MigrateError::Execute(e) => format!("while executing migrations: {}", error_summary(e)),
        MigrateError::ExecuteMigration(e, version) => {
            format!("while executing migration {version}: {}", error_summary(e))
        }
        MigrateError::Source(_) => "while resolving migrations".to_owned(),
        other => other.to_string(),
    }
}

/// Readiness probe: one round trip, bounded.
pub async fn ping(pool: &PgPool) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(pool),
    )
    .await
    .context("database ping timed out")?
    .db_context("database ping failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cret-canary";

    /// Neither the URL nor anything in it (the password above all) may
    /// appear in a refusal, however it is rendered.
    fn assert_value_free(message: &str, url: &str) {
        assert!(!message.contains(url), "{message}");
        assert!(!message.contains(SECRET), "{message}");
        assert!(!message.contains("postgres://"), "{message}");
    }

    #[test]
    fn migrations_refuse_a_neon_pooled_url_without_printing_it() {
        for url in [
            format!("postgres://app:{SECRET}@ep-cool-name-123456-pooler.us-east-2.aws.neon.tech/neondb?sslmode=verify-full"),
            // Named in the authority but redirected: still refused.
            format!("postgres://app:{SECRET}@ep-cool-name-123456-pooler.us-east-2.aws.neon.tech/neondb?host=127.0.0.1"),
        ] {
            let err = check_migration_url(&url).unwrap_err();
            assert!(matches!(err, DatabaseUrlError::Pooler), "{err}");
            let message = format!("{:#}", anyhow::Error::from(err));
            assert!(message.contains("-pooler"), "{message}");
            assert!(!message.contains("ep-cool-name"), "{message}");
            assert!(message.contains("RUN_MIGRATIONS=false"), "{message}");
            assert_value_free(&message, &url);
        }
        let direct = format!(
            "postgres://app:{SECRET}@ep-cool-name-123456.us-east-2.aws.neon.tech/neondb?sslmode=verify-full"
        );
        check_migration_url(&direct).expect("the direct URL migrates");
        // The service itself may use the pooled URL.
        connect_options(&direct.replace("123456.", "123456-pooler.")).expect("pooled is fine");
    }

    #[test]
    fn channel_binding_is_refused_by_name_without_printing_the_url() {
        let url = format!(
            "postgres://app:{SECRET}@ep-x.us-east-2.aws.neon.tech/neondb?sslmode=require&channel_binding=require"
        );
        for message in [
            check_migration_url(&url).unwrap_err().to_string(),
            format!("{:#}", connect_lazy(&url).unwrap_err()),
        ] {
            assert!(message.contains("`channel_binding`"), "{message}");
            assert_value_free(&message, &url);
        }
    }

    #[test]
    fn an_unknown_parameter_is_refused_without_its_value() {
        let url = format!("postgres://app@localhost:5439/parcel?token={SECRET}");
        let message = format!("{:#}", connect_lazy(&url).unwrap_err());
        assert!(message.contains("DATABASE_URL was refused"), "{message}");
        assert_value_free(&message, &url);
    }

    #[tokio::test]
    async fn the_local_url_builds_a_lazy_pool() {
        let pool = connect_lazy("postgres://postgres:localdev@localhost:5439/parcel").unwrap();
        assert_eq!(pool.options().get_max_connections(), 15);
        assert_eq!(
            pool.options().get_acquire_timeout(),
            Duration::from_secs(30)
        );
        assert_eq!(pool.size(), 0, "built without connecting");
    }
}
