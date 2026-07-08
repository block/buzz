//! Transport-neutral event ingestion pipeline.
//!
//! Both WebSocket `["EVENT", ...]` and HTTP `POST /events` feed into
//! [`ingest_event`] — two doors, one room.

use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use buzz_auth::Scope;
use buzz_core::kind::{
    event_kind_u32, is_identity_archive_request_kind, is_parameterized_replaceable,
    is_relay_admin_kind, KIND_AGENT_ENGRAM, KIND_AGENT_PROFILE, KIND_AGENT_TURN_METRIC,
    KIND_APPROVAL_DENY, KIND_APPROVAL_GRANT, KIND_AUTH, KIND_BOOKMARK_LIST, KIND_BOOKMARK_SET,
    KIND_CANVAS, KIND_CONTACT_LIST, KIND_DELETION, KIND_DM_ADD_MEMBER, KIND_DM_HIDE, KIND_DM_OPEN,
    KIND_EMOJI_LIST, KIND_EMOJI_SET, KIND_EVENT_REMINDER, KIND_FOLLOW_SET, KIND_FORUM_COMMENT,
    KIND_FORUM_POST, KIND_FORUM_VOTE, KIND_GIFT_WRAP, KIND_GIT_ISSUE, KIND_GIT_PATCH,
    KIND_GIT_PR_UPDATE, KIND_GIT_PULL_REQUEST, KIND_GIT_REPO_ANNOUNCEMENT, KIND_GIT_REPO_STATE,
    KIND_GIT_STATUS_CLOSED, KIND_GIT_STATUS_DRAFT, KIND_GIT_STATUS_MERGED, KIND_GIT_STATUS_OPEN,
    KIND_HUDDLE_ENDED, KIND_HUDDLE_GUIDELINES, KIND_HUDDLE_PARTICIPANT_JOINED,
    KIND_HUDDLE_PARTICIPANT_LEFT, KIND_HUDDLE_STARTED, KIND_IA_ARCHIVE_REQUEST,
    KIND_IA_UNARCHIVE_REQUEST, KIND_LONG_FORM, KIND_MANAGED_AGENT, KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION, KIND_MESH_LLM_RELAY_STATUS, KIND_MODERATION_BAN,
    KIND_MODERATION_RESOLVE_REPORT, KIND_MODERATION_TIMEOUT, KIND_MODERATION_UNBAN,
    KIND_MODERATION_UNTIMEOUT, KIND_MUTE_LIST, KIND_NIP29_CREATE_GROUP, KIND_NIP29_DELETE_EVENT,
    KIND_NIP29_DELETE_GROUP, KIND_NIP29_EDIT_METADATA, KIND_NIP29_JOIN_REQUEST,
    KIND_NIP29_LEAVE_REQUEST, KIND_NIP29_PUT_USER, KIND_NIP29_REMOVE_USER,
    KIND_NIP43_LEAVE_REQUEST, KIND_NIP65_RELAY_LIST_METADATA, KIND_PERSONA, KIND_PIN_LIST,
    KIND_PRESENCE_UPDATE, KIND_PROFILE, KIND_REACTION, KIND_READ_STATE, KIND_REPORT,
    KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_BOOKMARKED, KIND_STREAM_MESSAGE_DIFF,
    KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_PINNED, KIND_STREAM_MESSAGE_SCHEDULED,
    KIND_STREAM_MESSAGE_V2, KIND_STREAM_REMINDER, KIND_TEAM, KIND_TEXT_NOTE, KIND_USER_STATUS,
    KIND_WORKFLOW_DEF, KIND_WORKFLOW_TRIGGER, RELAY_ADMIN_ADD_MEMBER, RELAY_ADMIN_CHANGE_ROLE,
    RELAY_ADMIN_REMOVE_MEMBER, RELAY_ADMIN_SET_WORKSPACE_PROFILE,
};
use buzz_core::tenant::TenantContext;
use buzz_core::verification::verify_event;
use buzz_core::CommunityId;
use nostr::Event;

use crate::state::AppState;

use super::event::dispatch_persistent_event;

use crate::conformance::{
    self as conf, channel_label, claimed_community_from_event, emit, msg_id_label,
    state_for_request, EmitGuard, TraceAction, Verdict,
};

/// How the HTTP caller authenticated (for [`IngestAuth::Http`]).
#[derive(Debug, Clone)]
pub enum HttpAuthMethod {
    /// `Authorization: Nostr <base64>` — NIP-98 HTTP Auth.
    Nip98,
    /// `X-Pubkey: <hex>` dev-mode header (backward compat during transition).
    DevPubkey,
}

/// Authentication context for event ingestion — transport-neutral.
#[derive(Debug, Clone)]
pub enum IngestAuth {
    /// WebSocket NIP-42 authenticated connection.
    Nip42 {
        /// The authenticated Nostr public key.
        pubkey: nostr::PublicKey,
        /// Permission scopes granted to this connection.
        scopes: Vec<Scope>,
        /// Token-level channel restriction, if the WebSocket auth used an API token.
        channel_ids: Option<Vec<Uuid>>,
        /// WebSocket connection identifier.
        conn_id: Uuid,
    },
    /// HTTP bridge authenticated request (NIP-98 or dev X-Pubkey).
    Http {
        /// The authenticated Nostr public key.
        pubkey: nostr::PublicKey,
        /// Permission scopes granted to this request.
        scopes: Vec<Scope>,
        /// How the HTTP request was authenticated.
        auth_method: HttpAuthMethod,
    },
}

impl IngestAuth {
    /// The authenticated public key.
    pub fn pubkey(&self) -> &nostr::PublicKey {
        match self {
            Self::Nip42 { pubkey, .. } | Self::Http { pubkey, .. } => pubkey,
        }
    }

    /// Permission scopes for this auth context.
    pub fn scopes(&self) -> &[Scope] {
        match self {
            Self::Nip42 { scopes, .. } | Self::Http { scopes, .. } => scopes,
        }
    }

    /// WebSocket connection ID (Nip42 only).
    pub fn conn_id(&self) -> Option<Uuid> {
        match self {
            Self::Nip42 { conn_id, .. } => Some(*conn_id),
            Self::Http { .. } => None,
        }
    }

    /// Token-level channel restriction (WS connections with scoped tokens — legacy).
    /// In pure Nostr mode this always returns None; channel access is enforced
    /// via NIP-29 membership checks instead.
    pub fn channel_ids(&self) -> Option<&[Uuid]> {
        match self {
            Self::Nip42 {
                channel_ids: Some(ids),
                ..
            } => Some(ids),
            _ => None,
        }
    }

    /// Whether this auth context is an HTTP request (not WebSocket).
    pub fn is_http(&self) -> bool {
        matches!(self, Self::Http { .. })
    }
}

/// Successful ingestion result.
pub struct IngestResult {
    /// Hex-encoded event ID.
    pub event_id: String,
    /// Whether the event was accepted.
    pub accepted: bool,
    /// Optional message (e.g. "duplicate:" for dedup).
    pub message: String,
}

/// Ingestion error — the caller maps this to their transport's error format.
#[derive(Debug)]
pub enum IngestError {
    /// Client error (bad event) — WS: OK false, HTTP: 400.
    Rejected(String),
    /// Auth/scope error — WS: OK false, HTTP: 401/403.
    AuthFailed(String),
    /// Server error — WS: OK false, HTTP: 500.
    Internal(String),
}

/// Determine the required scope for a given event kind.
///
/// Returns `Err` for unknown kinds — the relay rejects them.
fn required_scope_for_kind(kind: u32, event: &Event) -> Result<Scope, &'static str> {
    match kind {
        KIND_PROFILE => Ok(Scope::UsersWrite),
        KIND_TEXT_NOTE | KIND_LONG_FORM => Ok(Scope::MessagesWrite),
        KIND_CONTACT_LIST | KIND_READ_STATE | KIND_USER_STATUS | KIND_AGENT_ENGRAM
        | KIND_EVENT_REMINDER | KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT => {
            Ok(Scope::UsersWrite)
        }
        // NIP-AM: agent turn metrics are agent-authored global events (encrypted to owner).
        KIND_AGENT_TURN_METRIC => Ok(Scope::MessagesWrite),
        // NIP-56 reports are ordinary member writes into the mod-only queue.
        // Ingest persists them to `moderation_reports` and suppresses public
        // storage/fanout; reports are signals, never enforcement triggers.
        KIND_REPORT => Ok(Scope::MessagesWrite),
        // Community moderation commands are direct, mod-authz-gated writes.
        // Scope only proves the transport can submit message writes; the
        // command handler owns role/capability authorization.
        k if buzz_core::kind::is_moderation_command_kind(k) => Ok(Scope::MessagesWrite),
        // NIP-51 standard lists and NIP-65 relay list — user-owned global state,
        // same ownership shape as kind:3 (contacts) and kind:0 (profile).
        KIND_MUTE_LIST
        | KIND_PIN_LIST
        | KIND_NIP65_RELAY_LIST_METADATA
        | KIND_BOOKMARK_LIST
        | KIND_FOLLOW_SET
        | KIND_BOOKMARK_SET
        // NIP-30/NIP-51: per-user custom emoji set (30030) and emoji list (10030).
        // User-owned global state, keyed by (pubkey, kind[, d_tag]); the workspace
        // palette is the client-side union of every member's own set.
        | KIND_EMOJI_SET
        | KIND_EMOJI_LIST
        | KIND_AGENT_PROFILE => Ok(Scope::UsersWrite),
        KIND_DELETION
        | KIND_REACTION
        | KIND_GIFT_WRAP
        | KIND_STREAM_MESSAGE
        | KIND_STREAM_MESSAGE_V2
        | KIND_NIP29_DELETE_EVENT
        | KIND_STREAM_MESSAGE_EDIT
        | KIND_STREAM_MESSAGE_PINNED
        | KIND_STREAM_MESSAGE_BOOKMARKED
        | KIND_STREAM_MESSAGE_SCHEDULED
        | KIND_STREAM_REMINDER
        | KIND_STREAM_MESSAGE_DIFF
        | KIND_FORUM_POST
        | KIND_FORUM_VOTE
        | KIND_FORUM_COMMENT => Ok(Scope::MessagesWrite),
        KIND_NIP29_PUT_USER | KIND_NIP29_REMOVE_USER | KIND_NIP29_DELETE_GROUP => {
            Ok(Scope::AdminChannels)
        }
        // NIP-43: relay membership admin commands (9030–9032) + Buzz
        // workspace-profile command (9033)
        k if k == RELAY_ADMIN_ADD_MEMBER
            || k == RELAY_ADMIN_REMOVE_MEMBER
            || k == RELAY_ADMIN_CHANGE_ROLE
            || k == RELAY_ADMIN_SET_WORKSPACE_PROFILE =>
        {
            Ok(Scope::AdminUsers)
        }
        // NIP-IA: identity archive/unarchive requests (9035/9036).
        // Scope is intentionally UsersWrite, not AdminUsers: NIP-IA's self and
        // owner-of-agent paths are open to ordinary users (a user retiring their
        // own key, or an owner archiving their agent). Real authorization is the
        // consent-path check inside handle_identity_archive_event — the relay
        // verifies self / admin-role / owner-via-live-kind:0 there. This gate
        // only ensures the actor can write user-scoped state, which any
        // profile-publishing user already holds.
        KIND_IA_ARCHIVE_REQUEST | KIND_IA_UNARCHIVE_REQUEST => Ok(Scope::UsersWrite),
        KIND_NIP29_EDIT_METADATA => {
            // kind:9002 scope split: archived tag → AdminChannels, else ChannelsWrite
            let has_archived = event
                .tags
                .iter()
                .any(|t| t.kind().to_string() == "archived");
            if has_archived {
                Ok(Scope::AdminChannels)
            } else {
                Ok(Scope::ChannelsWrite)
            }
        }
        KIND_NIP29_CREATE_GROUP | KIND_CANVAS => Ok(Scope::ChannelsWrite),
        KIND_NIP29_JOIN_REQUEST | KIND_NIP29_LEAVE_REQUEST | KIND_NIP43_LEAVE_REQUEST => {
            Ok(Scope::ChannelsRead)
        }
        // Huddle lifecycle events + guidelines
        KIND_HUDDLE_STARTED
        | KIND_HUDDLE_PARTICIPANT_JOINED
        | KIND_HUDDLE_PARTICIPANT_LEFT
        | KIND_HUDDLE_ENDED
        | KIND_HUDDLE_GUIDELINES => Ok(Scope::ChannelsWrite),
        // NIP-34: Git repository events
        KIND_GIT_REPO_ANNOUNCEMENT | KIND_GIT_REPO_STATE => Ok(Scope::ReposWrite),
        KIND_GIT_PATCH
        | KIND_GIT_PULL_REQUEST
        | KIND_GIT_PR_UPDATE
        | KIND_GIT_ISSUE
        | KIND_GIT_STATUS_OPEN
        | KIND_GIT_STATUS_MERGED
        | KIND_GIT_STATUS_CLOSED
        | KIND_GIT_STATUS_DRAFT => Ok(Scope::MessagesWrite),
        // Command kinds — DM management, workflows, approvals
        KIND_DM_OPEN | KIND_DM_ADD_MEMBER | KIND_DM_HIDE => Ok(Scope::MessagesWrite),
        KIND_WORKFLOW_DEF | KIND_WORKFLOW_TRIGGER => Ok(Scope::MessagesWrite),
        KIND_APPROVAL_GRANT | KIND_APPROVAL_DENY => Ok(Scope::MessagesWrite),
        _ => Err("restricted: unknown event kind"),
    }
}

/// Extract a channel UUID from the `"h"` NIP-29 group tag.
pub(crate) fn extract_channel_id(event: &Event) -> Option<Uuid> {
    for tag in event.tags.iter() {
        if tag.kind().to_string() == "h" {
            if let Some(val) = tag.content() {
                if let Ok(id) = val.parse::<Uuid>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Result of resolving a reaction's target channel.
pub(crate) enum ReactionChannelResult {
    Channel(Uuid),
    NoChannel,
    NotFound,
    NoTarget,
    DbError(String),
}

/// Derive channel_id from the target event for NIP-25 reactions.
pub(crate) async fn derive_reaction_channel(
    community_id: CommunityId,
    db: &buzz_db::Db,
    event: &Event,
) -> ReactionChannelResult {
    let target_hex = match event.tags.iter().rev().find_map(|tag| {
        if tag.kind().to_string() == "e" {
            tag.content().and_then(|v| {
                if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                    Some(v.to_string())
                } else {
                    None
                }
            })
        } else {
            None
        }
    }) {
        Some(h) => h,
        None => return ReactionChannelResult::NoTarget,
    };

    let id_bytes = match hex::decode(&target_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return ReactionChannelResult::NoTarget,
    };

    match db.get_event_by_id(community_id, &id_bytes).await {
        Ok(Some(target)) => match target.channel_id {
            Some(ch_id) => ReactionChannelResult::Channel(ch_id),
            None => ReactionChannelResult::NoChannel,
        },
        Ok(None) => ReactionChannelResult::NotFound,
        Err(e) => ReactionChannelResult::DbError(e.to_string()),
    }
}

/// Kinds that are always global (`channel_id = NULL`).
///
/// If a client includes a stray `h` tag on these kinds, the ingest pipeline
/// sets `channel_id = None` — these events are never channel-scoped.
///
/// Note: the raw `h` tag remains on the stored event (Nostr events are signed,
/// so tags cannot be stripped without invalidating the signature). The read-path
/// filter matching in `filter.rs` treats explicit `h` tags as authoritative,
/// which means a stray `h` tag can still match `#h` queries. This is a known
/// limitation affecting all global-only kinds and should be addressed in the
/// filter layer as a follow-up.
pub(crate) fn is_global_only_kind(kind: u32) -> bool {
    matches!(
        kind,
        KIND_PROFILE
            | KIND_TEXT_NOTE
            | KIND_CONTACT_LIST
            | KIND_LONG_FORM
            | KIND_USER_STATUS
            | KIND_READ_STATE
            // NIP-51 standard lists + sets and NIP-65 relay list — user-owned global state.
            // Same as kind:3 (contacts): keyed by (pubkey, kind) or (pubkey, kind, d_tag),
            // never channel-scoped. A stray `h` tag must not channel-scope them.
            | KIND_MUTE_LIST
            | KIND_PIN_LIST
            | KIND_NIP65_RELAY_LIST_METADATA
            | KIND_BOOKMARK_LIST
            | KIND_FOLLOW_SET
            | KIND_BOOKMARK_SET
            // NIP-30 custom emoji set (30030) + emoji list (10030): user-owned,
            // keyed by (pubkey, kind[, d_tag]). A stray `h` tag must not channel-scope them.
            | KIND_EMOJI_SET
            | KIND_EMOJI_LIST
            // NIP-AE agent engrams are addressed by (pubkey_a, kind, d_tag); never channel-scoped.
            | KIND_AGENT_ENGRAM
            // NIP-ER event reminders are addressed by (pubkey, kind, d_tag); never channel-scoped.
            | KIND_EVENT_REMINDER
            // Agent profile (10100): user-owned replaceable, keyed by pubkey.
            | KIND_AGENT_PROFILE
            // NIP-AP: persona definitions (30175): owner-authored, keyed by (pubkey, kind, d_tag).
            | KIND_PERSONA
            // NIP-AP: team (30176) + managed-agent (30177) definitions: owner-authored,
            // keyed by (pubkey, kind, d_tag). A stray `h` tag must not channel-scope them.
            | KIND_TEAM
            | KIND_MANAGED_AGENT
            // NIP-34: git events use `a` tags (repo reference), not `h` tags (channel scope).
            // Parameterized replaceable kinds are keyed by (pubkey, kind, d_tag).
            | KIND_GIT_REPO_ANNOUNCEMENT
            | KIND_GIT_REPO_STATE
            | KIND_GIT_PATCH
            | KIND_GIT_PULL_REQUEST
            | KIND_GIT_PR_UPDATE
            | KIND_GIT_ISSUE
            | KIND_GIT_STATUS_OPEN
            | KIND_GIT_STATUS_MERGED
            | KIND_GIT_STATUS_CLOSED
            | KIND_GIT_STATUS_DRAFT
            // Community moderation commands (9040–9044): community-global
            // direct commands, same model as the NIP-43 9030-series. A stray
            // `h` tag must never channel-scope them (pinned contract —
            // handlers/moderation_commands.rs routing docs).
            | KIND_MODERATION_BAN
            | KIND_MODERATION_UNBAN
            | KIND_MODERATION_TIMEOUT
            | KIND_MODERATION_UNTIMEOUT
            | KIND_MODERATION_RESOLVE_REPORT
            // NIP-43: relay admin commands and leave requests are global — they
            // must never be channel-scoped, even if the event carries a stray `h` tag.
            | RELAY_ADMIN_ADD_MEMBER
            | RELAY_ADMIN_REMOVE_MEMBER
            | RELAY_ADMIN_CHANGE_ROLE
            | RELAY_ADMIN_SET_WORKSPACE_PROFILE
            | KIND_NIP43_LEAVE_REQUEST
            // NIP-IA: identity archive/unarchive requests drive relay-global
            // archive state (8002/8003/13535) and are audited as global request
            // events. A stray `h` tag must not channel-scope them.
            | KIND_IA_ARCHIVE_REQUEST
            | KIND_IA_UNARCHIVE_REQUEST
            // Mesh-LLM relay status is relay-signed and global. Clients may
            // subscribe to it, but must not channel-scope or submit it.
            | KIND_MESH_LLM_RELAY_STATUS
            // NIP-AM: agent turn metrics are owner-scoped global events.
            // Channel identity is encrypted inside the payload — no `h` tag.
            | KIND_AGENT_TURN_METRIC
    )
}

/// Kinds that require an `h` tag for channel scoping.
pub(crate) fn requires_h_channel_scope(kind: u32) -> bool {
    matches!(
        kind,
        KIND_STREAM_MESSAGE
            | KIND_STREAM_MESSAGE_V2
            | KIND_STREAM_MESSAGE_EDIT
            | KIND_STREAM_MESSAGE_PINNED
            | KIND_STREAM_MESSAGE_BOOKMARKED
            | KIND_STREAM_MESSAGE_SCHEDULED
            | KIND_STREAM_REMINDER
            | KIND_STREAM_MESSAGE_DIFF
            | KIND_CANVAS
            | KIND_FORUM_POST
            | KIND_FORUM_VOTE
            | KIND_FORUM_COMMENT
            // NIP-29 admin kinds (except CREATE_GROUP which creates the channel)
            | KIND_NIP29_PUT_USER
            | KIND_NIP29_REMOVE_USER
            | KIND_NIP29_EDIT_METADATA
            | KIND_NIP29_DELETE_EVENT
            | KIND_NIP29_DELETE_GROUP
            | KIND_NIP29_LEAVE_REQUEST
            // Huddle lifecycle events + guidelines
            | KIND_HUDDLE_STARTED
            | KIND_HUDDLE_PARTICIPANT_JOINED
            | KIND_HUDDLE_PARTICIPANT_LEFT
            | KIND_HUDDLE_ENDED
            | KIND_HUDDLE_GUIDELINES
    )
}

/// Check channel membership: member OR open-visibility channel.
///
/// `channel` is the request's already-fetched channel row, when the caller has
/// one (E1 within-request threading; correctness ruling §4.8). Callers without
/// a row pass `None` and the open-visibility fallback reads the DB directly.
///
/// Returns `Ok(())` if allowed, `Err(reason)` if denied.
pub(crate) async fn check_channel_membership(
    tenant: &TenantContext,
    state: &AppState,
    ch_id: Uuid,
    pubkey_bytes: &[u8],
    channel: Option<&buzz_db::channel::ChannelRecord>,
) -> Result<(), String> {
    match state
        .is_member_cached(tenant.community(), ch_id, pubkey_bytes)
        .await
    {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => return Err(format!("error: database error: {e}")),
    }
    // Not a member — check if channel is open.
    let is_open = match channel {
        Some(ch) => ch.visibility == "open",
        None => state
            .db
            .get_channel(tenant.community(), ch_id)
            .await
            .map(|ch| ch.visibility == "open")
            .unwrap_or(false),
    };
    if is_open {
        Ok(())
    } else {
        Err("restricted: not a channel member".to_string())
    }
}

fn check_token_channel_access(auth: &IngestAuth, channel_id: Uuid) -> Result<(), String> {
    if let Some(allowed) = auth.channel_ids() {
        if !allowed.contains(&channel_id) {
            return Err("restricted: token does not have access to this channel".to_string());
        }
    }
    Ok(())
}

/// Owned thread metadata for the DB insert.
pub(crate) struct ThreadMetadataOwned {
    pub event_id: Vec<u8>,
    pub event_created_at: chrono::DateTime<Utc>,
    pub channel_id: Uuid,
    pub parent_event_id: Vec<u8>,
    pub parent_event_created_at: chrono::DateTime<Utc>,
    pub root_event_id: Vec<u8>,
    pub root_event_created_at: chrono::DateTime<Utc>,
    pub depth: i32,
    pub broadcast: bool,
}

impl ThreadMetadataOwned {
    pub fn as_params(&self) -> buzz_db::event::ThreadMetadataParams<'_> {
        buzz_db::event::ThreadMetadataParams {
            event_id: &self.event_id,
            event_created_at: self.event_created_at,
            channel_id: self.channel_id,
            parent_event_id: Some(&self.parent_event_id),
            parent_event_created_at: Some(self.parent_event_created_at),
            root_event_id: Some(&self.root_event_id),
            root_event_created_at: Some(self.root_event_created_at),
            depth: self.depth,
            broadcast: self.broadcast,
        }
    }
}

/// Resolve NIP-10 thread ancestry from e-tags.
pub(crate) async fn resolve_nip10_thread_meta(
    community_id: CommunityId,
    event: &Event,
    channel_id: Uuid,
    state: &AppState,
) -> Result<Option<ThreadMetadataOwned>, String> {
    let mut root_hex: Option<String> = None;
    let mut reply_hex: Option<String> = None;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() >= 4 && parts[0] == "e" {
            let hex_val = &parts[1];
            let marker = &parts[3];
            if hex_val.len() == 64 && hex_val.chars().all(|c| c.is_ascii_hexdigit()) {
                match marker.as_str() {
                    "root" => root_hex = Some(hex_val.to_string()),
                    "reply" => reply_hex = Some(hex_val.to_string()),
                    _ => {}
                }
            }
        }
    }

    if root_hex.is_none() && reply_hex.is_none() {
        return Ok(None);
    }

    let (root_hex, parent_hex) = match (root_hex, reply_hex) {
        (Some(r), Some(p)) => (r, p),
        (None, Some(p)) => (p.clone(), p),
        (Some(_), None) | (None, None) => return Ok(None),
    };

    let parent_bytes =
        hex::decode(&parent_hex).map_err(|_| "invalid parent event ID hex".to_string())?;

    let (parent_event_result, parent_meta_result) = tokio::join!(
        state.db.get_event_by_id(community_id, &parent_bytes),
        state
            .db
            .get_thread_metadata_by_event(community_id, &parent_bytes),
    );

    let parent_event = parent_event_result
        .map_err(|e| format!("db error looking up parent: {e}"))?
        .ok_or_else(|| "reply parent not found".to_string())?;

    match parent_event.channel_id {
        Some(parent_ch) if parent_ch != channel_id => {
            return Err("parent event belongs to a different channel".to_string());
        }
        None => return Err("parent event has no channel association".to_string()),
        _ => {}
    }

    let parent_created =
        chrono::DateTime::from_timestamp(parent_event.event.created_at.as_secs() as i64, 0)
            .unwrap_or_else(Utc::now);

    let client_root_bytes =
        hex::decode(&root_hex).map_err(|_| "invalid root event ID hex".to_string())?;

    let parent_meta =
        parent_meta_result.map_err(|e| format!("db error looking up thread metadata: {e}"))?;

    let (final_root_bytes, root_created, depth) = match parent_meta {
        Some(meta) => {
            let effective_root = meta.root_event_id.unwrap_or_else(|| parent_bytes.clone());
            if client_root_bytes != effective_root {
                return Err("root tag does not match thread ancestry".to_string());
            }
            let root_ts = if let Ok(Some(root_ev)) = state
                .db
                .get_event_by_id(community_id, &effective_root)
                .await
            {
                chrono::DateTime::from_timestamp(root_ev.event.created_at.as_secs() as i64, 0)
                    .unwrap_or(parent_created)
            } else {
                parent_created
            };
            let depth = meta.depth + 1;
            if depth > 100 {
                return Err("thread depth limit exceeded".to_string());
            }
            (effective_root, root_ts, depth)
        }
        None => {
            let parent_root = parent_event
                .event
                .tags
                .iter()
                .find_map(|t| {
                    let parts = t.as_slice();
                    if parts.len() >= 4 && parts[0] == "e" && parts[3] == "root" {
                        hex::decode(&parts[1]).ok().filter(|b| b.len() == 32)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    parent_event.event.tags.iter().find_map(|t| {
                        let parts = t.as_slice();
                        if parts.len() >= 4 && parts[0] == "e" && parts[3] == "reply" {
                            hex::decode(&parts[1]).ok().filter(|b| b.len() == 32)
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| parent_bytes.clone());

            if client_root_bytes != parent_root {
                return Err("root tag does not match thread ancestry".to_string());
            }
            let depth = if parent_root == parent_bytes { 1 } else { 2 };
            let root_created = if parent_root != parent_bytes {
                if let Ok(Some(root_ev)) =
                    state.db.get_event_by_id(community_id, &parent_root).await
                {
                    chrono::DateTime::from_timestamp(root_ev.event.created_at.as_secs() as i64, 0)
                        .unwrap_or(parent_created)
                } else {
                    parent_created
                }
            } else {
                parent_created
            };
            (parent_root, root_created, depth)
        }
    };

    let broadcast = event.tags.iter().any(|t| {
        let parts = t.as_slice();
        parts.len() >= 2 && parts[0] == "broadcast" && parts[1] == "1"
    });

    let event_created_at = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .unwrap_or_else(Utc::now);

    Ok(Some(ThreadMetadataOwned {
        event_id: event.id.as_bytes().to_vec(),
        event_created_at,
        channel_id,
        parent_event_id: parent_bytes,
        parent_event_created_at: parent_created,
        root_event_id: final_root_bytes,
        root_event_created_at: root_created,
        depth,
        broadcast,
    }))
}

/// Count all `e` tags regardless of content validity.
fn count_e_tags(event: &Event) -> usize {
    event
        .tags
        .iter()
        .filter(|t| t.kind().to_string() == "e")
        .count()
}

/// Extract the effective author of a stored event (handles relay-signed REST events).
pub(crate) fn effective_message_author(event: &Event, relay_pubkey: &nostr::PublicKey) -> Vec<u8> {
    if event.pubkey == *relay_pubkey {
        // Relay-signed REST event — real author in "actor" or "p" tag.
        if let Some(hex) = event.tags.iter().find_map(|t| {
            if t.kind().to_string() == "actor" {
                t.content().map(|s| s.to_string())
            } else {
                None
            }
        }) {
            if let Ok(bytes) = hex::decode(&hex) {
                if bytes.len() == 32 {
                    return bytes;
                }
            }
        }
        for tag in event.tags.iter() {
            if tag.kind().to_string() == "p" {
                if let Some(hex) = tag.content() {
                    if let Ok(bytes) = hex::decode(hex) {
                        if bytes.len() == 32 {
                            return bytes;
                        }
                    }
                }
            }
        }
    }
    event.pubkey.to_bytes().to_vec()
}

/// Validate kind:40003 edit ownership — event.pubkey must match target's effective author,
/// or the actor must be the owning human of the agent that authored the target message.
async fn validate_edit_ownership(
    community_id: CommunityId,
    event: &Event,
    state: &AppState,
) -> Result<(), String> {
    let target_hex = event
        .tags
        .iter()
        .find_map(|t| {
            if t.kind().to_string() == "e" {
                t.content().and_then(|v| {
                    if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .ok_or_else(|| "missing e tag for edit target".to_string())?;

    let target_bytes =
        hex::decode(&target_hex).map_err(|_| "invalid target event ID".to_string())?;
    let target_event = state
        .db
        .get_event_by_id(community_id, &target_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?
        .ok_or_else(|| "edit target event not found".to_string())?;

    // Verify target belongs to the same channel as the edit event.
    let edit_channel_id = extract_channel_id(event);
    match (edit_channel_id, target_event.channel_id) {
        (Some(edit_ch), Some(target_ch)) if edit_ch != target_ch => {
            return Err("target event belongs to a different channel".to_string());
        }
        (Some(_), None) => {
            return Err("target event has no channel".to_string());
        }
        _ => {} // Same channel or no channel context — OK
    }

    let author = effective_message_author(&target_event.event, &state.relay_keypair.public_key());
    let actor = event.pubkey.to_bytes().to_vec();
    if author == actor {
        // Author editing their own message: re-gate on membership/open visibility so that
        // a removed private-channel member cannot mutate old messages after access is revoked.
        if let Some(ch_id) = target_event.channel_id {
            let is_member = state
                .is_member_cached(community_id, ch_id, &actor)
                .await
                .map_err(|e| format!("db error checking membership: {e}"))?;
            if !is_member {
                let is_open = state
                    .db
                    .get_channel(community_id, ch_id)
                    .await
                    .map(|ch| ch.visibility == "open")
                    .unwrap_or(false);
                if !is_open {
                    return Err("restricted: not a channel member".to_string());
                }
            }
        }
    } else {
        // Allow the owning human to edit messages authored by their agent.
        let is_owner = state
            .db
            .is_agent_owner(community_id, &author, &actor)
            .await
            .map_err(|e| format!("db error checking agent ownership: {e}"))?;
        if !is_owner {
            return Err("must be event author to edit".to_string());
        }
    }
    Ok(())
}

/// Validate kind:45002 vote targets a forum post (45001) or comment (45003).
async fn validate_forum_vote_target(
    community_id: CommunityId,
    event: &Event,
    state: &AppState,
) -> Result<(), String> {
    let target_hex = event
        .tags
        .iter()
        .find_map(|t| {
            if t.kind().to_string() == "e" {
                t.content().and_then(|v| {
                    if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .ok_or_else(|| "missing e tag for vote target".to_string())?;

    let target_bytes =
        hex::decode(&target_hex).map_err(|_| "invalid target event ID".to_string())?;
    let target_event = state
        .db
        .get_event_by_id(community_id, &target_bytes)
        .await
        .map_err(|e| format!("db error: {e}"))?
        .ok_or_else(|| "vote target event not found".to_string())?;

    let target_kind = event_kind_u32(&target_event.event);
    if target_kind != KIND_FORUM_POST && target_kind != KIND_FORUM_COMMENT {
        return Err("vote target must be a forum post or comment".to_string());
    }

    // Verify target belongs to the same channel as the vote event.
    let vote_channel_id = extract_channel_id(event);
    match (vote_channel_id, target_event.channel_id) {
        (Some(vote_ch), Some(target_ch)) if vote_ch != target_ch => {
            return Err("target event belongs to a different channel".to_string());
        }
        (Some(_), None) => {
            return Err("target event has no channel".to_string());
        }
        _ => {}
    }
    Ok(())
}

/// Validate kind:40008 diff event metadata tags.
fn validate_diff_event(event: &Event) -> Result<(), String> {
    // Content max 60KB
    if event.content.len() > 61_440 {
        return Err(format!(
            "diff content exceeds 60KB limit (got {} bytes)",
            event.content.len()
        ));
    }

    let mut has_repo = false;
    let mut has_commit = false;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() < 2 {
            continue;
        }
        match parts[0].as_str() {
            "repo" => {
                let url = &parts[1];
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("repo URL must be http or https".to_string());
                }
                has_repo = true;
            }
            "commit" => {
                let sha = &parts[1];
                if sha.len() < 7 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("commit SHA must be at least 7 hex characters".to_string());
                }
                has_commit = true;
            }
            "parent-commit" => {
                let sha = &parts[1];
                if sha.len() < 7 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("parent-commit SHA must be at least 7 hex characters".to_string());
                }
            }
            "branch" if (parts.len() < 3 || parts[1].is_empty() || parts[2].is_empty()) => {
                return Err("branch tag requires both source and target".to_string());
            }
            "pr" if parts[1].parse::<u32>().map(|n| n == 0).unwrap_or(true) => {
                return Err("pr number must be a positive integer".to_string());
            }
            _ => {}
        }
    }

    if !has_repo {
        return Err("diff event requires a repo tag".to_string());
    }
    if !has_commit {
        return Err("diff event requires a commit tag".to_string());
    }
    Ok(())
}

/// Validate the public envelope of a NIP-AE `kind:30174` event before it
/// reaches NIP-33 parameterized replacement.
///
/// We deliberately do this here (not in the d-tag length check downstream)
/// because a malformed envelope can otherwise *replace* a valid head in
/// storage and then be invisible to readers querying `#p`. The relay sees
/// no plaintext, but it can — and must — enforce the public tag shape:
///
/// * exactly one `d` tag with a 64-hex value (`d_tag = lower_hex(HMAC...)`),
/// * exactly one `p` tag with a 64-hex pubkey (the owner counterparty).
///
/// Content is opaque NIP-44 ciphertext; we do not parse it.
fn validate_engram_envelope(event: &Event) -> Result<(), String> {
    let mut d_tags: Vec<&str> = Vec::new();
    let mut p_tags: Vec<&str> = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() < 2 {
            continue;
        }
        match parts[0].as_str() {
            "d" => d_tags.push(&parts[1]),
            "p" => p_tags.push(&parts[1]),
            _ => {}
        }
    }
    if d_tags.len() != 1 {
        return Err(format!(
            "agent-engram event must have exactly one `d` tag (got {})",
            d_tags.len()
        ));
    }
    if p_tags.len() != 1 {
        return Err(format!(
            "agent-engram event must have exactly one `p` tag (got {})",
            p_tags.len()
        ));
    }
    let d = d_tags[0];
    if d.len() != 64
        || !d
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("agent-engram `d` tag must be 64 lowercase hex chars".to_string());
    }
    let p = p_tags[0];
    // Lowercase-only: readers query `#p` with `owner.to_hex()` (lowercase) and
    // Nostr tag matching is byte-exact. Accepting uppercase here would let a
    // submitter replace the lowercase head with an event that subsequent
    // lowercase-`#p` queries cannot see — silently bricking the slug.
    if p.len() != 64
        || !p
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("agent-engram `p` tag must be 64 lowercase hex chars (pubkey)".to_string());
    }
    // Content must be a syntactically plausible NIP-44 v2 payload. We do not
    // (and cannot) verify the MAC at the relay, but we can reject obvious
    // garbage so a malformed event cannot supersede a valid head via NIP-33
    // replacement and then be silently discarded by readers.
    validate_engram_nip44_content(&event.content)?;
    Ok(())
}

/// Validate the envelope of a kind:30175 persona event.
///
/// Enforces:
/// * exactly one `d` tag with a non-empty value matching the slug grammar
///   `^[a-z0-9][a-z0-9_-]{0,63}$`.
///
/// Without this, an empty d-tag collapses every persona into the
/// `(pubkey, 30175, "")` slot — last-write-wins data loss.
fn validate_persona_envelope(event: &Event) -> Result<(), String> {
    let mut d_tags: Vec<&str> = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() >= 2 && parts[0].as_str() == "d" {
            d_tags.push(&parts[1]);
        }
    }
    if d_tags.len() != 1 {
        return Err(format!(
            "persona event must have exactly one `d` tag (got {})",
            d_tags.len()
        ));
    }
    let d = d_tags[0];
    if d.is_empty() {
        return Err("persona event `d` tag must not be empty".to_string());
    }
    // Slug grammar: ^[a-z0-9][a-z0-9_-]{0,63}$
    if d.len() > 64 {
        return Err(format!(
            "persona event `d` tag too long ({} chars, max 64)",
            d.len()
        ));
    }
    let bytes = d.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err(
            "persona event `d` tag must start with a lowercase letter or digit".to_string(),
        );
    }
    if !bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(
            "persona event `d` tag must match [a-z0-9_-] after the first character".to_string(),
        );
    }
    Ok(())
}

/// Validate that `content` is a syntactically plausible NIP-44 v2 ciphertext.
///
/// Checks:
/// - Non-empty.
/// - Standard base64 alphabet only (A-Z, a-z, 0-9, +, /, =), with padding only
///   at the end and total length a multiple of 4.
/// - Decoded length >= 99 bytes (1 version + 32 nonce + 32 MAC + minimum 34
///   bytes of length-prefixed padded ciphertext required by NIP-44 v2).
/// - First decoded byte is `0x02` (NIP-44 version 2).
///
/// This is an envelope sanity check, not full validation: the MAC and actual
/// decryption happen at the reader. The intent is to refuse obvious junk so a
/// malformed event cannot win NIP-33 replacement against a valid head and then
/// be silently skipped by `validate_and_decrypt`. Mirrors the validator in
/// `buzz-pair-relay::validate_nip44_content`.
fn validate_engram_nip44_content(content: &str) -> Result<(), String> {
    if content.is_empty() {
        return Err("agent-engram content must not be empty (NIP-44 ciphertext)".to_string());
    }
    let bytes = content.as_bytes();
    let len = bytes.len();
    if !len.is_multiple_of(4) {
        return Err("agent-engram content is not valid base64 (length)".to_string());
    }
    let mut pad_count = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                if pad_count > 0 {
                    return Err("agent-engram content is not valid base64".to_string());
                }
            }
            b'=' => {
                if i < len - 2 {
                    return Err("agent-engram content is not valid base64".to_string());
                }
                pad_count += 1;
                if pad_count > 2 {
                    return Err("agent-engram content is not valid base64".to_string());
                }
            }
            _ => return Err("agent-engram content is not valid base64".to_string()),
        }
    }
    let decoded_len = (len / 4) * 3 - pad_count;
    if decoded_len < 99 {
        return Err("agent-engram content too short for NIP-44 v2".to_string());
    }
    let b64_val = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let v0 =
        b64_val(bytes[0]).ok_or_else(|| "agent-engram content is not valid base64".to_string())?;
    let v1 =
        b64_val(bytes[1]).ok_or_else(|| "agent-engram content is not valid base64".to_string())?;
    let first_byte = (v0 << 2) | (v1 >> 4);
    if first_byte != 0x02 {
        return Err(
            "agent-engram content is not NIP-44 v2 (expected 0x02 version prefix)".to_string(),
        );
    }
    Ok(())
}

/// Validate the public envelope of a NIP-AM `kind:44200` event.
///
/// Enforces (without touching the encrypted payload):
/// - Exactly one `p` tag: 64 lowercase hex chars (the owner pubkey).
/// - Exactly one `agent` tag: 64 lowercase hex chars equal to `event.pubkey`.
/// - No `h` tag (channel identity belongs inside the encrypted payload).
/// - Content syntactically resembles NIP-44 v2 ciphertext (delegated to
///   `validate_engram_nip44_content`, which does the same length/base64/version check).
///
/// Ownership (`is_agent_owner`) is an async DB check performed separately in
/// `ingest_event_inner` after this synchronous envelope check.
fn validate_agent_turn_metric_envelope(event: &nostr::Event) -> Result<(), String> {
    let event_pubkey_hex = event.pubkey.to_hex();
    let mut p_tags: Vec<&str> = Vec::new();
    let mut agent_tags: Vec<&str> = Vec::new();
    let mut has_h_tag = false;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() < 2 {
            continue;
        }
        match parts[0].as_str() {
            "p" => p_tags.push(&parts[1]),
            "agent" => agent_tags.push(&parts[1]),
            "h" => has_h_tag = true,
            _ => {}
        }
    }

    if has_h_tag {
        return Err(
            "agent-turn-metric event must not have an `h` tag (channel identity belongs inside the encrypted payload)".to_string(),
        );
    }

    if p_tags.len() != 1 {
        return Err(format!(
            "agent-turn-metric event must have exactly one `p` tag (got {})",
            p_tags.len()
        ));
    }
    let p = p_tags[0];
    if p.len() != 64
        || !p
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("agent-turn-metric `p` tag must be 64 lowercase hex chars".to_string());
    }

    if agent_tags.len() != 1 {
        return Err(format!(
            "agent-turn-metric event must have exactly one `agent` tag (got {})",
            agent_tags.len()
        ));
    }
    let agent = agent_tags[0];
    if agent.len() != 64
        || !agent
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("agent-turn-metric `agent` tag must be 64 lowercase hex chars".to_string());
    }
    if agent != event_pubkey_hex {
        return Err("agent-turn-metric `agent` tag must equal event pubkey".to_string());
    }

    // Content must look like a NIP-44 v2 ciphertext (length, base64, version prefix).
    validate_engram_nip44_content(&event.content)
        .map_err(|e| e.replace("agent-engram", "agent-turn-metric"))?;

    Ok(())
}

/// Parse a NIP-ER `not_before` tag value into a Unix timestamp.
///
/// The value MUST be a decimal integer string containing only ASCII digits, with
/// no sign, whitespace, decimal point, or leading zero (except the literal `"0"`),
/// and MUST be in the range 0..=9007199254740991 (`Number.MAX_SAFE_INTEGER`, the
/// interoperable JSON integer bound the spec mandates). Parsing is exact integer
/// parsing — never lossy floating-point — so values that overflow are malformed.
fn validate_not_before(tag_value: &str) -> Result<u64, &'static str> {
    const MAX_NOT_BEFORE: u64 = 9_007_199_254_740_991;

    if tag_value.is_empty() || !tag_value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("malformed not_before");
    }
    // Reject leading zeros (e.g. "007") so each timestamp has one canonical form.
    // "0" itself is the only value allowed to begin with '0'.
    if tag_value.len() > 1 && tag_value.starts_with('0') {
        return Err("malformed not_before");
    }
    // Exact integer parse — `u64::from_str` rejects overflow rather than rounding,
    // so values that would lose precision as f64 are caught before the range check.
    let value: u64 = tag_value.parse().map_err(|_| "malformed not_before")?;
    if value > MAX_NOT_BEFORE {
        return Err("malformed not_before");
    }
    Ok(value)
}

/// Validate the public tag envelope of a NIP-ER `kind:30300` event before it
/// reaches NIP-33 parameterized replacement.
///
/// The relay never decrypts the reminder; it only enforces the public schedule
/// tags. A reminder carries at most one `not_before` (omitted on terminal
/// states), and — when both `not_before` and an optional NIP-40 `expiration`
/// are present — `expiration` MUST be strictly after `not_before` (an
/// `expiration <= not_before` window would expire the reminder before it ever
/// became due).
fn validate_event_reminder(event: &Event) -> Result<(), &'static str> {
    let mut not_before: Option<u64> = None;
    let mut expiration: Option<&str> = None;
    let mut d_count = 0u8;
    let mut d_empty = false;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() < 2 {
            continue;
        }
        match parts[0].as_str() {
            "not_before" => {
                // Spec (NIP-ER line 60) collapses invalid and duplicate
                // `not_before` into one wire string clients may match on.
                if not_before.is_some() {
                    return Err("malformed not_before");
                }
                not_before = Some(validate_not_before(&parts[1])?);
            }
            "expiration" => expiration = Some(&parts[1]),
            "d" => {
                d_count = d_count.saturating_add(1);
                if parts[1].is_empty() {
                    d_empty = true;
                }
            }
            _ => {}
        }
    }

    // d-tag: must have exactly one, non-empty
    if d_count == 0 {
        return Err("missing d tag");
    }
    if d_count > 1 {
        return Err("duplicate d tag");
    }
    if d_empty {
        return Err("empty d tag");
    }

    // `not_before` is optional — terminal states (done/cancelled) and bookmarks
    // omit it. The ordering check only applies when both are present.
    if let Some(nb) = not_before {
        // Reject reminders scheduled beyond the configured horizon. The same
        // SPROUT_MAX_NOT_BEFORE_DELTA env var is advertised in NIP-11.
        let max_delta: u64 = std::env::var("SPROUT_MAX_NOT_BEFORE_DELTA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(31_536_000); // 1 year default
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if nb > now + max_delta {
            return Err("not_before too far in future");
        }

        if let Some(exp) = expiration {
            if let Ok(exp) = exp.parse::<u64>() {
                if exp <= nb {
                    return Err("expiration before not_before");
                }
            }
        }
    }

    Ok(())
}

/// Ingest a signed Nostr event through the full validation pipeline.
///
/// Shared by WebSocket and HTTP transports. The caller constructs [`IngestAuth`]
/// from their transport-specific auth mechanism and maps the result to their
/// transport-specific response format.
///
/// Builds a [`crate::conformance::EmitGuard`] around the actual ingest
/// logic so the trace seam has fail-closed coverage: any exit path that
/// doesn't emit a Write*/SanitizedError action will be caught by the
/// guard's Drop → `ImplBug` → CoverageBreach. The wrapper also maps
/// `IngestError` → SanitizedError in one place, sparing every individual
/// `return Err(...)` from having to emit explicitly. See
/// `crates/buzz-relay/src/conformance/mod.rs` and
/// `docs/spec/MultiTenantRelay.tla`.
pub async fn ingest_event(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    let abstract_state = state_for_request(tenant, auth.pubkey());
    let (_guard, tracer) = EmitGuard::arm(
        state.tracer.clone(),
        abstract_state.clone(),
        "ingest_event_exited_without_trace",
    );

    let result = ingest_event_inner(state, &tracer, tenant, event, auth).await;

    // Map terminal error variants onto the closed SanitizedReason
    // alphabet (spec line 778). The inner fn's success path emits
    // WriteInsert/WriteInsertGlobal/WriteDuplicate explicitly at its
    // dispatch points — so on Ok we don't emit here.
    if let Err(err) = &result {
        let reason = conf::sanitized_reason_for(err);
        emit(
            &tracer,
            TraceAction::SanitizedError { reason },
            abstract_state.clone(),
        );
    }

    // _guard drops here. If `tracer` received no records during the
    // request (a panic before the first emit, or a future new exit
    // path that forgets to emit), Drop records an ImplBug step on
    // the underlying tracer — the checker treats that as
    // CoverageBreach.
    result
}

async fn ingest_event_inner(
    state: &Arc<AppState>,
    tracer: &Arc<dyn buzz_conformance::Tracer>,
    tenant: &TenantContext,
    event: Event,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    let event_id_hex = event.id.to_hex();
    let kind_u32 = event_kind_u32(&event);
    debug!(event_id = %event_id_hex, kind = kind_u32, "ingest_event");

    if kind_u32 == KIND_AUTH {
        return Err(IngestError::Rejected(
            "invalid: AUTH events cannot be submitted".into(),
        ));
    }
    if kind_u32 == KIND_MEMBER_ADDED_NOTIFICATION || kind_u32 == KIND_MEMBER_REMOVED_NOTIFICATION {
        return Err(IngestError::Rejected(
            "invalid: membership notifications are relay-signed only".into(),
        ));
    }

    if auth.is_http() && (kind_u32 == KIND_GIFT_WRAP || kind_u32 == KIND_PRESENCE_UPDATE) {
        return Err(IngestError::Rejected(format!(
            "invalid: kind {kind_u32} is only accepted via WebSocket"
        )));
    }

    if buzz_core::kind::is_relay_only_kind(kind_u32) {
        return Err(IngestError::Rejected("restricted: relay-only kind".into()));
    }

    // Share the event with the verify task via Arc instead of deep-cloning it
    // (tags + up to 256 KB of content). spawn_blocking only needs 'static, not
    // ownership; once it completes its Arc is dropped, so try_unwrap returns
    // the original event without ever having copied it.
    let event = std::sync::Arc::new(event);
    let event_for_verify = std::sync::Arc::clone(&event);
    let verify_result = tokio::task::spawn_blocking(move || verify_event(&event_for_verify)).await;
    match verify_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(IngestError::Rejected(format!("invalid: {e}")));
        }
        Err(e) => {
            error!("spawn_blocking panicked: {e}");
            return Err(IngestError::Internal(
                "error: internal verification error".into(),
            ));
        }
    }
    let event = std::sync::Arc::try_unwrap(event).unwrap_or_else(|arc| (*arc).clone());

    const MAX_TIMESTAMP_DRIFT_SECS: i64 = 900; // ±15 minutes
    let now = chrono::Utc::now().timestamp();
    let event_ts = event.created_at.as_secs() as i64;
    if (event_ts - now).abs() > MAX_TIMESTAMP_DRIFT_SECS {
        return Err(IngestError::Rejected(
            "invalid: event timestamp too far from server time".into(),
        ));
    }

    const MAX_EVENT_CONTENT_BYTES: usize = 256 * 1024; // 256 KB
    if event.content.len() > MAX_EVENT_CONTENT_BYTES {
        return Err(IngestError::Rejected(format!(
            "invalid: content exceeds maximum size of {} bytes (got {})",
            MAX_EVENT_CONTENT_BYTES,
            event.content.len()
        )));
    }

    let is_gift_wrap = kind_u32 == KIND_GIFT_WRAP;
    if event.pubkey != *auth.pubkey() && !is_gift_wrap {
        return Err(IngestError::AuthFailed(
            "invalid: event pubkey does not match authenticated identity".into(),
        ));
    }

    let required = match required_scope_for_kind(kind_u32, &event) {
        Ok(scope) => scope,
        Err(msg) => return Err(IngestError::Rejected(msg.into())),
    };
    // NIP-43: relay admin commands are global — channel-scoped tokens cannot
    // issue them even if the event has no `h` tag (is_global_only_kind strips
    // channel_id, but we still need to reject the token itself).
    if is_relay_admin_kind(kind_u32) && auth.channel_ids().is_some() {
        return Err(IngestError::AuthFailed(
            "restricted: relay admin commands require a global token, not a channel-scoped token"
                .into(),
        ));
    }
    // NIP-43: leave requests are also global — channel-scoped tokens cannot
    // issue them.
    if kind_u32 == KIND_NIP43_LEAVE_REQUEST && auth.channel_ids().is_some() {
        return Err(IngestError::AuthFailed(
            "restricted: leave requests require a global token".into(),
        ));
    }
    if !auth.scopes().contains(&required) {
        return Err(IngestError::AuthFailed(format!(
            "restricted: insufficient scope (need {})",
            required
        )));
    }

    // Command kinds are routed AFTER signature verification, timestamp check,
    // pubkey/auth match, and scope validation — never before.
    if buzz_core::kind::is_command_kind(kind_u32) {
        return super::command_executor::handle_command(tenant, state, event, auth).await;
    }

    // NIP-56 reports are persisted only to the mod queue. They are not stored in
    // the public events table and never fan out to subscribers. Reports remain
    // available while timed out so users can signal abuse during a write-block.
    // A banned actor in the rare missed-disconnect window may also submit a
    // report; that is tolerated because reports are non-actioning signals and
    // remain visible only to moderators.
    if kind_u32 == KIND_REPORT {
        super::report::handle_report_event(tenant, &event, state)
            .await
            .map_err(IngestError::Rejected)?;
        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: String::new(),
        });
    }

    // Community moderation commands (9040–9044) are direct, community-global
    // mutations. They are never stored or fanned out as ordinary events; the
    // handler writes the durable audit/enforcement rows after its own capability
    // authorization. These commands are intentionally routed before the
    // timeout/write-block gate below: restriction-lifting commands must remain
    // available so a wrongly restricted admin is not stranded, while banned
    // actors are handled by the auth seam and live-disconnect enforcement.
    if buzz_core::kind::is_moderation_command_kind(kind_u32) {
        super::moderation_commands::handle_moderation_command(tenant, state, &event)
            .await
            .map_err(IngestError::Rejected)?;
        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: String::new(),
        });
    }

    // Community ban / timeout write-block (COMMUNITY_MODERATION_PLAN.md §0
    // decision 4). A timeout is a write-block only — the connection stays open,
    // content writes are refused with `restricted: you are timed out until <ts>`
    // so the desktop can render a countdown. A ban is normally enforced at the
    // auth seam, but an already-authenticated connection never re-auths: if the
    // live-disconnect fan-out is missed (fire-and-forget publish, broadcast lag,
    // subscriber reconnect window), a banned member's open socket would keep
    // writing indefinitely. So the ban is re-checked here — this write-path gate
    // is the durable backstop the fan-out's best-effort delivery relies on.
    // Moderation/relay-admin commands are exempt: a restriction must never
    // disarm the tools used to lift or manage it.
    //
    // Scope: this gate checks the *authoring* pubkey only, with no NIP-OA
    // owner→agent cascade. That cascade lives at the auth seam for bans, where
    // it is structural: an agent whose owner is banned can never authenticate,
    // so its socket never exists to reach ingest. Timeout has no auth-seam
    // presence (it is write-block-only), so an owner-timeout does not cascade to
    // the owner's agents — a deliberate Phase-1 asymmetry. `IngestAuth` does not
    // carry the self-proving auth tag, so resolving the owner here would mean
    // plumbing it through the whole transport boundary; the follow-up shape is
    // the restriction-state cache (see should-fix), which can fold in owner
    // resolution without a per-write DB round-trip.
    if !buzz_core::kind::is_moderation_command_kind(kind_u32) && !is_relay_admin_kind(kind_u32) {
        match state
            .db
            .moderation_restriction_state(tenant.community(), auth.pubkey().as_bytes())
            .await
        {
            Ok(r) => {
                if r.banned {
                    return Err(IngestError::AuthFailed(
                        "blocked: you are banned from this community".to_string(),
                    ));
                }
                if let Some(until) = r.muted_until {
                    if until > chrono::Utc::now() {
                        return Err(IngestError::AuthFailed(format!(
                            "restricted: you are timed out until {}",
                            until.timestamp()
                        )));
                    }
                }
            }
            Err(e) => {
                // Fail closed: a DB error must not let a banned/timed-out actor
                // write.
                return Err(IngestError::Internal(format!(
                    "error: internal error checking restriction state: {e}"
                )));
            }
        }
    }

    let mut channel_id = if kind_u32 == KIND_REACTION {
        match derive_reaction_channel(tenant.community(), &state.db, &event).await {
            ReactionChannelResult::Channel(ch_id) => Some(ch_id),
            ReactionChannelResult::NoChannel => None,
            ReactionChannelResult::NotFound => {
                return Err(IngestError::Rejected(
                    "invalid: reaction target event not found".into(),
                ));
            }
            ReactionChannelResult::NoTarget => {
                return Err(IngestError::Rejected(
                    "invalid: reaction must reference a target event via e tag".into(),
                ));
            }
            ReactionChannelResult::DbError(e) => {
                return Err(IngestError::Internal(format!(
                    "error: internal error looking up reaction target: {e}"
                )));
            }
        }
    } else if is_gift_wrap {
        None
    } else if kind_u32 == KIND_DELETION {
        // Standard deletion (kind:5): derive channel from the target event.
        // kind:5 events don't carry an h-tag, so we look up the target event
        // and use its channel_id. This ensures token-channel, membership, and
        // archived checks run against the correct channel.
        let target_hex = event.tags.iter().find_map(|t| {
            if t.kind().to_string() == "e" {
                t.content().and_then(|v| {
                    if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        });
        match target_hex {
            Some(hex) => {
                let target_bytes = hex::decode(&hex).map_err(|_| {
                    IngestError::Rejected("invalid: malformed deletion target id".into())
                })?;
                match state
                    .db
                    .get_event_by_id(tenant.community(), &target_bytes)
                    .await
                {
                    Ok(Some(target)) => target.channel_id,
                    Ok(None) => None, // target not found — validate_standard_deletion will catch this
                    Err(e) => {
                        return Err(IngestError::Internal(format!(
                            "error: looking up deletion target: {e}"
                        )));
                    }
                }
            }
            None => None, // no e-tag — will be caught by single-target enforcement (step 12)
        }
    } else {
        extract_channel_id(&event)
    };

    if is_global_only_kind(kind_u32) {
        channel_id = None;
    }

    if requires_h_channel_scope(kind_u32) && channel_id.is_none() {
        return Err(IngestError::Rejected(
            "invalid: channel-scoped events must include an h tag".into(),
        ));
    }

    if let Some(ch_id) = channel_id {
        check_token_channel_access(&auth, ch_id).map_err(IngestError::AuthFailed)?;
    } else if auth.channel_ids().is_some() {
        // Channel-scoped tokens cannot publish global events — that would bypass
        // the token's channel restriction. This covers kind:1 (global text notes),
        // kind:3 (contact lists), kind:0 (profiles), and kind:9007 (create-group
        // without an h-tag, which would auto-assign a server UUID).
        return Err(IngestError::AuthFailed(
            "restricted: channel-scoped tokens cannot publish global events".into(),
        ));
    }

    let pubkey_bytes = auth.pubkey().to_bytes().to_vec();
    // E1 (§4.8): fetch the community-scoped channel row once per request and
    // thread it through the gates below (membership open-fallback, archived
    // check, join visibility) instead of re-SELECTing it at each. `None` when
    // the event is global or the channel doesn't exist yet (kind:9007 creates
    // it later in this request); each gate keeps its existing missing-row
    // behavior.
    let channel_row = match channel_id {
        Some(ch_id) => state.db.get_channel(tenant.community(), ch_id).await.ok(),
        None => None,
    };
    // E1 phase-2 (§4.8 phase-2 addendum): resolve the fan-out visibility once,
    // here, through the same `channel_visibility_cached` gate fan-out uses
    // (fence 2: cached `private` wins over the prefetched row; a `private`
    // read still populates the cache). The value travels to fan-out bundled
    // with the (community, channel) it was resolved under (fence 3). When the
    // row is missing (global event, kind:9007 pre-create) this is `None` and
    // fan-out performs its own fresh fail-closed lookup — `None` is never
    // "assume open" (fence 1).
    let threaded_visibility = match (channel_id, &channel_row) {
        (Some(ch_id), Some(row)) => state
            .channel_visibility_cached(tenant.community(), ch_id, Some(row))
            .await
            .ok()
            .map(|visibility| crate::state::ThreadedChannelVisibility {
                community_id: tenant.community(),
                channel_id: ch_id,
                visibility,
            }),
        _ => None,
    };
    if let Some(ch_id) = channel_id {
        // kind:9021 (join) doesn't require prior membership.
        // kind:9007 (create) — channel doesn't exist yet; creator becomes owner in step 16.
        // kind:40003/9002/9005/9008 — per-kind validators are the authority; they
        // individually enforce authorization and fail closed. Bypassing the generic
        // member/open gate here lets the owning human act on private agent channels
        // without being a member (OQ1 decision; see validate_edit_ownership /
        // validate_admin_event for per-kind enforcement).
        let skip_membership = kind_u32 == KIND_NIP29_JOIN_REQUEST
            || kind_u32 == KIND_NIP29_CREATE_GROUP
            || kind_u32 == KIND_STREAM_MESSAGE_EDIT
            || kind_u32 == KIND_NIP29_EDIT_METADATA
            || kind_u32 == KIND_NIP29_DELETE_EVENT
            || kind_u32 == KIND_NIP29_DELETE_GROUP;
        if !skip_membership {
            // Spec AuthCheck (line 794): emit the verdict at the actual
            // call site. claimed_community comes from the event's h tag
            // (recorded separately to bite M2 / M8 — claim or A-host
            // driving a B-channel verdict — at the checker). The verdict
            // basis is `tenant.community()` server-resolved, confirmed
            // at `check_channel_membership`'s `is_member_cached(tenant
            // .community(), …)` call (see crates/buzz-relay/src/handlers
            // /ingest.rs:424).
            let auth_result =
                check_channel_membership(tenant, state, ch_id, &pubkey_bytes, channel_row.as_ref())
                    .await;
            let claimed = claimed_community_from_event(&event);
            let verdict = if auth_result.is_ok() {
                Verdict::Allow
            } else {
                Verdict::Deny
            };
            emit(
                tracer,
                TraceAction::AuthCheck {
                    channel: channel_label(ch_id),
                    claimed_community: claimed,
                    verdict,
                },
                state_for_request(tenant, auth.pubkey()),
            );
            auth_result.map_err(IngestError::Rejected)?;
        }
    }

    // Handled directly — these mutate relay_members and do NOT get stored.
    if is_relay_admin_kind(event.kind.as_u16() as u32) {
        crate::handlers::relay_admin::handle_relay_admin_event(tenant, state, &event)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: String::new(),
        });
    }

    // Handled directly — removes the sender from relay_members. NOT stored.
    if kind_u32 == KIND_NIP43_LEAVE_REQUEST {
        if !state.config.require_relay_membership {
            return Err(IngestError::Rejected(
                "invalid: relay membership is not enabled".into(),
            ));
        }

        // Freshness check: reject events outside ±120s of now (same as admin commands).
        {
            let event_ts = event.created_at.as_secs() as i64;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if (event_ts - now).abs() > 120 {
                return Err(IngestError::Rejected(format!(
                    "invalid: leave request timestamp out of range (delta={}s, max ±120s)",
                    event_ts - now
                )));
            }
        }

        // NIP-43 spec: "This event MUST include a NIP-70 `-` tag."
        let has_protected_tag = event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("-"));
        if !has_protected_tag {
            return Err(IngestError::Rejected(
                "invalid: leave request must include NIP-70 protected event tag [\"-\"]".into(),
            ));
        }

        let sender_hex = event.pubkey.to_hex();

        // remove_relay_member handles both the NotFound and IsOwner cases atomically.
        let remove_result = state
            .db
            .remove_relay_member(tenant.community(), &sender_hex)
            .await
            .map_err(|e| IngestError::Internal(format!("database error: {e}")))?;

        match remove_result {
            buzz_db::relay_members::RemoveResult::Removed => {}
            buzz_db::relay_members::RemoveResult::NotFound => {
                return Err(IngestError::Rejected(
                    "invalid: you are not a relay member".into(),
                ));
            }
            buzz_db::relay_members::RemoveResult::IsOwner => {
                return Err(IngestError::Rejected(
                    "invalid: relay owner cannot leave".into(),
                ));
            }
            buzz_db::relay_members::RemoveResult::RoleMismatch => {
                // remove_relay_member (no role filter) never returns RoleMismatch —
                // this arm is unreachable but exhaustiveness requires it.
                return Err(IngestError::Internal(
                    "unexpected RoleMismatch from remove_relay_member".into(),
                ));
            }
        }

        // Publish NIP-43 announcements — fire-and-forget.
        if let Err(e) =
            crate::handlers::side_effects::publish_nip43_member_removed(tenant, state, &sender_hex)
                .await
        {
            warn!(error = %e, "failed to publish NIP-43 member removed event");
        }
        if let Err(e) =
            crate::handlers::side_effects::publish_nip43_membership_list(tenant, state).await
        {
            warn!(error = %e, "failed to publish NIP-43 membership list");
        }

        info!(pubkey = %sender_hex, "relay member left via NIP-43 leave request");

        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: "info: you have left this relay".into(),
        });
    }

    if crate::handlers::side_effects::is_admin_kind(kind_u32) {
        crate::handlers::side_effects::validate_admin_event(tenant, kind_u32, &event, state)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    // Processed here (verify consent, mutate archived_identities, emit the
    // relay-signed 8002/8003 delta + 13535 snapshot), then — unlike the
    // NIP-43 admin commands above — the request itself falls through to normal
    // storage so the delta's `["e", request_id]` audit reference resolves.
    if is_identity_archive_request_kind(kind_u32) {
        crate::handlers::identity_archive::handle_identity_archive_event(tenant, state, &event)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_DELETION {
        crate::handlers::side_effects::validate_standard_deletion_event(tenant, &event, state)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if channel_id.is_some() {
        // Allow kind:9002 with archived=false (unarchive operation)
        let is_unarchive = kind_u32 == KIND_NIP29_EDIT_METADATA
            && event.tags.iter().any(|t| {
                let parts = t.as_slice();
                parts.len() >= 2 && parts[0] == "archived" && parts[1] == "false"
            });

        if !is_unarchive {
            if let Some(channel) = &channel_row {
                if channel.archived_at.is_some() {
                    return Err(IngestError::Rejected("invalid: channel is archived".into()));
                }
            }
        }
    }

    // NIP-09: kind:5 may reference targets via `e` tag (regular events) OR
    // `a` tag (addressable/parameterized-replaceable events like kind:30620).
    if kind_u32 == KIND_NIP29_DELETE_EVENT || kind_u32 == KIND_DELETION {
        let e_count = count_e_tags(&event);
        let a_count = event
            .tags
            .iter()
            .filter(|t| t.kind().to_string() == "a")
            .count();
        if (e_count + a_count) != 1 {
            return Err(IngestError::Rejected(format!(
                "invalid: deletion events must reference exactly one target via e or a tag (got e={e_count}, a={a_count})"
            )));
        }
    }

    if kind_u32 == KIND_STREAM_MESSAGE_EDIT {
        validate_edit_ownership(tenant.community(), &event, state)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_FORUM_VOTE {
        validate_forum_vote_target(tenant.community(), &event, state)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_STREAM_MESSAGE_DIFF {
        validate_diff_event(&event).map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_AGENT_ENGRAM {
        validate_engram_envelope(&event)
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_AGENT_TURN_METRIC {
        validate_agent_turn_metric_envelope(&event)
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;

        // Ownership check: `p` tag must be the registered owner of `event.pubkey`.
        // Tag shape is already verified above; these extractions are infallible.
        let owner_hex = event
            .tags
            .iter()
            .find_map(|t| {
                let parts = t.as_slice();
                if parts.len() >= 2 && parts[0].as_str() == "p" {
                    Some(parts[1].as_str())
                } else {
                    None
                }
            })
            .expect("p tag present (validated above)");
        let agent_bytes = event.pubkey.to_bytes().to_vec();
        let owner_bytes = hex::decode(owner_hex).expect("hex validated above");
        let is_owner = state
            .db
            .is_agent_owner(tenant.community(), &agent_bytes, &owner_bytes)
            .await
            .map_err(|e| {
                IngestError::Internal(format!(
                    "error: db error checking agent-turn-metric ownership: {e}"
                ))
            })?;
        if !is_owner {
            return Err(IngestError::AuthFailed(
                "restricted: agent-turn-metric `p` tag must be the registered owner of this agent"
                    .into(),
            ));
        }
    }

    if kind_u32 == KIND_EVENT_REMINDER {
        validate_event_reminder(&event)
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    if kind_u32 == KIND_PERSONA {
        validate_persona_envelope(&event)
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    // Track pre-created channel UUID for compensation on insert failure.
    let mut pre_created_channel: Option<Uuid> = None;

    if kind_u32 == KIND_NIP29_CREATE_GROUP {
        // Validate name tag is present and non-empty before any DB work.
        let create_name = event.tags.iter().find_map(|t| {
            if t.kind().to_string() == "name" {
                t.content().map(|s| s.to_string())
            } else {
                None
            }
        });
        if create_name
            .as_ref()
            .map(|n| n.trim().is_empty())
            .unwrap_or(true)
        {
            return Err(IngestError::Rejected(
                "invalid: channel name is required".into(),
            ));
        }

        // Validate visibility/channel_type for ALL kind:9007 events (with or without h-tag).
        // This runs pre-storage so invalid enums are rejected before the event is persisted.
        let visibility_str = event
            .tags
            .iter()
            .find_map(|t| {
                if t.kind().to_string() == "visibility" {
                    t.content().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "open".to_string());
        let channel_type_str = event
            .tags
            .iter()
            .find_map(|t| {
                if t.kind().to_string() == "channel_type" {
                    t.content().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "stream".to_string());

        let visibility: buzz_db::channel::ChannelVisibility = visibility_str
            .parse()
            .map_err(|_| IngestError::Rejected(format!("invalid visibility: {visibility_str}")))?;
        let channel_type: buzz_db::channel::ChannelType =
            channel_type_str.parse().map_err(|_| {
                IngestError::Rejected(format!("invalid channel_type: {channel_type_str}"))
            })?;

        if let Some(client_uuid) = channel_id {
            let name = create_name.unwrap_or_default();

            let description = event.tags.iter().find_map(|t| {
                if t.kind().to_string() == "about" {
                    t.content().map(|s| s.to_string())
                } else {
                    None
                }
            });

            let ttl_seconds = super::resolve_ttl(&event, state.config.ephemeral_ttl_override);

            let actor_bytes = event.pubkey.to_bytes().to_vec();
            let (_, was_created) = state
                .db
                .create_channel_with_id(
                    tenant.community(),
                    client_uuid,
                    &name,
                    channel_type,
                    visibility,
                    description.as_deref(),
                    &actor_bytes,
                    ttl_seconds,
                )
                .await
                .map_err(|e| IngestError::Internal(format!("error: {e}")))?;

            if !was_created {
                return Ok(IngestResult {
                    event_id: event_id_hex,
                    accepted: false,
                    message: "duplicate: channel already exists".into(),
                });
            }
            pre_created_channel = Some(client_uuid);
        }
    }

    if kind_u32 == KIND_NIP29_JOIN_REQUEST {
        // A join without an h-tag is meaningless — reject early.
        if channel_id.is_none() {
            return Err(IngestError::Rejected(
                "invalid: join request must include an h tag".into(),
            ));
        }
        if channel_id.is_some() {
            match &channel_row {
                Some(ch) if ch.visibility == "private" => {
                    return Err(IngestError::Rejected(
                        "restricted: channel is private".into(),
                    ));
                }
                None => {
                    return Err(IngestError::Rejected("invalid: channel not found".into()));
                }
                _ => {} // open — OK
            }
        }
    }

    let imeta_tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .filter(|t| t.kind().to_string() == "imeta")
        .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
        .collect();
    if !imeta_tags.is_empty() {
        let tenant_media_base =
            crate::api::media::media_base_url_for_tenant(&state.config.relay_url, tenant.host());
        crate::api::validate_imeta_tags(&imeta_tags, &tenant_media_base)
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
        crate::api::verify_imeta_blobs(tenant, &imeta_tags, &state.media_storage)
            .await
            .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    }

    let thread_meta = if requires_h_channel_scope(kind_u32) {
        if let Some(ch_id) = channel_id {
            resolve_nip10_thread_meta(tenant.community(), &event, ch_id, state)
                .await
                .map_err(|msg| IngestError::Rejected(format!("invalid: {msg}")))?
        } else {
            None
        }
    } else {
        None
    };

    // Pre-validate kind:0 content before storage so we don't store an event
    // whose profile sync will silently fail in the side-effect handler.
    if kind_u32 == KIND_PROFILE
        && serde_json::from_str::<serde_json::Value>(&event.content).is_err()
    {
        return Err(IngestError::Rejected(
            "invalid: kind:0 content must be valid JSON".into(),
        ));
    }

    // Resolve the target reference, then use one DB transaction to upsert the
    // reaction row (dedup via ON CONFLICT) with reaction_event_id already set and
    // store the kind:7 event. This replaces the post-storage side-effect handler.
    if kind_u32 == KIND_REACTION {
        // Extract target event hex from last e-tag (NIP-25).
        let target_hex = event
            .tags
            .iter()
            .rev()
            .find_map(|tag| {
                if tag.kind().to_string() == "e" {
                    tag.content().and_then(|v| {
                        if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                IngestError::Rejected(
                    "invalid: reaction must reference a target event via e tag".into(),
                )
            })?;

        let target_id = hex::decode(&target_hex)
            .map_err(|_| IngestError::Rejected("invalid: malformed reaction target id".into()))?;

        let actor_bytes = effective_message_author(&event, &state.relay_keypair.public_key());
        let emoji = if event.content.is_empty() {
            "+"
        } else {
            &event.content
        };

        // Mirror the SDK's 64-character emoji limit server-side so raw clients
        // cannot bypass it. Uses chars().count() (not byte len) to match the
        // SDK's check_emoji_len, which also counts Unicode characters.
        const MAX_REACTION_EMOJI_CHARS: usize = 64;
        let emoji_char_count = emoji.chars().count();
        if emoji_char_count > MAX_REACTION_EMOJI_CHARS {
            return Err(IngestError::Rejected(format!(
                "invalid: reaction emoji exceeds {} characters (got {})",
                MAX_REACTION_EMOJI_CHARS, emoji_char_count
            )));
        }

        // Atomically upsert the reaction row with this kind:7 event id, then store
        // the event in the same transaction. Ordering is load-bearing: active
        // duplicate reactions must return before storing a duplicate kind:7 event.
        let thread_params = thread_meta.as_ref().map(|m| m.as_params());
        let (stored_event, was_inserted) = match state
            .db
            .insert_reaction_event_with_thread_metadata(
                tenant.community(),
                &event,
                channel_id,
                thread_params,
                &target_id,
                &actor_bytes,
                emoji,
            )
            .await
            .map_err(|e| IngestError::Internal(format!("error: {e}")))?
        {
            buzz_db::ReactionEventInsertOutcome::TargetMissing => {
                return Err(IngestError::Rejected(
                    "invalid: reaction target event not found".into(),
                ));
            }
            buzz_db::ReactionEventInsertOutcome::Duplicate => {
                return Ok(IngestResult {
                    event_id: event_id_hex,
                    accepted: false,
                    message: "duplicate: reaction already exists".into(),
                });
            }
            buzz_db::ReactionEventInsertOutcome::Inserted {
                stored_event,
                was_inserted,
            } => (stored_event, was_inserted),
        };

        let pubkey_hex = auth.pubkey().to_hex();
        // Spec WriteInsert (line 514) / WriteDuplicate (line 606): emit
        // the abstract write action. The persist API returns
        // `was_inserted` (true → Insert, false → Duplicate). This branch
        // is the reaction path; channel_id is always Some here, so
        // WriteInsertGlobal does not apply.
        let claimed = claimed_community_from_event(&event);
        let action = if was_inserted {
            TraceAction::WriteInsert {
                msg_id: msg_id_label(event.id.as_bytes()),
                channel: channel_label(channel_id.expect("reaction path has channel")),
                claimed_community: claimed,
            }
        } else {
            TraceAction::WriteDuplicate {
                msg_id: msg_id_label(event.id.as_bytes()),
                channel: channel_label(channel_id.expect("reaction path has channel")),
                claimed_community: claimed,
            }
        };
        emit(tracer, action, state_for_request(tenant, auth.pubkey()));
        dispatch_persistent_event(
            tenant,
            state,
            &stored_event,
            kind_u32,
            &pubkey_hex,
            threaded_visibility.clone(),
        )
        .await;

        info!(event_id = %event_id_hex, kind = kind_u32, "Event ingested via pipeline");
        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: String::new(),
        });
    }

    let (stored_event, was_inserted) = if buzz_core::kind::is_replaceable(kind_u32) {
        // NIP-16 replaceable event — atomic replace with stale-write protection.
        // channel_id is None for global kinds (0, 1, 3) due to step 5b above.
        state
            .db
            .replace_addressable_event(tenant.community(), &event, channel_id)
            .await
            .map_err(|e| IngestError::Internal(format!("error: {e}")))?
    } else if is_parameterized_replaceable(kind_u32) {
        // NIP-33 parameterized replaceable — keyed by (kind, pubkey, d_tag).
        let d_tag = buzz_db::event::extract_d_tag(&event).unwrap_or_default();
        if d_tag.len() > buzz_db::event::D_TAG_MAX_LEN {
            return Err(IngestError::Rejected(format!(
                "invalid: d tag too long ({} bytes, max {})",
                d_tag.len(),
                buzz_db::event::D_TAG_MAX_LEN,
            )));
        }
        state
            .db
            .replace_parameterized_event(tenant.community(), &event, &d_tag, channel_id)
            .await
            .map_err(|e| IngestError::Internal(format!("error: {e}")))?
    } else {
        let thread_params = thread_meta.as_ref().map(|m| m.as_params());
        match state
            .db
            .insert_event_with_thread_metadata(
                tenant.community(),
                &event,
                channel_id,
                thread_params,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                // Compensate: if we pre-created a channel for kind:9007,
                // soft-delete it so no orphaned channel row remains.
                if let Some(ch_id) = pre_created_channel {
                    if let Err(re) = state
                        .db
                        .soft_delete_channel(tenant.community(), ch_id)
                        .await
                    {
                        warn!(event_id = %event_id_hex, "channel compensation failed: {re}");
                    }
                    state.invalidate_channel_deleted(tenant);
                }
                return Err(match e {
                    buzz_db::DbError::AuthEventRejected => {
                        IngestError::Rejected("invalid: AUTH events cannot be stored".into())
                    }
                    other => IngestError::Internal(format!("error: database error: {other}")),
                });
            }
        }
    };

    if !was_inserted {
        return Ok(IngestResult {
            event_id: event_id_hex,
            accepted: true,
            message: "duplicate:".into(),
        });
    }

    // Any successfully stored channel-scoped event keeps the channel alive.
    // Skip kind:9007 (create) — the deadline was just set during creation.
    if let Some(ch_id) = channel_id {
        if kind_u32 != KIND_NIP29_CREATE_GROUP {
            if let Err(e) = state.db.bump_ttl_deadline(tenant.community(), ch_id).await {
                warn!(channel = %ch_id, "TTL deadline bump failed: {e}");
            }
        }
    }

    if crate::handlers::side_effects::is_side_effect_kind(kind_u32) {
        if let Err(e) =
            crate::handlers::side_effects::handle_side_effects(tenant, kind_u32, &event, state)
                .await
        {
            warn!(event_id = %event_id_hex, kind = kind_u32, "Side effect failed: {e}");
        }
    }

    // A freshly inserted reply changed its thread's counters (updated in the
    // same transaction as the insert) — push a fresh relay-signed 39005 so
    // subscribed clients can update badge counts without refetching the head
    // window. Page responses recompute summaries independently, so this is
    // fan-out-only and best-effort.
    if let Some(meta) = &thread_meta {
        crate::handlers::side_effects::emit_live_thread_summary(
            tenant,
            state,
            meta.channel_id,
            meta.root_event_id.clone(),
        );
    }

    let pubkey_hex = auth.pubkey().to_hex();
    // Spec WriteInsert (line 514) / WriteInsertGlobal (line 559) /
    // WriteDuplicate (line 606): emit the abstract write at the trailing
    // dispatch site. `channel_id.is_some()` distinguishes channel-bearing
    // (Insert/Duplicate) from channel-less (InsertGlobal); `was_inserted`
    // distinguishes accepted-new (Insert/Global) from no-op-on-conflict
    // (Duplicate). The WriteInsertGlobal duplicate case is not modeled
    // separately in the spec (channel-less duplicates collapse to the
    // same observation shape as channel-less inserts at this seam);
    // see docs/spec/MultiTenantRelay.tla lines 559-595.
    {
        let claimed = claimed_community_from_event(&event);
        let action = match (channel_id, was_inserted) {
            (Some(ch), true) => TraceAction::WriteInsert {
                msg_id: msg_id_label(event.id.as_bytes()),
                channel: channel_label(ch),
                claimed_community: claimed,
            },
            (Some(ch), false) => TraceAction::WriteDuplicate {
                msg_id: msg_id_label(event.id.as_bytes()),
                channel: channel_label(ch),
                claimed_community: claimed,
            },
            (None, _) => TraceAction::WriteInsertGlobal {
                msg_id: msg_id_label(event.id.as_bytes()),
                claimed_community: claimed,
            },
        };
        emit(tracer, action, state_for_request(tenant, auth.pubkey()));
    }
    dispatch_persistent_event(
        tenant,
        state,
        &stored_event,
        kind_u32,
        &pubkey_hex,
        threaded_visibility.clone(),
    )
    .await;

    info!(event_id = %event_id_hex, kind = kind_u32, "Event ingested via pipeline");

    Ok(IngestResult {
        event_id: event_id_hex,
        accepted: true,
        message: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::{
        KIND_CANVAS, KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_FORUM_VOTE, KIND_LONG_FORM,
        KIND_MANAGED_AGENT, KIND_PERSONA, KIND_PRESENCE_UPDATE, KIND_STREAM_MESSAGE,
        KIND_STREAM_MESSAGE_DIFF, KIND_TEAM, KIND_USER_STATUS,
    };

    #[test]
    fn nip_ia_requests_are_global_only() {
        // NIP-IA requests drive relay-global archive state; a stray `h` tag
        // must not channel-scope them, or the global audit trail breaks.
        for kind in [KIND_IA_ARCHIVE_REQUEST, KIND_IA_UNARCHIVE_REQUEST] {
            assert!(is_global_only_kind(kind), "kind {kind} must be global-only");
            assert!(
                !requires_h_channel_scope(kind),
                "kind {kind} must not require an h tag"
            );
        }
    }

    #[test]
    fn channel_scoped_content_kinds_require_h_tags() {
        for kind in [
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_DIFF,
            KIND_CANVAS,
            KIND_FORUM_POST,
            KIND_FORUM_VOTE,
            KIND_FORUM_COMMENT,
        ] {
            assert!(
                requires_h_channel_scope(kind),
                "kind {kind} should require h"
            );
        }
    }

    #[test]
    fn nip29_admin_kinds_require_h_tags() {
        for kind in [
            KIND_NIP29_PUT_USER,
            KIND_NIP29_REMOVE_USER,
            KIND_NIP29_EDIT_METADATA,
            KIND_NIP29_DELETE_EVENT,
            KIND_NIP29_DELETE_GROUP,
            KIND_NIP29_LEAVE_REQUEST,
        ] {
            assert!(
                requires_h_channel_scope(kind),
                "kind {kind} should require h"
            );
        }
    }

    #[test]
    fn create_group_does_not_require_h_tag() {
        // kind:9007 creates the channel — h-tag is optional (client-chosen UUID)
        assert!(!requires_h_channel_scope(KIND_NIP29_CREATE_GROUP));
    }

    #[test]
    fn join_request_does_not_require_h_tag_via_requires_h() {
        // kind:9021 uses h-tag for channel reference but doesn't go through
        // requires_h_channel_scope — it's handled separately in the pipeline
        // because it needs special "open-only" validation
        assert!(!requires_h_channel_scope(KIND_NIP29_JOIN_REQUEST));
    }

    #[test]
    fn reactions_do_not_require_h_tag() {
        assert!(!requires_h_channel_scope(KIND_REACTION));
    }

    #[test]
    fn long_form_is_in_scope_allowlist() {
        let dummy = make_dummy_event();
        assert!(
            required_scope_for_kind(KIND_LONG_FORM, &dummy).is_ok(),
            "KIND_LONG_FORM (30023) should be accepted"
        );
    }

    #[test]
    fn long_form_requires_messages_write_scope() {
        let dummy = make_dummy_event();
        assert_eq!(
            required_scope_for_kind(KIND_LONG_FORM, &dummy).unwrap(),
            Scope::MessagesWrite,
        );
    }

    #[test]
    fn long_form_does_not_require_h_tag() {
        // kind:30023 is global (author-owned, not channel-scoped)
        assert!(!requires_h_channel_scope(KIND_LONG_FORM));
    }

    #[test]
    fn long_form_is_global_only() {
        // kind:30023 is always global — ingest nulls channel_id even if an h-tag is present
        assert!(is_global_only_kind(KIND_LONG_FORM));
    }

    #[test]
    fn user_status_requires_users_write_scope() {
        let dummy = make_dummy_event();
        assert_eq!(
            required_scope_for_kind(KIND_USER_STATUS, &dummy).unwrap(),
            Scope::UsersWrite,
        );
    }

    #[test]
    fn user_status_is_global_only() {
        assert!(is_global_only_kind(KIND_USER_STATUS));
    }

    #[test]
    fn user_status_does_not_require_h_tag() {
        assert!(!requires_h_channel_scope(KIND_USER_STATUS));
    }

    #[test]
    fn reports_and_moderation_commands_require_messages_write_scope() {
        let dummy = make_dummy_event();
        for kind in [
            KIND_REPORT,
            KIND_MODERATION_BAN,
            KIND_MODERATION_UNBAN,
            KIND_MODERATION_TIMEOUT,
            KIND_MODERATION_UNTIMEOUT,
            KIND_MODERATION_RESOLVE_REPORT,
        ] {
            assert_eq!(
                required_scope_for_kind(kind, &dummy).unwrap(),
                Scope::MessagesWrite,
                "kind {kind} should require MessagesWrite scope"
            );
        }
    }

    #[test]
    fn moderation_commands_are_global_only() {
        for kind in [
            KIND_MODERATION_BAN,
            KIND_MODERATION_UNBAN,
            KIND_MODERATION_TIMEOUT,
            KIND_MODERATION_UNTIMEOUT,
            KIND_MODERATION_RESOLVE_REPORT,
        ] {
            assert!(is_global_only_kind(kind), "kind {kind} must be global-only");
            assert!(
                !requires_h_channel_scope(kind),
                "kind {kind} must not require an h tag"
            );
        }
    }

    #[test]
    fn moderation_command_rejection_from_ingest_preserves_prefix() {
        let rejection = "restricted: moderator access required".to_string();
        let map_rejection = IngestError::Rejected;
        let result: Result<(), String> = Err(rejection.clone());

        match result.map_err(map_rejection).unwrap_err() {
            IngestError::Rejected(message) => assert_eq!(message, rejection),
            _ => panic!("expected rejected ingest error"),
        }
    }

    #[test]
    fn global_only_and_channel_scoped_are_disjoint() {
        // A kind cannot be both global-only and channel-scoped
        for kind in 0..=65535u32 {
            assert!(
                !(is_global_only_kind(kind) && requires_h_channel_scope(kind)),
                "kind {kind} is both global-only and channel-scoped"
            );
        }
    }

    #[test]
    fn ephemeral_kinds_not_in_scope_allowlist() {
        assert!(required_scope_for_kind(KIND_PRESENCE_UPDATE, &make_dummy_event()).is_err());
    }

    #[test]
    fn per_kind_scope_allowlist_covers_all_migrated_kinds() {
        let dummy = make_dummy_event();
        let migrated = [
            KIND_PROFILE,
            KIND_DELETION,
            KIND_REACTION,
            KIND_REPORT,
            KIND_MODERATION_BAN,
            KIND_MODERATION_UNBAN,
            KIND_MODERATION_TIMEOUT,
            KIND_MODERATION_UNTIMEOUT,
            KIND_MODERATION_RESOLVE_REPORT,
            KIND_STREAM_MESSAGE,
            KIND_NIP29_PUT_USER,
            KIND_NIP29_REMOVE_USER,
            KIND_NIP29_EDIT_METADATA,
            KIND_NIP29_DELETE_EVENT,
            KIND_NIP29_CREATE_GROUP,
            KIND_NIP29_DELETE_GROUP,
            KIND_NIP29_JOIN_REQUEST,
            KIND_NIP29_LEAVE_REQUEST,
            KIND_STREAM_MESSAGE_EDIT,
            KIND_STREAM_MESSAGE_DIFF,
            KIND_CANVAS,
            KIND_FORUM_POST,
            KIND_FORUM_VOTE,
            KIND_FORUM_COMMENT,
            KIND_LONG_FORM,
            KIND_USER_STATUS,
            // NIP-51 lists + sets, NIP-65 relay list
            KIND_MUTE_LIST,
            KIND_PIN_LIST,
            KIND_NIP65_RELAY_LIST_METADATA,
            KIND_BOOKMARK_LIST,
            KIND_FOLLOW_SET,
            KIND_BOOKMARK_SET,
            KIND_EMOJI_SET,
            KIND_EMOJI_LIST,
            KIND_AGENT_ENGRAM,
            KIND_AGENT_PROFILE,
            KIND_PERSONA,
            KIND_TEAM,
            KIND_MANAGED_AGENT,
            KIND_AGENT_TURN_METRIC,
        ];
        for kind in migrated {
            assert!(
                required_scope_for_kind(kind, &dummy).is_ok(),
                "kind {kind} should be in the allowlist"
            );
        }
    }

    #[test]
    fn nip51_and_nip65_lists_require_users_write() {
        let dummy = make_dummy_event();
        for kind in [
            KIND_MUTE_LIST,
            KIND_PIN_LIST,
            KIND_NIP65_RELAY_LIST_METADATA,
            KIND_BOOKMARK_LIST,
            KIND_FOLLOW_SET,
            KIND_BOOKMARK_SET,
        ] {
            assert_eq!(
                required_scope_for_kind(kind, &dummy).ok(),
                Some(Scope::UsersWrite),
                "kind {kind} should require UsersWrite scope"
            );
        }
    }

    #[test]
    fn mesh_llm_relay_status_is_global_only_and_relay_only() {
        assert!(is_global_only_kind(KIND_MESH_LLM_RELAY_STATUS));
        assert!(buzz_core::kind::is_relay_only_kind(
            KIND_MESH_LLM_RELAY_STATUS
        ));
        assert!(!requires_h_channel_scope(KIND_MESH_LLM_RELAY_STATUS));
    }

    #[test]
    fn agent_turn_metric_is_global_only_and_in_scope_allowlist() {
        let dummy = make_dummy_event();
        assert!(
            is_global_only_kind(KIND_AGENT_TURN_METRIC),
            "kind:44200 must be global-only (no h tag)"
        );
        assert!(
            !requires_h_channel_scope(KIND_AGENT_TURN_METRIC),
            "kind:44200 must not require an h-tag"
        );
        assert_eq!(
            required_scope_for_kind(KIND_AGENT_TURN_METRIC, &dummy).unwrap(),
            Scope::MessagesWrite,
            "kind:44200 requires MessagesWrite scope"
        );
    }

    #[test]
    fn nip51_and_nip65_lists_are_global_only() {
        for kind in [
            KIND_MUTE_LIST,
            KIND_PIN_LIST,
            KIND_NIP65_RELAY_LIST_METADATA,
            KIND_BOOKMARK_LIST,
            KIND_FOLLOW_SET,
            KIND_BOOKMARK_SET,
        ] {
            assert!(
                is_global_only_kind(kind),
                "kind {kind} should be global-only (never channel-scoped)"
            );
            assert!(
                !requires_h_channel_scope(kind),
                "kind {kind} must not require an h-tag channel scope"
            );
        }
    }

    #[test]
    fn persona_is_in_scope_allowlist() {
        let dummy = make_dummy_event();
        assert_eq!(
            required_scope_for_kind(KIND_PERSONA, &dummy).unwrap(),
            Scope::UsersWrite,
        );
    }

    #[test]
    fn persona_is_global_only() {
        assert!(is_global_only_kind(KIND_PERSONA));
        assert!(!requires_h_channel_scope(KIND_PERSONA));
    }

    #[test]
    fn team_and_managed_agent_are_in_scope_allowlist() {
        let dummy = make_dummy_event();
        for kind in [KIND_TEAM, KIND_MANAGED_AGENT] {
            assert_eq!(
                required_scope_for_kind(kind, &dummy).unwrap(),
                Scope::UsersWrite,
                "kind {kind} should require UsersWrite scope"
            );
        }
    }

    #[test]
    fn team_and_managed_agent_are_global_only() {
        for kind in [KIND_TEAM, KIND_MANAGED_AGENT] {
            assert!(
                is_global_only_kind(kind),
                "kind {kind} should be global-only (never channel-scoped)"
            );
            assert!(
                !requires_h_channel_scope(kind),
                "kind {kind} must not require an h-tag channel scope"
            );
        }
    }

    #[test]
    fn unknown_kind_rejected() {
        let dummy = make_dummy_event();
        assert!(required_scope_for_kind(99999, &dummy).is_err());
    }

    #[test]
    fn gift_wrap_is_in_scope_allowlist() {
        // KIND_GIFT_WRAP is still in the per-kind scope allowlist.
        // The HTTP block is transport-level (is_http gate), not scope-level.
        let dummy = make_dummy_event();
        assert!(
            required_scope_for_kind(KIND_GIFT_WRAP, &dummy).is_ok(),
            "KIND_GIFT_WRAP should be in the scope allowlist"
        );
    }

    #[test]
    fn ingest_auth_is_http_returns_true_for_http_variant() {
        use crate::handlers::ingest::{HttpAuthMethod, IngestAuth};
        let keys = nostr::Keys::generate();
        let http_auth = IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: vec![],
            auth_method: HttpAuthMethod::Nip98,
        };
        assert!(
            http_auth.is_http(),
            "Http variant should return true for is_http()"
        );
    }

    #[test]
    fn ingest_auth_is_http_returns_false_for_nip42_variant() {
        use crate::handlers::ingest::IngestAuth;
        let keys = nostr::Keys::generate();
        let ws_auth = IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes: vec![],
            channel_ids: None,
            conn_id: uuid::Uuid::new_v4(),
        };
        assert!(
            !ws_auth.is_http(),
            "Nip42 variant should return false for is_http()"
        );
    }

    #[test]
    fn presence_update_not_in_scope_allowlist() {
        // KIND_PRESENCE_UPDATE is ephemeral — not in the allowlist regardless of transport.
        let dummy = make_dummy_event();
        assert!(
            required_scope_for_kind(KIND_PRESENCE_UPDATE, &dummy).is_err(),
            "KIND_PRESENCE_UPDATE should not be in the scope allowlist"
        );
    }

    #[test]
    fn diff_validation_rejects_missing_repo() {
        let event = make_event_with_tags(
            KIND_STREAM_MESSAGE_DIFF,
            "diff content",
            &[&["commit", "abc1234"]],
        );
        assert!(validate_diff_event(&event).is_err());
    }

    #[test]
    fn diff_validation_rejects_missing_commit() {
        let event = make_event_with_tags(
            KIND_STREAM_MESSAGE_DIFF,
            "diff content",
            &[&["repo", "https://github.com/example/repo"]],
        );
        assert!(validate_diff_event(&event).is_err());
    }

    #[test]
    fn diff_validation_accepts_valid() {
        let event = make_event_with_tags(
            KIND_STREAM_MESSAGE_DIFF,
            "diff content",
            &[
                &["repo", "https://github.com/example/repo"],
                &["commit", "abc1234"],
            ],
        );
        assert!(validate_diff_event(&event).is_ok());
    }

    #[test]
    fn diff_validation_rejects_oversized_content() {
        let big = "x".repeat(61_441);
        let event = make_event_with_tags(
            KIND_STREAM_MESSAGE_DIFF,
            &big,
            &[
                &["repo", "https://github.com/example/repo"],
                &["commit", "abc1234"],
            ],
        );
        assert!(validate_diff_event(&event).is_err());
    }

    fn make_dummy_event() -> Event {
        let keys = nostr::Keys::generate();
        nostr::EventBuilder::new(nostr::Kind::Custom(9), "")
            .tags([])
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn make_event_with_tags(kind: u32, content: &str, tags: &[&[&str]]) -> Event {
        let keys = nostr::Keys::generate();
        let nostr_tags: Vec<nostr::Tag> = tags
            .iter()
            .map(|t| nostr::Tag::parse(t.iter().copied()).unwrap())
            .collect();
        nostr::EventBuilder::new(nostr::Kind::Custom(kind as u16), content)
            .tags(nostr_tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn count_e_tags_includes_malformed() {
        // A deletion event with one valid e-tag and one malformed e-tag
        // should count as 2 e-tags (and be rejected by the "exactly 1" check).
        let event = make_event_with_tags(
            5, // kind:5 deletion
            "",
            &[&["e", "a".repeat(64).as_str()], &["e", "not-valid-hex"]],
        );
        assert_eq!(count_e_tags(&event), 2);
    }

    #[test]
    fn count_e_tags_single_valid() {
        let event = make_event_with_tags(5, "", &[&["e", "a".repeat(64).as_str()]]);
        assert_eq!(count_e_tags(&event), 1);
    }

    fn make_engram(tags: &[&[&str]], content: &str) -> Event {
        make_event_with_tags(KIND_AGENT_ENGRAM, content, tags)
    }

    /// Minimal syntactically-plausible NIP-44 v2 payload (99 zero-filled bytes
    /// with the 0x02 version prefix). Real ciphertexts are larger and have real
    /// MACs; the relay only checks shape, not authenticity.
    fn fake_nip44_v2() -> String {
        // base64(b"\x02" + b"\x00" * 98) — 132 chars, decoded length 99,
        // first byte 0x02.
        let mut s = String::from("Ag");
        s.push_str(&"A".repeat(130));
        s
    }

    #[test]
    fn engram_envelope_accepts_canonical() {
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], &fake_nip44_v2());
        assert!(validate_engram_envelope(&ev).is_ok());
    }

    #[test]
    fn engram_envelope_rejects_missing_p() {
        let d = "a".repeat(64);
        let ev = make_engram(&[&["d", &d]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`p` tag"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_duplicate_p() {
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p], &["p", &p]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`p` tag"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_short_d() {
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", "abcd"], &["p", &p]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_uppercase_d() {
        let p = "b".repeat(64);
        // 64 chars but uppercase — spec mandates lowercase hex.
        let d = "A".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    /// Regression: uppercase `p` tag must be rejected at ingest. Readers query
    /// `#p` lowercase; an uppercase-tagged event that wins NIP-33 replacement
    /// becomes invisible to readers, silently bricking the slug.
    #[test]
    fn engram_envelope_rejects_uppercase_p() {
        let d = "a".repeat(64);
        let p = "B".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`p` tag"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_short_p() {
        let d = "a".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", "abcd"]], &fake_nip44_v2());
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("`p` tag"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_empty_content() {
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], "");
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("content"), "got: {err}");
    }

    /// Regression: non-base64 content must be rejected. Otherwise a signed
    /// event with `content="x"` wins NIP-33 replacement against a valid head,
    /// and the new head is then skipped by `validate_and_decrypt` — making the
    /// slug appear absent to readers.
    #[test]
    fn engram_envelope_rejects_non_base64_content() {
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], "x");
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(
            err.contains("base64") || err.contains("too short"),
            "got: {err}"
        );
    }

    #[test]
    fn engram_envelope_rejects_wrong_nip44_version() {
        // 99 bytes of valid base64 alphabet, but first byte decodes to 0x00,
        // not the NIP-44 v2 prefix 0x02. Length OK (132 chars / 99 decoded).
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let bad = "A".repeat(132);
        let ev = make_engram(&[&["d", &d], &["p", &p]], &bad);
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(
            err.contains("NIP-44 v2") || err.contains("0x02"),
            "got: {err}"
        );
    }

    #[test]
    fn engram_envelope_rejects_short_content() {
        // Base64 of "Ag==" decodes to 1 byte — version prefix correct but
        // way under the 99-byte floor.
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        let ev = make_engram(&[&["d", &d], &["p", &p]], "Ag==");
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("too short"), "got: {err}");
    }

    #[test]
    fn engram_envelope_rejects_bad_base64_alphabet() {
        let d = "a".repeat(64);
        let p = "b".repeat(64);
        // Contains '!' which is not in the standard base64 alphabet. Length is
        // a multiple of 4 to defeat the length check.
        let bad = format!("Ag!!{}", "A".repeat(128));
        let ev = make_engram(&[&["d", &d], &["p", &p]], &bad);
        let err = validate_engram_envelope(&ev).unwrap_err();
        assert!(err.contains("base64"), "got: {err}");
    }

    #[test]
    fn not_before_accepts_zero() {
        assert_eq!(validate_not_before("0"), Ok(0));
    }

    #[test]
    fn not_before_accepts_typical_timestamp() {
        assert_eq!(validate_not_before("1717000000"), Ok(1_717_000_000));
    }

    #[test]
    fn not_before_accepts_max_safe_integer() {
        assert_eq!(
            validate_not_before("9007199254740991"),
            Ok(9_007_199_254_740_991)
        );
    }

    #[test]
    fn not_before_rejects_above_max_safe_integer() {
        assert_eq!(
            validate_not_before("9007199254740992"),
            Err("malformed not_before")
        );
    }

    #[test]
    fn not_before_rejects_leading_zero() {
        assert_eq!(validate_not_before("007"), Err("malformed not_before"));
    }

    #[test]
    fn not_before_rejects_empty() {
        assert_eq!(validate_not_before(""), Err("malformed not_before"));
    }

    #[test]
    fn not_before_rejects_non_digits() {
        // Sign, whitespace, decimal point, and non-decimal forms are all
        // rejected — only ASCII decimal digits are valid.
        for value in ["-1", "+1", " 1", "1 ", "1.0", "1e3", "0x10", "abc"] {
            assert_eq!(
                validate_not_before(value),
                Err("malformed not_before"),
                "value {value:?} should be malformed"
            );
        }
    }

    #[test]
    fn not_before_rejects_u64_overflow() {
        // Exceeds u64::MAX — `from_str` errors rather than wrapping, so the
        // value is malformed (not a lossy round-trip).
        assert_eq!(
            validate_not_before("99999999999999999999999999"),
            Err("malformed not_before")
        );
    }

    fn make_reminder(tags: &[&[&str]]) -> Event {
        make_event_with_tags(KIND_EVENT_REMINDER, "ciphertext", tags)
    }

    #[test]
    fn reminder_accepts_single_valid_not_before() {
        let ev = make_reminder(&[&["d", "abc"], &["not_before", "1717000000"]]);
        assert!(validate_event_reminder(&ev).is_ok());
    }

    #[test]
    fn reminder_accepts_expiration_after_not_before() {
        let ev = make_reminder(&[
            &["d", "abc"],
            &["not_before", "1717000000"],
            &["expiration", "1717000001"],
        ]);
        assert!(validate_event_reminder(&ev).is_ok());
    }

    #[test]
    fn reminder_accepts_missing_not_before() {
        // Terminal states (done/cancelled) and bookmarks omit not_before
        let ev = make_reminder(&[&["d", "abc"]]);
        assert!(validate_event_reminder(&ev).is_ok());
    }

    #[test]
    fn reminder_rejects_not_before_too_far_in_future() {
        // `not_before` beyond the max horizon (default 1 year) is rejected.
        let far_future = (chrono::Utc::now().timestamp() as u64) + 63_072_000; // ~2 years
        let ev = make_reminder(&[&["d", "abc"], &["not_before", &far_future.to_string()]]);
        assert_eq!(
            validate_event_reminder(&ev),
            Err("not_before too far in future")
        );
    }

    #[test]
    fn reminder_rejects_duplicate_not_before() {
        let ev = make_reminder(&[
            &["d", "abc"],
            &["not_before", "1717000000"],
            &["not_before", "1717000005"],
        ]);
        assert_eq!(validate_event_reminder(&ev), Err("malformed not_before"));
    }

    #[test]
    fn reminder_rejects_malformed_not_before() {
        let ev = make_reminder(&[&["d", "abc"], &["not_before", "007"]]);
        assert_eq!(validate_event_reminder(&ev), Err("malformed not_before"));
    }

    #[test]
    fn reminder_rejects_expiration_equal_to_not_before() {
        let ev = make_reminder(&[
            &["d", "abc"],
            &["not_before", "1717000000"],
            &["expiration", "1717000000"],
        ]);
        assert_eq!(
            validate_event_reminder(&ev),
            Err("expiration before not_before")
        );
    }

    #[test]
    fn reminder_rejects_expiration_before_not_before() {
        let ev = make_reminder(&[
            &["d", "abc"],
            &["not_before", "1717000000"],
            &["expiration", "1716000000"],
        ]);
        assert_eq!(
            validate_event_reminder(&ev),
            Err("expiration before not_before")
        );
    }

    #[test]
    fn reminder_ignores_malformed_expiration() {
        // A malformed `expiration` is NIP-40's concern, not this validator's:
        // the ordering check runs only when `expiration` parses, so a valid
        // `not_before` with an unparseable expiration is accepted here.
        let ev = make_reminder(&[
            &["d", "abc"],
            &["not_before", "1717000000"],
            &["expiration", "notanumber"],
        ]);
        assert!(validate_event_reminder(&ev).is_ok());
    }

    #[test]
    fn event_reminder_is_global_only_and_param_replaceable() {
        assert!(is_global_only_kind(KIND_EVENT_REMINDER));
        assert!(!requires_h_channel_scope(KIND_EVENT_REMINDER));
        assert!(is_parameterized_replaceable(KIND_EVENT_REMINDER));
    }

    #[test]
    fn reminder_accepts_expiration_without_not_before() {
        // A terminal/bookmark with expiration but no not_before is valid —
        // no ordering check applies when not_before is absent.
        let ev = make_reminder(&[&["d", "abc"], &["expiration", "1777542730"]]);
        assert!(validate_event_reminder(&ev).is_ok());
    }

    #[test]
    fn reminder_rejects_missing_d_tag() {
        let ev = make_event_with_tags(
            KIND_EVENT_REMINDER,
            "ciphertext",
            &[&["not_before", "1717000000"]],
        );
        assert_eq!(validate_event_reminder(&ev), Err("missing d tag"));
    }

    #[test]
    fn reminder_rejects_empty_d_tag() {
        let ev = make_event_with_tags(
            KIND_EVENT_REMINDER,
            "ciphertext",
            &[&["d", ""], &["not_before", "1717000000"]],
        );
        assert_eq!(validate_event_reminder(&ev), Err("empty d tag"));
    }

    #[test]
    fn reminder_rejects_duplicate_d_tag() {
        let ev = make_event_with_tags(
            KIND_EVENT_REMINDER,
            "ciphertext",
            &[&["d", "abc"], &["d", "def"], &["not_before", "1717000000"]],
        );
        assert_eq!(validate_event_reminder(&ev), Err("duplicate d tag"));
    }

    fn make_persona(tags: &[&[&str]]) -> Event {
        make_event_with_tags(
            KIND_PERSONA,
            r#"{"display_name":"x","system_prompt":"y"}"#,
            tags,
        )
    }

    #[test]
    fn persona_envelope_accepts_valid_slug() {
        let ev = make_persona(&[&["d", "my-persona-1"]]);
        assert!(validate_persona_envelope(&ev).is_ok());
    }

    #[test]
    fn persona_envelope_accepts_promptless_content() {
        // Unified agent model: system_prompt is optional — a definition can be
        // pure configuration. The relay validates only the envelope, so a
        // prompt-less body must ingest identically to a full one.
        let ev = make_event_with_tags(
            KIND_PERSONA,
            r#"{"display_name":"config-only"}"#,
            &[&["d", "config-only"]],
        );
        assert!(validate_persona_envelope(&ev).is_ok());
    }

    #[test]
    fn persona_envelope_accepts_behavioral_fields() {
        // Widened content (respond_to / mcp_toolsets / parallelism) is opaque
        // to the relay — unknown-field tolerance is the contract.
        let ev = make_event_with_tags(
            KIND_PERSONA,
            r#"{"display_name":"x","respond_to":"owner-only","respond_to_allowlist":[],"mcp_toolsets":"default","parallelism":2}"#,
            &[&["d", "behavioral"]],
        );
        assert!(validate_persona_envelope(&ev).is_ok());
    }

    #[test]
    fn persona_envelope_accepts_single_char() {
        let ev = make_persona(&[&["d", "a"]]);
        assert!(validate_persona_envelope(&ev).is_ok());
    }

    #[test]
    fn persona_envelope_accepts_max_length() {
        let slug = "a".repeat(64);
        let ev = make_persona(&[&["d", &slug]]);
        assert!(validate_persona_envelope(&ev).is_ok());
    }

    #[test]
    fn persona_envelope_rejects_missing_d_tag() {
        let ev = make_persona(&[]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_empty_d_tag() {
        let ev = make_persona(&[&["d", ""]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_duplicate_d_tags() {
        let ev = make_persona(&[&["d", "slug-a"], &["d", "slug-b"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_too_long() {
        let slug = "a".repeat(65);
        let ev = make_persona(&[&["d", &slug]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("too long"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_uppercase() {
        let ev = make_persona(&[&["d", "My-Persona"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_leading_underscore() {
        let ev = make_persona(&[&["d", "_invalid"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("start with"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_leading_hyphen() {
        let ev = make_persona(&[&["d", "-invalid"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("start with"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_spaces() {
        let ev = make_persona(&[&["d", "has space"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    #[test]
    fn persona_envelope_rejects_dots() {
        let ev = make_persona(&[&["d", "has.dot"]]);
        let err = validate_persona_envelope(&ev).unwrap_err();
        assert!(err.contains("`d` tag"), "got: {err}");
    }

    // ─── agent_turn_metric envelope tests ────────────────────────────────────

    /// Build an event for kind:44200 with the given tags and content.
    /// The signing key IS the agent key, so `event.pubkey` matches the agent.
    fn make_agent_turn_metric(
        agent_keys: &nostr::Keys,
        tags: &[&[&str]],
        content: &str,
    ) -> nostr::Event {
        let nostr_tags: Vec<nostr::Tag> = tags
            .iter()
            .map(|t| nostr::Tag::parse(t.iter().copied()).unwrap())
            .collect();
        nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_AGENT_TURN_METRIC as u16),
            content,
        )
        .tags(nostr_tags)
        .sign_with_keys(agent_keys)
        .unwrap()
    }

    #[test]
    fn agent_turn_metric_envelope_accepts_canonical() {
        let agent = nostr::Keys::generate();
        let owner_hex = "b".repeat(64);
        let agent_hex = agent.public_key().to_hex();
        let ev = make_agent_turn_metric(
            &agent,
            &[&["p", &owner_hex], &["agent", &agent_hex]],
            &fake_nip44_v2(),
        );
        assert!(validate_agent_turn_metric_envelope(&ev).is_ok());
    }

    #[test]
    fn agent_turn_metric_envelope_rejects_h_tag() {
        let agent = nostr::Keys::generate();
        let owner_hex = "b".repeat(64);
        let agent_hex = agent.public_key().to_hex();
        let ev = make_agent_turn_metric(
            &agent,
            &[
                &["p", &owner_hex],
                &["agent", &agent_hex],
                &["h", "some-channel-uuid"],
            ],
            &fake_nip44_v2(),
        );
        let err = validate_agent_turn_metric_envelope(&ev).unwrap_err();
        assert!(err.contains("`h` tag"), "got: {err}");
    }

    #[test]
    fn agent_turn_metric_envelope_rejects_missing_p() {
        let agent = nostr::Keys::generate();
        let agent_hex = agent.public_key().to_hex();
        let ev = make_agent_turn_metric(&agent, &[&["agent", &agent_hex]], &fake_nip44_v2());
        let err = validate_agent_turn_metric_envelope(&ev).unwrap_err();
        assert!(err.contains("`p` tag"), "got: {err}");
    }

    #[test]
    fn agent_turn_metric_envelope_rejects_missing_agent() {
        let agent = nostr::Keys::generate();
        let owner_hex = "b".repeat(64);
        let ev = make_agent_turn_metric(&agent, &[&["p", &owner_hex]], &fake_nip44_v2());
        let err = validate_agent_turn_metric_envelope(&ev).unwrap_err();
        assert!(err.contains("`agent` tag"), "got: {err}");
    }

    #[test]
    fn agent_turn_metric_envelope_rejects_agent_mismatch() {
        let agent = nostr::Keys::generate();
        let owner_hex = "b".repeat(64);
        let wrong_agent_hex = "c".repeat(64); // not event.pubkey
        let ev = make_agent_turn_metric(
            &agent,
            &[&["p", &owner_hex], &["agent", &wrong_agent_hex]],
            &fake_nip44_v2(),
        );
        let err = validate_agent_turn_metric_envelope(&ev).unwrap_err();
        assert!(err.contains("equal event pubkey"), "got: {err}");
    }

    #[test]
    fn agent_turn_metric_envelope_rejects_bad_content() {
        let agent = nostr::Keys::generate();
        let owner_hex = "b".repeat(64);
        let agent_hex = agent.public_key().to_hex();
        let ev = make_agent_turn_metric(
            &agent,
            &[&["p", &owner_hex], &["agent", &agent_hex]],
            "not-a-ciphertext",
        );
        let err = validate_agent_turn_metric_envelope(&ev).unwrap_err();
        // error comes from validate_engram_nip44_content with label replaced
        assert!(err.contains("agent-turn-metric"), "got: {err}");
    }
}
