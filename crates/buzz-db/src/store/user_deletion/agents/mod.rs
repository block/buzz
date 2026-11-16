//! Bounded agent database operations and atomic per-agent checkpoints.
use super::{execution, messages, UserDeletionInventory, UserDeletionStore};
use crate::entity_deletion::{lock_owned, validate_approved_inventory, EntityLease, EntityStage};
use crate::{AdmittedTx, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use execution::safety;
use uuid::Uuid;

mod delivery;
mod model;
mod operations;
mod workflows;
pub use model::{AgentDbPhase, AgentDbProgress, AgentDbStatus, AgentsDbBatch, AgentsDbProgress};

impl UserDeletionStore {
    /// Execute one bounded operation for the next unfinished approved agent.
    ///
    /// `advance` is the trusted handler's phase policy, called only when the
    /// current operation is exhausted. SQL effects and policy progress commit
    /// together. This does not select workflow stages or release ownership.
    /// Profiles/configuration, credentials, memberships, media and hosted resources
    /// are deliberately retained for finalization. Callers must gate and drain
    /// all affected writers before using this standalone operation.
    pub async fn purge_agents_db_batch<F>(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        community: CommunityId,
        relay_pubkey: &[u8; 32],
        batch_size: u16,
        advance: F,
    ) -> Result<AgentsDbBatch>
    where
        F: FnOnce(&mut AgentDbProgress, DateTime<Utc>),
    {
        if !(1..=messages::MAX_BATCH_SIZE).contains(&batch_size) {
            return Err(safety("agent batch size must be 1..=100"));
        }
        let (mut tx, stage, inventory, owner) = execution::begin(
            self,
            lease,
            stage_id,
            community,
            relay_pubkey,
            "purge_agents_db",
        )
        .await?;
        let agents = inventory.agents(*community.as_uuid(), &owner)?;
        if agents.iter().any(|key| key == &hex::encode(relay_pubkey)) {
            return Err(safety("cannot purge the relay signing identity"));
        }
        // The approved list is stable. Never silently include newly owned agents
        // or falsely finish an old inventory which omitted them.
        let unexpected: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE community_id=$1
             AND agent_owner_pubkey=decode($2,'hex') AND NOT (encode(pubkey,'hex')=ANY($3)))",
        )
        .bind(community.as_uuid())
        .bind(&owner)
        .bind(&agents)
        .fetch_one(tx.conn())
        .await?;
        if unexpected {
            return Err(safety(
                "owned agents changed since approval; reconcile the inventory",
            ));
        }
        let mut progress = AgentsDbProgress::read(&stage, &agents)?;
        if let Some(agent) = progress.next_agent(&agents).map(str::to_owned) {
            validate_agent(&mut tx, lease, &owner, &agent).await?;
            progress.current_agent = Some(agent.clone());
            let item = progress
                .agents
                .get_mut(&agent)
                .ok_or_else(|| safety("missing agent progress"))?;
            item.status = AgentDbStatus::Running;
            item.last_error = None;
            let subject = hex::decode(&agent).map_err(|_| safety("invalid agent key"))?;
            let exhausted = match item.phase {
                AgentDbPhase::Messages => {
                    messages::purge_for_subject(
                        &mut tx,
                        community,
                        &subject,
                        &agent,
                        relay_pubkey,
                        batch_size,
                    )
                    .await?
                    .exhausted
                }
                AgentDbPhase::PrivateState => {
                    operations::private_state(&mut tx, &subject, batch_size).await?
                }
                AgentDbPhase::JobRecords => {
                    messages::purge_event_kinds(
                        &mut tx,
                        &subject,
                        operations::JOB_KINDS,
                        batch_size,
                    )
                    .await?
                }
                AgentDbPhase::OwnedWorkflows => {
                    workflows::purge(&mut tx, &subject, batch_size).await?
                }
                AgentDbPhase::DeliveryState => {
                    delivery::purge(&mut tx, &subject, batch_size).await?
                }
                AgentDbPhase::VerifyDbContent => {
                    operations::verify(&mut tx, &subject, &agent, relay_pubkey).await?;
                    true
                }
            };
            if exhausted {
                let now = sqlx::query_scalar("SELECT clock_timestamp()")
                    .fetch_one(tx.conn())
                    .await?;
                advance(item, now);
            }
            if item.status == AgentDbStatus::Completed {
                progress.current_agent = None;
            }
        }
        let exhausted = progress.next_agent(&agents).is_none();
        // Validate the handler's serialized state before committing it.
        let mut check = stage.clone();
        check.metadata_json["progress"] = serde_json::to_value(&progress)?;
        AgentsDbProgress::read(&check, &agents)?;
        save_progress(&mut tx, stage_id, &progress).await?;
        execution::verify_live(&mut tx, lease).await?;
        tx.commit().await?;
        Ok(AgentsDbBatch {
            progress,
            exhausted,
        })
    }

    /// Save the current agent's failure and the normal stage/request retry state
    /// in one transaction. An expired/superseded worker cannot record failure.
    pub async fn fail_agents_db_stage(
        &self,
        lease: &EntityLease,
        stage_id: Uuid,
        message: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let request = lock_owned(&mut tx, lease).await?;
        validate_approved_inventory(&mut tx, &request).await?;
        let stage: EntityStage = sqlx::query_as(
            "SELECT id,request_id,target_community_id,stage,status,is_current,started_at,
             finished_at,metadata_json,last_error,updated_at FROM entity_deletion_stage_history
             WHERE id=$1 FOR UPDATE",
        )
        .bind(stage_id)
        .fetch_one(&mut *tx)
        .await?;
        let community = stage
            .target_community_id
            .ok_or_else(|| safety("missing agent community"))?;
        let inventory = UserDeletionInventory::read(&request)?;
        if stage.request_id != lease.request_id
            || stage.stage != "purge_agents_db"
            || stage.status != "running"
            || !stage.is_current
            || stage.metadata_json["inputs"]
                != inventory.inputs(&request.target, Some(community))?
        {
            return Err(safety("expected current running agent stage"));
        }
        let agents = inventory.agents(community, &request.target)?;
        let mut progress = AgentsDbProgress::read(&stage, &agents)?;
        let message: String = message.chars().take(2048).collect();
        if let Some(agent) = progress.next_agent(&agents).map(str::to_owned) {
            progress.current_agent = Some(agent.clone());
            let item = progress
                .agents
                .get_mut(&agent)
                .ok_or_else(|| safety("missing agent progress"))?;
            item.status = AgentDbStatus::Failed;
            item.last_error = Some(message.clone());
        }
        sqlx::query(
            "UPDATE entity_deletion_stage_history SET status='failed',last_error=$2,
             metadata_json=jsonb_set(metadata_json,'{progress}',$3),updated_at=clock_timestamp() WHERE id=$1",
        )
        .bind(stage_id)
        .bind(&message)
        .bind(serde_json::to_value(progress)?)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE entity_deletion_requests SET status='retry_wait',retry_count=retry_count+1,
             last_error=$2,next_attempt_at=clock_timestamp()+interval '30 seconds',updated_at=clock_timestamp()
             WHERE id=$1",
        )
        .bind(lease.request_id)
        .bind(message)
        .execute(&mut *tx)
        .await?;
        lock_owned(&mut tx, lease).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn validate_agent(
    tx: &mut AdmittedTx,
    lease: &EntityLease,
    owner: &str,
    agent: &str,
) -> Result<()> {
    let community = tx.community();
    let proof: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT request_id,source_owner_pubkey FROM user_deletion_fences
         WHERE scope_community_id=$1 AND pubkey=$2",
    )
    .bind(community.as_uuid())
    .bind(agent)
    .fetch_optional(tx.conn())
    .await?;
    if proof != Some((lease.request_id, Some(owner.to_owned()))) {
        return Err(safety(
            "approved agent must have this request's community fence and owner provenance",
        ));
    }
    // Missing rows are allowed on recovery; conflicting live ownership is not.
    let live: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex')",
    )
    .bind(community.as_uuid())
    .bind(agent)
    .fetch_optional(tx.conn())
    .await?;
    let owner = hex::decode(owner).map_err(|_| safety("invalid owner"))?;
    if live.is_some_and(|live| live.as_deref() != Some(owner.as_slice())) {
        return Err(safety(
            "agent ownership differs from the approved deletion scope",
        ));
    }
    Ok(())
}

async fn save_progress(
    tx: &mut AdmittedTx,
    stage: Uuid,
    progress: &AgentsDbProgress,
) -> Result<()> {
    sqlx::query(
        "UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{progress}',$2),
         updated_at=clock_timestamp() WHERE id=$1",
    ).bind(stage).bind(serde_json::to_value(progress)?).execute(tx.conn()).await?;
    Ok(())
}
