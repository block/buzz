//! Action sink trait — interface for workflow side-effects.
//!
//! The relay implements [`ActionSink`] to provide direct DB access to the
//! executor, replacing the HTTP loopback pattern.

use std::future::Future;
use std::pin::Pin;

use buzz_core::tenant::CommunityId;

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

    /// Publish the kind:46010 approval-requested event for a suspended run.
    ///
    /// Without this the run would wait on a token nobody has ever seen. The
    /// event carries the raw token so the approver can quote it back via
    /// `buzz workflows approve --token`.
    ///
    /// Note on the raw token in channel-visible content: the token identifies
    /// *which* gate is being answered, it is not the authorisation. The relay
    /// checks the grant's signing key against `approver_spec` independently, so
    /// reading the token does not let a non-approver pass the gate — unless the
    /// spec is `"any"`, in which case the token is the only barrier. Prefer a
    /// pubkey spec for anything with an external side effect.
    fn emit_approval_request(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        req: ApprovalRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}

/// Parameters for [`ActionSink::emit_approval_request`].
#[derive(Debug, Clone)]
pub struct ApprovalRequest<'a> {
    /// Raw approval token (UUID) the approver quotes back.
    pub token: &'a str,
    /// Hex-encoded SHA-256 of the token — the `d` tag an inbound grant carries.
    pub token_hash_hex: &'a str,
    /// The run awaiting approval.
    pub run_id: uuid::Uuid,
    /// The workflow the run belongs to.
    pub workflow_id: uuid::Uuid,
    /// Step id of the gate.
    pub step_id: &'a str,
    /// Approver spec: `"any"` or a 64-char hex pubkey.
    pub approver_spec: &'a str,
    /// Prompt shown to the approver.
    pub message: &'a str,
    /// Absolute expiry of the gate.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
