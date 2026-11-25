//! Standalone media drain handler. Access gating is the caller's prerequisite.

use anyhow::{ensure, Result};
use buzz_db::{deletion::MediaUploadActivity, Db, DbConfig};

/// Inspect media upload activity without waiting, polling, or deleting data.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key (hex or npub); includes community-scoped owned agents.
    #[arg(long, value_parser = buzz_db::user_deletion::normalize_user_deletion_pubkey)]
    pub pubkey: String,
    /// Confirm all upload replicas track leases and reject new uploads for this user and agents.
    #[arg(long, required = true)]
    pub uploads_blocked: bool,
}

/// Evidence from this invocation only; never a substitute for persistent gating.
#[derive(Debug, serde::Serialize)]
pub struct MediaDrainReport {
    /// Canonical human identity inspected.
    pub pubkey: String,
    /// This check covers media uploads only.
    pub scope: &'static str,
    /// No active, unresolved, or unattributed uploads were observed.
    pub drained: bool,
    /// Per-community counts; unrelated attributed uploads are excluded.
    pub activity: Vec<MediaUploadActivity>,
}

/// Observe once against the primary database. The caller must establish upload
/// gating on every replica before passing `uploads_blocked`; a false prerequisite
/// is rejected. No workflow stage is advanced and no access policy is changed.
pub async fn inspect(db: &Db, pubkey: &str, uploads_blocked: bool) -> Result<MediaDrainReport> {
    ensure!(
        uploads_blocked,
        "block new uploads on every replica before checking media drain"
    );
    let pubkey = buzz_db::user_deletion::normalize_user_deletion_pubkey(pubkey)?;
    let activity = db
        .deletion_store()
        .user_media_upload_activity(&pubkey)
        .await?;
    Ok(MediaDrainReport {
        pubkey,
        scope: "media_upload",
        drained: activity.is_empty(),
        activity,
    })
}

/// Fail immediately if an upload is active or its completion is uncertain.
/// A future workflow caller may save success only when this function succeeds.
pub async fn handle(db: &Db, pubkey: &str, uploads_blocked: bool) -> Result<MediaDrainReport> {
    let report = inspect(db, pubkey, uploads_blocked).await?;
    require_drained(&report)?;
    Ok(report)
}

fn require_drained(report: &MediaDrainReport) -> Result<()> {
    let active: i64 = report.activity.iter().map(|r| r.active).sum();
    let unresolved: i64 = report.activity.iter().map(|r| r.unresolved).sum();
    let unattributed: i64 = report.activity.iter().map(|r| r.unattributed).sum();
    ensure!(report.drained,
        "cannot continue: media uploads active={active}, unresolved={unresolved}, unattributed={unattributed}; user must remain blocked; retry after uploads finish or unresolved operations are reconciled");
    Ok(())
}

/// Run the read-only command. Busy returns a nonzero result with JSON evidence.
pub async fn run(args: Args) -> Result<i32> {
    let db = Db::connect_cold_start(
        DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: true,
            ..DbConfig::default()
        },
        "user_media_drain",
    )
    .await?;
    let report = inspect(&db, &args.pubkey, args.uploads_blocked).await?;
    crate::print_json(&report)?;
    require_drained(&report)?;
    Ok(0)
}

#[cfg(test)]
#[path = "wait_for_draining_postgres_tests.rs"]
mod postgres_tests;
