//! Action sink trait — interface for workflow side-effects.
//!
//! The relay implements [`ActionSink`] to provide direct DB access to the
//! executor, replacing the HTTP loopback pattern.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use buzz_core::tenant::CommunityId;
use uuid::Uuid;

use crate::executor::WorkflowCause;

/// Provenance attached to a workflow-generated agent doorbell.
#[derive(Debug, Clone)]
pub struct DoorbellContext {
    /// Exact owner-signed kind:30620 definition revision.
    pub definition_event_id: String,
    /// Semantic cause of this run.
    pub cause: WorkflowCause,
    /// Webhook-only untrusted external fields. Empty for signed causes.
    pub webhook_fields: HashMap<String, String>,
}

/// Errors from action sink operations.
#[derive(Debug, thiserror::Error)]
pub enum ActionSinkError {
    /// An input parameter is malformed (e.g. invalid UUID).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The target channel does not exist.
    #[error("channel not found: {0}")]
    ChannelNotFound(String),
    /// The target channel is archived.
    #[error("channel is archived: {0}")]
    ChannelArchived(String),
    /// Nostr event construction or signing failed.
    #[error("event construction failed: {0}")]
    EventBuild(String),
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(String),
    /// Message content is empty or whitespace-only.
    #[error("empty message content")]
    EmptyContent,
}

impl From<ActionSinkError> for crate::WorkflowError {
    fn from(e: ActionSinkError) -> Self {
        crate::WorkflowError::WebhookError(e.to_string())
    }
}

/// Interface for workflow actions that produce side effects.
///
/// Implemented by the relay to provide direct DB/event access to the executor.
/// This replaces the HTTP loopback where the executor POSTed to the relay's
/// REST API (which failed with 401 auth errors).
///
/// Returns `Pin<Box<dyn Future>>` for dyn-compatibility — required because
/// `WorkflowEngine` stores `Arc<dyn ActionSink>`.
pub trait ActionSink: Send + Sync {
    /// Post a message to a channel on behalf of a workflow owner.
    ///
    /// - `community_id`: the server-resolved community that owns the workflow
    ///   run driving this side effect. The relay-signed message is published
    ///   under *this* community, never the deployment/default tenant — the run
    ///   carries its owning community so a workflow in community B posts into B
    ///   even though the side effect has no inbound connection to bind.
    /// - `workflow_id`: UUID of the owner-signed kind:30620 definition
    /// - `step_id`: ID of the `send_message` step being executed
    /// - `channel_id`: UUID string of the target channel
    /// - `text`: message body (must not be empty/whitespace-only)
    /// - `author_pubkey`: hex-encoded pubkey of the workflow owner (used for
    ///   the dedicated `workflow-owner` authority tag and a `p` attribution
    ///   tag; the relay keypair signs the event)
    ///
    /// Returns the event ID hex string on success.
    #[allow(clippy::too_many_arguments)]
    fn send_message(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        step_id: &str,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        doorbell: &DoorbellContext,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}
