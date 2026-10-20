//! Broker actions — the closed set of operations an agent may ask a host to
//! perform. [`Action`] and the shared validators live here; the payload types
//! are split into [`args`] and [`outcomes`] so each side of a call reviews on
//! its own.

use serde::{Deserialize, Serialize};

use crate::SdkError;
use buzz_core::engram::validate_slug;

pub mod args;
pub mod outcomes;

pub use args::{
    ActionArgs, AgentTarget, AgentsCreateArgs, AgentsDeleteArgs, AgentsUpdateArgs, ChannelReadArgs,
    MessagePostArgs, MessageReplyArgs, ProfileSetArgs, ReactionAddArgs, StorageAddressArgs,
};
pub use outcomes::{
    ActionOutcome, AgentsCreateOutcome, AgentsDeleteOutcome, AgentsUpdateOutcome, BrokerMessage,
    EventPublished, MessagePage, StorageAddress,
};

/// Maximum characters in a display name or agent name.
pub const MAX_NAME_CHARS: usize = 120;

/// Maximum characters in a system prompt.
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// Maximum characters in a short scalar field (runtime, provider, model).
pub const MAX_SCALAR_CHARS: usize = 300;

/// Maximum characters in a profile `about` blurb.
pub const MAX_ABOUT_CHARS: usize = 2_000;

/// Maximum bytes of message content, matching the SDK's channel-message cap.
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// Maximum characters in a reaction payload (emoji or `:shortcode:`).
pub const MAX_EMOJI_CHARS: usize = 66;

/// Maximum mentions attachable to one message.
pub const MAX_MENTIONS: usize = 50;

/// Maximum events a single read may return.
pub const MAX_PAGE_LIMIT: u32 = 500;

/// Events a read returns when the request sets no explicit `limit`.
///
/// A caller that omits `limit` is not agreeing to an unbounded page, so this is
/// the number a response is held to in that case — see
/// [`crate::broker::BrokerResponse::validate_for`]. It is deliberately well
/// under [`MAX_PAGE_LIMIT`]: the cap is what a host may ever send, this is what
/// it may send unasked.
pub const DEFAULT_PAGE_LIMIT: u32 = 100;

/// Maximum accepted length of a read cursor, in bytes.
pub const MAX_CURSOR_LEN: usize = 256;

/// Inbound author gate modes a requester may ask for.
///
/// `allowlist` is deliberately absent: it needs a pubkey list this request
/// shape does not carry, and a mode without its list would mint an agent
/// nobody can talk to.
pub const RESPOND_TO_MODES: [&str; 2] = ["owner-only", "anyone"];

/// A public key in lowercase hex — the only identity this contract has. No
/// secret-key counterpart exists in this module (#6467's identity/signing
/// separation, made structural).
///
/// A value of this type is a **real x-only secp256k1 point**, not just 64 hex
/// characters — most 32-byte values lie on no curve. Accepting shape alone
/// would defer the first real rejection to whichever consumer eventually
/// converts the string to a key, after the request was already accepted. The
/// curve check is the `nostr` crate's, so the contract and the events it
/// carries agree on what a key is by construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PubkeyHex(String);

impl PubkeyHex {
    /// Parse a 64-character hex x-only public key, normalizing to lowercase.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] unless `value` is exactly 64 hex
    /// characters **and** those bytes are a point on secp256k1.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, SdkError> {
        let value = value.as_ref().trim();
        if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SdkError::InvalidInput(
                "pubkey must be 64 hex characters".into(),
            ));
        }
        let value = value.to_ascii_lowercase();
        // `from_hex` only decodes hex; `xonly` is what actually rejects a
        // value that is not on the curve.
        nostr::PublicKey::from_hex(&value)
            .and_then(|key| key.xonly().map(|_| ()))
            .map_err(|_| {
                SdkError::InvalidInput("pubkey is not a valid secp256k1 x-only public key".into())
            })?;
        Ok(Self(value))
    }

    /// The hex representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PubkeyHex {
    type Error = SdkError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<PubkeyHex> for String {
    fn from(value: PubkeyHex) -> Self {
        value.0
    }
}

impl std::fmt::Display for PubkeyHex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An action name the broker can dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Read messages from a channel, thread, or mention feed after a cursor.
    ChannelRead,
    /// Post a top-level channel message.
    MessagePost,
    /// Reply to an existing message.
    MessageReply,
    /// React to an existing message.
    ReactionAdd,
    /// Publish the requester's own profile metadata.
    ProfileSet,
    /// Derive the address of one encrypted-memory record.
    StorageAddress,
    /// Mint a managed agent owned by the requester.
    AgentsCreate,
    /// Patch a managed agent the requester owns.
    AgentsUpdate,
    /// Remove a managed agent the requester owns.
    AgentsDelete,
}

impl Action {
    /// Every action in this protocol version, in wire-name order.
    pub const ALL: [Self; 9] = [
        Self::AgentsCreate,
        Self::AgentsDelete,
        Self::AgentsUpdate,
        Self::ChannelRead,
        Self::MessagePost,
        Self::MessageReply,
        Self::ProfileSet,
        Self::ReactionAdd,
        Self::StorageAddress,
    ];

    /// Stable wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChannelRead => "channel.read",
            Self::MessagePost => "message.post",
            Self::MessageReply => "message.reply",
            Self::ReactionAdd => "reaction.add",
            Self::ProfileSet => "profile.set",
            Self::StorageAddress => "storage.address",
            Self::AgentsCreate => "agents.create",
            Self::AgentsUpdate => "agents.update",
            Self::AgentsDelete => "agents.delete",
        }
    }

    /// The action contract version this build implements.
    #[must_use]
    pub fn current_version(self) -> u16 {
        1
    }

    /// Whether a host may refuse this action without harming the agent.
    ///
    /// #6467 requires non-essential signed housekeeping to be skippable, so an
    /// agent can still run where it is unavailable. See
    /// [`super::BrokerErrorCode::Unsupported`] for how a caller reacts.
    #[must_use]
    pub fn is_best_effort(self) -> bool {
        matches!(self, Self::ReactionAdd)
    }

    /// Resolve a wire name.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an unknown action name.
    pub fn parse(name: &str) -> Result<Self, SdkError> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == name)
            .ok_or_else(|| SdkError::InvalidInput(format!("unknown broker action \"{name}\"")))
    }
}

// ── Shared validators ───────────────────────────────────────────────────────

fn is_false(value: &bool) -> bool {
    !*value
}

/// Deserialize an optional member that may be **absent but never `null`**.
///
/// This is the contract's one spelling-of-absence rule, and this is its
/// canonical rationale. `#[serde(default)] Option<T>` maps an explicit `null`
/// to `None`, *indistinguishable from absent* to downstream code — so a reader
/// that decides something from absence (the status match in
/// [`crate::broker::BrokerResponse`], or
/// [`args::ChannelReadArgs::effective_limit`]) would silently treat a member
/// the sender did supply as one it did not. In the response envelope that was
/// a real hole: `{"status":"failed","outcome":null}` parsed as a plain failure
/// and skipped the per-status contradiction check. Rejecting `null` outright
/// leaves exactly one way to say "absent" and no layer guessing what a
/// present-but-empty member meant.
///
/// Used with `#[serde(default, deserialize_with = "…")]`: serde calls this only
/// when the key is present, so reaching the `None` arm below means the member
/// was present and `null`. `deny_unknown_fields` stays in force alongside it.
///
/// A required member of a non-`Option` type already rejects `null` as a type
/// error; the guard is only load-bearing where `Option` plus `default` would
/// otherwise conflate `null` with absent.
pub(super) fn absent_or_valued<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    match Option::<T>::deserialize(deserializer)? {
        Some(value) => Ok(Some(value)),
        None => Err(D::Error::custom(
            "must not be null; omit the member to mean absent",
        )),
    }
}

fn required(value: &str, label: &str, max: usize) -> Result<String, SdkError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(SdkError::InvalidInput(format!("{label} must not be empty")));
    }
    if value.chars().count() > max {
        return Err(SdkError::InvalidInput(format!(
            "{label} is too long (max {max} characters)"
        )));
    }
    Ok(value.to_owned())
}

fn optional(value: Option<&String>, label: &str, max: usize) -> Result<Option<String>, SdkError> {
    value.map(|value| required(value, label, max)).transpose()
}

/// Validate a channel id and return its **canonical** spelling — lowercase
/// hyphenated. `Uuid::parse_str` accepts several spellings of one channel, and
/// freezing the caller's spelling would make the host's canonical echo of the
/// same identity look like a mismatch in
/// [`crate::broker::BrokerResponse::validate_for`]. Same treatment
/// [`PubkeyHex::parse`] gives the other identity in this contract.
fn channel(value: &str) -> Result<String, SdkError> {
    let value = required(value, "channel", 128)?;
    uuid::Uuid::parse_str(&value)
        .map(|id| id.as_hyphenated().to_string())
        .map_err(|_| SdkError::InvalidInput(format!("invalid channel UUID: {value}")))
}

/// Deserialize a `channelId`, canonicalizing it and rejecting a non-UUID.
///
/// The wire is the one door a validator cannot cover: fields holding a channel
/// id are public `String`s, so a payload parsed from JSON reaches a caller
/// without passing through any `validated()`. Delegating to [`channel`] keeps
/// the wire form and the constructed form canonicalized by the same code.
pub(super) fn channel_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let raw = String::deserialize(deserializer)?;
    channel(&raw).map_err(D::Error::custom)
}

fn event_id(value: &str, label: &str) -> Result<String, SdkError> {
    let value = required(value, label, 64)?;
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SdkError::InvalidInput(format!(
            "{label} must be 64 hex characters"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

/// Deserialize a 64-hex identifier (`eventId`, `dTag`), lowercasing it —
/// the [`channel_id`] rule applied to the contract's other multi-spelling
/// identities. The label is generic because serde already reports which
/// member failed.
pub(super) fn hex64_field<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let raw = String::deserialize(deserializer)?;
    event_id(&raw, "identifier").map_err(D::Error::custom)
}

/// [`hex64_field`] for an optional member: `null` is still rejected (see
/// [`absent_or_valued`]). One function because `deserialize_with` takes one,
/// and both rules apply to the same member.
pub(super) fn absent_or_valued_hex64<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    match Option::<String>::deserialize(deserializer)? {
        Some(raw) => Ok(Some(
            event_id(&raw, "identifier").map_err(D::Error::custom)?,
        )),
        None => Err(D::Error::custom(
            "must not be null; omit the member to mean absent",
        )),
    }
}

fn content(value: &str) -> Result<String, SdkError> {
    if value.trim().is_empty() {
        return Err(SdkError::InvalidInput("content must not be empty".into()));
    }
    if value.len() > MAX_CONTENT_BYTES {
        return Err(SdkError::ContentTooLarge {
            max: MAX_CONTENT_BYTES,
            got: value.len(),
        });
    }
    Ok(value.to_owned())
}

fn mentions(values: &[PubkeyHex]) -> Result<Vec<PubkeyHex>, SdkError> {
    if values.len() > MAX_MENTIONS {
        return Err(SdkError::TooManyMentions);
    }
    values
        .iter()
        .map(|pubkey| PubkeyHex::parse(pubkey.as_str()))
        .collect()
}

fn limit(value: Option<u32>) -> Result<Option<u32>, SdkError> {
    match value {
        None => Ok(None),
        Some(0) => Err(SdkError::InvalidInput("limit must be at least 1".into())),
        Some(limit) if limit > MAX_PAGE_LIMIT => Err(SdkError::InvalidInput(format!(
            "limit exceeds {MAX_PAGE_LIMIT} (got {limit})"
        ))),
        Some(limit) => Ok(Some(limit)),
    }
}

/// Validate an opaque read cursor: printable ASCII, bounded, never parsed.
///
/// The bound exists so a host cannot be made to store an unbounded token; the
/// character set keeps it safe to log. Nothing here interprets the value.
fn cursor(value: &str) -> Result<String, SdkError> {
    if value.is_empty() {
        return Err(SdkError::InvalidInput(
            "cursor must not be empty (omit it to start from the host's default window)".into(),
        ));
    }
    if value.len() > MAX_CURSOR_LEN {
        return Err(SdkError::InvalidInput(format!(
            "cursor exceeds {MAX_CURSOR_LEN} bytes (got {})",
            value.len()
        )));
    }
    if !value.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(SdkError::InvalidInput(
            "cursor must be printable ASCII without spaces".into(),
        ));
    }
    Ok(value.to_owned())
}

fn respond_to(value: Option<&String>) -> Result<Option<String>, SdkError> {
    let value = optional(value, "respond-to", MAX_SCALAR_CHARS)?;
    if let Some(mode) = value.as_deref() {
        if !RESPOND_TO_MODES.contains(&mode) {
            return Err(SdkError::InvalidInput(format!(
                "respond-to must be one of {}",
                RESPOND_TO_MODES.join(", ")
            )));
        }
    }
    Ok(value)
}
