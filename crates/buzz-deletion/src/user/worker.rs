//! Ownership and dispatch only; get_next_stage owns all workflow routing.

use std::time::Duration;

use anyhow::{bail, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage, NextStage};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::handlers::HandlerProgress;
use super::{handlers, transition::UserDeletionTransition};
use crate::{DEFAULT_LEASE_DURATION, HEARTBEAT_INTERVAL};

/// Real execution controls and demo-only failure/delay options.
pub(super) struct RunOptions {
    pub db: Option<buzz_db::Db>,
    pub media: Option<std::sync::Arc<buzz_media::MediaStorage>>,
    pub delete: bool,
    pub row_ids: Option<std::fs::File>,
    pub relay_pubkey: Option<String>,
    pub batch_size: u16,
    pub max_items: u32,
    pub next_step: bool,
    pub fail_stage: Option<String>,
    pub stage_delay: Duration,
    pub no_agents_in: Vec<Uuid>,
}

pub(super) async fn run(
    store: &EntityDeletionStore,
    id: Uuid,
    owner: &str,
    options: &RunOptions,
    shutdown: &CancellationToken,
) -> Result<i32> {
    // The command selects this workflow; shared storage accepts other entity types.
    let request = store.get(id).await?;
    if request.entity_type != "user" {
        bail!(
            "user-deletions run does not support entity type {}",
            request.entity_type
        );
    }
    if !request.target.starts_with("demo:") {
        // Preserve opaque account identities in existing inventories. A mode-tagged
        // manifest from an unsupported workflow must never authorize deletion.
        anyhow::ensure!(
            request
                .inventory_manifest
                .as_ref()
                .is_none_or(|m| m.get("execution_mode").is_none()),
            "unsupported saved execution mode"
        );
        anyhow::ensure!(options.delete != request.target.starts_with("read-only:"),
            "real workflow execution requires --delete for a deletion request or --read-only for a read-only request");
        anyhow::ensure!(options.db.is_some(), "real workflow requires a database");
    }
    if options.delete
        && !request.target.starts_with("demo:")
        && matches!(request.status.as_str(), "completed" | "running")
    {
        store.resume_delegated_review(id).await?;
    }
    let Some(lease) = store.claim(id, owner, DEFAULT_LEASE_DURATION).await? else {
        if store.get(id).await?.status == "completed" {
            return Ok(0);
        }
        bail!("request is unapproved, not yet due for retry, or leased by another worker");
    };
    let result = execute_claim(store, &lease, options, shutdown).await;
    // This generation-checked release is harmless after completion, a recorded
    // failure, or takeover. Never leave a background heartbeat running on return.
    let released = store.release(&lease).await;
    finish_claim(result, released)
}

fn finish_claim(result: Result<i32>, released: buzz_db::Result<bool>) -> Result<i32> {
    match (result, released) {
        (Err(error), Err(release_error)) => {
            // buzz-admin prints Display, so the outer message must include both.
            let message =
                format!("{error:#}; also failed to release deletion lease: {release_error}");
            Err(error.context(message))
        }
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(code), Ok(_)) => Ok(code),
    }
}

async fn execute_claim(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    options: &RunOptions,
    shutdown: &CancellationToken,
) -> Result<i32> {
    let transition = UserDeletionTransition::new(store);
    let mut previous_stage_id = None;
    let mut executed = 0;
    // Keep ordinary runs bounded; stepping uses the same transition boundary.
    let handler_budget = if options.next_step { 1 } else { 100 };
    loop {
        if shutdown.is_cancelled() {
            return Ok(0);
        }
        match transition.get_next_stage(lease, previous_stage_id).await? {
            NextStage::Finished => return Ok(0),
            NextStage::WaitUntil(deadline) => {
                eprintln!("request {} waits until {deadline}", lease.request_id);
                return Ok(1);
            }
            NextStage::Stage(row) => {
                if executed >= handler_budget {
                    return Ok(0);
                }
                match run_stage_with_heartbeat(
                    store,
                    lease,
                    &row,
                    options,
                    shutdown,
                    HEARTBEAT_INTERVAL,
                )
                .await?
                {
                    HandlerProgress::Advance => {}
                    HandlerProgress::Stop => return Ok(0),
                }
                previous_stage_id = Some(row.id);
                executed += 1;
            }
        }
    }
}

async fn run_stage_with_heartbeat(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    stage: &EntityStage,
    options: &RunOptions,
    shutdown: &CancellationToken,
    heartbeat_period: Duration,
) -> Result<HandlerProgress> {
    let handler = handlers::dispatch(store, lease, stage, options);
    tokio::pin!(handler);
    let mut interval = tokio::time::interval(heartbeat_period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Poll renewal alongside the handler. Awaiting renewal inside a selected
    // timer branch would stop polling a handler that may hold the parent lock.
    // Both futures live here, so cancellation leaves no detached heartbeat task.
    // A handler can finish mid-heartbeat; `heartbeat` is one autocommit
    // statement so that drop cannot strand an open transaction on the pool.
    let renewal = async {
        loop {
            interval.tick().await;
            if let Err(error) = store.heartbeat(lease, DEFAULT_LEASE_DURATION).await {
                return error;
            }
        }
    };
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => Ok(HandlerProgress::Stop),
        result = &mut handler => result,
        error = renewal => Err(error.into()),
    }
}

#[cfg(test)]
#[path = "worker_postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
#[path = "account_worker_postgres_tests.rs"]
mod account_postgres_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_failure_preserves_execution_error_and_reports_both() {
        let original = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "handler failed");
        let error = finish_claim(
            Err(original.into()),
            Err(buzz_db::DbError::InvalidData("release failed".into())),
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        let report = error.to_string();
        assert!(report.contains("handler failed"));
        assert!(report.contains("release failed"));
    }

    #[test]
    fn release_failure_after_success_is_not_swallowed() {
        assert!(finish_claim(
            Ok(0),
            Err(buzz_db::DbError::InvalidData("release failed".into()))
        )
        .is_err());
        assert_eq!(finish_claim(Ok(1), Ok(false)).unwrap(), 1);
    }
}
