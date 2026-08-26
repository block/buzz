//! Broker actions — the closed set of operations an agent may ask a host to perform.
//!
//! An action is the unit of policy; see the [module docs](super) for why this is
//! an operation enum rather than a signing primitive.
//!
//! Two consequences of that choice are visible in the types below. Every args
//! and outcome type is `deny_unknown_fields`, so a credential or environment map
//! smuggled into a payload fails to deserialize instead of reaching an executor.
//! And the wire key set of every type is pinned by test, so a secret-bearing
//! field cannot be added without a test failing.
//!
//! [`Action`] and the shared validators live here; the payload types are split
//! into [`args`] and [`outcomes`] so each side of a call reviews on its own.

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

/// A public key in lowercase hex — the only identity this contract has.
///
/// #6467 asks for identity to be separable from signing. This type is that
/// separation made structural: it holds a public key and has no counterpart in
/// this module for the corresponding secret.
///
/// # A value of this type is a real public key
///
/// 64 hex characters is a *shape*, not a key. The x coordinate of a
/// secp256k1 point is what a pubkey is, and most 32-byte values are not one —
/// `ffff…ff` is 64 valid hex characters and lies on no curve. Accepting shape
/// alone would make this type's name a claim it does not check, and would push
/// the first real rejection out to whichever consumer eventually converts the
/// string to a key: a host resolving a mention, a relay filter, a signature
/// check. That consumer fails at a point where the request has already been
/// accepted, so [`Self::parse`] requires the point here instead.
///
/// The curve check is the `nostr` crate's, which this crate already depends on
/// for events and signatures, so the contract and the events it carries agree on
/// what a key is by construction rather than by two parallel implementations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PubkeyHex(String);

impl PubkeyHex {
    /// Parse a 64-character hex x-only public key, normalizing to lowercase.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] unless `value` is exactly 64 hex
    /// characters **and** those bytes are a point on secp256k1. See the type
    /// docs for why the curve check belongs here.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, SdkError> {
        let value = value.as_ref().trim();
        if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SdkError::InvalidInput(
                "pubkey must be 64 hex characters".into(),
            ));
        }
        let value = value.to_ascii_lowercase();
        // `PublicKey::from_hex` only decodes hex; `xonly` is the conversion that
        // actually rejects a value that is not on the curve. Doing only the
        // former here would leave this check believing it had run.
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
/// Every optional member in this contract means "absent" by being absent. JSON
/// `null` is a second spelling of the same thing that no serializer here emits —
/// `skip_serializing_if` omits the member instead — so accepting it would define
/// a wire value the contract does not.
///
/// That is not merely untidy. `#[serde(default)] Option<T>` maps an explicit
/// `null` to `None`, which is *indistinguishable from absent* to any code
/// downstream. A reader that decides something from absence — the status match in
/// [`crate::broker::BrokerResponse`], or
/// [`args::ChannelReadArgs::effective_limit`] choosing [`DEFAULT_PAGE_LIMIT`] —
/// then silently treats a member the sender did supply as one it did not. In the
/// response envelope that was a real hole: `{"status":"failed","outcome":null}`
/// parsed as a plain failure, so the per-status contradiction check never fired.
///
/// Rejecting `null` outright is stronger than tracking presence beside the value
/// and cheaper to reason about: there is then exactly one way to say "absent",
/// and no layer has to decide what a present-but-empty member meant. Applied
/// uniformly, it also means a host implementer never has to guess whether
/// `{"limit": null}` means the default or no limit — it means neither, it is a
/// malformed request.
///
/// Used with `#[serde(default, deserialize_with = "…")]`: serde calls this only
/// when the key is present, so reaching the `None` arm below means the member was
/// present and `null`. `deny_unknown_fields` stays in force alongside it.
///
/// Not every member needs this. A required member of a non-`Option` type already
/// rejects `null` as a type error — which covers the request envelope's flattened
/// `action`/`args` (adjacently tagged, both required), every canonical member of
/// the strict event intermediary, and the two `bool` members here. The guard is
/// only load-bearing where `Option` plus `default` would otherwise make `null`
/// and absent the same value.
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

/// Validate a channel id and return its **canonical** spelling.
///
/// A UUID has several legal spellings — `Uuid::parse_str` accepts uppercase, the
/// unhyphenated 32-character form, `{braced}`, and `urn:uuid:` — and they all
/// name the same channel. Returning the caller's spelling would freeze a request
/// body that disagrees with the host's canonical echo of the same identity, and
/// the response correlation in
/// [`crate::broker::BrokerResponse::validate_for`] would then reject a correct
/// answer. So this returns the lowercase hyphenated form and nothing else: one
/// identity, one spelling, chosen at the only point that sees the value before it
/// is frozen.
///
/// This is the same treatment [`PubkeyHex::parse`] gives the other identity in
/// this contract, and it is applied at both doors — the validators, and the
/// [`channel_id`] deserializer — so a value cannot reach a caller
/// un-canonicalized whether it was built or parsed.
fn channel(value: &str) -> Result<String, SdkError> {
    let value = required(value, "channel", 128)?;
    uuid::Uuid::parse_str(&value)
        .map(|id| id.as_hyphenated().to_string())
        .map_err(|_| SdkError::InvalidInput(format!("invalid channel UUID: {value}")))
}

/// Deserialize a `channelId`, canonicalizing it and rejecting a non-UUID.
///
/// The wire is the one door a validator cannot cover: every args and outcome
/// field holding a channel id is a public `String`, so a payload parsed from
/// JSON reaches a caller without passing through any `validated()`. Delegating
/// to [`channel`] means the wire form and the constructed form are canonicalized
/// by the same code, so the two cannot drift, and a malformed channel id never
/// becomes a value at all — the same rule the no-null guard follows.
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

/// Deserialize a 64-hex identifier (`eventId`, `dTag`), lowercasing it and
/// rejecting anything that is not 64 hex characters.
///
/// Hex admits two spellings of one identifier, so this is the [`channel_id`] rule
/// applied to the contract's other multi-spelling identities. See that function
/// for why the wire needs a door of its own.
///
/// The label is generic because serde reports which member failed; naming the
/// member here as well would be a second copy to keep true.
pub(super) fn hex64_field<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let raw = String::deserialize(deserializer)?;
    event_id(&raw, "identifier").map_err(D::Error::custom)
}

/// [`hex64_field`] for an optional member: `null` is still rejected.
///
/// A dedicated function rather than a composition of the two guards, because
/// `deserialize_with` takes one function and both rules — reject `null`,
/// canonicalize the value — apply to the same member. Reaching the `None` arm
/// means the key was present and `null`; see [`absent_or_valued`].
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
