//! Action sink trait — interface for workflow side-effects.
//!
//! The relay implements [`ActionSink`] to provide direct DB access to the
//! executor, replacing the HTTP loopback pattern.

use std::future::Future;
use std::pin::Pin;

use buzz_core::tenant::CommunityId;
use uuid::Uuid;

/// Workflow authority carried into a message side effect.
#[derive(Debug, Clone)]
pub struct WorkflowMessageContext {
    /// Server-resolved community that owns the workflow run.
    pub community_id: CommunityId,
    /// Exact workflow run driving this side effect.
    pub run_id: Uuid,
    /// Exact signed-definition step being executed.
    pub step_id: String,
    /// Exact signed definition selected when the run was created, or `None`
    /// for a legacy run that must not emit an automatic wake.
    pub definition_event_id: Option<Vec<u8>>,
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
    /// - `context`: exact workflow authority driving this side effect, including
    ///   the server-resolved community and optional signed-definition revision
    /// - `channel_id`: UUID string of the target channel
    /// - `text`: rendered message body (must not be empty/whitespace-only)
    /// - `authored_text`: the workflow owner's stored, unrendered step template;
    ///   consumers must use this rather than trigger-controlled rendered output
    ///   when attaching authority-bearing metadata
    /// - `author_pubkey`: hex-encoded pubkey of the workflow owner (used for
    ///   the `p` attribution tag; the relay keypair signs the event)
    /// - `reply_to`: when `Some(event_id_hex)`, the message is posted as a
    ///   threaded reply to that event (NIP-10 root/reply tags + real thread
    ///   metadata); when `None`, it is a top-level channel message.
    ///
    /// Returns the event ID hex string on success.
    fn send_message(
        &self,
        context: WorkflowMessageContext,
        channel_id: &str,
        text: &str,
        authored_text: &str,
        author_pubkey: &str,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}
