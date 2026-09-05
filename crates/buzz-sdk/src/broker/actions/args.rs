//! Argument types — one per [`Action`], plus the tagged union that pairs a
//! wire action name with its arguments. Each type carries a `validated()`
//! returning a normalized copy; the shared validators and the contract-wide
//! strictness rules live in the [parent module](super).

use serde::{Deserialize, Serialize};

use super::{
    absent_or_valued, absent_or_valued_hex64, channel, channel_id, content, cursor, event_id,
    hex64_field, is_false, limit, mentions, optional, required, respond_to, validate_slug, Action,
    PubkeyHex, DEFAULT_PAGE_LIMIT, MAX_ABOUT_CHARS, MAX_EMOJI_CHARS, MAX_NAME_CHARS,
    MAX_OBSERVER_BATCH_BYTES, MAX_OBSERVER_FRAMES, MAX_OBSERVER_FRAME_BYTES, MAX_PROMPT_CHARS,
    MAX_SCALAR_CHARS,
};
use crate::SdkError;
use buzz_core::{
    engram::{Body, CORE_SLUG, NIP44_PLAINTEXT_MAX},
    presence::PresenceStatus,
};

/// Arguments for `channel.read` — the one read action.
///
/// One action covers channel, thread, and mention-feed scope, because they
/// differ only by filter and a name per scope would split one permission —
/// *may this agent see this channel* — across three policy decisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelReadArgs {
    /// Channel to read.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Narrow to one thread by its root event.
    #[serde(
        default,
        deserialize_with = "absent_or_valued_hex64",
        skip_serializing_if = "Option::is_none"
    )]
    pub root_event_id: Option<String>,
    /// Narrow to messages mentioning the requester — the wake path. The
    /// requester is never named; no body names its own subject.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mentions_only: bool,
    /// Opaque position to resume from, as returned in [`super::outcomes::MessagePage::next_cursor`].
    ///
    /// Absent on a first read, which starts at the host's default window.
    /// Callers must round-trip a cursor verbatim, never parse or synthesize
    /// one: the host defines ordering and cursor stability, including whether
    /// a cursor stays valid across restarts.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub cursor: Option<String>,
    /// Maximum events to return, capped at [`super::MAX_PAGE_LIMIT`].
    ///
    /// Absent means [`super::DEFAULT_PAGE_LIMIT`], not "unbounded": see
    /// [`Self::effective_limit`].
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<u32>,
}

impl ChannelReadArgs {
    /// The page size a response to these arguments is held to: explicit
    /// `limit` when set, otherwise [`super::DEFAULT_PAGE_LIMIT`] — omitting a
    /// limit asks for a sensible page, not an unbounded one.
    #[must_use]
    pub fn effective_limit(&self) -> u32 {
        self.limit.unwrap_or(DEFAULT_PAGE_LIMIT)
    }

    /// Read a whole channel from the host's default window.
    #[must_use]
    pub fn channel(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            ..Self::default()
        }
    }

    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID, a
    /// malformed root event id, an over-long or non-printable cursor, or an
    /// out-of-range limit.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            root_event_id: self
                .root_event_id
                .as_deref()
                .map(|id| event_id(id, "rootEventId"))
                .transpose()?,
            mentions_only: self.mentions_only,
            cursor: self.cursor.as_deref().map(cursor).transpose()?,
            limit: limit(self.limit)?,
        })
    }
}

// ── Write arguments ─────────────────────────────────────────────────────────

/// Arguments for `message.post`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessagePostArgs {
    /// Channel to post in.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Message body.
    pub content: String,
    /// Pubkeys to notify.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<PubkeyHex>,
}

impl MessagePostArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID or empty
    /// content, [`SdkError::ContentTooLarge`] for oversized content, and
    /// [`SdkError::TooManyMentions`] past [`super::MAX_MENTIONS`].
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            content: content(&self.content)?,
            mentions: mentions(&self.mentions)?,
        })
    }
}

/// Arguments for `message.reply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageReplyArgs {
    /// Channel containing the parent.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Event being replied to.
    #[serde(deserialize_with = "hex64_field")]
    pub reply_to_event_id: String,
    /// Reply body.
    pub content: String,
    /// Pubkeys to notify.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<PubkeyHex>,
}

impl MessageReplyArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID, a
    /// malformed event id, or empty content; [`SdkError::ContentTooLarge`] for
    /// oversized content; [`SdkError::TooManyMentions`] past [`super::MAX_MENTIONS`].
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            reply_to_event_id: event_id(&self.reply_to_event_id, "replyToEventId")?,
            content: content(&self.content)?,
            mentions: mentions(&self.mentions)?,
        })
    }
}

/// Arguments for `reaction.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReactionAddArgs {
    /// Channel containing the target.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Event being reacted to.
    #[serde(deserialize_with = "hex64_field")]
    pub target_event_id: String,
    /// Reaction payload — an emoji or a `:shortcode:`.
    pub reaction: String,
}

impl ReactionAddArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID, a
    /// malformed event id, or an empty reaction, and [`SdkError::EmojiTooLong`]
    /// past [`MAX_EMOJI_CHARS`].
    pub fn validated(&self) -> Result<Self, SdkError> {
        let reaction = self.reaction.trim();
        if reaction.is_empty() {
            return Err(SdkError::InvalidInput("reaction must not be empty".into()));
        }
        if reaction.chars().count() > MAX_EMOJI_CHARS {
            return Err(SdkError::EmojiTooLong);
        }
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            target_event_id: event_id(&self.target_event_id, "targetEventId")?,
            reaction: reaction.to_owned(),
        })
    }
}

/// Arguments for `profile.set`.
///
/// Only the requester's own profile is addressable, so there is no subject
/// field. Absent fields are left as they are; the host does not clear them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileSetArgs {
    /// Replacement display name.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_name: Option<String>,
    /// Replacement bio.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub about: Option<String>,
    /// Replacement avatar URL.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub picture: Option<String>,
}

impl ProfileSetArgs {
    /// Validate and normalize, requiring at least one field to change.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an over-long field or a request
    /// that changes nothing.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let normalized = Self {
            display_name: optional(self.display_name.as_ref(), "display name", MAX_NAME_CHARS)?,
            about: optional(self.about.as_ref(), "about", MAX_ABOUT_CHARS)?,
            picture: optional(self.picture.as_ref(), "picture", MAX_SCALAR_CHARS)?,
        };
        if normalized.display_name.is_none()
            && normalized.about.is_none()
            && normalized.picture.is_none()
        {
            return Err(SdkError::InvalidInput(
                "include at least one profile field to set".into(),
            ));
        }
        Ok(normalized)
    }
}

/// Arguments for `storage.address`.
///
/// Deriving a record's address needs the secret this contract exists to avoid
/// holding, which is why it routes through the interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageAddressArgs {
    /// Memory slug — `core` or `mem/…`, per NIP-AE.
    pub slug: String,
}

impl StorageAddressArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] when the slug fails the NIP-AE
    /// grammar.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let slug = required(&self.slug, "slug", 255)?;
        validate_slug(&slug).map_err(|e| SdkError::InvalidInput(e.to_string()))?;
        Ok(Self { slug })
    }
}

/// Arguments for `storage.get`.
///
/// Slug-addressed like [`StorageAddressArgs`]: the host derives the record's
/// address from the slug and the secret, fetches it, and decrypts. The keyless
/// caller sends a name and receives plaintext — never a key or a relay filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageGetArgs {
    /// Memory slug — `core` or `mem/…`, per NIP-AE.
    pub slug: String,
}

impl StorageGetArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] when the slug fails the NIP-AE
    /// grammar.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let slug = required(&self.slug, "slug", 255)?;
        validate_slug(&slug).map_err(|e| SdkError::InvalidInput(e.to_string()))?;
        Ok(Self { slug })
    }
}

/// Arguments for `storage.put`.
///
/// The caller supplies plaintext; the host encrypts it under the record's key
/// and publishes it. Encryption stays host-side for the same reason addressing
/// does — the secret is the thing this contract exists to avoid handing over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoragePutArgs {
    /// Memory slug — `core` or `mem/…`, per NIP-AE.
    pub slug: String,
    /// Plaintext record body; the host encrypts it before publishing.
    pub value: String,
}

impl StoragePutArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] when the slug fails the NIP-AE grammar
    /// or the value is empty, and [`SdkError::ContentTooLarge`] when the
    /// complete canonical NIP-AE body exceeds its plaintext limit.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let slug = required(&self.slug, "slug", 255)?;
        validate_slug(&slug).map_err(|e| SdkError::InvalidInput(e.to_string()))?;
        if self.value.trim().is_empty() {
            return Err(SdkError::InvalidInput("value must not be empty".into()));
        }
        let body = if slug == CORE_SLUG {
            Body::Core {
                profile: self.value.clone(),
            }
        } else {
            Body::Memory {
                slug: slug.clone(),
                value: Some(self.value.clone()),
            }
        };
        let encoded_len = body.to_json_bytes().len();
        if encoded_len > NIP44_PLAINTEXT_MAX {
            return Err(SdkError::ContentTooLarge {
                max: NIP44_PLAINTEXT_MAX,
                got: encoded_len,
            });
        }
        Ok(Self {
            slug,
            value: self.value.clone(),
        })
    }
}

// ── Live-signal arguments ───────────────────────────────────────────────────

/// Arguments for `presence.set`.
///
/// The status is the whole payload; there is no subject, since presence is
/// always the requester's own. The host publishes it as the ephemeral presence
/// kind on the requester's behalf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresenceSetArgs {
    /// Presence to publish.
    pub status: PresenceStatus,
}

impl PresenceSetArgs {
    /// Validate and normalize.
    ///
    /// The status enum is closed, so an unknown value is already rejected at
    /// deserialization; there is nothing further to normalize.
    ///
    /// # Errors
    ///
    /// Never fails today; the signature matches its siblings so a future
    /// invariant has a place to live.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            status: self.status,
        })
    }
}

/// Arguments for `typing.set`.
///
/// A momentary "composing" signal scoped to one channel. There is no stop
/// counterpart: the indicator is ephemeral and lapses on its own, so a client
/// signals by re-sending and stops by falling silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypingSetArgs {
    /// Channel the requester is composing in.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
}

impl TypingSetArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
        })
    }
}

/// One observer frame the host publishes on the requester's behalf.
///
/// The payload is **opaque to this contract**: the host encrypts it to the
/// owner and wraps it in the observer kind without parsing it, so its structure
/// is the runtime's concern, not the wire's. `kind` stays a top-level field so a
/// host can apply per-kind policy (pacing, dropping liveness under load) without
/// decrypting anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserverFrame {
    /// Frame kind — the runtime's telemetry discriminator (`acp_write`,
    /// `turn_started`, …), not a Nostr kind.
    pub kind: String,
    /// Opaque serialized frame body the host encrypts verbatim.
    pub payload: String,
}

impl ObserverFrame {
    /// Validate and normalize one frame.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an empty `kind` or `payload`, and
    /// [`SdkError::ContentTooLarge`] past [`super::MAX_OBSERVER_FRAME_BYTES`].
    pub fn validated(&self) -> Result<Self, SdkError> {
        let kind = required(&self.kind, "frame kind", MAX_SCALAR_CHARS)?;
        if self.payload.trim().is_empty() {
            return Err(SdkError::InvalidInput(
                "frame payload must not be empty".into(),
            ));
        }
        if self.payload.len() > MAX_OBSERVER_FRAME_BYTES {
            return Err(SdkError::ContentTooLarge {
                max: MAX_OBSERVER_FRAME_BYTES,
                got: self.payload.len(),
            });
        }
        Ok(Self {
            kind,
            payload: self.payload.clone(),
        })
    }
}

/// Arguments for `observer.emit`.
///
/// A batch of frames, since trajectory is high-volume; the host re-batches and
/// paces publication. Frames carry no owner, key, or Nostr metadata — those are
/// host-derived, the same separation the rest of this contract keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserverEmitArgs {
    /// Frames to publish, in order.
    pub frames: Vec<ObserverFrame>,
}

impl ObserverEmitArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an empty batch or one past
    /// [`super::MAX_OBSERVER_FRAMES`], propagates each frame's own validation
    /// error, and returns [`SdkError::ContentTooLarge`] when the complete
    /// serialized argument exceeds [`super::MAX_OBSERVER_BATCH_BYTES`].
    pub fn validated(&self) -> Result<Self, SdkError> {
        if self.frames.is_empty() {
            return Err(SdkError::InvalidInput(
                "observer.emit requires at least one frame".into(),
            ));
        }
        if self.frames.len() > MAX_OBSERVER_FRAMES {
            return Err(SdkError::InvalidInput(format!(
                "observer.emit carries {} frames, over the {MAX_OBSERVER_FRAMES} cap",
                self.frames.len()
            )));
        }
        let validated = Self {
            frames: self
                .frames
                .iter()
                .map(ObserverFrame::validated)
                .collect::<Result<_, _>>()?,
        };
        let encoded_len = serde_json::to_vec(&validated)
            .map_err(|error| {
                SdkError::InvalidInput(format!("observer batch is not serializable: {error}"))
            })?
            .len();
        if encoded_len > MAX_OBSERVER_BATCH_BYTES {
            return Err(SdkError::ContentTooLarge {
                max: MAX_OBSERVER_BATCH_BYTES,
                got: encoded_len,
            });
        }
        Ok(validated)
    }
}

/// Arguments for `liveness.ping`.
///
/// A turn-scoped keepalive: it tells the host a turn is still running, which a
/// host can act on (resetting a stall watchdog) rather than merely forward.
/// That host-side meaning is what distinguishes it from an `observer.emit` frame
/// carrying the same context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LivenessPingArgs {
    /// Channel the turn belongs to.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Opaque, process-local turn identifier the keepalive refers to.
    pub turn_id: String,
}

impl LivenessPingArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID or an
    /// empty or over-long turn id.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            turn_id: required(&self.turn_id, "turnId", MAX_SCALAR_CHARS)?,
        })
    }
}

// ── Agent arguments ─────────────────────────────────────────────────────────

/// Which agent an update or delete targets — exactly one selector, so a host
/// never has to guess which of two names wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentTarget {
    /// Target by agent pubkey.
    Pubkey(PubkeyHex),
    /// Target by the agent's current name.
    Name(String),
}

impl AgentTarget {
    /// Validate and normalize the selector.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for an empty or over-long name.
    pub fn validated(&self) -> Result<Self, SdkError> {
        match self {
            Self::Pubkey(pubkey) => Ok(Self::Pubkey(PubkeyHex::parse(pubkey.as_str())?)),
            Self::Name(name) => Ok(Self::Name(required(name, "agent name", MAX_NAME_CHARS)?)),
        }
    }
}

/// Arguments for `agents.create`.
///
/// There is no owner field: the owner is whoever the host authenticated. See
/// the [contract docs](crate::broker) on ownership recursion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsCreateArgs {
    /// Channel the new agent is attached to.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
    /// Name for the new agent.
    pub display_name: String,
    /// Instructions the new agent runs with.
    pub system_prompt: String,
    /// Preferred harness id; the host refuses a runtime it cannot resolve.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub runtime: Option<String>,
    /// Inference provider.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider: Option<String>,
    /// Model identifier, interpreted relative to the runtime.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<String>,
    /// Inbound author gate mode; absent = the host's owner-only default.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub respond_to: Option<String>,
}

impl AgentsCreateArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed channel UUID, an
    /// empty or over-long name or prompt, or an unsupported respond-to mode.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            channel_id: channel(&self.channel_id)?,
            display_name: required(&self.display_name, "display name", MAX_NAME_CHARS)?,
            system_prompt: required(&self.system_prompt, "system prompt", MAX_PROMPT_CHARS)?,
            runtime: optional(self.runtime.as_ref(), "runtime", MAX_SCALAR_CHARS)?,
            provider: optional(self.provider.as_ref(), "provider", MAX_SCALAR_CHARS)?,
            model: optional(self.model.as_ref(), "model", MAX_SCALAR_CHARS)?,
            respond_to: respond_to(self.respond_to.as_ref())?,
        })
    }
}

/// Arguments for `agents.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsUpdateArgs {
    /// Which agent to patch.
    pub target: AgentTarget,
    /// Rename the agent.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_name: Option<String>,
    /// Replacement instructions.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_prompt: Option<String>,
    /// Harness id to pin.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub runtime: Option<String>,
    /// Inference provider.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider: Option<String>,
    /// Model identifier.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Option<String>,
    /// Inbound author gate mode.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub respond_to: Option<String>,
}

impl AgentsUpdateArgs {
    /// Validate and normalize, requiring at least one field to change.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed target, an over-long
    /// field, an unsupported respond-to mode, or a request that changes nothing.
    pub fn validated(&self) -> Result<Self, SdkError> {
        let normalized = Self {
            target: self.target.validated()?,
            display_name: optional(self.display_name.as_ref(), "display name", MAX_NAME_CHARS)?,
            system_prompt: optional(
                self.system_prompt.as_ref(),
                "system prompt",
                MAX_PROMPT_CHARS,
            )?,
            runtime: optional(self.runtime.as_ref(), "runtime", MAX_SCALAR_CHARS)?,
            provider: optional(self.provider.as_ref(), "provider", MAX_SCALAR_CHARS)?,
            model: optional(self.model.as_ref(), "model", MAX_SCALAR_CHARS)?,
            respond_to: respond_to(self.respond_to.as_ref())?,
        };
        let unchanged = normalized.display_name.is_none()
            && normalized.system_prompt.is_none()
            && normalized.runtime.is_none()
            && normalized.provider.is_none()
            && normalized.model.is_none()
            && normalized.respond_to.is_none();
        if unchanged {
            return Err(SdkError::InvalidInput(
                "include at least one field to update".into(),
            ));
        }
        Ok(normalized)
    }
}

/// Arguments for `agents.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsDeleteArgs {
    /// Which agent to remove.
    pub target: AgentTarget,
}

impl AgentsDeleteArgs {
    /// Validate and normalize.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed target selector.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(Self {
            target: self.target.validated()?,
        })
    }
}

// ── Action union ────────────────────────────────────────────────────────────

/// An action name paired with its strictly typed arguments.
///
/// Flattened into [`crate::broker::BrokerRequest`], so the wire form is
/// `{ "action": "message.post", "args": { … } }` and an args shape can never be
/// paired with the wrong action name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "args", deny_unknown_fields)]
pub enum ActionArgs {
    /// Read a channel, thread, or mention feed.
    #[serde(rename = "channel.read")]
    ChannelRead(ChannelReadArgs),
    /// Post a message.
    #[serde(rename = "message.post")]
    MessagePost(MessagePostArgs),
    /// Reply to a message.
    #[serde(rename = "message.reply")]
    MessageReply(MessageReplyArgs),
    /// React to a message.
    #[serde(rename = "reaction.add")]
    ReactionAdd(ReactionAddArgs),
    /// Set the requester's profile.
    #[serde(rename = "profile.set")]
    ProfileSet(ProfileSetArgs),
    /// Derive an encrypted-memory address.
    #[serde(rename = "storage.address")]
    StorageAddress(StorageAddressArgs),
    /// Read an encrypted-memory record.
    #[serde(rename = "storage.get")]
    StorageGet(StorageGetArgs),
    /// Write an encrypted-memory record.
    #[serde(rename = "storage.put")]
    StoragePut(StoragePutArgs),
    /// Set the requester's presence.
    #[serde(rename = "presence.set")]
    PresenceSet(PresenceSetArgs),
    /// Signal the requester is composing.
    #[serde(rename = "typing.set")]
    TypingSet(TypingSetArgs),
    /// Publish a batch of observer frames.
    #[serde(rename = "observer.emit")]
    ObserverEmit(ObserverEmitArgs),
    /// Signal a turn is still alive.
    #[serde(rename = "liveness.ping")]
    LivenessPing(LivenessPingArgs),
    /// Mint a managed agent.
    #[serde(rename = "agents.create")]
    AgentsCreate(AgentsCreateArgs),
    /// Patch a managed agent.
    #[serde(rename = "agents.update")]
    AgentsUpdate(AgentsUpdateArgs),
    /// Remove a managed agent.
    #[serde(rename = "agents.delete")]
    AgentsDelete(AgentsDeleteArgs),
}

impl ActionArgs {
    /// The action these args belong to.
    #[must_use]
    pub fn action(&self) -> Action {
        match self {
            Self::ChannelRead(_) => Action::ChannelRead,
            Self::MessagePost(_) => Action::MessagePost,
            Self::MessageReply(_) => Action::MessageReply,
            Self::ReactionAdd(_) => Action::ReactionAdd,
            Self::ProfileSet(_) => Action::ProfileSet,
            Self::StorageAddress(_) => Action::StorageAddress,
            Self::StorageGet(_) => Action::StorageGet,
            Self::StoragePut(_) => Action::StoragePut,
            Self::PresenceSet(_) => Action::PresenceSet,
            Self::TypingSet(_) => Action::TypingSet,
            Self::ObserverEmit(_) => Action::ObserverEmit,
            Self::LivenessPing(_) => Action::LivenessPing,
            Self::AgentsCreate(_) => Action::AgentsCreate,
            Self::AgentsUpdate(_) => Action::AgentsUpdate,
            Self::AgentsDelete(_) => Action::AgentsDelete,
        }
    }

    /// Return a normalized copy with every field validated.
    ///
    /// There is deliberately no non-consuming `validate(&self)` beside this;
    /// see [`crate::broker::BrokerRequest::validated`] for the trap it was.
    ///
    /// # Errors
    ///
    /// Propagates the per-action validation error.
    pub fn validated(&self) -> Result<Self, SdkError> {
        Ok(match self {
            Self::ChannelRead(args) => Self::ChannelRead(args.validated()?),
            Self::MessagePost(args) => Self::MessagePost(args.validated()?),
            Self::MessageReply(args) => Self::MessageReply(args.validated()?),
            Self::ReactionAdd(args) => Self::ReactionAdd(args.validated()?),
            Self::ProfileSet(args) => Self::ProfileSet(args.validated()?),
            Self::StorageAddress(args) => Self::StorageAddress(args.validated()?),
            Self::StorageGet(args) => Self::StorageGet(args.validated()?),
            Self::StoragePut(args) => Self::StoragePut(args.validated()?),
            Self::PresenceSet(args) => Self::PresenceSet(args.validated()?),
            Self::TypingSet(args) => Self::TypingSet(args.validated()?),
            Self::ObserverEmit(args) => Self::ObserverEmit(args.validated()?),
            Self::LivenessPing(args) => Self::LivenessPing(args.validated()?),
            Self::AgentsCreate(args) => Self::AgentsCreate(args.validated()?),
            Self::AgentsUpdate(args) => Self::AgentsUpdate(args.validated()?),
            Self::AgentsDelete(args) => Self::AgentsDelete(args.validated()?),
        })
    }
}
