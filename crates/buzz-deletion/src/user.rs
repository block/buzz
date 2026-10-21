//! PostgreSQL-only operator commands and dummy actions for user deletion.

use anyhow::Result;
use buzz_db::entity_deletion::{EntityClaim, EntityDeletionStore, StageRecord};
use clap::{Args, Subcommand};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{default_executor_id, print_json, LoopMode};

/// Commands for synthetic user-deletion demonstrations; no real erasure occurs.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Save a synthetic request. Repeating the same submission returns its row.
    Submit {
        /// Synthetic user label, for example demo:alice.
        #[arg(long)]
        target: String,
        /// Submitting operator identity.
        #[arg(long)]
        requested_by: String,
        /// Optional operator reason.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Prepare and freeze the three-participant-community demonstration plan.
    Inventory {
        /// Request UUID.
        id: Uuid,
    },
    /// List demonstration requests.
    List {
        /// Maximum number of rows.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
    /// Show inventory, approval, ownership, and all reached stages.
    Inspect {
        /// Request UUID.
        id: Uuid,
    },
    /// Independently approve the frozen demonstration inventory.
    Approve {
        /// Request UUID.
        id: Uuid,
        /// Must differ from the submitting operator.
        #[arg(long)]
        approved_by: String,
    },
    /// Run an approved demonstration through its remaining dummy stages.
    Run {
        /// Request UUID.
        id: Uuid,
        /// Executor identity, otherwise a unique process identity.
        #[arg(long)]
        executor_id: Option<String>,
        /// Controls for demonstrating failure and recovery.
        #[command(flatten)]
        controls: DemoControls,
    },
    /// Drain eligible synthetic requests; exits when none are runnable.
    Drain {
        /// Executor identity.
        #[arg(long)]
        executor_id: Option<String>,
    },
    /// Resume blocked work after operator remediation.
    Unblock {
        /// Request UUID.
        id: Uuid,
        /// Operator identity recorded in current-stage metadata.
        #[arg(long)]
        unblocked_by: String,
        /// Explanation of the remediation.
        #[arg(long)]
        reason: String,
    },
}

/// Bounded, explicit failure controls for an artificial workflow.
#[derive(Debug, Clone, Default, Args)]
pub struct DemoControls {
    /// Fail at this one-based plan sequence on each invocation using this flag.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=15))]
    pub fail_at: Option<u16>,
    /// Release ownership after this many successful steps in this invocation.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=15))]
    pub stop_after: Option<u16>,
    /// Delay each dummy action to exercise heartbeats or process interruption.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=60000))]
    pub step_delay_ms: u64,
}

#[derive(Clone)]
pub(crate) struct Services {
    pub store: EntityDeletionStore,
    pub controls: DemoControls,
}

/// Execute one user-deletion command with a PostgreSQL-only connection.
pub async fn run(command: Command) -> Result<i32> {
    let db = crate::connect_database().await?;
    run_with_store(command, db.entity_deletion_store()).await
}

pub(crate) async fn run_with_store(command: Command, store: EntityDeletionStore) -> Result<i32> {
    match command {
        Command::Submit {
            target,
            requested_by,
            reason,
        } => {
            print_json(
                &store
                    .submit(&target, &requested_by, reason.as_deref())
                    .await?,
            )?;
        }
        Command::Inventory { id } => print_json(&store.prepare_inventory(id).await?)?,
        Command::List { limit } => print_json(&store.list(limit).await?)?,
        Command::Inspect { id } => print_json(&store.inspect(id).await?)?,
        Command::Approve { id, approved_by } => {
            print_json(&store.approve(id, &approved_by).await?)?
        }
        Command::Unblock {
            id,
            unblocked_by,
            reason,
        } => {
            print_json(&store.unblock(id, &unblocked_by, &reason).await?)?;
        }
        Command::Run {
            id,
            executor_id,
            controls,
        } => {
            return crate::executor::run_loop(
                Services { store, controls },
                LoopMode::Run,
                Some(id),
                executor_id.unwrap_or_else(default_executor_id),
            )
            .await;
        }
        Command::Drain { executor_id } => {
            return crate::executor::run_loop(
                Services {
                    store,
                    controls: DemoControls::default(),
                },
                LoopMode::Drain,
                None,
                executor_id.unwrap_or_else(default_executor_id),
            )
            .await;
        }
    }
    Ok(0)
}

pub(crate) async fn execute_stage(
    services: &Services,
    claim: &EntityClaim,
    expected: &StageRecord,
    heartbeat_lost: &CancellationToken,
) -> Result<()> {
    let stage = services.store.start_stage(&claim.lease).await?;
    if stage.id != expected.id {
        anyhow::bail!("current user stage changed before dispatch");
    }
    let scope = stage
        .target_community_id
        .map_or_else(|| "request".into(), |id| id.to_string());
    let message = format!(
        "DEMO ONLY: would execute {} for {} in {}; no user data is deleted",
        stage.stage, claim.request.target, scope,
    );
    eprintln!("{message}");
    tokio::select! {
        biased;
        _ = heartbeat_lost.cancelled() => return Err(crate::DeletionLeaseLost.into()),
        _ = tokio::time::sleep(std::time::Duration::from_millis(services.controls.step_delay_ms)) => {}
    }
    if services.controls.fail_at.map(i32::from) == Some(stage.sequence) {
        return Err(crate::transient(format!(
            "injected demo failure at stage {}",
            stage.sequence
        )));
    }
    services
        .store
        .complete_stage(
            &claim.lease,
            stage.id,
            &serde_json::json!({"message": message, "simulated": true}),
        )
        .await?;
    Ok(())
}
