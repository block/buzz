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
    /// The target agent is not a member of the destination channel.
    ///
    /// `assign_agent` is fail-closed: the agent must already be a channel
    /// member. Silently adding them would let a workflow escalate authority
    /// beyond what the owner granted at save time.
    #[error("assignee is not a channel member: {0}")]
    AssigneeNotMember(String),
}

impl From<ActionSinkError> for crate::WorkflowError {
    fn from(e: ActionSinkError) -> Self {
        match e {
            // Keep a database failure classified as one. An operator triaging
            // a failed run has to be able to tell "the database was down"
            // from "the assignee was removed from the channel"; both landing
            // on `webhook_failed` made that impossible.
            ActionSinkError::Database(msg) => crate::WorkflowError::Database(msg),
            // Everything else is a genuine action failure. `webhook_failed`
            // was always a misnomer here — none of these actions is a webhook.
            other => crate::WorkflowError::ActionFailed(other.to_string()),
        }
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
    /// - `reply_to`: when `Some(event_id_hex)`, the message is posted as a
    ///   threaded reply to that event (NIP-10 root/reply tags + real thread
    ///   metadata); when `None`, it is a top-level channel message.
    ///
    /// Returns the event ID hex string on success.
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;

    /// Dispatch a task to exactly one agent by immutable pubkey.
    ///
    /// The relay-side implementation emits two `p` tags on the resulting
    /// `kind:9` message — `author_pubkey` (owner attribution) and
    /// `agent_pubkey` (wake) — collapsing to one when the owner *is* the
    /// assignee. It also emits `buzz:workflow-owner`, which is what the
    /// harness's inbound author gate actually reads; the event is signed by
    /// the relay keypair, so without that tag it is gated on the relay's own
    /// pubkey and dropped under `owner-only`. The `text` is **not** scanned
    /// for `@Name` mentions — that reverse-parse is the failure mode
    /// `assign_agent` exists to avoid.
    ///
    /// Fails with [`ActionSinkError::AssigneeNotMember`] if `agent_pubkey`
    /// is not a current member of `channel_id`. Adding them silently would
    /// let a workflow escalate beyond the owner's saved authority.
    ///
    /// - `agent_pubkey`: hex-encoded pubkey of the sole assignee.
    /// - `task_id`: optional caller-supplied correlation id emitted as a
    ///   `task` tag, trimmed, and omitted entirely when empty after trimming.
    ///
    ///   It is **not** guaranteed to be a UUID at this boundary. Definition
    ///   validation requires one, but the executor re-validates only
    ///   `agent_pubkey` before calling, and this is a public trait any caller
    ///   can implement against — so an implementation must not assume the
    ///   shape. (The previous wording claimed "the executor performs shape
    ///   validation before calling", which was not true of `task_id`.)
    ///
    /// Returns the event ID hex string on success.
    ///
    /// No default implementation is provided intentionally: there is only
    /// one production sink, and a runtime "unimplemented" would defeat the
    /// identity-safety guarantees this method is being added to enforce.
    fn assign_agent(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        agent_pubkey: &str,
        task_id: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>>;
}
