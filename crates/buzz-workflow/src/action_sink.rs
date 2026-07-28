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
    /// - `workflow_depth`: the trigger-chain depth of the run emitting this
    ///   message. Stamped into the `buzz:workflow` tag so a downstream workflow
    ///   that triggers off this event can compute its own depth and the engine
    ///   can cap the chain (see `MAX_WORKFLOW_DEPTH`).
    ///
    /// Returns the event ID hex string on success.
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        workflow_depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Update a channel's topic on behalf of a workflow owner.
    ///
    /// Emits a NIP-29 edit-metadata event (kind:9002) carrying a `topic` tag.
    /// The relay's side-effect handler applies the topic change after
    /// membership/permission checks. Same community-scoping and keypair-signs
    /// semantics as [`send_message`].
    ///
    /// - `community_id`: the owning community of the workflow run.
    /// - `channel_id`: UUID string of the target channel.
    /// - `topic`: the new topic string (must not be empty).
    /// - `author_pubkey`: hex pubkey of the workflow owner (attribution `p` tag).
    /// - `workflow_depth`: trigger-chain depth for loop prevention.
    ///
    /// Returns the event ID hex string on success.
    fn set_channel_topic(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        topic: &str,
        author_pubkey: &str,
        workflow_depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Add an emoji reaction to a message on behalf of a workflow owner.
    ///
    /// Emits a NIP-25 reaction event (kind:7) targeting `target_event_id`.
    /// The relay keypair signs the event; attribution flows through the
    /// standard reaction storage path.
    ///
    /// - `community_id`: the owning community of the workflow run.
    /// - `target_event_id`: hex event ID of the message to react to.
    /// - `emoji`: emoji character or shortcode (e.g. `"👍"`, `"thumbsup"`).
    /// - `author_pubkey`: hex pubkey of the workflow owner (attribution `p` tag).
    /// - `workflow_depth`: trigger-chain depth for loop prevention.
    ///
    /// Returns the reaction event ID hex string on success.
    fn add_reaction(
        &self,
        community_id: CommunityId,
        target_event_id: &str,
        emoji: &str,
        author_pubkey: &str,
        workflow_depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Send a direct message to a user on behalf of a workflow owner.
    ///
    /// Opens (or reuses) a private DM channel between the workflow owner and
    /// `recipient_pubkey`, then posts a kind:9 message into it. Buzz models
    /// DMs as private channels rather than NIP-17 gift-wraps.
    ///
    /// - `community_id`: the owning community of the workflow run.
    /// - `recipient_pubkey`: hex pubkey of the DM recipient.
    /// - `text`: message body (must not be empty/whitespace-only).
    /// - `author_pubkey`: hex pubkey of the workflow owner (DM participant +
    ///   attribution `p` tag).
    /// - `workflow_depth`: trigger-chain depth for loop prevention.
    ///
    /// Returns the message event ID hex string on success.
    fn send_dm(
        &self,
        community_id: CommunityId,
        recipient_pubkey: &str,
        text: &str,
        author_pubkey: &str,
        workflow_depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}
