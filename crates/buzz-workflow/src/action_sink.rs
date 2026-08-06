//! Action sink trait — interface for workflow side-effects.
//!
//! The relay implements [`ActionSink`] to provide direct DB access to the
//! executor, replacing the HTTP loopback pattern.

use std::future::Future;
use std::pin::Pin;

use buzz_core::tenant::CommunityId;
use serde_json::Value;
use uuid::Uuid;

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

/// A durable workflow lifecycle event to be signed and published by the relay.
///
/// The workflow engine owns the state transition; the relay owns the Nostr
/// event, its signature, persistence, and fan-out. Keeping the raw approval
/// token out of this shape ensures lifecycle events can expose only the stored
/// token hash used by approval actions.
#[derive(Clone, Debug)]
pub struct WorkflowLifecycleEvent {
    /// The workflow lifecycle kind (for example, 46001 or 46010).
    pub kind: u32,
    /// The workflow that owns this lifecycle record.
    pub workflow_id: Uuid,
    /// The workflow run represented by this record.
    pub run_id: Uuid,
    /// The channel that scopes the workflow, if it has one.
    pub channel_id: Option<Uuid>,
    /// The JSON wire payload consumed by Buzz clients.
    pub content: Value,
    /// The SHA-256 approval token hash, when this lifecycle event represents an approval.
    pub token_hash: Option<String>,
    /// Exact recipients for a targeted approval notification.
    pub target_pubkeys: Vec<String>,
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
    /// - `channel_id`: UUID string of the target channel
    /// - `text`: message body (must not be empty/whitespace-only)
    /// - `author_pubkey`: hex-encoded pubkey of the workflow owner (used for
    ///   the `p` attribution tag; the relay keypair signs the event)
    ///
    /// Returns the event ID hex string on success.
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Persist and fan out a relay-signed workflow lifecycle event.
    fn emit_workflow_lifecycle(
        &self,
        community_id: CommunityId,
        lifecycle: WorkflowLifecycleEvent,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}
