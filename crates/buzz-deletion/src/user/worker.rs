//! Ownership and dispatch only; get_next_stage owns all workflow routing.

use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use buzz_db::entity_deletion::{EntityDeletionStore, EntityLease, EntityStage, NextStage};
use buzz_db::user_deletion::{ExecutionMode, UserDeletionInventory};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{handlers, transition::UserDeletionTransition};
use crate::{StageOutcome, DEFAULT_LEASE_DURATION, HEARTBEAT_INTERVAL};

/// Real execution controls and demo-only failure/delay options.
pub(super) struct RunOptions {
    pub db: Option<buzz_db::Db>,
    pub media: Option<std::sync::Arc<buzz_media::MediaStorage>>,
    pub delete: bool,
    pub read_only: bool,
    pub community_store: Option<buzz_db::deletion::DeletionStore>,
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
    let demo = request.target.starts_with("demo:");
    if !demo || options.read_only {
        validate_mode(&request, options.delete, options.read_only)?;
    }
    if !demo {
        anyhow::ensure!(
            super::inventory::saved_relay_key(
                request.inventory_manifest.as_ref(),
                options.relay_pubkey.as_deref()
            )? == options.relay_pubkey,
            "worker relay key must match the saved discovery key"
        );
        if options.db.is_none() {
            bail!("real workflow requires a source database");
        }
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

pub(super) fn validate_mode(
    request: &buzz_db::entity_deletion::EntityRequest,
    delete: bool,
    read_only: bool,
) -> Result<()> {
    anyhow::ensure!(
        delete != read_only,
        "select exactly one worker mode: --delete or --read-only"
    );
    anyhow::ensure!(
        buzz_db::user_deletion::UserDeletionInventory::is_rehearsal(request)? == read_only,
        "worker mode differs from request; submit a separate rehearsal/deletion request"
    );
    Ok(())
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
    // A complete inventory has at most 1,000 communities plus request-wide stages.
    let handler_budget = if options.next_step { 1 } else { 10_000 };
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
                .await
                {
                    StageOutcome::Completed => {}
                    StageOutcome::Shutdown => return Ok(0),
                    StageOutcome::Failed(error) => return Err(error),
                }
                previous_stage_id = Some(row.id);
                executed += 1;
            }
        }
    }
}

pub(super) async fn execute_stage(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    row: &EntityStage,
    options: &RunOptions,
) -> Result<()> {
    let request = store.get(lease.request_id).await?;
    let demo = request.target.starts_with("demo:");
    if !demo || options.read_only {
        validate_mode(&request, options.delete, options.read_only)?;
    }
    if !demo {
        let inventory = UserDeletionInventory::read(&request)?;
        ensure!(
            row.request_id == request.id
                && row.metadata_json["inputs"]
                    == inventory.inputs(&request.target, row.target_community_id)?,
            "stage differs from approved inventory"
        );
    }
    let source = if demo {
        None
    } else {
        Some(options.db.as_ref().context("missing source database")?)
    };
    let row = store.start_stage(lease, row.id).await?;
    let result = if let Some(db) = source {
        let handler = handlers::Handler {
            db,
            request: &request,
            options,
            mode: if options.delete {
                ExecutionMode::Delete
            } else {
                ExecutionMode::ReadOnly
            },
            current: Some((lease, &row)),
        };
        handler.run(&row.stage, row.target_community_id).await
    } else {
        handlers::demo(&row, options).await
    };
    match result {
        Ok(metadata) => {
            store.save_stage_success(lease, row.id, metadata).await?;
            if options.read_only && row.stage == "purge_media" && !options.next_step {
                use std::io::Write;
                let _ = writeln!(std::io::stderr(), "Media preview saved. View file decisions now with: buzz-admin user-deletions inspect {}", request.id);
            }
        }
        Err(mut error) => {
            if let Some(child) = error.downcast_ref::<super::owned_community::BlockedChild>() {
                if let Err(save) = store
                    .save_stage_waiting(lease, row.id, child.metadata.clone())
                    .await
                {
                    error = error.context(format!("also could not save child observation: {save}"));
                }
            }
            if let Err(save) = store
                .fail_stage(lease, row.id, &format!("{error:#}"), crate::RETRY_DELAY)
                .await
            {
                return Err(error.context(format!("also could not record retry: {save}")));
            }
            // Demo failure injection returns the persisted retry deadline to the operator.
            if !demo {
                return Err(error);
            }
            eprintln!("dummy stage {} failed: {error:#}", row.stage);
        }
    }
    Ok(())
}

async fn run_stage_with_heartbeat(
    store: &EntityDeletionStore,
    lease: &EntityLease,
    stage: &EntityStage,
    options: &RunOptions,
    shutdown: &CancellationToken,
    heartbeat_period: Duration,
) -> StageOutcome {
    let handler = execute_stage(store, lease, stage, options);
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
        _ = shutdown.cancelled() => StageOutcome::Shutdown,
        result = &mut handler => match result {
            Ok(()) => StageOutcome::Completed,
            Err(error) => StageOutcome::Failed(error),
        },
        error = renewal => StageOutcome::Failed(error.into()),
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
