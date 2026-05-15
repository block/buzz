use nostr::{Alphabet, EventBuilder, EventId, Filter, JsonUtil, Kind, SingleLetterTag, Tag};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::relay_client::RelayClient;
use sprout_core::kind;
use sprout_core::PresenceStatus;

/// Helper to create a lowercase single-letter tag for Nostr filter custom_tag.
fn tag_h() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::H)
}
fn tag_d() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::D)
}
fn tag_e() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::E)
}
fn tag_p() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

/// Convert a sprout-core kind constant (u32) to a nostr Kind.
fn k(kind_num: u32) -> Kind {
    Kind::Custom(kind_num as u16)
}

/// Percent-encode a string for safe inclusion in a URL query parameter value.
/// Encodes all characters except unreserved ones (A-Z a-z 0-9 - _ . ~).
#[cfg(test)]
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                // SAFETY: nibble values 0–15 are always valid hex digits.
                let hi = char::from_digit((byte >> 4) as u32, 16)
                    .expect("nibble 0-15 is always a valid hex digit")
                    .to_ascii_uppercase();
                let lo = char::from_digit((byte & 0xf) as u32, 16)
                    .expect("nibble 0-15 is always a valid hex digit")
                    .to_ascii_uppercase();
                out.push('%');
                out.push(hi);
                out.push(lo);
            }
        }
    }
    out
}

/// Validate that a string is exactly 64 hex characters.
fn validate_hex64(s: &str, label: &str) -> Result<(), String> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        Err(format!(
            "Error: {label} must be exactly 64 hex characters, got len={}",
            s.len()
        ))
    } else {
        Ok(())
    }
}

/// Validate that `s` is a well-formed UUID (any version/variant).
/// Returns `Ok(())` on success, or an error string on failure.
fn validate_uuid(s: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(s).map_err(|_| format!("invalid UUID: {s}"))?;
    Ok(())
}

/// Extract the thread root event ID from a serialized Nostr tag array.
///
/// Parses `"e"` tags with NIP-10 markers:
/// - If a `"root"` marker exists, returns that event ID.
/// - If only a `"reply"` marker exists, returns the reply target (it IS the root
///   for a direct reply — needed so nested replies can supply the correct root).
/// - If no thread markers exist, returns `None` (top-level message).
fn find_root_from_tags(tags: &serde_json::Value) -> Option<String> {
    let arr = tags.as_array()?;
    let mut root = None;
    let mut reply = None;
    for tag in arr {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        if parts.len() >= 4 && parts[0].as_str() == Some("e") {
            match parts[3].as_str() {
                Some("root") => root = parts[1].as_str().map(|s| s.to_string()),
                Some("reply") => reply = parts[1].as_str().map(|s| s.to_string()),
                _ => {}
            }
        }
    }
    root.or(reply)
}

/// Maximum allowed content size for a single message (64 KiB).
const MAX_CONTENT_BYTES: usize = 65_536;

/// Resolve `@name` mentions in `content` against this channel's members.
///
/// Performs the I/O (member + profile queries) and delegates parsing
/// and matching to [`sprout_sdk::mentions`]. On any query failure or
/// missing data, returns an empty vec — auto-tagging is best-effort
/// and must never block a send.
async fn resolve_content_mentions(
    client: &RelayClient,
    channel_id: &str,
    content: &str,
) -> Vec<String> {
    let names = sprout_sdk::mentions::extract_at_names(content);
    if names.is_empty() {
        return vec![];
    }
    // Query channel membership (kind 39002, addressed by `d` tag).
    let filter = Filter::new()
        .kind(k(kind::KIND_NIP29_GROUP_MEMBERS))
        .custom_tag(tag_d(), [channel_id])
        .limit(1);
    let Ok(events) = client.query(vec![filter]).await else {
        return vec![];
    };
    let Some(event) = events.first() else {
        return vec![];
    };
    let member_pubkeys: Vec<&str> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("p"))
        .filter_map(|t| t.as_slice().get(1).map(|v| v.as_str()))
        .collect();
    if member_pubkeys.is_empty() {
        return vec![];
    }
    // Fetch profiles for those members.
    let authors: Vec<nostr::PublicKey> = member_pubkeys
        .iter()
        .filter_map(|pk| nostr::PublicKey::from_hex(pk).ok())
        .collect();
    let profile_filter = Filter::new()
        .kind(k(kind::KIND_PROFILE))
        .authors(authors)
        .limit(member_pubkeys.len());
    let Ok(profiles) = client.query(vec![profile_filter]).await else {
        return vec![];
    };
    // Stable lowercase-hex strings for borrowing into MentionProfile.
    let hex_pubkeys: Vec<String> = profiles.iter().map(|p| p.pubkey.to_hex()).collect();
    let entries: Vec<sprout_sdk::mentions::MentionProfile<'_>> = profiles
        .iter()
        .zip(hex_pubkeys.iter())
        .map(|(p, pk)| sprout_sdk::mentions::MentionProfile {
            pubkey: pk.as_str(),
            content_json: p.content.as_str(),
        })
        .collect();
    sprout_sdk::mentions::match_names_to_profiles(&names, &entries)
}

/// Parameters for the `send_message` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SendMessageParams {
    /// UUID of the channel to post to.
    pub channel_id: String,
    /// Message body text.
    pub content: String,
    /// Nostr event kind. Defaults to KIND_STREAM_MESSAGE (NIP-29 group chat message).
    #[serde(default = "default_kind")]
    pub kind: Option<u16>,
    /// Optional parent event ID for threading. If provided, NIP-10 reply tags are added.
    #[serde(default)]
    pub parent_event_id: Option<String>,
    /// If true and parent_event_id is set, surface the reply in the main channel timeline.
    #[serde(default)]
    pub broadcast_to_channel: Option<bool>,
    /// Pubkeys to @mention in the message.
    #[serde(default)]
    pub mention_pubkeys: Option<Vec<String>>,
    /// Optional file paths to upload and attach as media. Each file is uploaded
    /// to the relay and included as an imeta tag + markdown image in the message.
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
}
fn default_kind() -> Option<u16> {
    Some(sprout_core::kind::KIND_STREAM_MESSAGE as u16)
}

/// Parameters for the `get_messages` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetMessagesParams {
    /// UUID of the channel to fetch history from.
    pub channel_id: String,
    /// Maximum number of messages to return (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Legacy parameter (thread summaries are now always included). Kept for backward compatibility.
    #[serde(default)]
    pub with_threads: Option<bool>,
    /// Unix timestamp cursor for pagination. Returns messages before this time.
    #[serde(default)]
    pub before: Option<i64>,
    /// Unix timestamp cursor. Returns messages created strictly after this time.
    /// When used without `before`, results are ordered oldest-first (chronological).
    /// Useful for polling: pass the timestamp of the last seen message to get only newer ones.
    #[serde(default)]
    pub since: Option<i64>,
    /// Comma-separated event kind numbers to filter by (e.g. "45001" for forum posts,
    /// "45002" for votes). When omitted, all kinds are returned.
    #[serde(default)]
    pub kinds: Option<String>,
}

/// Parameters for the `list_channels` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListChannelsParams {
    /// Optional visibility filter: `"open"` or `"private"`.
    #[serde(default)]
    pub visibility: Option<String>,
}

/// Parameters for the `create_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateChannelParams {
    /// Display name for the new channel.
    pub name: String,
    /// Channel type: `"stream"` (real-time chat) or `"forum"` (threaded discussions).
    pub channel_type: String,
    /// Channel visibility: `"open"` (anyone can join) or `"private"` (invite-only).
    pub visibility: String,
    /// Optional human-readable description of the channel's purpose.
    #[serde(default)]
    pub description: Option<String>,
}

/// Parameters for the `get_canvas` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetCanvasParams {
    /// UUID of the channel whose canvas to retrieve.
    pub channel_id: String,
}

/// Parameters for the `set_canvas` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetCanvasParams {
    /// UUID of the channel whose canvas to update.
    pub channel_id: String,
    /// New canvas content (replaces any existing canvas).
    pub content: String,
}

// ── Workflow tool parameter structs ──────────────────────────────────────────

/// Parameters for the `list_workflows` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListWorkflowsParams {
    /// UUID of the channel whose workflows to list.
    pub channel_id: String,
}

/// Parameters for the `create_workflow` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateWorkflowParams {
    /// UUID of the channel to own this workflow.
    pub channel_id: String,
    /// Full workflow definition in YAML format. Required fields: name (string), trigger (object with
    /// 'on' field: 'message_posted', 'diff_posted', 'reaction_added', or 'webhook'), steps (array).
    /// Each step needs: id (alphanumeric/underscore), action (e.g. 'send_message'), and action-specific
    /// fields as direct properties (NOT nested under 'params'). Example:
    /// ```yaml
    /// name: My Workflow
    /// trigger:
    ///   on: message_posted
    /// steps:
    ///   - id: notify
    ///     action: send_message
    ///     text: Hello from workflow!
    /// ```
    pub yaml_definition: String,
}

/// Parameters for the `update_workflow` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateWorkflowParams {
    /// UUID of the workflow to update.
    pub workflow_id: String,
    /// Replacement YAML definition.
    pub yaml_definition: String,
}

/// Parameters for the `delete_workflow` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteWorkflowParams {
    /// UUID of the workflow to delete.
    pub workflow_id: String,
}

/// Parameters for the `trigger_workflow` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriggerWorkflowParams {
    /// UUID of the workflow to trigger.
    pub workflow_id: String,
    /// Optional JSON object of input variables passed to the workflow.
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
}

/// Parameters for the `get_workflow_runs` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetWorkflowRunsParams {
    /// UUID of the workflow whose run history to fetch.
    pub workflow_id: String,
    /// Maximum number of runs to return. Default 20, max 100.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for the `approve_step` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApproveStepParams {
    /// Opaque approval token from the kind:46010 event.
    pub approval_token: String,
    /// true = approve, false = deny.
    pub approved: bool,
    /// Optional human-readable note to attach to the decision.
    #[serde(default)]
    pub note: Option<String>,
}

// ── Feed tool parameter structs ───────────────────────────────────────────────

// ── Membership tool parameter structs ────────────────────────────────────────

/// Parameters for the `add_channel_member` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddChannelMemberParams {
    /// UUID of the channel.
    pub channel_id: String,
    /// Hex-encoded public key of the user to add.
    pub pubkey: String,
    /// Role to assign: `"member"` (default) or `"admin"`.
    #[serde(default)]
    pub role: Option<String>,
}

/// Parameters for the `remove_channel_member` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RemoveChannelMemberParams {
    /// UUID of the channel.
    pub channel_id: String,
    /// Hex-encoded public key of the user to remove.
    pub pubkey: String,
}

/// Parameters for the `list_channel_members` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListChannelMembersParams {
    /// UUID of the channel whose members to list.
    pub channel_id: String,
}

/// Parameters for the `join_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct JoinChannelParams {
    /// UUID of the channel to join.
    pub channel_id: String,
}

/// Parameters for the `leave_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LeaveChannelParams {
    /// UUID of the channel to leave.
    pub channel_id: String,
}

/// Parameters for the `get_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetChannelParams {
    /// UUID of the channel to retrieve.
    pub channel_id: String,
}

// ── Metadata tool parameter structs ──────────────────────────────────────────

/// Parameters for the `update_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateChannelParams {
    /// UUID of the channel to update.
    pub channel_id: String,
    /// New display name for the channel.
    #[serde(default)]
    pub name: Option<String>,
    /// New description for the channel.
    #[serde(default)]
    pub description: Option<String>,
}

/// Parameters for the `set_channel_topic` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetChannelTopicParams {
    /// UUID of the channel.
    pub channel_id: String,
    /// New topic string.
    pub topic: String,
}

/// Parameters for the `set_channel_purpose` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetChannelPurposeParams {
    /// UUID of the channel.
    pub channel_id: String,
    /// New purpose string.
    pub purpose: String,
}

/// Parameters for the `archive_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArchiveChannelParams {
    /// UUID of the channel to archive.
    pub channel_id: String,
}

/// Parameters for the `unarchive_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UnarchiveChannelParams {
    /// UUID of the channel to unarchive.
    pub channel_id: String,
}

/// Parameters for the `delete_channel` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteChannelParams {
    /// UUID of the channel to permanently delete.
    pub channel_id: String,
}

// ── Thread tool parameter structs ─────────────────────────────────────────────

/// Parameters for the `get_thread` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetThreadParams {
    /// UUID of the channel containing the thread.
    pub channel_id: String,
    /// Event ID of the root (or any ancestor) message of the thread.
    pub event_id: String,
    /// Maximum nesting depth to return (default: unlimited).
    #[serde(default)]
    pub depth_limit: Option<u32>,
    /// Maximum number of replies to return (default 50).
    #[serde(default)]
    pub limit: Option<u32>,
}

// ── DM tool parameter structs ─────────────────────────────────────────────────

/// Parameters for the `open_dm` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OpenDmParams {
    /// Hex-encoded public keys of the other participants (1–8).
    pub pubkeys: Vec<String>,
}

/// Parameters for the `add_dm_member` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddDmMemberParams {
    /// UUID of the DM channel.
    pub channel_id: String,
    /// Hex-encoded public key of the user to add.
    pub pubkey: String,
}

/// Parameters for the `hide_dm` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HideDmParams {
    /// UUID of the DM channel to hide.
    pub channel_id: String,
}

// ── Reaction tool parameter structs ──────────────────────────────────────────

/// Parameters for the `add_reaction` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddReactionParams {
    /// Event ID of the message to react to.
    pub event_id: String,
    /// Emoji to react with (e.g. `"👍"` or `":thumbsup:"`).
    pub emoji: String,
}

/// Parameters for the `remove_reaction` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RemoveReactionParams {
    /// Event ID of the message whose reaction to remove.
    pub event_id: String,
    /// Emoji to remove.
    pub emoji: String,
}

/// Parameters for the `get_reactions` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetReactionsParams {
    /// Event ID of the message whose reactions to fetch.
    pub event_id: String,
}

// ── User profile tool parameter structs ──────────────────────────────────────

/// Parameters for the `set_profile` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetProfileParams {
    /// New display name for the agent's profile.
    #[serde(default)]
    pub display_name: Option<String>,
    /// URL of the agent's avatar image.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Short bio or description.
    #[serde(default)]
    pub about: Option<String>,
    /// NIP-05 identifier (e.g. "alice@example.com"), or None to leave unchanged.
    #[serde(default)]
    pub nip05_handle: Option<String>,
}

/// Parameters for the `get_users` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetUsersParams {
    /// Pubkey(s) to look up. Omit for your own profile. Provide one hex pubkey
    /// for a single user, or multiple for batch lookup (max 200).
    pub pubkeys: Option<Vec<String>>,
}

/// Parameters for the `search` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Full-text search query string.
    pub q: String,
    /// Maximum results to return (default 20, max 100).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters for the `get_presence` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetPresenceParams {
    /// Comma-separated hex-encoded public keys to look up presence for (max 200).
    pub pubkeys: String,
}

/// Parameters for the `set_presence` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetPresenceParams {
    /// Presence status to set.
    pub status: PresenceStatus,
}

/// Parameters for the `set_channel_add_policy` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetChannelAddPolicyParams {
    /// Channel add policy: "anyone" (default), "owner_only", or "nobody".
    /// - "anyone": any authenticated user can add you to open channels.
    /// - "owner_only": only your provisioned owner can add you.
    /// - "nobody": no one can add you; you must self-join channels.
    pub policy: String,
}

/// Parameters for the `get_feed` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetFeedParams {
    /// Only return feed items newer than this Unix timestamp.
    /// Defaults to now - 7 days if omitted.
    #[serde(default)]
    pub since: Option<i64>,
    /// Maximum items per category. Default 50, max 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Comma-separated category filter: "mentions,needs_action,activity,agent_activity".
    /// Omit to return all categories.
    #[serde(default)]
    pub types: Option<String>,
}

/// Parameters for the `edit_message` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditMessageParams {
    /// Channel ID (UUID) containing the message to edit.
    pub channel_id: String,
    /// Event ID (64-char hex) of the message to edit.
    pub event_id: String,
    /// New content for the message.
    pub content: String,
}

/// Parameters for the `delete_message` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteMessageParams {
    /// Event ID (64-char hex) of the message to delete.
    pub event_id: String,
}

/// Parameters for the `send_diff_message` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SendDiffMessageParams {
    /// UUID of the channel to post to.
    pub channel_id: String,
    /// Unified diff content (git diff format).
    pub diff: String,
    /// URL of the source repository (e.g. "https://github.com/org/repo").
    pub repo_url: String,
    /// Full commit SHA this diff applies to.
    pub commit_sha: String,
    /// Optional file path within the repo (used for language inference and display).
    #[serde(default)]
    pub file_path: Option<String>,
    /// Optional parent commit SHA (the base of the diff).
    #[serde(default)]
    pub parent_commit_sha: Option<String>,
    /// Optional source branch name (e.g. "feat/my-feature").
    #[serde(default)]
    pub source_branch: Option<String>,
    /// Optional target branch name (e.g. "main").
    #[serde(default)]
    pub target_branch: Option<String>,
    /// Optional pull request number associated with this diff.
    #[serde(default)]
    pub pr_number: Option<u32>,
    /// Optional language hint for syntax highlighting (e.g. "rust", "typescript").
    /// Inferred from file_path extension if omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Optional human-readable description of the change.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional parent event ID. If provided, sends the diff as a threaded reply.
    #[serde(default)]
    pub parent_event_id: Option<String>,
}

/// Vote direction for forum posts.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VoteDirection {
    /// Upvote.
    Up,
    /// Downvote.
    Down,
}

/// Parameters for the `vote_on_post` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct VoteOnPostParams {
    /// UUID of the forum channel.
    pub channel_id: String,
    /// 64-character hex event ID of the post or comment being voted on.
    pub event_id: String,
    /// Vote direction.
    pub direction: VoteDirection,
}

/// Parameters for [`SproutServer::publish_note`].
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PublishNoteParams {
    /// Text content of the note (max 64 KiB).
    pub content: String,
    /// 64-char hex event ID to reply to. Adds a single e-tag with "reply" marker.
    #[serde(default)]
    pub reply_to_event_id: Option<String>,
}

/// A single contact entry for [`SetContactListParams`].
///
/// Kept local to MCP — not part of sprout-sdk. The SDK builder takes primitive slices.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContactEntry {
    /// 64-char hex pubkey (any case accepted, normalized to lowercase).
    pub pubkey: String,
    /// Optional relay URL hint (NIP-02). Empty string if omitted.
    #[serde(default)]
    pub relay_url: Option<String>,
    /// Optional petname / display alias.
    #[serde(default)]
    pub petname: Option<String>,
}

/// Parameters for [`SproutServer::set_contact_list`].
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SetContactListParams {
    /// Replaces the **entire** contact list. Call `get_contact_list` first for delta updates.
    pub contacts: Vec<ContactEntry>,
}

/// Parameters for [`SproutServer::get_event`].
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetEventParams {
    /// 64-char hex event ID.
    pub event_id: String,
}

/// Parameters for [`SproutServer::get_user_notes`].
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetUserNotesParams {
    /// 64-char hex pubkey of the author.
    pub pubkey: String,
    /// Maximum number of notes to return (default 50, max 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Unix timestamp cursor — return notes created before this time.
    /// Use with `before_id` for stable composite cursor pagination.
    #[serde(default)]
    pub before: Option<i64>,
    /// Hex event ID cursor for composite keyset pagination. Use together with
    /// `before` to avoid skipping same-second events. Pass the `before_id` value
    /// from the previous page's `next_cursor` response.
    #[serde(default)]
    pub before_id: Option<String>,
}

/// Parameters for [`SproutServer::get_contact_list`].
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetContactListParams {
    /// 64-char hex pubkey of the user whose contact list to fetch.
    pub pubkey: String,
}

// ── Diff utility functions ────────────────────────────────────────────────────

// Truncation notice appended when a diff is cut. This constant is used to
// reserve space so the final result never exceeds max_bytes.
// NOTE: This function is only called with max_bytes = 60 * 1024, so the
// hardcoded "60KB" in the notice is intentional and always accurate.
const TRUNCATION_NOTICE: &str =
    "\n\\ Diff truncated at 60KB. Full diff available at the source repository.";

/// Truncate a diff to at most `max_bytes` bytes, cutting at a hunk boundary
/// where possible. Returns the (possibly truncated) string and a flag indicating
/// whether truncation occurred.
///
/// The truncation notice is included within the `max_bytes` budget — the
/// returned string is guaranteed to be `<= max_bytes` in length.
fn truncate_diff(diff: &str, max_bytes: usize) -> (String, bool) {
    debug_assert!(
        max_bytes >= TRUNCATION_NOTICE.len(),
        "max_bytes ({max_bytes}) must be >= TRUNCATION_NOTICE length ({})",
        TRUNCATION_NOTICE.len()
    );

    if diff.len() <= max_bytes {
        return (diff.to_string(), false);
    }

    // Reserve space for the truncation notice so the final result stays within max_bytes.
    let effective_limit = max_bytes.saturating_sub(TRUNCATION_NOTICE.len());

    // Step 1: Find the last UTF-8 char boundary at or before effective_limit
    let utf8_boundary = diff
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= effective_limit)
        .last()
        .unwrap_or(0);

    // Step 2: Within safe prefix, find last complete hunk boundary
    let safe_prefix = &diff[..utf8_boundary];
    let last_hunk_start = safe_prefix.rfind("\n@@");

    let cut_point = match last_hunk_start {
        Some(pos) if pos > 0 => pos,
        _ => safe_prefix.rfind('\n').unwrap_or(utf8_boundary),
    };

    let mut result = diff[..cut_point].to_string();
    result.push_str(TRUNCATION_NOTICE);
    (result, true)
}

/// Infer a language name from a file path's extension for syntax highlighting.
/// Returns `None` if the extension is unknown or absent.
fn infer_language(file_path: &str) -> Option<String> {
    // Note: rsplit always yields at least one element (the full string if no '.' found),
    // so .next() always returns Some. The ? is effectively a no-op here.
    let ext = file_path.rsplit('.').next()?;
    let lang = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "html" | "htm" => "html",
        "css" | "scss" | "sass" => "css",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "md" | "markdown" => "markdown",
        "dockerfile" => "dockerfile",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Parameters for the `upload_file` tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UploadFileParams {
    /// Local filesystem path to the file to upload.
    pub file_path: String,
}

/// The MCP server that exposes Sprout relay functionality as tools.
#[derive(Clone)]
pub struct SproutMcpServer {
    client: RelayClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SproutMcpServer {
    /// Create a new [`SproutMcpServer`] backed by the given relay client.
    ///
    /// Pass `tools_to_remove` to filter out tools by name (e.g. from toolset config).
    pub fn new(
        client: RelayClient,
        tools_to_remove: Option<std::collections::HashSet<&'static str>>,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        if let Some(ref remove) = tools_to_remove {
            for name in remove {
                tool_router.remove_route(name);
            }
        }

        Self {
            client,
            tool_router,
        }
    }

    /// Resolve a `ThreadRef` for SDK builders by fetching the parent event.
    ///
    /// Determines root vs. parent for NIP-10 markers:
    /// - Direct reply: root == parent
    /// - Nested reply: root is the thread root, parent is the immediate reply target
    async fn resolve_thread_ref(
        &self,
        parent_event_id: &str,
        parent_eid: EventId,
    ) -> Result<sprout_sdk::ThreadRef, String> {
        let filter = Filter::new().id(parent_eid).limit(1);
        let events = self
            .client
            .query(vec![filter])
            .await
            .map_err(|e| format!("failed to fetch parent event: {e}"))?;

        let parent_event = events
            .first()
            .ok_or_else(|| "parent event not found".to_string())?;

        let tags_json = serde_json::to_value(&parent_event.tags)
            .map_err(|e| format!("failed to serialize tags: {e}"))?;

        let root_eid = match find_root_from_tags(&tags_json) {
            Some(root_hex) if root_hex != parent_event_id => EventId::from_hex(&root_hex)
                .map_err(|e| format!("failed to parse root event id: {e}"))?,
            _ => parent_eid,
        };

        Ok(sprout_sdk::ThreadRef {
            root_event_id: root_eid,
            parent_event_id: parent_eid,
        })
    }

    /// Send a message to a Sprout channel.
    #[tool(
        name = "send_message",
        description = "Send a message to a Sprout channel. Include `parent_event_id` to reply in a thread. \
Set `broadcast_to_channel` to also surface the reply in the main channel timeline. \
For forum channels, set `kind` to 45001 (post) or 45003 (comment with `parent_event_id`). \
Default kind is 9 (stream message)."
    )]
    pub async fn send_message(&self, Parameters(p): Parameters<SendMessageParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        if p.content.len() > MAX_CONTENT_BYTES {
            return format!(
                "Error: content exceeds maximum size of {} bytes (got {})",
                MAX_CONTENT_BYTES,
                p.content.len()
            );
        }
        if let Some(ref parent_id) = p.parent_event_id {
            if parent_id.len() != 64 || !parent_id.chars().all(|c| c.is_ascii_hexdigit()) {
                return format!(
                    "Error: parent_event_id must be a 64-character hex string (got {:?})",
                    parent_id
                );
            }
        }
        if let Some(ref mentions) = p.mention_pubkeys {
            for pk in mentions {
                if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
                    return format!(
                        "Error: mention_pubkeys entry must be a 64-character hex string (got {:?})",
                        pk
                    );
                }
            }
        }

        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let kind_num = p
            .kind
            .unwrap_or(sprout_core::kind::KIND_STREAM_MESSAGE as u16);
        // Normalize explicit mentions, then merge auto-resolved `@names` from
        // the body up to the SDK mention cap.
        let explicit = p.mention_pubkeys.as_deref().unwrap_or(&[]);
        let mut mentions = sprout_sdk::mentions::normalize_mention_pubkeys(explicit, None);
        let auto = resolve_content_mentions(&self.client, &p.channel_id, &p.content).await;
        sprout_sdk::mentions::merge_mentions(
            &mut mentions,
            &auto,
            sprout_sdk::mentions::MENTION_CAP,
        );

        let mention_refs: Vec<&str> = mentions.iter().map(String::as_str).collect();
        let broadcast = p.broadcast_to_channel.unwrap_or(false);

        // Upload files and build media tags
        let mut media_tags: Vec<Vec<String>> = Vec::new();
        let mut media_content = String::new();
        if let Some(ref paths) = p.file_paths {
            for path in paths {
                match crate::upload::upload_file(
                    self.client.http_client(),
                    self.client.keys(),
                    &self.client.relay_http_url(),
                    self.client.server_domain().as_deref(),
                    path,
                    self.client.auth_tag_json().as_deref(),
                )
                .await
                {
                    Ok(desc) => {
                        media_tags.push(crate::upload::build_imeta_tag(&desc));
                        if desc.mime_type.starts_with("video/") {
                            media_content.push_str("\n![video](");
                        } else {
                            media_content.push_str("\n![image](");
                        }
                        media_content.push_str(&desc.url);
                        media_content.push(')');
                    }
                    Err(e) => {
                        return format!("Error uploading {}: {e}", path);
                    }
                }
            }
        }
        let final_content = if media_content.is_empty() {
            p.content.clone()
        } else {
            format!("{}{}", p.content, media_content)
        };

        // Build the event builder via SDK, routing by kind.
        let builder = match kind_num as u32 {
            sprout_core::kind::KIND_FORUM_POST => {
                // kind 45001: forum post (no thread ref, no broadcast)
                match sprout_sdk::build_forum_post(
                    channel_uuid,
                    &final_content,
                    &mention_refs,
                    &media_tags,
                ) {
                    Ok(b) => b,
                    Err(e) => return format!("Error: {e}"),
                }
            }
            sprout_core::kind::KIND_FORUM_COMMENT => {
                // kind 45003: forum comment — requires parent_event_id
                let parent_id = match p.parent_event_id.as_deref() {
                    Some(id) => id,
                    None => return "Error: kind 45003 requires parent_event_id".to_string(),
                };
                let parent_eid = match EventId::from_hex(parent_id) {
                    Ok(id) => id,
                    Err(e) => return format!("Error: invalid parent_event_id: {e}"),
                };
                // Fetch parent to resolve thread root for NIP-10 markers.
                let thread_ref = match self.resolve_thread_ref(parent_id, parent_eid).await {
                    Ok(tr) => tr,
                    Err(e) => return format!("Error: {e}"),
                };
                match sprout_sdk::build_forum_comment(
                    channel_uuid,
                    &final_content,
                    &thread_ref,
                    &mention_refs,
                    &media_tags,
                ) {
                    Ok(b) => b,
                    Err(e) => return format!("Error: {e}"),
                }
            }
            _ => {
                // kind 9 (default) and any other stream message kinds.
                let thread_ref = if let Some(ref parent_id) = p.parent_event_id {
                    let parent_eid = match EventId::from_hex(parent_id) {
                        Ok(id) => id,
                        Err(e) => return format!("Error: invalid parent_event_id: {e}"),
                    };
                    match self.resolve_thread_ref(parent_id, parent_eid).await {
                        Ok(tr) => Some(tr),
                        Err(e) => return format!("Error: {e}"),
                    }
                } else {
                    None
                };
                match sprout_sdk::build_message(
                    channel_uuid,
                    &final_content,
                    thread_ref.as_ref(),
                    &mention_refs,
                    broadcast && p.parent_event_id.is_some(),
                    &media_tags,
                ) {
                    Ok(b) => b,
                    Err(e) => return format!("Error: {e}"),
                }
            }
        };

        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign message event: {e}"),
        };

        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Send a code diff to a Sprout channel as kind:40008.
    #[tool(
        name = "send_diff_message",
        description = "Send a code diff to a Sprout channel with syntax highlighting and structured metadata. \
Include `parent_event_id` to post the diff as a thread reply. \
The diff is rendered with GitHub-quality visualization in the desktop client."
    )]
    pub async fn send_diff_message(
        &self,
        Parameters(p): Parameters<SendDiffMessageParams>,
    ) -> String {
        let SendDiffMessageParams {
            channel_id,
            diff,
            repo_url,
            commit_sha,
            file_path,
            parent_commit_sha,
            source_branch,
            target_branch,
            pr_number,
            language,
            description,
            parent_event_id,
        } = p;

        if let Err(e) = validate_uuid(&channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {channel_id}"),
        };

        // 1. Truncate diff at 60KB (UTF-8 safe)
        let (diff_content, truncated) = truncate_diff(&diff, 60 * 1024);

        // 2. Infer language from file extension if not provided
        let lang = language.or_else(|| file_path.as_deref().and_then(infer_language));

        // 3. Build NIP-31 alt text
        let alt_text = match &description {
            Some(desc) => format!(
                "Diff: {} — {}",
                file_path.as_deref().unwrap_or("diff"),
                desc
            ),
            None => format!("Diff: {}", file_path.as_deref().unwrap_or("diff")),
        };

        // 4. Warn on partial branch metadata (both or neither required)
        match (&source_branch, &target_branch) {
            (Some(_), None) | (None, Some(_)) => {
                tracing::warn!("send_diff_message: only one of source_branch/target_branch provided — both required, branch metadata omitted");
            }
            _ => {}
        }
        let branch = match (source_branch, target_branch) {
            (Some(src), Some(tgt)) => Some((src, tgt)),
            _ => None,
        };

        // 5. Resolve optional thread ref
        let thread_ref = if let Some(ref parent_id) = parent_event_id {
            let parent_eid = match EventId::from_hex(parent_id) {
                Ok(id) => id,
                Err(e) => return format!("Error: invalid parent_event_id: {e}"),
            };
            match self.resolve_thread_ref(parent_id, parent_eid).await {
                Ok(tr) => Some(tr),
                Err(e) => return format!("Error: {e}"),
            }
        } else {
            None
        };

        // 6. Build signed event via SDK
        let diff_meta = sprout_sdk::DiffMeta {
            repo_url,
            commit_sha,
            file_path,
            parent_commit: parent_commit_sha,
            branch,
            pr_number,
            language: lang,
            description,
            truncated,
            alt_text: Some(alt_text),
        };
        let builder = match sprout_sdk::build_diff_message(
            channel_uuid,
            &diff_content,
            &diff_meta,
            thread_ref.as_ref(),
        ) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign diff event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Edit a message you previously sent.
    #[tool(
        name = "edit_message",
        description = "Edit a message you previously sent. Creates an edit event (kind 40003) referencing the original."
    )]
    pub async fn edit_message(&self, Parameters(p): Parameters<EditMessageParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        if p.event_id.len() != 64 || !p.event_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!(
                "Error: event_id must be a 64-character hex string (got {:?})",
                p.event_id
            );
        }
        if p.content.len() > MAX_CONTENT_BYTES {
            return format!(
                "Error: content exceeds maximum size of {} bytes (got {})",
                MAX_CONTENT_BYTES,
                p.content.len()
            );
        }

        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let target_eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };

        let builder = match sprout_sdk::build_edit(channel_uuid, target_eid, &p.content) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign edit event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Delete a message.
    #[tool(
        name = "delete_message",
        description = "Delete a message. You must be the message author or a channel owner/admin."
    )]
    pub async fn delete_message(&self, Parameters(p): Parameters<DeleteMessageParams>) -> String {
        if p.event_id.len() != 64 || !p.event_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!(
                "Error: event_id must be a 64-character hex string (got {:?})",
                p.event_id
            );
        }
        let target_eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };

        // Fetch the event to extract its channel_id (h-tag) — required by build_delete_message.
        let filter = Filter::new().id(target_eid).limit(1);
        let fetched = match self.client.query(vec![filter]).await {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to fetch event: {e}"),
        };
        let fetched_event = match fetched.first() {
            Some(e) => e,
            None => return format!("Error: event '{}' not found", p.event_id),
        };
        let channel_id_str = match fetched_event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("h"))
            .and_then(|t| t.as_slice().get(1).map(|v| v.to_string()))
        {
            Some(id) => id,
            None => return "Error: could not find channel_id (h-tag) on event".to_string(),
        };
        let channel_uuid = match uuid::Uuid::parse_str(&channel_id_str) {
            Ok(u) => u,
            Err(_) => return format!("Error: event h-tag is not a valid UUID: {channel_id_str}"),
        };

        let builder = match sprout_sdk::build_delete_message(channel_uuid, target_eid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign delete event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get recent messages from a Sprout channel.
    #[tool(
        name = "get_messages",
        description = "Fetch recent top-level messages from a Sprout channel. Use `before` for backward \
pagination and `since` for forward pagination (both Unix timestamps). When `since` is \
provided without `before`, results are ordered oldest-first — useful for polling new \
messages. Use `kinds` to filter by event type (e.g. \"45001\" for forum posts, \
\"45002\" for votes). Thread summaries are included automatically. Threaded replies \
are not returned — use `get_thread` to fetch the full reply tree for a specific message."
    )]
    pub async fn get_messages(&self, Parameters(p): Parameters<GetMessagesParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }

        const MAX_HISTORY_LIMIT: usize = 200;
        let limit = p.limit.unwrap_or(50).min(MAX_HISTORY_LIMIT as u32) as usize;

        // Build filter for channel messages (stream messages, edits, diffs, forum posts/comments).
        let message_kinds: Vec<Kind> = if let Some(ref kinds_str) = p.kinds {
            kinds_str
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .map(Kind::from)
                .collect()
        } else {
            vec![
                k(kind::KIND_STREAM_MESSAGE),
                k(kind::KIND_STREAM_MESSAGE_V2),
                k(kind::KIND_STREAM_MESSAGE_DIFF),
                k(kind::KIND_FORUM_POST),
                k(kind::KIND_FORUM_COMMENT),
            ]
        };

        let mut filter = Filter::new()
            .kinds(message_kinds)
            .custom_tag(tag_h(), [&p.channel_id])
            .limit(limit);

        if let Some(before) = p.before {
            filter = filter.until(nostr::Timestamp::from(before as u64));
        }
        if let Some(since) = p.since {
            filter = filter.since(nostr::Timestamp::from(since as u64));
        }

        match self.client.query(vec![filter]).await {
            Ok(mut events) => {
                events.sort_by_key(|e| e.created_at);
                let result: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id.to_hex(),
                            "pubkey": e.pubkey.to_hex(),
                            "kind": e.kind.as_u16(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "tags": e.tags.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::to_string(&result).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// List Sprout channels accessible to this agent.
    #[tool(
        name = "list_channels",
        description = "List Sprout channels accessible to this agent"
    )]
    pub async fn list_channels(&self, Parameters(p): Parameters<ListChannelsParams>) -> String {
        // Query channel metadata events (kind:41 = NIP-29 group metadata).
        let filter = Filter::new()
            .kind(k(kind::KIND_CHANNEL_METADATA))
            .limit(500);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let channels: Vec<serde_json::Value> = events
                    .iter()
                    .filter(|e| {
                        if let Some(ref vis) = p.visibility {
                            // Filter by visibility tag if specified
                            e.tags
                                .iter()
                                .any(|t| {
                                    let s = t.as_slice();
                                    s.first().map(|v| v.as_str()) == Some("visibility")
                                        && s.get(1).map(|v| v.as_str()) == Some(vis.as_str())
                                })
                        } else {
                            true
                        }
                    })
                    .map(|e| {
                        let content: serde_json::Value =
                            serde_json::from_str(&e.content).unwrap_or(serde_json::json!({}));
                        let channel_id = e
                            .tags
                            .iter()
                            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("d"))
                            .and_then(|t| t.as_slice().get(1))
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        serde_json::json!({
                            "channel_id": channel_id,
                            "name": content.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                            "description": content.get("about").and_then(|v| v.as_str()).unwrap_or(""),
                            "created_at": e.created_at.as_u64(),
                        })
                    })
                    .collect();
                serde_json::to_string(&channels).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Create a new Sprout channel.
    #[tool(
        name = "create_channel",
        description = "Create a new Sprout channel. channel_type must be 'stream' or 'forum'. visibility must be 'open' or 'private'."
    )]
    pub async fn create_channel(&self, Parameters(p): Parameters<CreateChannelParams>) -> String {
        let channel_uuid = uuid::Uuid::new_v4();
        let visibility = match p.visibility.as_str() {
            "open" => sprout_sdk::Visibility::Open,
            "private" => sprout_sdk::Visibility::Private,
            other => {
                return format!(
                    "Error: invalid visibility: {other:?} (must be 'open' or 'private')"
                )
            }
        };
        let channel_type = match p.channel_type.as_str() {
            "stream" => sprout_sdk::ChannelKind::Stream,
            "forum" => sprout_sdk::ChannelKind::Forum,
            other => {
                return format!(
                    "Error: invalid channel_type: {other:?} (must be 'stream' or 'forum')"
                )
            }
        };
        let builder = match sprout_sdk::build_create_channel(
            channel_uuid,
            &p.name,
            Some(visibility),
            Some(channel_type),
            p.description.as_deref(),
        ) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign create_channel event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "channel_id": channel_uuid.to_string(),
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get the canvas (shared document) for a channel.
    #[tool(
        name = "get_canvas",
        description = "Get the canvas (shared document) for a channel"
    )]
    pub async fn get_canvas(&self, Parameters(p): Parameters<GetCanvasParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        // Canvas is kind:40100 with #h tag = channel_id
        let filter = Filter::new()
            .kind(k(kind::KIND_CANVAS))
            .custom_tag(tag_h(), [&p.channel_id])
            .limit(1);

        match self.client.query(vec![filter]).await {
            Ok(events) => match events.first() {
                Some(event) => event.content.clone(),
                None => "No canvas set for this channel.".to_string(),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Set or update the canvas (shared document) for a channel.
    #[tool(
        name = "set_canvas",
        description = "Set or update the canvas (shared document) for a channel"
    )]
    pub async fn set_canvas(&self, Parameters(p): Parameters<SetCanvasParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_set_canvas(channel_uuid, &p.content) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign set_canvas event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Workflow tools ────────────────────────────────────────────────────────

    /// List workflows defined in a Sprout channel.
    #[tool(
        name = "list_workflows",
        description = "List workflows defined in a Sprout channel"
    )]
    pub async fn list_workflows(&self, Parameters(p): Parameters<ListWorkflowsParams>) -> String {
        if uuid::Uuid::parse_str(&p.channel_id).is_err() {
            return format!("Error: channel_id '{}' is not a valid UUID", p.channel_id);
        }
        // Workflows are kind:30620 (param-replaceable) with #h tag = channel_id
        let filter = Filter::new()
            .kind(k(kind::KIND_WORKFLOW_DEF))
            .custom_tag(tag_h(), [&p.channel_id])
            .limit(100);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let workflows: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        let d_tag = e
                            .tags
                            .iter()
                            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("d"))
                            .and_then(|t| t.as_slice().get(1))
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        serde_json::json!({
                            "workflow_id": d_tag,
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "pubkey": e.pubkey.to_hex(),
                        })
                    })
                    .collect();
                serde_json::to_string(&workflows).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Create a new workflow in a channel from a YAML definition.
    #[tool(
        name = "create_workflow",
        description = "Create a new workflow from a YAML definition. Steps need 'id' (not 'name'), and action fields are direct properties (not nested under 'params'). Triggers: message_posted, reaction_added, webhook."
    )]
    pub async fn create_workflow(&self, Parameters(p): Parameters<CreateWorkflowParams>) -> String {
        if uuid::Uuid::parse_str(&p.channel_id).is_err() {
            return format!("Error: channel_id '{}' is not a valid UUID", p.channel_id);
        }
        // Workflow definition is a kind:30620 (param-replaceable) event.
        // d-tag = workflow UUID, h-tag = channel_id, content = YAML.
        let workflow_id = uuid::Uuid::new_v4().to_string();
        let tags = vec![
            Tag::parse(&["d", &workflow_id]).unwrap(),
            Tag::parse(&["h", &p.channel_id]).unwrap(),
        ];
        let builder = EventBuilder::new(k(kind::KIND_WORKFLOW_DEF), &p.yaml_definition, tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign workflow event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "workflow_id": workflow_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Replace a workflow's YAML definition.
    #[tool(
        name = "update_workflow",
        description = "Replace a workflow's YAML definition"
    )]
    pub async fn update_workflow(&self, Parameters(p): Parameters<UpdateWorkflowParams>) -> String {
        if uuid::Uuid::parse_str(&p.workflow_id).is_err() {
            return format!("Error: workflow_id '{}' is not a valid UUID", p.workflow_id);
        }
        // Publish a new kind:30620 event with the same d-tag to replace the existing one.
        let tags = vec![Tag::parse(&["d", &p.workflow_id]).unwrap()];
        let builder = EventBuilder::new(k(kind::KIND_WORKFLOW_DEF), &p.yaml_definition, tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign workflow event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Delete a workflow by ID.
    #[tool(name = "delete_workflow", description = "Delete a workflow by ID")]
    pub async fn delete_workflow(&self, Parameters(p): Parameters<DeleteWorkflowParams>) -> String {
        if uuid::Uuid::parse_str(&p.workflow_id).is_err() {
            return format!("Error: workflow_id '{}' is not a valid UUID", p.workflow_id);
        }
        // Delete via kind:5 event referencing the workflow's d-tag coordinate.
        let coordinate = format!(
            "{}:{}:{}",
            kind::KIND_WORKFLOW_DEF,
            self.client.pubkey_hex(),
            p.workflow_id
        );
        let tags = vec![Tag::parse(&["a", &coordinate]).unwrap()];
        let builder = EventBuilder::new(k(kind::KIND_DELETION), "", tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign deletion event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) if ok.accepted => "Workflow deleted.".to_string(),
            Ok(ok) => format!("Error: relay rejected deletion: {}", ok.message),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Manually trigger a workflow with optional input variables.
    #[tool(
        name = "trigger_workflow",
        description = "Manually trigger a workflow with optional input variables"
    )]
    pub async fn trigger_workflow(
        &self,
        Parameters(p): Parameters<TriggerWorkflowParams>,
    ) -> String {
        if uuid::Uuid::parse_str(&p.workflow_id).is_err() {
            return format!("Error: workflow_id '{}' is not a valid UUID", p.workflow_id);
        }
        let inputs = p
            .inputs
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let content = serde_json::to_string(&inputs).unwrap_or_default();
        let tags = vec![Tag::parse(&["d", &p.workflow_id]).unwrap()];
        let builder = EventBuilder::new(k(kind::KIND_WORKFLOW_TRIGGER), &content, tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign trigger event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get execution history for a workflow.
    #[tool(
        name = "get_workflow_runs",
        description = "Get execution history for a workflow"
    )]
    pub async fn get_workflow_runs(
        &self,
        Parameters(p): Parameters<GetWorkflowRunsParams>,
    ) -> String {
        if uuid::Uuid::parse_str(&p.workflow_id).is_err() {
            return format!("Error: workflow_id '{}' is not a valid UUID", p.workflow_id);
        }
        let limit = p.limit.unwrap_or(20).min(100) as usize;
        // Query workflow execution events (kind:46001–46010) referencing this workflow.
        let filter = Filter::new()
            .kinds(vec![
                k(kind::KIND_WORKFLOW_TRIGGERED),
                k(kind::KIND_WORKFLOW_TRIGGERED + 1), // completed
                k(kind::KIND_WORKFLOW_TRIGGERED + 2), // failed
            ])
            .custom_tag(tag_d(), [&p.workflow_id])
            .limit(limit);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let runs: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "event_id": e.id.to_hex(),
                            "kind": e.kind.as_u16(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "tags": e.tags.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::to_string(&runs).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Approve or deny a pending workflow approval step.
    #[tool(
        name = "approve_step",
        description = "Approve or deny a pending workflow approval step"
    )]
    pub async fn approve_step(&self, Parameters(p): Parameters<ApproveStepParams>) -> String {
        if uuid::Uuid::parse_str(&p.approval_token).is_err() {
            return format!(
                "Error: approval_token '{}' is not a valid UUID",
                p.approval_token
            );
        }
        let kind_num = if p.approved {
            kind::KIND_APPROVAL_GRANT
        } else {
            kind::KIND_APPROVAL_DENY
        };
        let content = p.note.as_deref().unwrap_or("");
        // The relay expects d-tag = hex(SHA256(token)), not the raw token UUID.
        let token_hash = hex::encode(Sha256::digest(p.approval_token.as_bytes()));
        let tags = vec![Tag::parse(&["d", &token_hash]).unwrap()];
        let builder = EventBuilder::new(k(kind_num), content, tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign approval event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Feed tools ────────────────────────────────────────────────────────────

    /// Get the agent's personalized home feed from the Sprout relay.
    #[tool(
        name = "get_feed",
        description = "Get the agent's personalized home feed from the Sprout relay. \
                       Returns mentions, needs-action items, channel activity, and agent activity. \
                       Equivalent to what a human sees on the Home tab in the desktop app."
    )]
    pub async fn get_feed(&self, Parameters(p): Parameters<GetFeedParams>) -> String {
        const MAX_FEED_LIMIT: usize = 50;
        let limit = p
            .limit
            .map(|l| l.min(MAX_FEED_LIMIT as u32) as usize)
            .unwrap_or(MAX_FEED_LIMIT);

        // Query events that mention this agent (p-tag) or are in channels we're in.
        let my_pubkey = self.client.pubkey_hex();
        let mut filter = Filter::new().custom_tag(tag_p(), [&my_pubkey]).limit(limit);

        if let Some(since) = p.since {
            filter = filter.since(nostr::Timestamp::from(since as u64));
        }

        match self.client.query(vec![filter]).await {
            Ok(mut events) => {
                events.sort_by_key(|e| std::cmp::Reverse(e.created_at));
                let feed: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id.to_hex(),
                            "pubkey": e.pubkey.to_hex(),
                            "kind": e.kind.as_u16(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "tags": e.tags.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::to_string(&feed).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error fetching feed: {e}"),
        }
    }

    // ── Membership tools ──────────────────────────────────────────────────────

    /// Add a member to a channel.
    #[tool(
        name = "add_channel_member",
        description = "Add a member to a Sprout channel. Optionally specify a role (default: \"member\")."
    )]
    pub async fn add_channel_member(
        &self,
        Parameters(p): Parameters<AddChannelMemberParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let role = match p.role.as_deref() {
            None => None,
            Some("owner") => Some(sprout_sdk::MemberRole::Owner),
            Some("admin") => Some(sprout_sdk::MemberRole::Admin),
            Some("member") => Some(sprout_sdk::MemberRole::Member),
            Some("guest") => Some(sprout_sdk::MemberRole::Guest),
            Some("bot") => Some(sprout_sdk::MemberRole::Bot),
            Some(other) => {
                return format!(
                    "Error: invalid role: {other:?} (must be owner/admin/member/guest/bot)"
                )
            }
        };
        let builder = match sprout_sdk::build_add_member(channel_uuid, &p.pubkey, role) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign add_member event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Remove a member from a channel.
    #[tool(
        name = "remove_channel_member",
        description = "Remove a member from a Sprout channel by their public key."
    )]
    pub async fn remove_channel_member(
        &self,
        Parameters(p): Parameters<RemoveChannelMemberParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_remove_member(channel_uuid, &p.pubkey) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign remove_member event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// List all members of a channel.
    #[tool(
        name = "list_channel_members",
        description = "List all members of a Sprout channel."
    )]
    pub async fn list_channel_members(
        &self,
        Parameters(p): Parameters<ListChannelMembersParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        // Query membership list event (kind:39002 = NIP-29 group members) for this channel.
        let filter = Filter::new()
            .kind(k(kind::KIND_NIP29_GROUP_MEMBERS))
            .custom_tag(tag_d(), [&p.channel_id])
            .limit(1);

        match self.client.query(vec![filter]).await {
            Ok(events) => match events.first() {
                Some(event) => {
                    // Members are in p-tags: ["p", "<pubkey>", "<role>"]
                    let members: Vec<serde_json::Value> = event
                        .tags
                        .iter()
                        .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("p"))
                        .map(|t| {
                            let s = t.as_slice();
                            serde_json::json!({
                                "pubkey": s.get(1).map(|v| v.as_str()).unwrap_or(""),
                                "role": s.get(2).map(|v| v.as_str()).unwrap_or("member"),
                            })
                        })
                        .collect();
                    serde_json::to_string(&members).unwrap_or_else(|e| format!("Error: {e}"))
                }
                None => "[]".to_string(),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Join a channel (add yourself as a member).
    #[tool(
        name = "join_channel",
        description = "Join a Sprout channel (adds the agent as a member)."
    )]
    pub async fn join_channel(&self, Parameters(p): Parameters<JoinChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_join(channel_uuid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign join event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Leave a channel (remove yourself as a member).
    #[tool(
        name = "leave_channel",
        description = "Leave a Sprout channel (removes the agent as a member)."
    )]
    pub async fn leave_channel(&self, Parameters(p): Parameters<LeaveChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_leave(channel_uuid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign leave event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get details for a single channel.
    #[tool(
        name = "get_channel",
        description = "Get metadata and details for a single Sprout channel by ID."
    )]
    pub async fn get_channel(&self, Parameters(p): Parameters<GetChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        // Query channel metadata (kind:41) with d-tag = channel_id.
        let filter = Filter::new()
            .kind(k(kind::KIND_CHANNEL_METADATA))
            .custom_tag(tag_d(), [&p.channel_id])
            .limit(1);

        match self.client.query(vec![filter]).await {
            Ok(events) => match events.first() {
                Some(event) => {
                    let content: serde_json::Value =
                        serde_json::from_str(&event.content).unwrap_or(serde_json::json!({}));
                    serde_json::json!({
                        "channel_id": p.channel_id,
                        "name": content.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": content.get("about").and_then(|v| v.as_str()).unwrap_or(""),
                        "created_at": event.created_at.as_u64(),
                        "pubkey": event.pubkey.to_hex(),
                    })
                    .to_string()
                }
                None => format!("Error: channel '{}' not found", p.channel_id),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Metadata tools ────────────────────────────────────────────────────────

    /// Update a channel's name and/or description.
    #[tool(
        name = "update_channel",
        description = "Update a Sprout channel's name and/or description."
    )]
    pub async fn update_channel(&self, Parameters(p): Parameters<UpdateChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_update_channel(
            channel_uuid,
            p.name.as_deref(),
            p.description.as_deref(),
        ) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign update_channel event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Set the topic for a channel.
    #[tool(
        name = "set_channel_topic",
        description = "Set the topic for a Sprout channel."
    )]
    pub async fn set_channel_topic(
        &self,
        Parameters(p): Parameters<SetChannelTopicParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_set_topic(channel_uuid, &p.topic) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign set_topic event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Set the purpose for a channel.
    #[tool(
        name = "set_channel_purpose",
        description = "Set the purpose for a Sprout channel."
    )]
    pub async fn set_channel_purpose(
        &self,
        Parameters(p): Parameters<SetChannelPurposeParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_set_purpose(channel_uuid, &p.purpose) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign set_purpose event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Archive a channel (makes it read-only).
    #[tool(
        name = "archive_channel",
        description = "Archive a Sprout channel, making it read-only."
    )]
    pub async fn archive_channel(&self, Parameters(p): Parameters<ArchiveChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_archive(channel_uuid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign archive event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Unarchive a channel (restores it to active).
    #[tool(
        name = "unarchive_channel",
        description = "Unarchive a Sprout channel, restoring it to active status."
    )]
    pub async fn unarchive_channel(
        &self,
        Parameters(p): Parameters<UnarchiveChannelParams>,
    ) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_unarchive(channel_uuid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign unarchive event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Thread tools ──────────────────────────────────────────────────────────

    /// Get a message thread (replies to a message).
    #[tool(
        name = "get_thread",
        description = "Fetch a full thread tree rooted at an event. Returns the root message and all nested \
replies. Works for both stream message threads and forum post threads (kind:45001 root \
with kind:45003 comments)."
    )]
    pub async fn get_thread(&self, Parameters(p): Parameters<GetThreadParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }

        let limit = p.limit.map(|l| l.min(200) as usize).unwrap_or(100);

        // Query events that reference the root event via e-tag.
        let filter = Filter::new()
            .custom_tag(tag_e(), [&p.event_id])
            .custom_tag(tag_h(), [&p.channel_id])
            .limit(limit);

        // Also fetch the root event itself.
        let root_eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };
        let root_filter = Filter::new().id(root_eid).limit(1);

        match self.client.query(vec![filter, root_filter]).await {
            Ok(mut events) => {
                events.sort_by_key(|e| e.created_at);
                let result: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id.to_hex(),
                            "pubkey": e.pubkey.to_hex(),
                            "kind": e.kind.as_u16(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "tags": e.tags.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::to_string(&result).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── DM tools ──────────────────────────────────────────────────────────────

    /// Open or retrieve a direct message channel with one or more participants.
    #[tool(
        name = "open_dm",
        description = "Open (or retrieve an existing) direct message channel with 1–8 other participants. \
                       Returns the DM channel details including its ID."
    )]
    pub async fn open_dm(&self, Parameters(p): Parameters<OpenDmParams>) -> String {
        if p.pubkeys.is_empty() {
            return "Error: pubkeys must contain at least one participant".to_string();
        }
        if p.pubkeys.len() > 8 {
            return format!(
                "Error: too many participants (max 8, got {})",
                p.pubkeys.len()
            );
        }
        // Command event kind:41010 with p-tags for each participant.
        let mut tags: Vec<Tag> = p
            .pubkeys
            .iter()
            .filter_map(|pk| Tag::parse(&["p", pk]).ok())
            .collect();
        // Add a unique d-tag so the relay can deduplicate.
        let dm_id = uuid::Uuid::new_v4().to_string();
        tags.push(Tag::parse(&["d", &dm_id]).unwrap());

        let builder = EventBuilder::new(k(kind::KIND_DM_OPEN), "", tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign open_dm event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "dm_id": dm_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Add a participant to an existing DM channel.
    #[tool(
        name = "add_dm_member",
        description = "Add a participant to an existing Sprout DM channel."
    )]
    pub async fn add_dm_member(&self, Parameters(p): Parameters<AddDmMemberParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        // Command event kind:41011 with h-tag = DM channel, p-tag = new member.
        let tags = vec![
            Tag::parse(&["h", &p.channel_id]).unwrap(),
            Tag::parse(&["p", &p.pubkey]).unwrap(),
        ];
        let builder = EventBuilder::new(k(kind::KIND_DM_ADD_MEMBER), "", tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign add_dm_member event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// List all DM channels the agent is a participant in.
    #[tool(
        name = "list_dms",
        description = "List all direct message channels the agent is a participant in."
    )]
    pub async fn list_dms(&self) -> String {
        // Query DM-created events (kind:41001) where we are a participant (p-tag).
        let my_pubkey = self.client.pubkey_hex();
        let filter = Filter::new()
            .kind(k(kind::KIND_DM_CREATED))
            .custom_tag(tag_p(), [&my_pubkey])
            .limit(100);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let dms: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        let participants: Vec<&str> = e
                            .tags
                            .iter()
                            .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("p"))
                            .filter_map(|t| t.as_slice().get(1).map(|v| v.as_str()))
                            .collect();
                        let dm_id = e
                            .tags
                            .iter()
                            .find(|t| t.as_slice().first().map(|v| v.as_str()) == Some("d"))
                            .and_then(|t| t.as_slice().get(1))
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        serde_json::json!({
                            "dm_id": dm_id,
                            "participants": participants,
                            "created_at": e.created_at.as_u64(),
                        })
                    })
                    .collect();
                serde_json::to_string(&dms).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Hide a DM channel from the agent's DM list.
    #[tool(
        name = "hide_dm",
        description = "Hide a direct message channel from the agent's DM list. The DM can be restored by opening a new DM with the same participants."
    )]
    pub async fn hide_dm(&self, Parameters(p): Parameters<HideDmParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        // Command event kind:41012 with h-tag = DM channel.
        let tags = vec![Tag::parse(&["h", &p.channel_id]).unwrap()];
        let builder = EventBuilder::new(k(kind::KIND_DM_HIDE), "", tags);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign hide_dm event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) if ok.accepted => "DM hidden successfully.".to_string(),
            Ok(ok) => format!("Error: relay rejected: {}", ok.message),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Reaction tools ────────────────────────────────────────────────────────

    /// Add an emoji reaction to a message.
    #[tool(
        name = "add_reaction",
        description = "Add an emoji reaction to a Sprout message."
    )]
    pub async fn add_reaction(&self, Parameters(p): Parameters<AddReactionParams>) -> String {
        let target_eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };
        let builder = match sprout_sdk::build_reaction(target_eid, &p.emoji) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign reaction event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Remove an emoji reaction from a message.
    #[tool(
        name = "remove_reaction",
        description = "Remove an emoji reaction from a Sprout message."
    )]
    pub async fn remove_reaction(&self, Parameters(p): Parameters<RemoveReactionParams>) -> String {
        // Validate event_id format.
        if EventId::from_hex(&p.event_id).is_err() {
            return format!("Error: invalid event_id: {}", p.event_id);
        }
        let my_pubkey_parsed = self.client.keys().public_key();
        let filter = Filter::new()
            .kind(k(kind::KIND_REACTION))
            .author(my_pubkey_parsed)
            .custom_tag(tag_e(), [&p.event_id])
            .limit(50);

        let events = match self.client.query(vec![filter]).await {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to fetch reactions: {e}"),
        };

        // Find the reaction with matching emoji content.
        let reaction_event = events.iter().find(|e| e.content == p.emoji);

        match reaction_event {
            Some(re) => {
                let builder = match sprout_sdk::build_remove_reaction(re.id) {
                    Ok(b) => b,
                    Err(e) => return format!("Error: {e}"),
                };
                let event = match self.client.sign_event(builder) {
                    Ok(e) => e,
                    Err(e) => return format!("Error: failed to sign remove_reaction event: {e}"),
                };
                match self.client.send_event(event).await {
                    Ok(ok) => serde_json::json!({
                        "event_id": ok.event_id,
                        "accepted": ok.accepted,
                        "message": ok.message,
                    })
                    .to_string(),
                    Err(e) => format!("Error: {e}"),
                }
            }
            None => "Error: could not find your reaction event for this emoji. \
                 The reaction may not exist."
                .to_string(),
        }
    }

    /// Get all reactions for a message.
    #[tool(
        name = "get_reactions",
        description = "Get all emoji reactions for a Sprout message."
    )]
    pub async fn get_reactions(&self, Parameters(p): Parameters<GetReactionsParams>) -> String {
        // Query kind:7 reactions referencing this event via e-tag.
        let filter = Filter::new()
            .kind(k(kind::KIND_REACTION))
            .custom_tag(tag_e(), [&p.event_id])
            .limit(200);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                // Group reactions by emoji content.
                let mut grouped: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for e in &events {
                    grouped
                        .entry(e.content.clone())
                        .or_default()
                        .push(e.pubkey.to_hex());
                }
                let reactions: Vec<serde_json::Value> = grouped
                    .into_iter()
                    .map(|(emoji, pubkeys)| {
                        serde_json::json!({
                            "emoji": emoji,
                            "count": pubkeys.len(),
                            "pubkeys": pubkeys,
                        })
                    })
                    .collect();
                serde_json::to_string(&serde_json::json!({ "reactions": reactions }))
                    .unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── User profile tools ────────────────────────────────────────────────────

    /// Update the agent's user profile.
    #[tool(
        name = "set_profile",
        description = "Update the agent's user profile (display name, about, avatar URL, and/or NIP-05 handle)."
    )]
    pub async fn set_profile(&self, Parameters(p): Parameters<SetProfileParams>) -> String {
        // Read-merge-write: fetch current profile, merge desired changes, sign kind:0.
        let my_pubkey = self.client.keys().public_key();
        let filter = Filter::new()
            .kind(k(kind::KIND_PROFILE))
            .author(my_pubkey)
            .limit(1);
        let current_profile: serde_json::Value = match self.client.query(vec![filter]).await {
            Ok(events) => events
                .first()
                .and_then(|e| serde_json::from_str(&e.content).ok())
                .unwrap_or(serde_json::Value::Object(Default::default())),
            Err(_) => serde_json::Value::Object(Default::default()),
        };

        // Resolve each field: use new value if provided, else keep existing.
        let display_name = p
            .display_name
            .as_deref()
            .or_else(|| current_profile.get("display_name").and_then(|v| v.as_str()));
        let name = current_profile.get("name").and_then(|v| v.as_str());
        let picture = p
            .avatar_url
            .as_deref()
            .or_else(|| current_profile.get("avatar_url").and_then(|v| v.as_str()));
        let about = p
            .about
            .as_deref()
            .or_else(|| current_profile.get("about").and_then(|v| v.as_str()));
        let nip05 = p
            .nip05_handle
            .as_deref()
            .or_else(|| current_profile.get("nip05_handle").and_then(|v| v.as_str()));

        let builder = match sprout_sdk::build_profile(display_name, name, picture, about, nip05) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign profile event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get user profile(s) by pubkey.
    #[tool(
        name = "get_users",
        description = "Get user profile(s). Omit pubkeys for your own profile, provide one for a specific user, or provide multiple for batch lookup (max 200)."
    )]
    pub async fn get_users(&self, Parameters(p): Parameters<GetUsersParams>) -> String {
        let pubkeys = p.pubkeys.unwrap_or_default();
        if pubkeys.len() > 200 {
            return "Error: max 200 pubkeys for batch lookup".to_string();
        }
        for pk in &pubkeys {
            if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
                return format!(
                    "Error: pubkey must be a 64-character hex string (got {:?})",
                    pk
                );
            }
        }

        // If no pubkeys specified, fetch our own profile.
        let authors: Vec<nostr::PublicKey> = if pubkeys.is_empty() {
            vec![self.client.keys().public_key()]
        } else {
            pubkeys
                .iter()
                .filter_map(|pk| nostr::PublicKey::from_hex(pk).ok())
                .collect()
        };

        let filter = Filter::new()
            .kind(k(kind::KIND_PROFILE))
            .authors(authors)
            .limit(pubkeys.len().max(1));

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let profiles: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        let mut profile: serde_json::Value =
                            serde_json::from_str(&e.content).unwrap_or(serde_json::json!({}));
                        if let Some(obj) = profile.as_object_mut() {
                            obj.insert("pubkey".to_string(), serde_json::json!(e.pubkey.to_hex()));
                        }
                        profile
                    })
                    .collect();
                if profiles.len() == 1 {
                    serde_json::to_string(&profiles[0]).unwrap_or_else(|e| format!("Error: {e}"))
                } else {
                    serde_json::to_string(&profiles).unwrap_or_else(|e| format!("Error: {e}"))
                }
            }
            Err(e) => format!("Error fetching profile: {e}"),
        }
    }

    /// Full-text search across messages.
    #[tool(
        name = "search",
        description = "Full-text search across messages in accessible channels. Returns matching messages with channel context. Powered by Typesense."
    )]
    pub async fn search(&self, Parameters(p): Parameters<SearchParams>) -> String {
        let limit = p.limit.unwrap_or(20).min(100) as usize;
        // Use NIP-50 search filter if the relay supports it.
        // The `search` field in a filter is a NIP-50 extension.
        let filter = Filter::new()
            .kinds(vec![
                k(kind::KIND_STREAM_MESSAGE),
                k(kind::KIND_STREAM_MESSAGE_V2),
                k(kind::KIND_FORUM_POST),
                k(kind::KIND_FORUM_COMMENT),
            ])
            .search(&p.q)
            .limit(limit);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let results: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id.to_hex(),
                            "pubkey": e.pubkey.to_hex(),
                            "kind": e.kind.as_u16(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                            "tags": e.tags.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                serde_json::to_string(&results).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error searching: {e}"),
        }
    }

    /// Get presence status for one or more users.
    #[tool(
        name = "get_presence",
        description = "Get presence status (online/away/offline) for one or more users by pubkey. Pass comma-separated hex pubkeys."
    )]
    pub async fn get_presence(&self, Parameters(p): Parameters<GetPresenceParams>) -> String {
        // Query ephemeral presence events (kind:20001) for the given pubkeys.
        let authors: Vec<nostr::PublicKey> = p
            .pubkeys
            .split(',')
            .filter_map(|pk| nostr::PublicKey::from_hex(pk.trim()).ok())
            .collect();

        if authors.is_empty() {
            return "Error: no valid pubkeys provided".to_string();
        }

        // Presence snapshots are kind:40902 (relay-generated, latest state).
        let filter = Filter::new()
            .kind(k(kind::KIND_PRESENCE_SNAPSHOT))
            .authors(authors)
            .limit(200);

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let presence: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "pubkey": e.pubkey.to_hex(),
                            "status": e.content,
                            "updated_at": e.created_at.as_u64(),
                        })
                    })
                    .collect();
                serde_json::to_string(&presence).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error fetching presence: {e}"),
        }
    }

    /// Set the agent's presence status.
    #[tool(
        name = "set_presence",
        description = "Set the agent's presence status. Valid values: 'online', 'away', 'offline'. Presence auto-expires after 90 seconds — call periodically to stay online."
    )]
    pub async fn set_presence(&self, Parameters(p): Parameters<SetPresenceParams>) -> String {
        // Validate status value.
        // Publish ephemeral presence event (kind:20001).
        let builder = EventBuilder::new(k(kind::KIND_PRESENCE_UPDATE), p.status.as_str(), []);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign presence event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "status": p.status,
                "accepted": ok.accepted,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Set this agent's channel addition policy.
    #[tool(
        name = "set_channel_add_policy",
        description = "Set your channel addition policy. 'anyone' = any authenticated user can add you to open channels (default). 'owner_only' = only your provisioned owner can add you. 'nobody' = no one can add you; you may self-join open channels via join_channel, but private channels are inaccessible until a consent flow is implemented."
    )]
    pub async fn set_channel_add_policy(
        &self,
        Parameters(p): Parameters<SetChannelAddPolicyParams>,
    ) -> String {
        if !matches!(p.policy.as_str(), "anyone" | "owner_only" | "nobody") {
            return format!(
                "Error: invalid policy {:?} — must be 'anyone', 'owner_only', or 'nobody'",
                p.policy
            );
        }
        // Store as a kind:10100 (agent profile) replaceable event with the policy in content.
        let content = serde_json::json!({ "channel_add_policy": p.policy }).to_string();
        let builder = EventBuilder::new(k(kind::KIND_AGENT_PROFILE), &content, []);
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign agent profile event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "policy": p.policy,
                "accepted": ok.accepted,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Vote on a forum post or comment (kind:45002).
    #[tool(
        name = "vote_on_post",
        description = "Vote on a forum post or comment. Creates a kind:45002 event. \
                       Each vote is a separate event — vote deduplication is not yet enforced."
    )]
    pub async fn vote_on_post(&self, Parameters(p): Parameters<VoteOnPostParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        if p.event_id.len() != 64 || !p.event_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!(
                "Error: event_id must be a 64-character hex string (got {:?})",
                p.event_id
            );
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let target_eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };
        let direction = match p.direction {
            VoteDirection::Up => sprout_sdk::VoteDirection::Up,
            VoteDirection::Down => sprout_sdk::VoteDirection::Down,
        };
        let builder = match sprout_sdk::build_vote(channel_uuid, target_eid, direction) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign vote event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Permanently delete a Sprout channel.
    #[tool(
        name = "delete_channel",
        description = "Permanently delete a Sprout channel. You must be the channel owner. This action is irreversible."
    )]
    pub async fn delete_channel(&self, Parameters(p): Parameters<DeleteChannelParams>) -> String {
        if let Err(e) = validate_uuid(&p.channel_id) {
            return format!("Error: {e}");
        }
        let channel_uuid = match uuid::Uuid::parse_str(&p.channel_id) {
            Ok(u) => u,
            Err(_) => return format!("Error: invalid UUID: {}", p.channel_id),
        };
        let builder = match sprout_sdk::build_delete_channel(channel_uuid) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };
        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign delete_channel event: {e}"),
        };
        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ── Social tools ─────────────────────────────────────────────────────────

    /// Publish a kind:1 text note (global, no channel scope).
    #[tool(
        name = "publish_note",
        description = "Publish a short text note (kind:1) to the global feed. Optionally reply to another note by event ID."
    )]
    pub async fn publish_note(&self, Parameters(p): Parameters<PublishNoteParams>) -> String {
        let reply_id = match p.reply_to_event_id.as_deref() {
            Some(hex) => match EventId::from_hex(hex) {
                Ok(id) => Some(id),
                Err(e) => return format!("Error: invalid reply_to_event_id: {e}"),
            },
            None => None,
        };

        if p.content.len() > 64 * 1024 {
            return format!(
                "Error: content exceeds 64 KiB limit ({} bytes)",
                p.content.len()
            );
        }

        let builder = match sprout_sdk::build_note(&p.content, reply_id) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };

        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign event: {e}"),
        };

        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Replace the authenticated user's contact list (kind:3).
    #[tool(
        name = "set_contact_list",
        description = "Set the authenticated user's contact/follow list (kind:3). Replaces the entire list. Call get_contact_list first for delta updates."
    )]
    pub async fn set_contact_list(
        &self,
        Parameters(p): Parameters<SetContactListParams>,
    ) -> String {
        let contacts: Vec<(&str, Option<&str>, Option<&str>)> = p
            .contacts
            .iter()
            .map(|c| {
                (
                    c.pubkey.as_str(),
                    c.relay_url.as_deref(),
                    c.petname.as_deref(),
                )
            })
            .collect();

        let builder = match sprout_sdk::build_contact_list(&contacts) {
            Ok(b) => b,
            Err(e) => return format!("Error: {e}"),
        };

        let event = match self.client.sign_event(builder) {
            Ok(e) => e,
            Err(e) => return format!("Error: failed to sign event: {e}"),
        };

        match self.client.send_event(event).await {
            Ok(ok) => serde_json::json!({
                "event_id": ok.event_id,
                "accepted": ok.accepted,
                "message": ok.message,
            })
            .to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Fetch a single event by event ID.
    #[tool(
        name = "get_event",
        description = "Fetch a single event by its 64-char hex event ID. For global events: kind:0 profiles and kind:3 contacts require UsersRead scope; kind:1 notes and kind:30023 articles require MessagesRead scope. For channel events: requires MessagesRead scope and channel membership. Unknown kinds return 404."
    )]
    pub async fn get_event(&self, Parameters(p): Parameters<GetEventParams>) -> String {
        if let Err(e) = validate_hex64(&p.event_id, "event_id") {
            return e;
        }
        let eid = match EventId::from_hex(&p.event_id) {
            Ok(id) => id,
            Err(e) => return format!("Error: invalid event_id: {e}"),
        };
        let filter = Filter::new().id(eid).limit(1);
        match self.client.query(vec![filter]).await {
            Ok(events) => match events.first() {
                Some(event) => event.as_json(),
                None => format!("Error: event '{}' not found", p.event_id),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    /// List notes by a specific user.
    #[tool(
        name = "get_user_notes",
        description = "List kind:1 text notes by a specific user (by hex pubkey). Returns id, pubkey, created_at, and content per note (tags and sig omitted — use get_event for full events). Supports composite cursor pagination via `before` (Unix timestamp) and `before_id` (hex event ID)."
    )]
    pub async fn get_user_notes(&self, Parameters(p): Parameters<GetUserNotesParams>) -> String {
        if let Err(e) = validate_hex64(&p.pubkey, "pubkey") {
            return e;
        }
        let author = match nostr::PublicKey::from_hex(&p.pubkey) {
            Ok(pk) => pk,
            Err(e) => return format!("Error: invalid pubkey: {e}"),
        };
        let limit = p.limit.unwrap_or(20).min(200) as usize;
        let mut filter = Filter::new()
            .kind(k(kind::KIND_TEXT_NOTE))
            .author(author)
            .limit(limit);

        if let Some(before) = p.before {
            filter = filter.until(nostr::Timestamp::from(before as u64));
        }

        match self.client.query(vec![filter]).await {
            Ok(events) => {
                let notes: Vec<serde_json::Value> = events
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "id": e.id.to_hex(),
                            "pubkey": e.pubkey.to_hex(),
                            "content": e.content,
                            "created_at": e.created_at.as_u64(),
                        })
                    })
                    .collect();
                serde_json::to_string(&notes).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Get a user's contact/follow list.
    #[tool(
        name = "get_contact_list",
        description = "Get a user's contact/follow list (kind:3) by hex pubkey. Returns the latest replaceable event."
    )]
    pub async fn get_contact_list(
        &self,
        Parameters(p): Parameters<GetContactListParams>,
    ) -> String {
        if let Err(e) = validate_hex64(&p.pubkey, "pubkey") {
            return e;
        }
        let author = match nostr::PublicKey::from_hex(&p.pubkey) {
            Ok(pk) => pk,
            Err(e) => return format!("Error: invalid pubkey: {e}"),
        };
        // Kind:3 is replaceable — query latest.
        let filter = Filter::new()
            .kind(k(kind::KIND_CONTACT_LIST))
            .author(author)
            .limit(1);

        match self.client.query(vec![filter]).await {
            Ok(events) => match events.first() {
                Some(event) => event.as_json(),
                None => format!("Error: no contact list found for {}", p.pubkey),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Upload a local file to the Sprout relay.
    #[tool(
        name = "upload_file",
        description = "Upload a local file (image or video) to the Sprout relay. \
Returns a BlobDescriptor with the URL, hash, dimensions, and other metadata. \
Supported types: JPEG, PNG, GIF, WebP, MP4. \
The returned URL can be included in messages, or use the file_paths parameter \
on send_message to upload and attach in one step."
    )]
    pub async fn upload_file(&self, Parameters(p): Parameters<UploadFileParams>) -> String {
        match crate::upload::upload_file(
            self.client.http_client(),
            self.client.keys(),
            &self.client.relay_http_url(),
            self.client.server_domain().as_deref(),
            &p.file_path,
            self.client.auth_tag_json().as_deref(),
        )
        .await
        {
            Ok(desc) => {
                serde_json::to_string_pretty(&desc).unwrap_or_else(|e| format!("Error: {e}"))
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SproutMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "sprout-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Sprout MCP server — interact with the Sprout relay. \
                 Send messages, read channel history, create channels, \
                 manage canvases, create and manage workflows, \
                 and read your personalized home feed."
                    .to_string(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── percent_encode ────────────────────────────────────────────────────────

    #[test]
    fn percent_encode_empty_string() {
        assert_eq!(percent_encode(""), "");
    }

    #[test]
    fn percent_encode_already_safe_chars() {
        // Unreserved chars (RFC 3986) must pass through unchanged.
        let safe = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~";
        assert_eq!(percent_encode(safe), safe);
    }

    #[test]
    fn percent_encode_space() {
        assert_eq!(percent_encode(" "), "%20");
    }

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(percent_encode("foo?bar"), "foo%3Fbar");
    }

    #[test]
    fn percent_encode_slash() {
        assert_eq!(percent_encode("/"), "%2F");
    }

    #[test]
    fn percent_encode_unicode_multibyte() {
        // "é" is 0xC3 0xA9 in UTF-8.
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    // ── validate_uuid ─────────────────────────────────────────────────────────

    #[test]
    fn validate_uuid_valid() {
        assert!(validate_uuid("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn validate_uuid_valid_v4() {
        assert!(validate_uuid("f47ac10b-58cc-4372-a567-0e02b2c3d479").is_ok());
    }

    #[test]
    fn validate_uuid_invalid_string() {
        let result = validate_uuid("not-a-uuid");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid UUID"));
    }

    #[test]
    fn validate_uuid_empty_string() {
        let result = validate_uuid("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid UUID"));
    }

    #[test]
    fn validate_uuid_almost_valid() {
        // Missing one character in the last group.
        let result = validate_uuid("550e8400-e29b-41d4-a716-44665544000");
        assert!(result.is_err());
    }

    // ── MAX_CONTENT_BYTES ─────────────────────────────────────────────────────

    #[test]
    fn max_content_bytes_value() {
        assert_eq!(MAX_CONTENT_BYTES, 65_536);
    }

    // ── VoteDirection serde ───────────────────────────────────────────────────

    #[test]
    fn vote_direction_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&VoteDirection::Up).unwrap(), "\"up\"");
        assert_eq!(
            serde_json::to_string(&VoteDirection::Down).unwrap(),
            "\"down\""
        );
    }

    #[test]
    fn vote_direction_deserializes_lowercase() {
        assert!(matches!(
            serde_json::from_str::<VoteDirection>("\"up\"").unwrap(),
            VoteDirection::Up
        ));
        assert!(matches!(
            serde_json::from_str::<VoteDirection>("\"down\"").unwrap(),
            VoteDirection::Down
        ));
    }

    #[test]
    fn vote_direction_rejects_invalid() {
        assert!(serde_json::from_str::<VoteDirection>("\"sideways\"").is_err());
        assert!(serde_json::from_str::<VoteDirection>("\"UP\"").is_err());
        assert!(serde_json::from_str::<VoteDirection>("\"\"").is_err());
    }

    #[test]
    fn vote_on_post_params_round_trip() {
        let params = VoteOnPostParams {
            channel_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            event_id: "a".repeat(64),
            direction: VoteDirection::Up,
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: VoteOnPostParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channel_id, params.channel_id);
        assert_eq!(parsed.event_id, params.event_id);
        assert!(matches!(parsed.direction, VoteDirection::Up));
    }

    // Note: `extract_at_names` is now exercised by `sprout-sdk::mentions`
    // tests. The MCP-side I/O wrapper around it (`resolve_content_mentions`)
    // is covered by integration tests against a running relay.
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn truncate_diff_small_passes_through() {
        let diff = "--- a/file\n+++ b/file\n@@ -1,3 +1,3 @@\n context\n-old\n+new\n";
        let (result, truncated) = truncate_diff(diff, 60 * 1024);
        assert_eq!(result, diff);
        assert!(!truncated);
    }

    #[test]
    fn truncate_diff_cuts_at_hunk_boundary() {
        // Build a diff large enough that truncation is meaningful.
        // Repeat the first hunk many times so the total is well above any
        // reasonable max_bytes, then append a second hunk we want excluded.
        let hunk_unit = "--- a/file\n+++ b/file\n@@ -1,3 +1,3 @@\n context\n-old\n+new\n";
        let mut diff = hunk_unit.repeat(20); // ~1140 bytes of first-hunk content
        diff.push_str("@@ -10,3 +10,3 @@\n more context\n-old2\n+new2\n");

        // max_bytes sits inside the repeated first-hunk region (well below total)
        // but above TRUNCATION_NOTICE.len() so effective_limit > 0.
        // effective_limit = max_bytes - TRUNCATION_NOTICE.len() ≈ 500 - 72 = 428,
        // which lands inside the repeated first-hunk block.
        let max_bytes = 500;
        let (result, truncated) = truncate_diff(&diff, max_bytes);
        assert!(truncated);
        assert!(
            result.contains("context"),
            "should contain first-hunk content"
        );
        assert!(result.contains("Diff truncated"));
        assert!(
            !result.contains("@@ -10,3"),
            "second hunk should be excluded"
        );
        // Result must not exceed max_bytes.
        assert!(
            result.len() <= max_bytes,
            "truncated result ({}) exceeds max_bytes ({})",
            result.len(),
            max_bytes
        );
    }

    #[test]
    fn truncate_diff_utf8_safe() {
        // Create a diff with multi-byte chars near the boundary
        let mut diff = String::from("--- a/file\n+++ b/file\n@@ -1,1 +1,1 @@\n-");
        // Add enough content to exceed a small limit, with multi-byte chars
        for _ in 0..100 {
            diff.push('日'); // 3-byte UTF-8 char
        }
        diff.push('\n');
        let (result, truncated) = truncate_diff(&diff, 80);
        assert!(truncated);
        // Must not panic and must produce valid UTF-8
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn truncate_diff_result_within_limit() {
        let mut diff = String::new();
        for i in 0..2000 {
            diff.push_str(&format!(
                "@@ -{i},1 +{i},1 @@\n-old line {i}\n+new line {i}\n"
            ));
        }
        let max = 1024;
        let (result, truncated) = truncate_diff(&diff, max);
        assert!(truncated);
        assert!(
            result.len() <= max,
            "truncated result ({}) exceeds max_bytes ({})",
            result.len(),
            max
        );
    }

    #[test]
    fn infer_language_known_extensions() {
        assert_eq!(infer_language("src/main.rs"), Some("rust".to_string()));
        assert_eq!(infer_language("app.tsx"), Some("typescript".to_string()));
        assert_eq!(infer_language("script.py"), Some("python".to_string()));
        assert_eq!(infer_language("Makefile"), None);
    }

    #[test]
    fn infer_language_no_extension() {
        assert_eq!(infer_language("Dockerfile"), None);
        // But "foo.dockerfile" should match
        assert_eq!(
            infer_language("foo.dockerfile"),
            Some("dockerfile".to_string())
        );
    }
}
