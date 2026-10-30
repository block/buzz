//! Outcome types — the success payload of each [`Action`], and the tagged union
//! that pairs a wire action name with its outcome. Outcomes are shared where
//! actions agree on what success means: the four event-publishing actions all
//! return [`EventPublished`].

use serde::{Deserialize, Serialize};

use super::{
    absent_or_valued, channel, channel_id, cursor, event_id, hex64_field, required, Action,
    PubkeyHex, MAX_NAME_CHARS, MAX_PAGE_LIMIT,
};
use crate::SdkError;
use nostr::{Event, EventId, Kind, PublicKey, Tags, Timestamp};

/// The seven canonical members of a Nostr event object, and nothing else.
///
/// `nostr`'s own `Event` deserializer accepts and *discards* unknown members,
/// which would put read results outside the contract's strict-wire rule.
/// Routing through a `deny_unknown_fields` intermediary restores the rule at
/// the one place the contract does not own the type.
///
/// Field names are the wire names from NIP-01 (`created_at`, not `createdAt`) —
/// this is the event's own encoding, not ours to rename.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictEvent {
    id: EventId,
    pubkey: PublicKey,
    created_at: Timestamp,
    kind: Kind,
    tags: Tags,
    content: String,
    sig: nostr::secp256k1::schnorr::Signature,
}

/// One message returned by a read: the signed Nostr event, verbatim.
///
/// The event is carried whole — signature and tags included — rather than
/// reduced to a projection, because Schnorr verification is local (see
/// [`Self::verify`]): a keyless agent gets independently verifiable authorship
/// and content, and only trusts the host for *completeness* and authorization.
/// Ancestry and mentions are derived accessors rather than sibling fields, so
/// nothing can disagree with the signed bytes.
///
/// Deserialization is **strict**, via a private `deny_unknown_fields`
/// intermediary; serialization is the event's own, so the wire form is
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BrokerMessage(pub Event);

impl<'de> Deserialize<'de> for BrokerMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let strict = StrictEvent::deserialize(deserializer)?;
        Ok(Self(Event::new(
            strict.id,
            strict.pubkey,
            strict.created_at,
            strict.kind,
            strict.tags,
            strict.content,
            strict.sig,
        )))
    }
}

impl BrokerMessage {
    /// The signed event.
    #[must_use]
    pub fn event(&self) -> &Event {
        &self.0
    }

    /// Verify the event's id and Schnorr signature — entirely local; a host
    /// that fabricated or altered a message fails here regardless of what it
    /// claims. Deliberately *not* called by
    /// [`crate::broker::BrokerResponse::validate_for`]: whether to pay for
    /// verification, and what to do when it fails, is the caller's policy.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] when the id does not match the
    /// content or the signature does not match the author.
    pub fn verify(&self) -> Result<(), SdkError> {
        self.0.verify().map_err(|e| {
            SdkError::InvalidInput(format!("broker returned an unverifiable event: {e}"))
        })
    }

    /// The author's pubkey, in this contract's identity type.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] if the event's author is not
    /// expressible as 64 hex characters.
    pub fn author(&self) -> Result<PubkeyHex, SdkError> {
        PubkeyHex::parse(self.0.pubkey.to_hex())
    }

    /// NIP-10 `root`/`reply` ancestry, parsed from the signed tags.
    #[must_use]
    pub fn thread(&self) -> buzz_core::nip10::ThreadMarkers {
        buzz_core::nip10::parse_thread_markers(&self.0.tags)
    }

    /// Pubkeys this message mentions, from the signed `p` tags.
    #[must_use]
    pub fn mentions(&self) -> Vec<String> {
        self.0
            .tags
            .public_keys()
            .map(nostr::PublicKey::to_hex)
            .collect()
    }
}

/// Outcome of any read action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessagePage {
    /// Messages in the host's declared order.
    pub messages: Vec<BrokerMessage>,
    /// Opaque cursor to pass as [`super::args::ChannelReadArgs::cursor`] on the next call.
    ///
    /// Absent when the host has nothing further, which is how a caller learns
    /// to stop rather than by comparing lengths against a limit it may not have
    /// set.
    #[serde(
        default,
        deserialize_with = "absent_or_valued",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_cursor: Option<String>,
}

/// Outcome of an action that published one event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventPublished {
    /// The published event's id (hex).
    #[serde(deserialize_with = "hex64_field")]
    pub event_id: String,
    /// The published event's kind.
    pub kind: u32,
    /// Creation time the host stamped, Unix seconds.
    pub created_at: u64,
}

/// Outcome of `storage.address`.
///
/// Addressing material only. A `d` tag is a keyed hash of the slug, so it
/// identifies a record without revealing the slug or the key that derived it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageAddress {
    /// Author the record is addressed under.
    pub author_pubkey: PubkeyHex,
    /// Event kind holding the record.
    pub kind: u32,
    /// Derived `d` tag (64 hex characters).
    #[serde(deserialize_with = "hex64_field")]
    pub d_tag: String,
}

/// Outcome of a successful `agents.create`.
///
/// Carries the new agent's **public** identity only — there is no field for
/// the minted secret, and `deny_unknown_fields` plus the key-set test is what
/// enforces that rather than a comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsCreateOutcome {
    /// The new agent's pubkey.
    pub agent_pubkey: PubkeyHex,
    /// The new agent's name as stored.
    pub display_name: String,
    /// Channel the agent was attached to.
    #[serde(deserialize_with = "channel_id")]
    pub channel_id: String,
}

/// Outcome of a successful `agents.update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsUpdateOutcome {
    /// The patched agent's pubkey.
    pub agent_pubkey: PubkeyHex,
    /// The agent's name after the update.
    pub display_name: String,
    /// Names of the fields the host actually changed, sorted.
    pub updated_fields: Vec<String>,
}

/// Outcome of a successful `agents.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentsDeleteOutcome {
    /// The removed agent's pubkey.
    pub agent_pubkey: PubkeyHex,
    /// The removed agent's name.
    pub display_name: String,
}

/// An action-specific success payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "outcome", deny_unknown_fields)]
pub enum ActionOutcome {
    /// `channel.read` succeeded.
    #[serde(rename = "channel.read")]
    ChannelRead(MessagePage),
    /// `message.post` succeeded.
    #[serde(rename = "message.post")]
    MessagePost(EventPublished),
    /// `message.reply` succeeded.
    #[serde(rename = "message.reply")]
    MessageReply(EventPublished),
    /// `reaction.add` succeeded.
    #[serde(rename = "reaction.add")]
    ReactionAdd(EventPublished),
    /// `profile.set` succeeded.
    #[serde(rename = "profile.set")]
    ProfileSet(EventPublished),
    /// `storage.address` succeeded.
    #[serde(rename = "storage.address")]
    StorageAddress(StorageAddress),
    /// `agents.create` succeeded.
    #[serde(rename = "agents.create")]
    AgentsCreate(AgentsCreateOutcome),
    /// `agents.update` succeeded.
    #[serde(rename = "agents.update")]
    AgentsUpdate(AgentsUpdateOutcome),
    /// `agents.delete` succeeded.
    #[serde(rename = "agents.delete")]
    AgentsDelete(AgentsDeleteOutcome),
}

impl ActionOutcome {
    /// The action that produced this outcome.
    #[must_use]
    pub fn action(&self) -> Action {
        match self {
            Self::ChannelRead(_) => Action::ChannelRead,
            Self::MessagePost(_) => Action::MessagePost,
            Self::MessageReply(_) => Action::MessageReply,
            Self::ReactionAdd(_) => Action::ReactionAdd,
            Self::ProfileSet(_) => Action::ProfileSet,
            Self::StorageAddress(_) => Action::StorageAddress,
            Self::AgentsCreate(_) => Action::AgentsCreate,
            Self::AgentsUpdate(_) => Action::AgentsUpdate,
            Self::AgentsDelete(_) => Action::AgentsDelete,
        }
    }

    /// Validate the identifiers and cursors this outcome asserts.
    ///
    /// A well-typed outcome can still carry a malformed id or an unusable
    /// cursor. Signature verification is deliberately *not* here — see
    /// [`BrokerMessage::verify`].
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a malformed event id, `d` tag,
    /// channel UUID, or cursor, an empty name, or an over-long page.
    pub fn validate(&self) -> Result<(), SdkError> {
        match self {
            Self::ChannelRead(page) => {
                if page.messages.len() > MAX_PAGE_LIMIT as usize {
                    return Err(SdkError::InvalidInput(format!(
                        "page holds {} messages, over the {MAX_PAGE_LIMIT} cap",
                        page.messages.len()
                    )));
                }
                page.next_cursor.as_deref().map(cursor).transpose()?;
            }
            Self::MessagePost(published)
            | Self::MessageReply(published)
            | Self::ReactionAdd(published)
            | Self::ProfileSet(published) => {
                event_id(&published.event_id, "eventId")?;
            }
            Self::StorageAddress(address) => {
                event_id(&address.d_tag, "dTag")?;
            }
            Self::AgentsCreate(outcome) => {
                channel(&outcome.channel_id)?;
                required(&outcome.display_name, "display name", MAX_NAME_CHARS)?;
            }
            Self::AgentsUpdate(AgentsUpdateOutcome { display_name, .. })
            | Self::AgentsDelete(AgentsDeleteOutcome { display_name, .. }) => {
                required(display_name, "display name", MAX_NAME_CHARS)?;
            }
        }
        Ok(())
    }
}
