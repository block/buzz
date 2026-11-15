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

/// A message an [`ActionSink`] posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMessage {
    /// Hex event ID.
    pub event_id: String,
    /// Whether at least one agent-authority tag was left off because the run
    /// may not direct agents.
    pub agent_mentions_withheld: bool,
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
    /// - `text`: rendered message body (must not be empty/whitespace-only)
    /// - `authored_text`: the workflow owner's stored, unrendered step
    ///   template. Consumers attach authority-bearing metadata only for
    ///   targets named in this text, never from trigger-controlled rendered
    ///   output alone.
    /// - `may_direct_agents`: whether this run may attach that metadata; when
    ///   `false`, the consumer withholds it.
    /// - `author_pubkey`: hex-encoded pubkey of the workflow owner (used for
    ///   the `p` attribution tag; the relay keypair signs the event)
    /// - `reply_to`: when `Some(event_id_hex)`, the message is posted as a
    ///   threaded reply to that event (NIP-10 root/reply tags + real thread
    ///   metadata); when `None`, it is a top-level channel message.
    ///
    /// Returns the sent message on success.
    #[allow(clippy::too_many_arguments)]
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        authored_text: &str,
        may_direct_agents: bool,
        author_pubkey: &str,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<SentMessage, ActionSinkError>> + Send + '_>>;
}
