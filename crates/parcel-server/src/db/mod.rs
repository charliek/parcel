//! Database access: pool, migrations, and per-table repositories.
//!
//! Queries use sqlx's runtime API (no compile-time database), keeping `cargo
//! build` and CI free of a live Postgres. Repository functions return
//! `anyhow::Result`; handlers convert at the `AppError::Internal` boundary.

pub mod blobs;
pub mod comments;
pub mod data;
pub mod device;
pub mod parcels;
pub mod shares;
pub mod tokens;
pub mod users;

use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// A pool that connects on first use, so building state (and every
/// non-database route) works before Postgres answers.
pub fn connect_lazy(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(15)
        .acquire_timeout(Duration::from_secs(30))
        .connect_lazy(database_url)
        .context("creating Postgres connection pool")
}

/// Apply embedded migrations (idempotent). sqlx takes an advisory lock, so
/// concurrently booting instances serialize here rather than racing.
pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("running database migrations")
}

/// Readiness probe: one round trip, bounded.
pub async fn ping(pool: &PgPool) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(pool),
    )
    .await
    .context("database ping timed out")?
    .context("database ping failed")?;
    Ok(())
}
