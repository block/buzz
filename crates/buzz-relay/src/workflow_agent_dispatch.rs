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
}
