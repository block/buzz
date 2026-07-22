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

    /// Announce a pending approval as a kind:46010 event (WF-08).
    ///
    /// The event carries the raw token in its JSON content (approvers hash it
    /// into the grant's `d` tag via `buzz workflows approve --token …`), a
    /// `d` tag holding the SHA-256 token hash so relay handlers and clients
    /// can correlate it with grant/deny events, and a `p` tag for the notify
    /// pubkey so the request surfaces in that user's needs-action feed.
    ///
    /// Returns the event ID hex string on success.
    fn emit_approval_requested(
        &self,
        community_id: CommunityId,
        notice: ApprovalRequestNotice,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}

/// Everything the sink needs to announce a pending approval (kind:46010).
#[derive(Debug, Clone)]
pub struct ApprovalRequestNotice {
    /// Channel to post into (the workflow's channel), if the workflow is
    /// channel-scoped. `None` emits an unscoped event that still reaches the
    /// notify pubkey's feed.
    pub channel_id: Option<uuid::Uuid>,
    /// Raw approval token — never persisted raw; delivered so approvers can
    /// present it back to the grant endpoint.
    pub raw_token: String,
    /// Template-resolved, human-facing approval message.
    pub message: String,
    /// Pubkey (hex) to `p`-tag: the resolved approver, or the workflow owner
    /// when the spec is `"any"`.
    pub notify_pubkey_hex: String,
    /// The workflow the suspended run belongs to.
    pub workflow_id: uuid::Uuid,
    /// The suspended run awaiting this approval.
    pub run_id: uuid::Uuid,
    /// The step that requested approval.
    pub step_id: String,
    /// When the approval window closes.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
