//! Buzz V2 kind number registry.
//!
//! This module is the authoritative source for Buzz kind numbers.
//! All constants are `u32` — NIP-01 specifies kind as an unsigned integer,
//! and u32 covers the full range without truncation.

// Standard NIP kinds
/// NIP-01: User profile metadata.
pub const KIND_PROFILE: u32 = 0;
/// NIP-01: Short text note.
pub const KIND_TEXT_NOTE: u32 = 1;
/// NIP-02: Contact list / follow list.
pub const KIND_CONTACT_LIST: u32 = 3;
/// NIP-51: Mute list (replaceable, 10000–19999 range) — pubkeys/events/threads/words a user has muted.
///
/// User-owned global state, keyed by `(pubkey, kind)`. Same ownership/scope shape as kind:3.
pub const KIND_MUTE_LIST: u32 = 10000;
/// NIP-51: Pin list (replaceable) — events the user has pinned to their profile.
///
/// User-owned global state, keyed by `(pubkey, kind)`. The events referenced may live in
/// channels, but the pin list itself is profile-level state.
pub const KIND_PIN_LIST: u32 = 10001;
/// NIP-65: Relay list metadata (replaceable) — read/write relay preferences for the outbox model.
///
/// User-owned global state, keyed by `(pubkey, kind)`. Tags are `["r", url]` or
/// `["r", url, "read"]` / `["r", url, "write"]`.
pub const KIND_NIP65_RELAY_LIST_METADATA: u32 = 10002;
/// NIP-51: Bookmark list (replaceable) — events/articles/hashtags/URLs the user has bookmarked.
///
/// User-owned global state, keyed by `(pubkey, kind)`. References content but is not itself
/// channel-scoped content.
pub const KIND_BOOKMARK_LIST: u32 = 10003;
/// NIP-51: Emoji list (replaceable) — user preferred emojis and pointers to emoji sets.
pub const KIND_EMOJI_LIST: u32 = 10030;
/// NIP-51: Follow set (parameterized replaceable, 30000–39999 range) — named curated lists of pubkeys.
///
/// User-owned, keyed by `(pubkey, kind, d_tag)`. Allows multiple named follow lists on top of
/// the single kind:3 contact list (e.g. "close-friends", "news", "devs").
pub const KIND_FOLLOW_SET: u32 = 30000;
/// NIP-51: Bookmark set (parameterized replaceable) — named curated bookmark collections.
///
/// User-owned, keyed by `(pubkey, kind, d_tag)`.
pub const KIND_BOOKMARK_SET: u32 = 30003;
/// NIP-51 / NIP-30: Emoji set (parameterized replaceable).
///
/// User-owned, keyed by `(pubkey, kind, d_tag)`. Each member publishes their own
/// kind:30030 set (signed as themselves); the workspace emoji "palette" is the
/// client-side union of everyone's sets — a view computed on read, not stored
/// state. Ingest allowlists member-authored kind:30030/10030 (see
/// `required_scope_for_kind`), and the generic NIP-33 replace path keeps only the
/// latest per `(pubkey, d_tag)`.
pub const KIND_EMOJI_SET: u32 = 30030;
/// NIP-01: Channel metadata (replaceable). Not used by Buzz today.
pub const KIND_CHANNEL_METADATA: u32 = 41;
/// NIP-09: Event deletion request.
pub const KIND_DELETION: u32 = 5;
/// NIP-25: Content is emoji char or `+`/`-`.
pub const KIND_REACTION: u32 = 7;
/// NIP-17: Outer envelope for private DMs — hides sender, content, timestamp.
pub const KIND_GIFT_WRAP: u32 = 1059;
/// NIP-94: File metadata attachment.
pub const KIND_FILE_METADATA: u32 = 1063;
/// NIP-23: Long-form content (articles, blog posts, RFCs).
/// Parameterized replaceable (NIP-33, 30000–39999 range) — keyed by `(pubkey, kind, d_tag)`.
/// Stored globally (channel_id = NULL); author-owned, not channel-scoped.
pub const KIND_LONG_FORM: u32 = 30023;
/// NIP-38: User status (general, music, or custom d-tag).
/// Parameterized replaceable (NIP-33, 30000–39999 range) — keyed by `(pubkey, kind, d_tag)`.
/// Stored globally (channel_id = NULL); user-owned personal data, not channel-scoped.
pub const KIND_USER_STATUS: u32 = 30315;
/// NIP-78 / NIP-RS: Per-client read state blob for cross-device read position sync.
/// Parameterized replaceable (NIP-33, 30000–39999 range) — keyed by `(pubkey, kind, d_tag)`.
/// Stored globally (channel_id = NULL); user-owned personal data, not channel-scoped.
/// Content is NIP-44 encrypted to the user's own keypair.
pub const KIND_READ_STATE: u32 = 30078;
/// NIP-42 auth event — never stored (carries bearer tokens).
pub const KIND_AUTH: u32 = 22242;
/// BUD-01: Blossom upload auth (used in upload.rs, not stored).
pub const KIND_BLOSSOM_AUTH: u32 = 24242;
/// NIP-98: HTTP auth event (used in nip98.rs, not stored).
pub const KIND_HTTP_AUTH: u32 = 27235;

// NEW: Buzz command kinds (Pure Nostr plan)
/// Agent metadata + owner reference (replaceable, agent-authored).
pub const KIND_AGENT_PROFILE: u32 = 10100;

/// NIP-AE: Agent Engram (parameterized replaceable, agent-authored).
///
/// Encrypted memory record for AI agents. Addressed by `(pubkey_a, kind, d_tag)`,
/// where `d_tag` is an HMAC over the agent↔owner conversation key. See
/// `docs/nips/NIP-AE.md` and [`crate::engram`].
pub const KIND_AGENT_ENGRAM: u32 = 30174;

// NIP-29 group admin events
/// NIP-29: Add a user to a group.
pub const KIND_NIP29_PUT_USER: u32 = 9000;
/// NIP-29: Remove a user from a group.
pub const KIND_NIP29_REMOVE_USER: u32 = 9001;
/// NIP-29: Edit group metadata.
pub const KIND_NIP29_EDIT_METADATA: u32 = 9002;
/// NIP-29: Delete an event from a group.
pub const KIND_NIP29_DELETE_EVENT: u32 = 9005;
/// NIP-29: Create a new group.
pub const KIND_NIP29_CREATE_GROUP: u32 = 9007;
/// NIP-29: Delete a group.
pub const KIND_NIP29_DELETE_GROUP: u32 = 9008;
/// NIP-29: Create an invite to a group.
pub const KIND_NIP29_CREATE_INVITE: u32 = 9009;
/// NIP-29: Request to join a group.
pub const KIND_NIP29_JOIN_REQUEST: u32 = 9021;
/// NIP-29: Request to leave a group.
pub const KIND_NIP29_LEAVE_REQUEST: u32 = 9022;

// NIP-43 relay membership admin commands
/// NIP-43: Add a pubkey to the relay member list.
pub const RELAY_ADMIN_ADD_MEMBER: u32 = 9030;
/// NIP-43: Remove a pubkey from the relay member list.
pub const RELAY_ADMIN_REMOVE_MEMBER: u32 = 9031;
/// NIP-43: Change the role of an existing relay member.
pub const RELAY_ADMIN_CHANGE_ROLE: u32 = 9032;
// NIP-43 relay membership announcement events (relay-signed)
/// NIP-43: Relay membership list snapshot (relay-signed, replaceable by convention).
pub const KIND_NIP43_MEMBERSHIP_LIST: u32 = 13534;
/// NIP-43: Member added announcement (relay-signed).
pub const KIND_NIP43_MEMBER_ADDED: u32 = 8000;
/// NIP-43: Member removed announcement (relay-signed).
pub const KIND_NIP43_MEMBER_REMOVED: u32 = 8001;
/// NIP-43: User leave request (user-signed, ephemeral).
pub const KIND_NIP43_LEAVE_REQUEST: u32 = 28936;

// NIP-IA identity archival requests (user/agent/owner-signed)
/// NIP-IA: Request that the relay archive a target identity.
pub const KIND_IA_ARCHIVE_REQUEST: u32 = 9035;
/// NIP-IA: Request that the relay unarchive a target identity.
pub const KIND_IA_UNARCHIVE_REQUEST: u32 = 9036;

// NIP-IA identity archival announcement events (relay-signed)
/// NIP-IA: Archived-identity delta (relay-signed).
pub const KIND_IA_ARCHIVED: u32 = 8002;
/// NIP-IA: Unarchived-identity delta (relay-signed).
pub const KIND_IA_UNARCHIVED: u32 = 8003;
/// NIP-IA: Archived identities list snapshot (relay-signed, replaceable).
pub const KIND_IA_ARCHIVED_LIST: u32 = 13535;

// NIP-29 group state (addressable range 39000–39003)
/// NIP-29: Addressable group metadata state.
pub const KIND_NIP29_GROUP_METADATA: u32 = 39000;
/// NIP-29: Addressable group admins list.
pub const KIND_NIP29_GROUP_ADMINS: u32 = 39001;
/// NIP-29: Addressable group members list.
pub const KIND_NIP29_GROUP_MEMBERS: u32 = 39002;
/// NIP-29: Addressable group roles definition.
pub const KIND_NIP29_GROUP_ROLES: u32 = 39003;

/// Workflow definition (parameterized replaceable, d=workflow_uuid).
pub const KIND_WORKFLOW_DEF: u32 = 30620;

/// Mesh-LLM relay status (relay-signed, parameterized replaceable, d=buzz-relay-mesh).
///
/// Published only by the relay. Carries a sanitized, member-readable projection
/// of mesh status, including EndpointAddr dial pointers for serving nodes.
pub const KIND_MESH_LLM_RELAY_STATUS: u32 = 30621;

/// NIP-DV: per-viewer DM visibility snapshot (relay-signed, parameterized
/// replaceable, d=viewer_pubkey). Carries one `h` tag per DM the viewer has
/// hidden from their sidebar. Re-published by the relay on every hide/unhide so
/// the latest event is always the authoritative hidden set. The relay knows
/// `hidden_at` per viewer; this is the only Nostr-visible projection of it.
pub const KIND_DM_VISIBILITY: u32 = 30622;

/// Lower bound of the NIP-33 parameterized replaceable range (30000–39999).
pub const PARAM_REPLACEABLE_KIND_MIN: u32 = 30000;
/// Upper bound of the NIP-33 parameterized replaceable range (30000–39999).
pub const PARAM_REPLACEABLE_KIND_MAX: u32 = 39999;

/// Lower bound of the ephemeral event range (20000–29999). Never stored.
pub const EPHEMERAL_KIND_MIN: u32 = 20000;
/// Upper bound of the ephemeral event range (20000–29999). Never stored.
pub const EPHEMERAL_KIND_MAX: u32 = 29999;

// Ephemeral events (20000–29999) — Redis pub/sub only, never stored.
/// Ephemeral: user presence update (online/away/offline).
pub const KIND_PRESENCE_UPDATE: u32 = 20001;
/// NIP-AB: Device pairing event. Ephemeral — relay may discard after delivery.
pub const KIND_PAIRING: u32 = 24134;
/// Ephemeral: typing indicator for a channel.
pub const KIND_TYPING_INDICATOR: u32 = 20002;
/// Ephemeral: owner-scoped encrypted agent observer telemetry and control frame.
pub const KIND_AGENT_OBSERVER_FRAME: u32 = 24200;
/// Ephemeral: mesh status report (desktop → relay). A relay member reports its
/// current mesh serve availability + EndpointAddr(s) so the relay can project a
/// sanitized, relay-signed kind:30621 discovery note keyed per reporter. Tagged
/// `["p", <self>]` optional; never stored — the durable record is the relay's
/// 30621, not this transient input.
pub const KIND_MESH_STATUS_REPORT: u32 = 24620;
/// Ephemeral: mesh connect request (desktop → relay). A relay member asks the
/// relay to coordinate a direct iroh hole-punch to a peer it discovered via
/// kind:30621. Tagged `["p", <target_pubkey>]`. Never stored; the relay
/// validates membership of both ends, then emits paired KIND_MESH_CALL_ME_NOW.
pub const KIND_MESH_CONNECT_REQUEST: u32 = 24621;
/// Ephemeral: mesh call-me-now signal (relay → desktop, relay-signed). The live
/// dial trigger for a direct iroh hole-punch — carries the peer's EndpointAddr
/// so both ends dial near-simultaneously. Tagged `["p", <recipient_pubkey>]`.
/// Never stored; seconds expiry.
pub const KIND_MESH_CALL_ME_NOW: u32 = 24622;

// Stream messaging
/// NIP-29 group chat message kind. V1 used kind:10001 (replaceable range — wrong), then 40001.
///
/// Agent shutdown convention: the agent's owner sends a kind:9 message with content
/// `"!shutdown"` and a `#p` tag mentioning the agent. The harness exits gracefully.
/// This is a convention, not a new event kind — uses regular stream messages.
pub const KIND_STREAM_MESSAGE: u32 = 9;
/// V1 used kind:10002 (replaceable range — wrong).
pub const KIND_STREAM_MESSAGE_V2: u32 = 40002;
/// V1 used kind:10004 (replaceable range + NIP-51 collision — wrong).
pub const KIND_STREAM_MESSAGE_EDIT: u32 = 40003;
/// A stream message that has been pinned in a channel.
pub const KIND_STREAM_MESSAGE_PINNED: u32 = 40004;
/// A stream message that has been bookmarked by a user.
pub const KIND_STREAM_MESSAGE_BOOKMARKED: u32 = 40005;
/// A stream message scheduled for future delivery.
pub const KIND_STREAM_MESSAGE_SCHEDULED: u32 = 40006;
/// A reminder attached to a stream message or time.
pub const KIND_STREAM_REMINDER: u32 = 40007;
/// A diff/patch message showing file changes (unified diff format).
pub const KIND_STREAM_MESSAGE_DIFF: u32 = 40008;
/// Canvas (shared document) for a channel.
pub const KIND_CANVAS: u32 = 40100;
/// System message for channel state changes (join, leave, rename, etc.).
pub const KIND_SYSTEM_MESSAGE: u32 = 40099;

// Relay-only sidecar kinds (never client-submitted)
/// Channel metadata with computed fields (relay-signed sidecar).
pub const KIND_CHANNEL_SUMMARY: u32 = 40901;
/// Bulk presence state (relay-signed sidecar).
pub const KIND_PRESENCE_SNAPSHOT: u32 = 40902;

// Direct messages (41000–41999)
/// Open/create DM (p-tags = participants).
pub const KIND_DM_OPEN: u32 = 41010;
/// Add member to group DM.
pub const KIND_DM_ADD_MEMBER: u32 = 41011;
/// Hide DM from sidebar.
pub const KIND_DM_HIDE: u32 = 41012;
/// A new direct-message conversation was created.
pub const KIND_DM_CREATED: u32 = 41001;

// Agent job protocol (43000–43999)
// Not using NIP-90 kinds (5000–6999) — Buzz requires auth chains (depth ≤ 3, breadth ≤ 10).
/// An agent job was requested.
pub const KIND_JOB_REQUEST: u32 = 43001;
/// An agent accepted a job request.
pub const KIND_JOB_ACCEPTED: u32 = 43002;
/// Progress update for an in-flight agent job.
pub const KIND_JOB_PROGRESS: u32 = 43003;
/// Final result of a completed agent job.
pub const KIND_JOB_RESULT: u32 = 43004;
/// A job cancellation was requested.
pub const KIND_JOB_CANCEL: u32 = 43005;
/// An agent job failed with an error.
pub const KIND_JOB_ERROR: u32 = 43006;

/// Relay-signed notification: the target pubkey was added to a channel.
/// Stored globally (channel_id = None) with p-tag = target, h-tag = channel UUID.
pub const KIND_MEMBER_ADDED_NOTIFICATION: u32 = 44100;

/// Relay-signed notification: the target pubkey was removed from a channel.
/// Stored globally (channel_id = None) with p-tag = target, h-tag = channel UUID.
pub const KIND_MEMBER_REMOVED_NOTIFICATION: u32 = 44101;

// Forum / social (45000–45999)
// V1 used addressable range (30001–30003) — wrong.
/// A forum post (thread root).
pub const KIND_FORUM_POST: u32 = 45001;
/// A vote on a forum post.
pub const KIND_FORUM_VOTE: u32 = 45002;
/// A comment reply on a forum post.
pub const KIND_FORUM_COMMENT: u32 = 45003;

// Workflow engine (46000–46999)
/// Trigger workflow execution.
pub const KIND_WORKFLOW_TRIGGER: u32 = 46020;
/// Grant pending approval.
pub const KIND_APPROVAL_GRANT: u32 = 46030;
/// Deny pending approval.
pub const KIND_APPROVAL_DENY: u32 = 46031;
/// A workflow was triggered by a matching event.
pub const KIND_WORKFLOW_TRIGGERED: u32 = 46001;
/// A workflow step began execution.
pub const KIND_WORKFLOW_STEP_STARTED: u32 = 46002;
/// A workflow step completed successfully.
pub const KIND_WORKFLOW_STEP_COMPLETED: u32 = 46003;
/// A workflow step failed.
pub const KIND_WORKFLOW_STEP_FAILED: u32 = 46004;
/// The entire workflow completed successfully.
pub const KIND_WORKFLOW_COMPLETED: u32 = 46005;
/// The entire workflow failed.
pub const KIND_WORKFLOW_FAILED: u32 = 46006;
/// The workflow was cancelled before completion.
pub const KIND_WORKFLOW_CANCELLED: u32 = 46007;
/// A workflow step is waiting for human approval.
pub const KIND_WORKFLOW_APPROVAL_REQUESTED: u32 = 46010;
/// A pending workflow approval was granted.
pub const KIND_WORKFLOW_APPROVAL_GRANTED: u32 = 46011;
/// A pending workflow approval was denied.
pub const KIND_WORKFLOW_APPROVAL_DENIED: u32 = 46012;

// User groups (47000–47999)

// System / admin custom range (48000–48999)
/// An audit log entry was recorded.
pub const KIND_AUDIT_ENTRY: u32 = 48001;
/// A huddle (audio/video session) was started.
pub const KIND_HUDDLE_STARTED: u32 = 48100;
/// A participant joined a huddle.
pub const KIND_HUDDLE_PARTICIPANT_JOINED: u32 = 48101;
/// A participant left a huddle.
pub const KIND_HUDDLE_PARTICIPANT_LEFT: u32 = 48102;
/// A huddle ended.
pub const KIND_HUDDLE_ENDED: u32 = 48103;
/// Huddle channel guidelines/rules document.
pub const KIND_HUDDLE_GUIDELINES: u32 = 48106;

// Media (49000–49999)
/// Internal kind for media upload audit entries. Not a relay event kind.
pub const KIND_MEDIA_UPLOAD: u32 = 49001;

// ── NIP-34: Git repository events ────────────────────────────────────────────
/// NIP-34: Repository announcement (parameterized replaceable, d-tag = repo-id).
pub const KIND_GIT_REPO_ANNOUNCEMENT: u32 = 30617;
/// NIP-34: Repository state — current branch/tag refs (parameterized replaceable, d-tag = repo-id).
pub const KIND_GIT_REPO_STATE: u32 = 30618;
/// NIP-34: Patch (git format-patch output).
pub const KIND_GIT_PATCH: u32 = 1617;
/// NIP-34: Pull request.
pub const KIND_GIT_PULL_REQUEST: u32 = 1618;
/// NIP-34: Pull request update (tip commit change).
pub const KIND_GIT_PR_UPDATE: u32 = 1619;
/// NIP-34: Issue.
pub const KIND_GIT_ISSUE: u32 = 1621;
/// NIP-34: Status — Open.
pub const KIND_GIT_STATUS_OPEN: u32 = 1630;
/// NIP-34: Status — Applied / Merged.
pub const KIND_GIT_STATUS_MERGED: u32 = 1631;
/// NIP-34: Status — Closed.
pub const KIND_GIT_STATUS_CLOSED: u32 = 1632;
/// NIP-34: Status — Draft.
pub const KIND_GIT_STATUS_DRAFT: u32 = 1633;

/// All registered kind constants — used for duplicate detection and iteration.
pub const ALL_KINDS: &[u32] = &[
    KIND_PROFILE,
    KIND_TEXT_NOTE,
    KIND_CONTACT_LIST,
    KIND_MUTE_LIST,
    KIND_PIN_LIST,
    KIND_NIP65_RELAY_LIST_METADATA,
    KIND_BOOKMARK_LIST,
    KIND_EMOJI_LIST,
    KIND_FOLLOW_SET,
    KIND_BOOKMARK_SET,
    KIND_EMOJI_SET,
    KIND_CHANNEL_METADATA,
    KIND_DELETION,
    KIND_REACTION,
    KIND_GIFT_WRAP,
    KIND_FILE_METADATA,
    KIND_AGENT_PROFILE,
    KIND_AGENT_ENGRAM,
    KIND_NIP29_PUT_USER,
    KIND_NIP29_REMOVE_USER,
    KIND_NIP29_EDIT_METADATA,
    KIND_NIP29_DELETE_EVENT,
    KIND_NIP29_CREATE_GROUP,
    KIND_NIP29_DELETE_GROUP,
    KIND_NIP29_CREATE_INVITE,
    KIND_NIP29_JOIN_REQUEST,
    KIND_NIP29_LEAVE_REQUEST,
    RELAY_ADMIN_ADD_MEMBER,
    RELAY_ADMIN_REMOVE_MEMBER,
    RELAY_ADMIN_CHANGE_ROLE,
    KIND_NIP43_MEMBERSHIP_LIST,
    KIND_NIP43_MEMBER_ADDED,
    KIND_NIP43_MEMBER_REMOVED,
    KIND_NIP43_LEAVE_REQUEST,
    KIND_IA_ARCHIVE_REQUEST,
    KIND_IA_UNARCHIVE_REQUEST,
    KIND_IA_ARCHIVED,
    KIND_IA_UNARCHIVED,
    KIND_IA_ARCHIVED_LIST,
    KIND_NIP29_GROUP_METADATA,
    KIND_NIP29_GROUP_ADMINS,
    KIND_NIP29_GROUP_MEMBERS,
    KIND_NIP29_GROUP_ROLES,
    KIND_PRESENCE_UPDATE,
    KIND_TYPING_INDICATOR,
    KIND_MESH_STATUS_REPORT,
    KIND_MESH_CONNECT_REQUEST,
    KIND_MESH_CALL_ME_NOW,
    KIND_BLOSSOM_AUTH,
    KIND_PAIRING,
    KIND_AGENT_OBSERVER_FRAME,
    KIND_HTTP_AUTH,
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_EDIT,
    KIND_STREAM_MESSAGE_PINNED,
    KIND_STREAM_MESSAGE_BOOKMARKED,
    KIND_STREAM_MESSAGE_SCHEDULED,
    KIND_STREAM_REMINDER,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_CANVAS,
    KIND_SYSTEM_MESSAGE,
    KIND_CHANNEL_SUMMARY,
    KIND_PRESENCE_SNAPSHOT,
    KIND_MESH_LLM_RELAY_STATUS,
    KIND_DM_VISIBILITY,
    KIND_DM_OPEN,
    KIND_DM_ADD_MEMBER,
    KIND_DM_HIDE,
    KIND_DM_CREATED,
    KIND_JOB_REQUEST,
    KIND_JOB_ACCEPTED,
    KIND_JOB_PROGRESS,
    KIND_JOB_RESULT,
    KIND_JOB_CANCEL,
    KIND_JOB_ERROR,
    KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION,
    KIND_WORKFLOW_DEF,
    KIND_LONG_FORM,
    KIND_USER_STATUS,
    KIND_READ_STATE,
    KIND_FORUM_POST,
    KIND_FORUM_VOTE,
    KIND_FORUM_COMMENT,
    KIND_WORKFLOW_TRIGGER,
    KIND_APPROVAL_GRANT,
    KIND_APPROVAL_DENY,
    KIND_WORKFLOW_TRIGGERED,
    KIND_WORKFLOW_STEP_STARTED,
    KIND_WORKFLOW_STEP_COMPLETED,
    KIND_WORKFLOW_STEP_FAILED,
    KIND_WORKFLOW_COMPLETED,
    KIND_WORKFLOW_FAILED,
    KIND_WORKFLOW_CANCELLED,
    KIND_WORKFLOW_APPROVAL_REQUESTED,
    KIND_WORKFLOW_APPROVAL_GRANTED,
    KIND_WORKFLOW_APPROVAL_DENIED,
    KIND_AUDIT_ENTRY,
    KIND_HUDDLE_STARTED,
    KIND_HUDDLE_PARTICIPANT_JOINED,
    KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_ENDED,
    KIND_HUDDLE_GUIDELINES,
    KIND_MEDIA_UPLOAD,
    KIND_GIT_REPO_ANNOUNCEMENT,
    KIND_GIT_REPO_STATE,
    KIND_GIT_PATCH,
    KIND_GIT_PULL_REQUEST,
    KIND_GIT_PR_UPDATE,
    KIND_GIT_ISSUE,
    KIND_GIT_STATUS_OPEN,
    KIND_GIT_STATUS_MERGED,
    KIND_GIT_STATUS_CLOSED,
    KIND_GIT_STATUS_DRAFT,
];

/// Returns `true` if `kind` is in the ephemeral range (20000–29999).
pub const fn is_ephemeral(kind: u32) -> bool {
    kind >= EPHEMERAL_KIND_MIN && kind <= EPHEMERAL_KIND_MAX
}

/// Returns `true` if `kind` is replaceable (NIP-01: kinds 0, 3, 41, 10000–19999).
/// NIP-33 parameterized-replaceable kinds (30000–39999) use a different replacement
/// key (includes `d`-tag) and are handled separately via `replace_parameterized_event`.
pub const fn is_replaceable(kind: u32) -> bool {
    matches!(kind, 0 | 3 | KIND_CHANNEL_METADATA | 10000..=19999)
}

/// Returns `true` if `kind` is in the NIP-33 parameterized replaceable range (30000–39999).
///
/// These events are keyed by `(pubkey, kind, d_tag)` — the latest `created_at` wins.
pub const fn is_parameterized_replaceable(kind: u32) -> bool {
    kind >= PARAM_REPLACEABLE_KIND_MIN && kind <= PARAM_REPLACEABLE_KIND_MAX
}

/// Returns `true` if `kind` is a workflow execution event (46001–46012).
/// These must not trigger workflows (prevents infinite loops).
pub const fn is_workflow_execution_kind(kind: u32) -> bool {
    kind >= KIND_WORKFLOW_TRIGGERED && kind <= KIND_WORKFLOW_APPROVAL_DENIED
}

/// Returns `true` if `kind` is a NIP-43 relay membership admin command (9030–9032).
pub const fn is_relay_admin_kind(kind: u32) -> bool {
    matches!(
        kind,
        RELAY_ADMIN_ADD_MEMBER | RELAY_ADMIN_REMOVE_MEMBER | RELAY_ADMIN_CHANGE_ROLE
    )
}

/// Returns `true` if `kind` is a NIP-IA identity archival request (9035–9036).
///
/// Only the user-signed *request* kinds are matched. The relay-signed delta and
/// snapshot kinds (8002/8003/13535) are emitted by the relay, never ingested as
/// commands, so they are intentionally excluded.
pub const fn is_identity_archive_request_kind(kind: u32) -> bool {
    matches!(kind, KIND_IA_ARCHIVE_REQUEST | KIND_IA_UNARCHIVE_REQUEST)
}

/// Returns `true` if `kind` is a Buzz command kind that requires transactional execution.
pub const fn is_command_kind(kind: u32) -> bool {
    matches!(
        kind,
        KIND_WORKFLOW_DEF
            | KIND_DM_OPEN
            | KIND_DM_ADD_MEMBER
            | KIND_DM_HIDE
            | KIND_WORKFLOW_TRIGGER
            | KIND_APPROVAL_GRANT
            | KIND_APPROVAL_DENY
    )
}

/// Returns `true` if `kind` is a relay-only sidecar kind.
/// Client submission of these kinds must be rejected.
pub const fn is_relay_only_kind(kind: u32) -> bool {
    matches!(
        kind,
        KIND_CHANNEL_SUMMARY
            | KIND_PRESENCE_SNAPSHOT
            | KIND_MESH_LLM_RELAY_STATUS
            | KIND_DM_VISIBILITY
    )
}

/// Extract the kind from a nostr Event as u32.
/// NIP-01 specifies kind as an unsigned integer; u32 covers the full range.
pub fn event_kind_u32(event: &nostr::Event) -> u32 {
    event.kind.as_u16() as u32
}

/// Extract the kind from a nostr Event as i32 (for Postgres INT columns).
/// Safe: all Buzz kinds fit in i32 (max 65535 < i32::MAX).
pub fn event_kind_i32(event: &nostr::Event) -> i32 {
    event.kind.as_u16() as i32
}

// Compile-time: new kinds are in the expected ranges.
const _: () = assert!(is_replaceable(KIND_AGENT_PROFILE)); // 10100 ∈ 10000–19999
const _: () = assert!(is_parameterized_replaceable(KIND_WORKFLOW_DEF)); // 30620 ∈ 30000–39999
const _: () = assert!(is_parameterized_replaceable(KIND_MESH_LLM_RELAY_STATUS)); // 30621 ∈ 30000–39999
const _: () = assert!(is_parameterized_replaceable(KIND_DM_VISIBILITY)); // 30622 ∈ 30000–39999

// Compile-time: NIP-34 parameterized replaceable kinds are in the correct range.
const _: () = assert!(
    KIND_GIT_REPO_ANNOUNCEMENT >= PARAM_REPLACEABLE_KIND_MIN
        && KIND_GIT_REPO_ANNOUNCEMENT <= PARAM_REPLACEABLE_KIND_MAX
);
const _: () = assert!(
    KIND_GIT_REPO_STATE >= PARAM_REPLACEABLE_KIND_MIN
        && KIND_GIT_REPO_STATE <= PARAM_REPLACEABLE_KIND_MAX
);

// Compile-time: all Buzz kind constants fit in nostr's u16-backed Kind.
const _: () = assert!(KIND_AUTH <= u16::MAX as u32);
const _: () = assert!(KIND_CANVAS <= u16::MAX as u32);
const _: () = assert!(KIND_HUDDLE_GUIDELINES <= u16::MAX as u32);
const _: () = assert!(EPHEMERAL_KIND_MIN < EPHEMERAL_KIND_MAX);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_kind_values() {
        let mut seen = std::collections::HashSet::new();
        for &k in ALL_KINDS {
            assert!(seen.insert(k), "duplicate kind value: {k}");
        }
    }

    #[test]
    fn parameterized_replaceable_range() {
        assert!(!is_parameterized_replaceable(29999));
        assert!(is_parameterized_replaceable(30000));
        assert!(is_parameterized_replaceable(30023)); // NIP-23 long-form
        assert!(is_parameterized_replaceable(39000)); // NIP-29 group metadata
        assert!(is_parameterized_replaceable(39999));
        assert!(!is_parameterized_replaceable(40000));
    }

    #[test]
    fn replaceable_and_parameterized_are_disjoint() {
        for kind in 0..=65535u32 {
            assert!(
                !(is_replaceable(kind) && is_parameterized_replaceable(kind)),
                "kind {kind} is both replaceable and parameterized replaceable"
            );
        }
    }
}
