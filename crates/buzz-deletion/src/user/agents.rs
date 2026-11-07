//! Standalone agent database cleanup. The main worker/router does not invoke it.
use anyhow::{bail, Context, Result};
use buzz_core::CommunityId;
use buzz_db::{
    entity_deletion::{EntityLease, EntityStage},
    user_deletion::agents::{AgentDbPhase, AgentDbStatus, AgentsDbBatch},
    Db,
};

/// Run one bounded operation and persist per-agent progress on the current stage.
///
/// Repeat while `exhausted` is false, renewing the request lease between calls.
/// Agents run in approved inventory order. Failed agents stop the stage and
/// resume in place after the normal retry deadline; completed agents are skipped.
/// Caller must gate/drain writers. Media-bearing events fail closed. Completion
/// means supported agent DB content only: profiles, memberships, credentials,
/// owner configuration, shared artifacts, media and hosted resources remain.
pub async fn handle(
    db: &Db,
    lease: &EntityLease,
    stage: &EntityStage,
    relay_pubkey: &[u8; 32],
    batch_size: u16,
) -> Result<AgentsDbBatch> {
    if stage.request_id != lease.request_id || stage.stage != "purge_agents_db" {
        bail!("expected this request's purge_agents_db stage");
    }
    let community = CommunityId::from_uuid(
        stage
            .target_community_id
            .context("missing agent community")?,
    );
    let store = db.entity_deletion_store();
    let stage = store.start_stage(lease, stage.id).await?;
    let agents = db.user_deletion_store();
    let result = agents
        .purge_agents_db_batch(
            lease,
            stage.id,
            community,
            relay_pubkey,
            batch_size,
            |agent, now| {
                // Business phase order lives here; database operations commit this
                // policy result in the same transaction as the completed batch.
                agent.phase = match agent.phase {
                    AgentDbPhase::Messages => AgentDbPhase::PrivateState,
                    AgentDbPhase::PrivateState => AgentDbPhase::JobRecords,
                    AgentDbPhase::JobRecords => AgentDbPhase::OwnedWorkflows,
                    AgentDbPhase::OwnedWorkflows => AgentDbPhase::DeliveryState,
                    AgentDbPhase::DeliveryState => AgentDbPhase::VerifyDbContent,
                    AgentDbPhase::VerifyDbContent => {
                        agent.status = AgentDbStatus::Completed;
                        agent.completed_at = Some(now);
                        AgentDbPhase::VerifyDbContent
                    }
                };
            },
        )
        .await;
    match result {
        Ok(batch) => {
            if batch.exhausted {
                store
                    .save_stage_success(
                        lease,
                        stage.id,
                        serde_json::json!({
                            "agent_db_content_exhausted": true,
                            "agents_completed": batch.progress.agents.len(),
                        }),
                    )
                    .await?;
            }
            Ok(batch)
        }
        Err(error) => {
            if let Err(save_error) = agents
                .fail_agents_db_stage(lease, stage.id, &error.to_string())
                .await
            {
                return Err(anyhow::Error::new(error)
                    .context(format!("also could not save agent retry: {save_error}")));
            }
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod postgres_tests;
