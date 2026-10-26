//! Garbage collection of unreferenced blobs and stale device logins.
//!
//! Runs on an interval inside the server (and on demand via
//! `parcel-server gc`). On Cloud Run, CPU is throttled between requests, so the
//! interval is best-effort; nothing depends on GC promptness beyond storage cost.

use std::time::Duration;

use anyhow::Result;

use crate::db::error::DbContext;
use crate::db::{blobs, device};
use crate::state::AppState;
use crate::storage::blob_key;

const BATCH: i64 = 500;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    pub blobs_deleted: u64,
    pub device_codes_purged: u64,
}

/// Collect every blob that is unreferenced and untouched for `grace`.
///
/// Each batch deletes the objects while holding row locks on their blob rows,
/// and only then deletes the rows. A publish that wants one of those blobs
/// blocks on the row until this commits, finds it gone, and rewrites the
/// object — so an object is never deleted out from under a version.
pub async fn run_once(state: &AppState, grace: Duration) -> Result<GcReport> {
    let mut report = GcReport::default();
    loop {
        let mut tx = state
            .db
            .begin()
            .await
            .db_context("beginning blob collection")?;
        let locked = blobs::lock_collectable(&mut tx, grace, BATCH).await?;
        if locked.is_empty() {
            tx.rollback().await.db_context("ending blob collection")?;
            break;
        }
        let shas = blobs::still_unreferenced(&mut tx, &locked).await?;
        for sha in &shas {
            state.blobs.delete(&blob_key(sha)).await?;
            state.blob_cache.invalidate(sha).await;
        }
        report.blobs_deleted += blobs::delete_rows(&mut tx, &shas).await?;
        tx.commit().await.db_context("committing blob collection")?;
        if (locked.len() as i64) < BATCH {
            break;
        }
    }
    report.device_codes_purged = device::purge_expired(&state.db).await?;
    Ok(report)
}

/// Background loop for the server process.
pub fn spawn(state: AppState) {
    let interval = state.config.gc_interval;
    let grace = state.config.gc_grace;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // the first tick fires immediately; skip it
        loop {
            ticker.tick().await;
            match run_once(&state, grace).await {
                Ok(r) if r.blobs_deleted > 0 || r.device_codes_purged > 0 => {
                    tracing::info!(
                        blobs_deleted = r.blobs_deleted,
                        device_codes_purged = r.device_codes_purged,
                        "gc completed"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "gc failed"),
            }
        }
    });
}
