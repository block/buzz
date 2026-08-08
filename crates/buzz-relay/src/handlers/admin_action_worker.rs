//! Action recovery worker for stranded `relay_admin_actions`.
//!
//! Scans for `relay_admin_actions` rows in `pending` or `enforcing` state whose
//! action lease has expired (or was never set), claims them via an exclusive lease
//! (`SELECT FOR UPDATE SKIP LOCKED`), and re-drives each through the enforcement
//! state machine via `drive_enforcement`.
//!
//! This is the crash-recovery path: if the request-handling process dies between
//! claim and finalization, this worker picks up the stranded action and resumes
//! from the persisted `step_marker` state without re-running the mutation.
//!
//! Multiple pod replicas can run this worker concurrently — the DB-level action
//! lease (`action_lease_token` / `action_lease_expires_at`) prevents double-mutation.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

use buzz_db::relay_admin_actions::StrandedActionClaim;

use crate::state::AppState;

/// Lease duration for stranded-action claims. Long enough for the mutation to
/// complete; the HTTP driver uses 60 s, so we use 120 s for recovery.
const LEASE_SECS: i64 = 120;
/// Actions claimed per tick.
const BATCH_SIZE: i64 = 8;

/// Run the action recovery worker. Never returns; intended for `tokio::spawn`.
pub async fn run(state: Arc<AppState>) {
    let worker_id = format!("admin-action-worker-{}", Uuid::new_v4());
    info!(worker_id = %worker_id, "Admin action recovery worker started");

    let mut idle_delay = Duration::from_secs(5);

    loop {
        let lease_until = Utc::now() + chrono::Duration::seconds(LEASE_SECS);
        let batch = match state
            .db
            .claim_stranded_admin_action_batch(&worker_id, lease_until, BATCH_SIZE)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                error!(worker_id = %worker_id, "Admin action recovery claim failed: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        if batch.is_empty() {
            // Back off exponentially when idle, capped at 60 s.
            tokio::time::sleep(idle_delay).await;
            idle_delay = (idle_delay * 2).min(Duration::from_secs(60));
            continue;
        }

        idle_delay = Duration::from_secs(5);

        for claim in batch {
            recover_one(&state, claim).await;
        }
    }
}

/// Recover one stranded action from the batch claim.
/// Made pub(crate) for integration tests — allows tests to call through
/// the real production recovery path without the infinite worker loop.
pub(crate) async fn recover_one(state: &Arc<AppState>, claim: StrandedActionClaim) {
    let rec = &claim.record;
    let action_id = rec.id;

    // Resolve the community tenant for this action.
    let community_id = buzz_core::CommunityId::from_uuid(rec.report_community_id);
    let tenant = match state.db.lookup_community_host(community_id).await {
        Ok(Some(host)) => buzz_core::tenant::TenantContext::resolved(community_id, host),
        Ok(None) => {
            warn!(
                action_id = %action_id,
                "Action recovery: community not found, skipping"
            );
            return;
        }
        Err(e) => {
            warn!(
                action_id = %action_id,
                "Action recovery: host lookup failed: {e}"
            );
            return;
        }
    };

    info!(
        action_id = %action_id,
        report_id = %rec.report_id,
        state = %rec.state,
        step_marker = ?rec.step_marker,
        "Action recovery worker re-driving stranded action"
    );

    // Decode the target from the report row.
    let report = match state.db.admin_get_report(rec.report_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            warn!(action_id = %action_id, "Action recovery: report not found");
            return;
        }
        Err(e) => {
            warn!(action_id = %action_id, "Action recovery: report lookup failed: {e}");
            return;
        }
    };

    let (target_pubkey_opt, target_event_id_opt) =
        match crate::handlers::report_resolution::decode_report_target_pub(
            &report.report.target_kind,
            &report.report.target,
        ) {
            Ok(pair) => pair,
            Err(e) => {
                warn!(action_id = %action_id, "Action recovery: target decode failed: {e:?}");
                return;
            }
        };

    let timeout_until = rec.timeout_until;
    let action = rec.action.clone();
    let reason = rec.reason.clone();
    let actor_pubkey = rec.actor_pubkey.clone();
    let report_id = rec.report_id;
    let channel_id = report.report.channel_id;

    match crate::handlers::report_resolution::drive_enforcement_pub(
        state,
        &tenant,
        community_id,
        report_id,
        &action,
        reason.as_deref(),
        timeout_until,
        &actor_pubkey,
        target_pubkey_opt.as_deref(),
        target_event_id_opt.as_deref(),
        channel_id,
        rec,
        Some(claim.lease_token), // hold the batch-claim lease
    )
    .await
    {
        Ok(_) => {
            info!(action_id = %action_id, "Action recovery worker: action converged");
        }
        Err(e) => {
            warn!(
                action_id = %action_id,
                "Action recovery worker: re-drive failed: {e:?}"
            );
        }
    }
}
