//! NIP-CI conversation import: payload types, tag helpers and pure rules.
//!
//! A conversation import brings a shared history from an external messaging
//! source into one Buzz channel. Every imported event is signed by its real
//! author and keeps its original `created_at`. The relay only accepts such a
//! backdated event while an import session is `active`, meaning every
//! required participant has consented to a proposal bound to a manifest hash
//! of the exact corpus.
//!
//! This module is zero-I/O: it holds the wire payloads for kinds
//! 42110 to 42113 and 30623, the import tag parser, and the pure state and
//! acceptance rules the relay evaluates against stored session state. See
//! `docs/nips/NIP-CI.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::kind::{KIND_REACTION, KIND_STREAM_MESSAGE};

/// Wire version carried in the `v` field of every NIP-CI payload.
pub const IMPORT_PAYLOAD_VERSION: u32 = 1;

/// Maximum serialized size of a 42110 proposal content, in bytes (16 KiB).
pub const MAX_PROPOSAL_CONTENT_BYTES: usize = 16 * 1024;

/// Minimum number of participants a proposal may list.
pub const MIN_IMPORT_PARTICIPANTS: usize = 1;

/// Maximum number of participants a proposal may list.
pub const MAX_IMPORT_PARTICIPANTS: usize = 64;

/// Maximum length of `source.label`, in characters.
pub const MAX_SOURCE_LABEL_CHARS: usize = 80;

/// Maximum length of a 42112 withdraw `reason`, in characters.
pub const MAX_WITHDRAW_REASON_CHARS: usize = 200;

/// Maximum length of the `source.type` token, in bytes.
pub const MAX_SOURCE_TYPE_LEN: usize = 32;

/// Allowed forward clock skew for an imported event's `created_at`, in seconds.
pub const IMPORT_FUTURE_SKEW_SECS: i64 = 900;

/// The relay republishes the session snapshot after this many accepted
/// imported events per participant.
pub const SNAPSHOT_REPUBLISH_EVERY: u64 = 50;

/// Literal `source_user` value that marks a 42111 as a channel-admin consent.
pub const ADMIN_CONSENT_SOURCE_USER: &str = "admin";

/// Tag name that binds an imported content event to its proposal.
pub const IMPORT_TAG: &str = "import";

/// Tag name carrying `["source", <type>, <source message ref>]`.
pub const SOURCE_TAG: &str = "source";

/// Tag name carrying the manifest hash on proposals and consents.
pub const MANIFEST_TAG: &str = "manifest";

/// Tag name carrying the source type on proposals.
pub const IMPORT_SOURCE_TAG: &str = "import-source";

/// Snapshot tag carrying the unix second from which an active session
/// accepts imported events.
pub const ACCEPTS_AFTER_TAG: &str = "accepts_after";

/// Exact relay rejection reasons for imported content events.
pub mod reason {
    /// The `import` tag names no stored 42110 in this community.
    pub const PROPOSAL_NOT_FOUND: &str = "invalid: import proposal not found";
    /// The session is not `active` (or this author already completed).
    pub const SESSION_NOT_ACTIVE: &str = "invalid: import session not active";
    /// The author is not a participant who accepted.
    pub const NOT_CONSENTING: &str = "invalid: import author not a consenting participant";
    /// `created_at` falls outside the declared range or too far in the future.
    pub const TIMESTAMP_OUT_OF_RANGE: &str = "invalid: import timestamp outside declared range";
    /// The author already published its declared count for this kind.
    pub const COUNT_EXCEEDED: &str = "invalid: import count exceeded";
    /// A reaction's target does not carry the same `import` tag.
    pub const REACTION_TARGET_NOT_IMPORTED: &str = "invalid: import reaction target not imported";
    /// The event's `h` differs from the proposal's `h`.
    pub const CHANNEL_MISMATCH: &str = "invalid: import channel mismatch";
    /// An `import` tag on a kind other than 9 or 7.
    pub const KIND_NOT_ALLOWED: &str = "invalid: import kind not allowed";
    /// The import tags are duplicated or malformed.
    pub const TAGS_MALFORMED: &str = "invalid: import tags malformed";
    /// The per-pubkey `ImportEvents` budget is exhausted.
    pub const RATE_LIMITED: &str = "rate-limited: import";
    /// A control event's author is neither the proposer nor a participant
    /// (nor, for an admin consent, a current channel admin).
    pub const NOT_PARTICIPANT: &str = "restricted: import author not a participant";
    /// The proposer is not an active member of the destination channel.
    pub const PROPOSER_NOT_MEMBER: &str =
        "restricted: import proposer not an active channel member";
    /// A listed participant is not an active member of the destination channel.
    pub const PARTICIPANT_NOT_MEMBER: &str =
        "invalid: import participant not an active channel member";
    /// A DM proposal whose participants differ from the DM member set.
    pub const DM_PARTICIPANTS_MISMATCH: &str =
        "invalid: import participants must equal the DM members";
    /// A consent whose manifest or `source_user` does not match the proposal.
    pub const CONSENT_MISMATCH: &str = "invalid: import consent does not match proposal";
    /// A proposal lists a participant who is an agent.
    pub const PARTICIPANTS_INCLUDE_AGENTS: &str =
        "restricted: import participants cannot include agents";
    /// A DM or private destination has an agent among its current members.
    pub const DESTINATION_HAS_AGENTS: &str = "restricted: import destination has agent members";
    /// Declared counts exceed the relay's per-participant or per-session cap.
    pub const COUNTS_EXCEED_LIMIT: &str = "invalid: import counts exceed the relay limit";
    /// An imported reaction collides with a reaction row already stored.
    pub const REACTION_CONFLICT: &str =
        "invalid: import reaction conflicts with an existing reaction";
    /// The event's `source` type differs from the proposal's source type.
    pub const SOURCE_TYPE_MISMATCH: &str = "invalid: import source type does not match the session";
    /// Prefix for a malformed control event; a detail follows.
    pub const MALFORMED_PREFIX: &str = "invalid: import event malformed: ";
    /// The session is active but its `accepts_after` time has not passed.
    /// Retryable: the snapshot names the time to wait for.
    pub const SESSION_WARMING: &str = "invalid: import session warming";
    /// The destination already has the relay's maximum of unended sessions.
    pub const CHANNEL_SESSION_LIMIT: &str =
        "restricted: too many active import sessions in this channel";
    /// The proposer already has the relay's maximum of unended sessions.
    pub const PROPOSER_SESSION_LIMIT: &str =
        "restricted: too many active import sessions for this proposer";
}

/// Source conversation shape declared by a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSourceKind {
    /// Two-person direct message.
    Dm,
    /// Multi-person direct message.
    GroupDm,
    /// Private (invite-only) channel.
    PrivateChannel,
    /// Public channel.
    PublicChannel,
}

/// The `source` object of a 42110 proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSource {
    /// Lowercase source type token, `[a-z0-9_-]{1,32}`.
    #[serde(rename = "type")]
    pub source_type: String,
    /// sha256 hex of the source conversation id.
    pub conversation: String,
    /// Source conversation shape.
    pub kind: ImportSourceKind,
    /// Display label, at most 80 characters.
    pub label: String,
}

/// The `range` object of a 42110 proposal, unix seconds, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRange {
    /// Earliest allowed `created_at`.
    pub from: i64,
    /// Latest allowed `created_at`.
    pub to: i64,
}

/// One participant entry of a 42110 proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportParticipant {
    /// Participant Buzz pubkey, lowercase hex.
    pub pubkey: String,
    /// sha256 hex of `type:source_user_id`, the identity binding.
    pub source_user: String,
    /// Declared number of kind 9 events this participant will publish.
    pub messages: u64,
    /// Declared number of kind 7 events this participant will publish.
    pub reactions: u64,
}

/// Content of a kind 42110 Import Proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProposalContent {
    /// Payload version, always 1.
    pub v: u32,
    /// Source conversation description.
    pub source: ImportSource,
    /// Declared time range of the imported history.
    pub range: ImportRange,
    /// Participants, one per Buzz author.
    pub participants: Vec<ImportParticipant>,
    /// Count of source authors with no Buzz participant.
    pub excluded_authors: u64,
    /// sha256 hex manifest of the exact corpus.
    pub manifest: String,
    /// Requested session expiry; the relay caps it.
    pub expires_at: i64,
}

/// A consent decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDecision {
    /// The participant agrees to the import.
    Accept,
    /// The participant refuses the import.
    Decline,
}

impl ImportDecision {
    /// Wire string for this decision.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Decline => "decline",
        }
    }

    /// Parse a wire string; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "accept" => Some(Self::Accept),
            "decline" => Some(Self::Decline),
            _ => None,
        }
    }
}

/// Content of a kind 42111 Import Consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportConsentContent {
    /// Payload version, always 1.
    pub v: u32,
    /// Accept or decline.
    pub decision: ImportDecision,
    /// Must equal the author's participant `source_user`, or `"admin"`.
    pub source_user: String,
}

/// Content of a kind 42112 Import Withdraw. The content may also be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportWithdrawContent {
    /// Payload version, always 1.
    pub v: u32,
    /// Optional human-readable reason, at most 200 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Message and reaction counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCounts {
    /// Kind 9 count.
    pub messages: u64,
    /// Kind 7 count.
    pub reactions: u64,
}

/// Content of a kind 42113 Import Complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCompleteContent {
    /// Payload version, always 1.
    pub v: u32,
    /// What the participant reports it published.
    pub published: ImportCounts,
}

/// Session status carried by the 30623 snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSessionStatus {
    /// Waiting for required consents.
    Proposed,
    /// Every required consent is present; imported events are accepted.
    Active,
    /// A required participant declined before activation (or any DM decline).
    Declined,
    /// The proposer or a participant withdrew the session.
    Withdrawn,
    /// The session passed `expires_at` without completing.
    Expired,
    /// Every accepting participant sent 42113.
    Completed,
}

impl ImportSessionStatus {
    /// Wire and storage string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Declined => "declined",
            Self::Withdrawn => "withdrawn",
            Self::Expired => "expired",
            Self::Completed => "completed",
        }
    }

    /// Parse a storage string; `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(Self::Proposed),
            "active" => Some(Self::Active),
            "declined" => Some(Self::Declined),
            "withdrawn" => Some(Self::Withdrawn),
            "expired" => Some(Self::Expired),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    /// Whether no further transition is possible.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Declined | Self::Withdrawn | Self::Expired | Self::Completed
        )
    }
}

/// Content of a kind 30623 Import Session Snapshot.
///
/// `required` lists every pubkey whose accept is needed, plus the literal
/// `"admin"` when a channel-admin consent is also required. `consents` uses
/// the same keys; the admin consent is keyed `"admin"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSessionSnapshotContent {
    /// Payload version, always 1.
    pub v: u32,
    /// Current session status.
    pub status: ImportSessionStatus,
    /// Who must accept before the session becomes active.
    pub required: Vec<String>,
    /// Latest decision per consenter.
    pub consents: BTreeMap<String, ImportDecision>,
    /// Accepted imported event counts per participant.
    pub published: BTreeMap<String, ImportCounts>,
    /// Participants that sent 42113, sorted.
    pub completed: Vec<String>,
    /// Effective (relay-capped) expiry.
    pub expires_at: i64,
}

/// Whether `value` is exactly 64 lowercase hex characters.
pub fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `value` is a valid `source.type` token, `[a-z0-9_-]{1,32}`.
pub fn is_valid_source_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SOURCE_TYPE_LEN
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

fn tag_name(tag: &nostr::Tag) -> Option<&str> {
    tag.as_slice().first().map(String::as_str)
}

/// Whether the event carries at least one `import` tag.
///
/// Every event that returns `true` is treated as an import event for
/// side-effect suppression.
pub fn is_import_event(event: &nostr::Event) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag_name(tag) == Some(IMPORT_TAG))
}

/// Parsed `import` and `source` tags of an imported content event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportTags {
    /// Proposal event id, lowercase hex.
    pub proposal_id: String,
    /// The same proposal id, decoded.
    pub proposal_bytes: [u8; 32],
    /// Source type token.
    pub source_type: String,
    /// Source message reference.
    pub source_ref: String,
}

/// Rule 1 of NIP-CI acceptance: kind is 9 or 7, exactly one well-formed
/// `import` tag and exactly one well-formed `source` tag.
///
/// Returns `Ok(None)` when the event carries no `import` tag at all (a normal
/// event). Returns the exact rejection reason otherwise.
pub fn parse_import_tags(event: &nostr::Event) -> Result<Option<ImportTags>, &'static str> {
    let mut import: Vec<&[String]> = Vec::new();
    let mut source: Vec<&[String]> = Vec::new();
    for tag in event.tags.iter() {
        match tag_name(tag) {
            Some(IMPORT_TAG) => import.push(tag.as_slice()),
            Some(SOURCE_TAG) => source.push(tag.as_slice()),
            _ => {}
        }
    }
    if import.is_empty() {
        return Ok(None);
    }
    let kind = u32::from(event.kind.as_u16());
    if kind != KIND_STREAM_MESSAGE && kind != KIND_REACTION {
        return Err(reason::KIND_NOT_ALLOWED);
    }
    if import.len() != 1 || source.len() != 1 {
        return Err(reason::TAGS_MALFORMED);
    }
    let (import, source) = (import[0], source[0]);
    let proposal_id = match import.get(1) {
        Some(id) if import.len() == 2 && is_lower_hex_64(id) => id.clone(),
        _ => return Err(reason::TAGS_MALFORMED),
    };
    let mut proposal_bytes = [0u8; 32];
    hex::decode_to_slice(&proposal_id, &mut proposal_bytes).map_err(|_| reason::TAGS_MALFORMED)?;
    match (source.get(1), source.get(2)) {
        (Some(source_type), Some(source_ref))
            if source.len() == 3 && is_valid_source_type(source_type) && !source_ref.is_empty() =>
        {
            Ok(Some(ImportTags {
                proposal_id,
                proposal_bytes,
                source_type: source_type.clone(),
                source_ref: source_ref.clone(),
            }))
        }
        _ => Err(reason::TAGS_MALFORMED),
    }
}

/// Caps a relay enforces on proposals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    /// Cap on one participant's declared messages plus reactions.
    pub max_events_per_participant: u64,
    /// Cap on the declared messages plus reactions of all participants.
    pub max_events_per_session: u64,
    /// Cap on unended (proposed or active, unexpired) sessions per channel.
    pub max_active_sessions_per_channel: u64,
    /// Cap on unended sessions one proposer holds across the community.
    pub max_active_sessions_per_proposer: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_events_per_participant: 50_000,
            max_events_per_session: 200_000,
            max_active_sessions_per_channel: 2,
            max_active_sessions_per_proposer: 8,
        }
    }
}

/// Earliest `range.from` a proposal may declare: 2000-01-01T00:00:00Z.
pub const MIN_IMPORT_RANGE_FROM: i64 = 946_684_800;

/// Why a proposal body was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalRejection {
    /// A shape rule failed; the detail follows [`reason::MALFORMED_PREFIX`].
    Malformed(&'static str),
    /// Declared counts exceed [`ImportLimits`] (or overflow).
    CountsExceedLimit,
}

impl ProposalRejection {
    /// The exact relay rejection message.
    pub fn reason(self) -> String {
        match self {
            Self::Malformed(detail) => format!("{}{detail}", reason::MALFORMED_PREFIX),
            Self::CountsExceedLimit => reason::COUNTS_EXCEED_LIMIT.to_string(),
        }
    }
}

/// Validate a decoded proposal body against NIP-CI shape rules and the
/// relay's count caps, none of which need storage. `now` is the relay clock
/// in unix seconds.
pub fn validate_proposal_content(
    content: &ImportProposalContent,
    now: i64,
    limits: ImportLimits,
) -> Result<(), ProposalRejection> {
    use ProposalRejection::{CountsExceedLimit, Malformed};
    if content.v != IMPORT_PAYLOAD_VERSION {
        return Err(Malformed("unsupported version"));
    }
    if !is_valid_source_type(&content.source.source_type) {
        return Err(Malformed("source type must match [a-z0-9_-]{1,32}"));
    }
    if !is_lower_hex_64(&content.source.conversation) {
        return Err(Malformed("source conversation must be 64 lowercase hex"));
    }
    if content.source.label.chars().count() > MAX_SOURCE_LABEL_CHARS {
        return Err(Malformed("source label exceeds 80 characters"));
    }
    if content.range.from < MIN_IMPORT_RANGE_FROM {
        return Err(Malformed("range must not start before 2000-01-01"));
    }
    if content.range.from >= content.range.to || content.range.to > now {
        return Err(Malformed("range must satisfy from < to <= now"));
    }
    let count = content.participants.len();
    if !(MIN_IMPORT_PARTICIPANTS..=MAX_IMPORT_PARTICIPANTS).contains(&count) {
        return Err(Malformed("participants must number 1 to 64"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut session_total: u64 = 0;
    for participant in &content.participants {
        if !is_lower_hex_64(&participant.pubkey) {
            return Err(Malformed("participant pubkey must be 64 lowercase hex"));
        }
        if !is_lower_hex_64(&participant.source_user) {
            return Err(Malformed(
                "participant source_user must be 64 lowercase hex",
            ));
        }
        if !seen.insert(participant.pubkey.as_str()) {
            return Err(Malformed("duplicate participant pubkey"));
        }
        let total = participant
            .messages
            .checked_add(participant.reactions)
            .filter(|total| *total <= limits.max_events_per_participant)
            .ok_or(CountsExceedLimit)?;
        session_total = session_total
            .checked_add(total)
            .filter(|total| *total <= limits.max_events_per_session)
            .ok_or(CountsExceedLimit)?;
    }
    if !is_lower_hex_64(&content.manifest) {
        return Err(Malformed("manifest must be 64 lowercase hex"));
    }
    Ok(())
}

/// Validate a decoded proposal's tags against its content: the `p` tags
/// equal the participant set (sorted ascending), the single `manifest` tag
/// equals `content.manifest`, and the single `import-source` tag equals
/// `source.type`.
pub fn validate_proposal_tags(
    event: &nostr::Event,
    content: &ImportProposalContent,
) -> Result<(), &'static str> {
    let mut p_tags = Vec::new();
    let mut manifest = Vec::new();
    let mut import_source = Vec::new();
    for tag in event.tags.iter() {
        let slice = tag.as_slice();
        match (slice.first().map(String::as_str), slice.get(1)) {
            (Some("p"), Some(v)) => p_tags.push(v.as_str()),
            (Some(MANIFEST_TAG), Some(v)) => manifest.push(v.as_str()),
            (Some(IMPORT_SOURCE_TAG), Some(v)) => import_source.push(v.as_str()),
            _ => {}
        }
    }
    let mut expected: Vec<&str> = content
        .participants
        .iter()
        .map(|p| p.pubkey.as_str())
        .collect();
    expected.sort_unstable();
    if p_tags != expected {
        return Err("p tags must list every participant, sorted ascending");
    }
    if manifest != [content.manifest.as_str()] {
        return Err("manifest tag must equal content manifest");
    }
    if import_source != [content.source.source_type.as_str()] {
        return Err("import-source tag must equal source type");
    }
    Ok(())
}

/// Tag names a control event of `kind` may carry. Anything else is refused,
/// so a record's tags are exactly what the session state was derived from.
pub fn allowed_control_tags(kind: u32) -> &'static [&'static str] {
    use crate::kind::{
        KIND_IMPORT_COMPLETE, KIND_IMPORT_CONSENT, KIND_IMPORT_PROPOSAL, KIND_IMPORT_WITHDRAW,
    };
    match kind {
        KIND_IMPORT_PROPOSAL => &["h", "p", MANIFEST_TAG, IMPORT_SOURCE_TAG],
        KIND_IMPORT_CONSENT => &["e", "h", MANIFEST_TAG, "p"],
        KIND_IMPORT_WITHDRAW | KIND_IMPORT_COMPLETE => &["e", "h"],
        _ => &[],
    }
}

/// The first tag name on a control event that its kind does not allow.
pub fn first_unexpected_control_tag(event: &nostr::Event, kind: u32) -> Option<&str> {
    let allowed = allowed_control_tags(kind);
    event
        .tags
        .iter()
        .filter_map(|tag| tag.as_slice().first())
        .map(String::as_str)
        .find(|name| !allowed.contains(name))
}

/// Cap a requested expiry at `now + max_days`. A requested expiry already in
/// the past is kept, so the session is immediately expired.
pub fn cap_expires_at(requested: i64, now: i64, max_days: u32) -> i64 {
    let cap = now.saturating_add(i64::from(max_days).saturating_mul(86_400));
    requested.min(cap)
}

/// Per-participant state used by [`derive_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantState {
    /// Participant pubkey, lowercase hex.
    pub pubkey: String,
    /// Latest consent; the proposer starts as `Accept`.
    pub consent: Option<ImportDecision>,
    /// Whether this participant sent 42113.
    pub completed: bool,
}

/// Stored session facts used to derive the next status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    /// Status currently stored.
    pub status: ImportSessionStatus,
    /// Whether the destination channel is a DM.
    pub is_dm: bool,
    /// Whether an admin consent is required.
    pub requires_admin: bool,
    /// Latest channel-admin consent, if any.
    pub admin_consent: Option<ImportDecision>,
    /// Effective expiry, unix seconds.
    pub expires_at: i64,
    /// Every participant, in any order.
    pub participants: Vec<ParticipantState>,
}

/// Derive the session status from stored facts at time `now`.
///
/// Terminal statuses never change. Past `expires_at`, a non-terminal session
/// is `expired`. In a DM any decline ends the session as `declined`. A
/// required admin decline ends the session as `declined` before activation
/// and `withdrawn` after it. In a
/// channel, a decline before activation is `declined`; a decline after
/// activation withdraws only that participant and the session stays
/// `active`, or becomes `withdrawn` once no accepting participant remains.
/// The session is `completed` once it is active and every accepting
/// participant has completed.
pub fn derive_status(facts: &SessionFacts, now: i64) -> ImportSessionStatus {
    if facts.status.is_terminal() {
        return facts.status;
    }
    if now > facts.expires_at {
        return ImportSessionStatus::Expired;
    }
    if facts.requires_admin && facts.admin_consent == Some(ImportDecision::Decline) {
        return if facts.status == ImportSessionStatus::Active {
            ImportSessionStatus::Withdrawn
        } else {
            ImportSessionStatus::Declined
        };
    }
    let is_accept = |p: &&ParticipantState| p.consent == Some(ImportDecision::Accept);
    let any_decline = facts
        .participants
        .iter()
        .any(|p| p.consent == Some(ImportDecision::Decline));
    if any_decline && (facts.is_dm || facts.status == ImportSessionStatus::Proposed) {
        return ImportSessionStatus::Declined;
    }
    let all_accepted = facts.participants.iter().all(|p| is_accept(&p));
    let admin_ok = !facts.requires_admin || facts.admin_consent == Some(ImportDecision::Accept);
    let active = facts.status == ImportSessionStatus::Active || (all_accepted && admin_ok);
    if !active {
        return ImportSessionStatus::Proposed;
    }
    let mut accepting = facts.participants.iter().filter(is_accept).peekable();
    if accepting.peek().is_none() {
        return ImportSessionStatus::Withdrawn;
    }
    if accepting.all(|p| p.completed) {
        return ImportSessionStatus::Completed;
    }
    ImportSessionStatus::Active
}

/// Author-specific facts for rules 3 to 6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorFacts {
    /// The author's latest consent, `None` while undecided.
    pub consent: Option<ImportDecision>,
    /// Whether the author already sent 42113.
    pub completed: bool,
    /// Counts declared in the proposal.
    pub declared: ImportCounts,
    /// Counts already accepted under this session.
    pub published: ImportCounts,
}

/// Evaluate rules 3 to 6 for one imported event.
///
/// `author` is `None` when the event author is not a listed participant.
/// `status` must already be derived with [`derive_status`] at `now`.
pub fn evaluate_import_rules(
    status: ImportSessionStatus,
    range: ImportRange,
    author: Option<&AuthorFacts>,
    kind: u32,
    created_at: i64,
    now: i64,
) -> Result<(), &'static str> {
    if status != ImportSessionStatus::Active {
        return Err(reason::SESSION_NOT_ACTIVE);
    }
    let Some(author) = author else {
        return Err(reason::NOT_CONSENTING);
    };
    if author.consent != Some(ImportDecision::Accept) {
        return Err(reason::NOT_CONSENTING);
    }
    if author.completed {
        return Err(reason::SESSION_NOT_ACTIVE);
    }
    if created_at < range.from
        || created_at > range.to
        || created_at > now.saturating_add(IMPORT_FUTURE_SKEW_SECS)
    {
        return Err(reason::TIMESTAMP_OUT_OF_RANGE);
    }
    let within = if kind == KIND_REACTION {
        author.published.reactions < author.declared.reactions
    } else {
        author.published.messages < author.declared.messages
    };
    if !within {
        return Err(reason::COUNT_EXCEEDED);
    }
    Ok(())
}

/// Whether an accepted import event crosses a snapshot republish boundary
/// for its author (every [`SNAPSHOT_REPUBLISH_EVERY`] events, messages and
/// reactions combined).
pub fn crosses_republish_boundary(published_after: ImportCounts) -> bool {
    let total = published_after
        .messages
        .saturating_add(published_after.reactions);
    total > 0 && total.is_multiple_of(SNAPSHOT_REPUBLISH_EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const HEX_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HEX_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const HEX_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn proposal() -> ImportProposalContent {
        ImportProposalContent {
            v: 1,
            source: ImportSource {
                source_type: "test".into(),
                conversation: HEX_C.into(),
                kind: ImportSourceKind::Dm,
                label: "Direct message".into(),
            },
            range: ImportRange {
                from: 1_000,
                to: 2_000,
            },
            participants: vec![
                ImportParticipant {
                    pubkey: HEX_A.into(),
                    source_user: HEX_C.into(),
                    messages: 3,
                    reactions: 1,
                },
                ImportParticipant {
                    pubkey: HEX_B.into(),
                    source_user: HEX_C.into(),
                    messages: 2,
                    reactions: 0,
                },
            ],
            excluded_authors: 0,
            manifest: HEX_C.into(),
            expires_at: 5_000,
        }
    }

    fn event(kind: u16, tags: &[&[&str]]) -> nostr::Event {
        let tags: Vec<Tag> = tags
            .iter()
            .map(|t| Tag::parse(t.iter().copied()).expect("tag"))
            .collect();
        EventBuilder::new(Kind::Custom(kind), "x")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn proposal_round_trips_with_spec_key_order() {
        let content = proposal();
        let json = serde_json::to_string(&content).expect("serialize");
        assert!(json.starts_with(r#"{"v":1,"source":{"type":"test","conversation":"#));
        let order = [
            "\"source\"",
            "\"range\"",
            "\"participants\"",
            "\"excluded_authors\"",
            "\"manifest\"",
            "\"expires_at\"",
        ];
        let positions: Vec<usize> = order.iter().map(|k| json.find(k).expect("key")).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
        assert!(json.contains(r#""kind":"dm""#));
        let back: ImportProposalContent = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, content);
    }

    #[test]
    fn source_kinds_use_snake_case() {
        for (kind, wire) in [
            (ImportSourceKind::Dm, "\"dm\""),
            (ImportSourceKind::GroupDm, "\"group_dm\""),
            (ImportSourceKind::PrivateChannel, "\"private_channel\""),
            (ImportSourceKind::PublicChannel, "\"public_channel\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).expect("ser"), wire);
        }
    }

    #[test]
    fn consent_withdraw_complete_round_trip() {
        let consent = ImportConsentContent {
            v: 1,
            decision: ImportDecision::Accept,
            source_user: HEX_A.into(),
        };
        let json = serde_json::to_string(&consent).expect("ser");
        assert_eq!(
            json,
            format!(r#"{{"v":1,"decision":"accept","source_user":"{HEX_A}"}}"#)
        );
        assert_eq!(
            serde_json::from_str::<ImportConsentContent>(&json).expect("de"),
            consent
        );

        let withdraw: ImportWithdrawContent = serde_json::from_str(r#"{"v":1}"#).expect("de");
        assert_eq!(withdraw.reason, None);
        assert_eq!(serde_json::to_string(&withdraw).expect("ser"), r#"{"v":1}"#);
        let withdraw = ImportWithdrawContent {
            v: 1,
            reason: Some("changed my mind".into()),
        };
        let json = serde_json::to_string(&withdraw).expect("ser");
        assert_eq!(
            serde_json::from_str::<ImportWithdrawContent>(&json).expect("de"),
            withdraw
        );

        let complete = ImportCompleteContent {
            v: 1,
            published: ImportCounts {
                messages: 4,
                reactions: 2,
            },
        };
        let json = serde_json::to_string(&complete).expect("ser");
        assert_eq!(json, r#"{"v":1,"published":{"messages":4,"reactions":2}}"#);
        assert_eq!(
            serde_json::from_str::<ImportCompleteContent>(&json).expect("de"),
            complete
        );
    }

    #[test]
    fn snapshot_round_trips() {
        let mut consents = BTreeMap::new();
        consents.insert(HEX_A.to_string(), ImportDecision::Accept);
        consents.insert(HEX_B.to_string(), ImportDecision::Decline);
        let mut published = BTreeMap::new();
        published.insert(
            HEX_A.to_string(),
            ImportCounts {
                messages: 1,
                reactions: 0,
            },
        );
        let snapshot = ImportSessionSnapshotContent {
            v: 1,
            status: ImportSessionStatus::Declined,
            required: vec![HEX_A.into(), HEX_B.into()],
            consents,
            published,
            completed: vec![],
            expires_at: 9,
        };
        let json = serde_json::to_string(&snapshot).expect("ser");
        assert!(json.starts_with(r#"{"v":1,"status":"declined","required":["#));
        assert_eq!(
            serde_json::from_str::<ImportSessionSnapshotContent>(&json).expect("de"),
            snapshot
        );
    }

    #[test]
    fn negative_counts_and_unknown_fields_fail_to_parse() {
        let mut value = serde_json::to_value(proposal()).expect("ser");
        value["participants"][0]["messages"] = serde_json::json!(-1);
        assert!(serde_json::from_value::<ImportProposalContent>(value).is_err());
        let mut value = serde_json::to_value(proposal()).expect("ser");
        value["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ImportProposalContent>(value).is_err());
    }

    #[test]
    fn status_and_decision_strings_round_trip() {
        for status in [
            ImportSessionStatus::Proposed,
            ImportSessionStatus::Active,
            ImportSessionStatus::Declined,
            ImportSessionStatus::Withdrawn,
            ImportSessionStatus::Expired,
            ImportSessionStatus::Completed,
        ] {
            assert_eq!(ImportSessionStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(
            ImportDecision::parse("accept"),
            Some(ImportDecision::Accept)
        );
        assert_eq!(ImportDecision::parse("nope"), None);
    }

    #[test]
    fn proposal_validation_rules() {
        const T: i64 = 1_700_000_000;
        let now = T + 3_000;
        let limits = ImportLimits::default();
        let good = || {
            let mut p = proposal();
            p.range = ImportRange {
                from: T + 1_000,
                to: T + 2_000,
            };
            p
        };
        let check = |p: &ImportProposalContent| validate_proposal_content(p, now, limits);
        assert_eq!(check(&good()), Ok(()));

        let mut bad = good();
        bad.range.from = bad.range.to;
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.range.to = now + 1;
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.range.from = MIN_IMPORT_RANGE_FROM - 1;
        assert!(matches!(check(&bad), Err(ProposalRejection::Malformed(_))));
        let mut bad = good();
        bad.participants.clear();
        assert!(check(&bad).is_err());
        let mut bad = good();
        let template = bad.participants[0].clone();
        bad.participants = (0..65)
            .map(|i| ImportParticipant {
                pubkey: format!("{i:064x}"),
                ..template.clone()
            })
            .collect();
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.manifest = HEX_A.to_uppercase();
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.source.source_type = "Upper".into();
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.source.label = "x".repeat(81);
        assert!(check(&bad).is_err());
        let mut bad = good();
        bad.participants[1].pubkey = HEX_A.into();
        assert!(check(&bad).is_err());
    }

    #[test]
    fn proposal_counts_are_capped() {
        const T: i64 = 1_700_000_000;
        let limits = ImportLimits {
            max_events_per_participant: 10,
            max_events_per_session: 15,
            ..ImportLimits::default()
        };
        let mut p = proposal();
        p.range = ImportRange { from: T, to: T + 1 };
        let check = |p: &ImportProposalContent| validate_proposal_content(p, T + 2, limits);
        p.participants[0].messages = 6;
        p.participants[0].reactions = 4;
        p.participants[1].messages = 5;
        assert_eq!(check(&p), Ok(()));
        p.participants[0].reactions = 5;
        assert_eq!(check(&p), Err(ProposalRejection::CountsExceedLimit));
        p.participants[0].reactions = 4;
        p.participants[1].reactions = 1;
        assert_eq!(check(&p), Err(ProposalRejection::CountsExceedLimit));
        p.participants[1].reactions = 0;
        p.participants[0].messages = u64::MAX;
        assert_eq!(check(&p), Err(ProposalRejection::CountsExceedLimit));
        assert_eq!(
            ProposalRejection::CountsExceedLimit.reason(),
            "invalid: import counts exceed the relay limit"
        );
        assert_eq!(
            ProposalRejection::Malformed("x").reason(),
            "invalid: import event malformed: x"
        );
    }

    #[test]
    fn proposal_tags_must_match_content() {
        let content = proposal();
        let good = event(
            42110,
            &[
                &["h", "chan"],
                &["p", HEX_A],
                &["p", HEX_B],
                &["manifest", HEX_C],
                &["import-source", "test"],
            ],
        );
        assert_eq!(validate_proposal_tags(&good, &content), Ok(()));
        let unsorted = event(
            42110,
            &[
                &["h", "chan"],
                &["p", HEX_B],
                &["p", HEX_A],
                &["manifest", HEX_C],
                &["import-source", "test"],
            ],
        );
        assert!(validate_proposal_tags(&unsorted, &content).is_err());
        let wrong_manifest = event(
            42110,
            &[
                &["h", "chan"],
                &["p", HEX_A],
                &["p", HEX_B],
                &["manifest", HEX_A],
                &["import-source", "test"],
            ],
        );
        assert!(validate_proposal_tags(&wrong_manifest, &content).is_err());
    }

    #[test]
    fn control_events_carry_only_their_spec_tags() {
        let cases: [(u32, &[&[&str]]); 4] = [
            (
                42110,
                &[
                    &["h", "chan"],
                    &["p", HEX_A],
                    &["manifest", HEX_C],
                    &["import-source", "test"],
                ],
            ),
            (
                42111,
                &[
                    &["e", HEX_A],
                    &["h", "chan"],
                    &["manifest", HEX_C],
                    &["p", HEX_B],
                ],
            ),
            (42112, &[&["e", HEX_A], &["h", "chan"]]),
            (42113, &[&["e", HEX_A], &["h", "chan"]]),
        ];
        for (kind, tags) in cases {
            assert_eq!(
                first_unexpected_control_tag(&event(kind as u16, tags), kind),
                None
            );
            for extra in ["ms", "client-id"] {
                let mut with_extra = tags.to_vec();
                let tag = [extra, "1"];
                with_extra.push(&tag);
                assert_eq!(
                    first_unexpected_control_tag(&event(kind as u16, &with_extra), kind),
                    Some(extra),
                    "kind {kind} accepted {extra}"
                );
            }
        }
        let withdraw_with_manifest = event(42112, &[&["e", HEX_A], &["manifest", HEX_C]]);
        assert_eq!(
            first_unexpected_control_tag(&withdraw_with_manifest, 42112),
            Some("manifest")
        );
    }

    #[test]
    fn import_tag_parsing_rule_one() {
        let plain = event(9, &[&["h", "chan"]]);
        assert_eq!(parse_import_tags(&plain), Ok(None));
        assert!(!is_import_event(&plain));

        let good = event(
            9,
            &[
                &["h", "chan"],
                &["import", HEX_A],
                &["source", "test", "1.2"],
            ],
        );
        assert!(is_import_event(&good));
        assert_eq!(
            parse_import_tags(&good),
            Ok(Some(ImportTags {
                proposal_id: HEX_A.into(),
                proposal_bytes: [0xaa; 32],
                source_type: "test".into(),
                source_ref: "1.2".into(),
            }))
        );

        let wrong_kind = event(1, &[&["import", HEX_A], &["source", "test", "1"]]);
        assert_eq!(
            parse_import_tags(&wrong_kind),
            Err(reason::KIND_NOT_ALLOWED)
        );

        let two_imports = event(
            7,
            &[
                &["import", HEX_A],
                &["import", HEX_B],
                &["source", "test", "1"],
            ],
        );
        assert_eq!(parse_import_tags(&two_imports), Err(reason::TAGS_MALFORMED));

        let no_source = event(9, &[&["import", HEX_A]]);
        assert_eq!(parse_import_tags(&no_source), Err(reason::TAGS_MALFORMED));

        let bad_id = event(9, &[&["import", "xyz"], &["source", "test", "1"]]);
        assert_eq!(parse_import_tags(&bad_id), Err(reason::TAGS_MALFORMED));
    }

    #[test]
    fn expiry_is_capped() {
        assert_eq!(cap_expires_at(10_000_000, 0, 30), 30 * 86_400);
        assert_eq!(cap_expires_at(100, 0, 30), 100);
    }

    fn facts(
        status: ImportSessionStatus,
        is_dm: bool,
        consents: &[Option<ImportDecision>],
    ) -> SessionFacts {
        SessionFacts {
            status,
            is_dm,
            requires_admin: false,
            admin_consent: None,
            expires_at: 100,
            participants: consents
                .iter()
                .enumerate()
                .map(|(i, c)| ParticipantState {
                    pubkey: format!("{i:064x}"),
                    consent: *c,
                    completed: false,
                })
                .collect(),
        }
    }

    #[test]
    fn status_derivation() {
        use ImportDecision::{Accept, Decline};
        use ImportSessionStatus::*;
        let f = facts(Proposed, true, &[Some(Accept), None]);
        assert_eq!(derive_status(&f, 50), Proposed);
        let f = facts(Proposed, true, &[Some(Accept), Some(Accept)]);
        assert_eq!(derive_status(&f, 50), Active);
        assert_eq!(derive_status(&f, 101), Expired);
        let f = facts(Active, true, &[Some(Accept), Some(Decline)]);
        assert_eq!(derive_status(&f, 50), Declined);
        let f = facts(Proposed, false, &[Some(Accept), Some(Decline)]);
        assert_eq!(derive_status(&f, 50), Declined);
        let f = facts(Active, false, &[Some(Accept), Some(Decline)]);
        assert_eq!(derive_status(&f, 50), Active);
        let f = facts(Active, false, &[Some(Decline), Some(Decline)]);
        assert_eq!(derive_status(&f, 50), Withdrawn);
        let f = facts(Withdrawn, false, &[Some(Accept), Some(Accept)]);
        assert_eq!(derive_status(&f, 50), Withdrawn);

        let mut f = facts(Proposed, false, &[Some(Accept), Some(Accept)]);
        f.requires_admin = true;
        assert_eq!(derive_status(&f, 50), Proposed);
        f.admin_consent = Some(Accept);
        assert_eq!(derive_status(&f, 50), Active);
        f.admin_consent = Some(Decline);
        assert_eq!(derive_status(&f, 50), Declined);
        f.status = Active;
        assert_eq!(derive_status(&f, 50), Withdrawn);

        let mut f = facts(Active, false, &[Some(Accept), Some(Decline)]);
        f.participants[0].completed = true;
        assert_eq!(derive_status(&f, 50), Completed);
    }

    #[test]
    fn acceptance_rules_three_to_six() {
        use ImportSessionStatus::*;
        let range = ImportRange { from: 10, to: 20 };
        let author = AuthorFacts {
            consent: Some(ImportDecision::Accept),
            completed: false,
            declared: ImportCounts {
                messages: 1,
                reactions: 1,
            },
            published: ImportCounts::default(),
        };
        let now = 1_000;
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&author), 9, 15, now),
            Ok(())
        );
        assert_eq!(
            evaluate_import_rules(Proposed, range, Some(&author), 9, 15, now),
            Err(reason::SESSION_NOT_ACTIVE)
        );
        assert_eq!(
            evaluate_import_rules(Active, range, None, 9, 15, now),
            Err(reason::NOT_CONSENTING)
        );
        let undecided = AuthorFacts {
            consent: None,
            ..author.clone()
        };
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&undecided), 9, 15, now),
            Err(reason::NOT_CONSENTING)
        );
        let done = AuthorFacts {
            completed: true,
            ..author.clone()
        };
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&done), 9, 15, now),
            Err(reason::SESSION_NOT_ACTIVE)
        );
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&author), 9, 9, now),
            Err(reason::TIMESTAMP_OUT_OF_RANGE)
        );
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&author), 9, 21, now),
            Err(reason::TIMESTAMP_OUT_OF_RANGE)
        );
        let future_range = ImportRange {
            from: 10,
            to: 5_000,
        };
        assert_eq!(
            evaluate_import_rules(Active, future_range, Some(&author), 9, now + 901, now),
            Err(reason::TIMESTAMP_OUT_OF_RANGE)
        );
        let full = AuthorFacts {
            published: ImportCounts {
                messages: 1,
                reactions: 0,
            },
            ..author.clone()
        };
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&full), 9, 15, now),
            Err(reason::COUNT_EXCEEDED)
        );
        assert_eq!(
            evaluate_import_rules(Active, range, Some(&full), 7, 15, now),
            Ok(())
        );
    }

    #[test]
    fn republish_boundary_every_fifty() {
        assert!(!crosses_republish_boundary(ImportCounts::default()));
        assert!(!crosses_republish_boundary(ImportCounts {
            messages: 49,
            reactions: 0
        }));
        assert!(crosses_republish_boundary(ImportCounts {
            messages: 30,
            reactions: 20
        }));
        assert!(crosses_republish_boundary(ImportCounts {
            messages: 100,
            reactions: 0
        }));
    }
}
