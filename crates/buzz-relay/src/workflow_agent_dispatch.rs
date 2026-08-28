//! Relay-side dispatch of durable tasks to independent agent identities.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};

use buzz_core::kind::KIND_AGENT_WORKFLOW_TASK;
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_workflow::action_sink::{AgentDispatch, AgentDispatchError};
use uuid::Uuid;

use crate::handlers::event::dispatch_persistent_event;
use crate::state::AppState;

/// Relay bridge for durable task invitations addressed to independent workers.
///
/// The relay signs the coordinator invitation. The designated worker is carried
/// in a p tag and later signs its own checkpoint and artifact receipts.
pub struct RelayAgentDispatch {
    state: Weak<AppState>,
}

impl RelayAgentDispatch {
    /// Create a durable task dispatcher from shared relay state.
    pub fn new(state: &Arc<AppState>) -> Self {
        Self {
            state: Arc::downgrade(state),
        }
    }
}

impl AgentDispatch for RelayAgentDispatch {
    fn dispatch_task(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        run_id: Uuid,
        task_id: Uuid,
        agent_pubkey: &[u8],
        prompt: &str,
        output_schema: Option<&serde_json::Value>,
        checkpoint: Option<&serde_json::Value>,
    ) -> Pin<Box<dyn Future<Output = Result<String, AgentDispatchError>> + Send + '_>> {
        let agent_pubkey = agent_pubkey.to_vec();
        let prompt = prompt.to_owned();
        let output_schema = output_schema.cloned();
        let checkpoint = checkpoint.cloned();
        Box::pin(async move {
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| AgentDispatchError::Publish("relay is shutting down".into()))?;
            let host = state
                .db
                .lookup_community_host(community_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
                .ok_or_else(|| {
                    AgentDispatchError::Publish(format!(
                        "workflow run community {community_id} is not mapped to a host"
                    ))
                })?;
            let tenant = TenantContext::resolved(community_id, host);
            if !state
                .is_member_cached(community_id, channel_id, &agent_pubkey)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
            {
                return Err(AgentDispatchError::InvalidIdentity(format!(
                    "designated agent is not a member of channel {channel_id}"
                )));
            }
            let worker = nostr::PublicKey::from_slice(&agent_pubkey)
                .map_err(|error| AgentDispatchError::InvalidIdentity(error.to_string()))?;
            let run = state
                .db
                .get_workflow_run(community_id, run_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let content = serde_json::to_string(&serde_json::json!({
                "status": "assigned",
                "prompt": prompt,
                "output_schema": output_schema,
                "checkpoint": checkpoint,
            }))
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let worker_hex = worker.to_hex();
            let event = buzz_sdk::build_agent_workflow_task(
                channel_id,
                run.workflow_id,
                run_id,
                task_id,
                &[worker_hex.as_str()],
                &content,
            )
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
            .sign_with_keys(&state.relay_keypair)
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let event_id = event.id.to_hex();
            let relay_pubkey = state.relay_keypair.public_key().to_hex();
            let (stored, inserted) = state
                .db
                .insert_event(community_id, &event, Some(channel_id))
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            if inserted {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    &stored,
                    KIND_AGENT_WORKFLOW_TASK,
                    &relay_pubkey,
                    None,
                )
                .await;
            }
            Ok(event_id)
        })
    }

    fn publish_artifact(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        run_id: Uuid,
        publish_task_id: Uuid,
        created_at_secs: u64,
        source_author: Option<&[u8]>,
        content: &serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<String, AgentDispatchError>> + Send + '_>> {
        let source_author = source_author.map(ToOwned::to_owned);
        let content = content.clone();
        Box::pin(async move {
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| AgentDispatchError::Publish("relay is shutting down".into()))?;
            let host = state
                .db
                .lookup_community_host(community_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
                .ok_or_else(|| {
                    AgentDispatchError::Publish(format!(
                        "workflow run community {community_id} is not mapped to a host"
                    ))
                })?;
            let tenant = TenantContext::resolved(community_id, host);
            let run = state
                .db
                .get_workflow_run(community_id, run_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let participant = source_author
                .as_deref()
                .map(nostr::PublicKey::from_slice)
                .transpose()
                .map_err(|error| AgentDispatchError::InvalidIdentity(error.to_string()))?
                .map(|pubkey| pubkey.to_hex());
            let participants = participant
                .as_deref()
                .map(|pubkey| vec![pubkey])
                .unwrap_or_default();
            let content = serde_json::to_string(&content)
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let event = buzz_sdk::build_agent_workflow_transition(
                channel_id,
                run.workflow_id,
                run_id,
                &participants,
                &content,
            )
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
            .custom_created_at(nostr::Timestamp::from(created_at_secs))
            .sign_with_keys(&state.relay_keypair)
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let event_id = event.id.to_hex();
            let relay_pubkey = state.relay_keypair.public_key().to_hex();
            let (stored, inserted) = state
                .db
                .insert_event(community_id, &event, Some(channel_id))
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            if inserted {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    &stored,
                    buzz_core::kind::KIND_AGENT_WORKFLOW_TRANSITION,
                    &relay_pubkey,
                    None,
                )
                .await;
            }
            tracing::debug!(%run_id, %publish_task_id, %event_id, "Published approved artifact projection");
            Ok(event_id)
        })
    }

    fn publish_run_snapshot(
        &self,
        community_id: CommunityId,
        run_id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<String, AgentDispatchError>> + Send + '_>> {
        Box::pin(async move {
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| AgentDispatchError::Publish("relay is shutting down".into()))?;
            let host = state
                .db
                .lookup_community_host(community_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
                .ok_or_else(|| {
                    AgentDispatchError::Publish(format!(
                        "workflow run community {community_id} is not mapped to a host"
                    ))
                })?;
            let tenant = TenantContext::resolved(community_id, host);
            let run = state
                .db
                .get_workflow_run(community_id, run_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let workflow = state
                .db
                .get_workflow(community_id, run.workflow_id)
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let channel_id = workflow.channel_id.ok_or_else(|| {
                AgentDispatchError::Publish("durable workflow has no channel".into())
            })?;
            let store = state.db.agent_workflow_store();
            let (run_state, tasks, artifacts, checkpoints, approvals, sequence) = tokio::try_join!(
                store.get_run_state(community_id, run_id),
                store.list_tasks(community_id, run_id, Some(1_000)),
                store.list_artifacts(community_id, run_id, Some(1_000)),
                store.list_latest_checkpoints(community_id, run_id),
                store.list_approvals(community_id, run_id),
                store.latest_transition_sequence(community_id, run_id),
            )
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let run_state = run_state.ok_or_else(|| {
                AgentDispatchError::Publish("durable workflow run state is missing".into())
            })?;
            let mut participant_keys = tasks
                .iter()
                .filter_map(|task| task.agent_pubkey.as_deref())
                .map(nostr::PublicKey::from_slice)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AgentDispatchError::InvalidIdentity(error.to_string()))?
                .into_iter()
                .map(|pubkey| pubkey.to_hex())
                .collect::<Vec<_>>();
            participant_keys.sort();
            participant_keys.dedup();
            let participant_refs = participant_keys
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let content = serde_json::json!({
                "run_id": run.id,
                "workflow_id": run.workflow_id,
                "status": run.status.to_string(),
                "phase": run_state.phase,
                "state_version": run_state.state_version,
                "manifest_sha256": run_state.manifest_hash.as_deref().map(hex::encode),
                "started_at": run.started_at,
                "completed_at": run.completed_at,
                "error_code": run.error_code,
                "tasks": tasks.iter().map(|task| serde_json::json!({
                    "id": task.id,
                    "task_key": task.task_key,
                    "phase": task.phase,
                    "status": task.status.to_string(),
                    "agent_pubkey": task.agent_pubkey.as_deref().map(hex::encode),
                    "attempt": task.attempt,
                    "max_attempts": task.max_attempts,
                    "not_before": task.not_before,
                    "started_at": task.started_at,
                    "completed_at": task.completed_at,
                    "error_code": task.error_code,
                    "version": task.version,
                })).collect::<Vec<_>>(),
                "checkpoints": checkpoints.iter().map(|checkpoint| serde_json::json!({
                    "task_id": checkpoint.task_id,
                    "sequence": checkpoint.sequence,
                    "artifact_id": checkpoint.artifact_id,
                    "created_at": checkpoint.created_at,
                })).collect::<Vec<_>>(),
                "artifacts": artifacts.iter().map(|artifact| serde_json::json!({
                    "id": artifact.id,
                    "task_id": artifact.task_id,
                    "kind": artifact.kind,
                    "version": artifact.version,
                    "content_type": artifact.content_type,
                    "uri": artifact.uri,
                    "sha256": hex::encode(&artifact.sha256),
                    "created_by": artifact.created_by.as_deref().map(hex::encode),
                    "created_at": artifact.created_at,
                })).collect::<Vec<_>>(),
                "approvals": approvals.iter().map(|approval| serde_json::json!({
                    "task_id": approval.task_id,
                    "step_id": approval.step_id,
                    "request_message": approval.request_message,
                    "approver_spec": approval.approver_spec,
                    "status": approval.status.to_string(),
                    "approver_pubkey": approval.approver_pubkey.as_deref().map(hex::encode),
                    "expires_at": approval.expires_at,
                })).collect::<Vec<_>>(),
                "transition_sequence": sequence,
            });
            let created_at = run
                .created_at
                .timestamp()
                .checked_add(sequence)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or_else(|| AgentDispatchError::Publish("snapshot timestamp overflow".into()))?;
            let content = serde_json::to_string(&content)
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let event = buzz_sdk::build_agent_workflow_run(
                channel_id,
                run.workflow_id,
                run_id,
                &participant_refs,
                &content,
            )
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(&state.relay_keypair)
            .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            let event_id = event.id.to_hex();
            let relay_pubkey = state.relay_keypair.public_key().to_hex();
            let (stored, inserted) = state
                .db
                .insert_event(community_id, &event, Some(channel_id))
                .await
                .map_err(|error| AgentDispatchError::Publish(error.to_string()))?;
            if inserted {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    &stored,
                    buzz_core::kind::KIND_AGENT_WORKFLOW_RUN,
                    &relay_pubkey,
                    None,
                )
                .await;
            }
            Ok(event_id)
        })
    }
}
