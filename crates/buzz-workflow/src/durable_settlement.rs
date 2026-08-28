//! Durable run advancement, terminal settlement, and read-only projection.

use buzz_core::tenant::CommunityId;
use serde_json::json;
use uuid::Uuid;

use crate::durable::{self, DurableProgress};
use crate::{WorkflowEngine, WorkflowError};

/// Advance one durable run, settle its lifecycle once, and project its latest snapshot.
pub async fn advance_settle_and_project(
    engine: &WorkflowEngine,
    community: CommunityId,
    run_id: Uuid,
) -> Result<DurableProgress, WorkflowError> {
    let progress = durable::advance_run(engine, community, run_id).await?;
    let store = engine.db.agent_workflow_store();
    let metadata = json!({ "task_count": progress.task_count });
    if progress.all_complete {
        store
            .complete_active_run(community, run_id, progress.task_count as i32, &metadata)
            .await?;
    } else if progress.terminal_failure {
        store
            .fail_active_run(
                community,
                run_id,
                progress.task_count as i32,
                "durable_task_terminal_failure",
                "one or more durable tasks failed, were blocked, or were cancelled",
                &metadata,
            )
            .await?;
    }
    if let Ok(dispatcher) = engine.agent_dispatch() {
        if let Err(error) = dispatcher.publish_run_snapshot(community, run_id).await {
            tracing::warn!(%run_id, "Durable run snapshot projection failed: {error}");
        }
    }
    Ok(progress)
}
