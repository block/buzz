//! Shared message-forward envelope contract (kind 40009).
//!
//! A forward is a root message authored by the *forwarder*: its `content` is
//! the forwarder's optional note, and the complete signed original event is
//! carried verbatim in a `fwd` tag. The original's text is never merged into
//! the content, so attribution and full-text search stay intact.
//!
//! Both the relay validator and the SDK builder use this module so producer and
//! consumer cannot drift: [`forward_tags`] emits the canonical tag set,
//! [`ForwardEnvelope::parse`] reads it back, and
//! [`ForwardEnvelope::verify_embedded`] upgrades the embedded copy from a claim
//! to a fact by re-deriving its NIP-01 id and checking its Schnorr signature.

use std::fmt;
use std::str::FromStr;

use nostr::{Event, JsonUtil};
use uuid::Uuid;

use crate::error::VerificationError;
use crate::kind::{
    event_kind_u32, KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use crate::verification::verify_event;

/// Tag carrying the stringified JSON of the complete original signed event.
pub const FWD_TAG: &str = "fwd";

/// Tag carrying the source channel UUID and its source-type label.
pub const FWD_SRC_TAG: &str = "fwd-src";

/// NIP-18 generic-repost tag carrying the stringified original kind.
pub const FWD_KIND_TAG: &str = "k";

/// NIP-21 quote tag, emitted only for open-channel sources.
pub const FWD_QUOTE_TAG: &str = "q";

/// Maximum byte length of the [`FWD_TAG`] value (64 KiB).
pub const FWD_MAX_BYTES: usize = 64 * 1024;

/// Kinds that may be forwarded. Kind 40009 is deliberately absent: forwarding a
/// forward flattens client-side to the embedded original, so depth is always 1.
pub const FORWARDABLE_KINDS: &[u32] = &[
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];

/// `fwd-src` label for a source channel with open visibility.
pub const FWD_SRC_TYPE_CHANNEL: &str = "channel";

/// `fwd-src` label for a non-open group source channel.
pub const FWD_SRC_TYPE_PRIVATE: &str = "private";

/// `fwd-src` label for a direct-message source channel.
pub const FWD_SRC_TYPE_DM: &str = "dm";

/// Visibility class of the channel a message was forwarded from.
///
/// The label is part of the wire contract: the relay checks it against the
/// actual source channel row, and clients use it to pick the attribution line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardSourceType {
    /// Source channel has open visibility — attribution is linkable.
    Channel,
    /// Source channel is a non-open group channel.
    Private,
    /// Source channel is a direct-message conversation.
    Dm,
}

impl ForwardSourceType {
    /// Canonical wire label.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Channel => FWD_SRC_TYPE_CHANNEL,
            Self::Private => FWD_SRC_TYPE_PRIVATE,
            Self::Dm => FWD_SRC_TYPE_DM,
        }
    }

    /// Whether this source class permits a `q` tag (and a linkable attribution).
    pub fn allows_quote(&self) -> bool {
        matches!(self, Self::Channel)
    }
}

impl fmt::Display for ForwardSourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ForwardSourceType {
    type Err = ForwardError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            FWD_SRC_TYPE_CHANNEL => Ok(Self::Channel),
            FWD_SRC_TYPE_PRIVATE => Ok(Self::Private),
            FWD_SRC_TYPE_DM => Ok(Self::Dm),
            other => Err(ForwardError::UnknownSourceType {
                label: other.to_string(),
            }),
        }
    }
}

/// The `q` tag reference to the original, present only for open sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardQuote {
    /// Original event id, lowercase hex.
    pub event_id: String,
    /// Original author pubkey, lowercase hex.
    pub author_pubkey: String,
}

/// A parsed kind 40009 forward envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardEnvelope {
    /// The complete original event decoded from the `fwd` tag. Signature is
    /// *not* checked by [`ForwardEnvelope::parse`] — call
    /// [`ForwardEnvelope::verify_embedded`] for that.
    pub original: Event,
    /// Source channel UUID from `fwd-src`, equal to the original's `h` tag.
    pub source_channel_id: Uuid,
    /// Source-type label from `fwd-src`.
    pub source_type: ForwardSourceType,
    /// The `q` reference, present only when `source_type` is
    /// [`ForwardSourceType::Channel`].
    pub quote: Option<ForwardQuote>,
    /// `imeta` tags copied verbatim from the original.
    pub imeta: Vec<Vec<String>>,
}

/// Reasons a forward envelope is malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForwardError {
    /// No `fwd` tag on the event.
    #[error("missing {FWD_TAG} tag")]
    MissingFwd,

    /// More than one `fwd` tag; exactly one is allowed.
    #[error("expected exactly one {FWD_TAG} tag, found {count}")]
    DuplicateFwd {
        /// Number of `fwd` tags seen.
        count: usize,
    },

    /// The `fwd` tag value exceeds [`FWD_MAX_BYTES`].
    #[error("{FWD_TAG} tag is {bytes} bytes, limit is {FWD_MAX_BYTES}")]
    FwdTooLarge {
        /// Byte length of the offending value.
        bytes: usize,
    },

    /// The `fwd` tag value is not a parseable Nostr event.
    #[error("{FWD_TAG} tag is not a parseable event: {reason}")]
    UnparseableEmbedded {
        /// Underlying parse failure.
        reason: String,
    },

    /// The embedded event's kind is not in [`FORWARDABLE_KINDS`].
    #[error("kind {kind} is not forwardable")]
    KindNotForwardable {
        /// The embedded event's kind.
        kind: u32,
    },

    /// No `k` tag on the event.
    #[error("missing {FWD_KIND_TAG} tag")]
    MissingKind,

    /// The `k` tag does not match the embedded event's kind.
    #[error("{FWD_KIND_TAG} tag {declared:?} does not match embedded kind {embedded}")]
    KindMismatch {
        /// Value of the `k` tag.
        declared: String,
        /// The embedded event's actual kind.
        embedded: u32,
    },

    /// No `fwd-src` tag on the event.
    #[error("missing {FWD_SRC_TAG} tag")]
    MissingSource,

    /// More than one `fwd-src` tag; exactly one is allowed.
    #[error("expected exactly one {FWD_SRC_TAG} tag, found {count}")]
    DuplicateSource {
        /// Number of `fwd-src` tags seen.
        count: usize,
    },

    /// The `fwd-src` tag is missing elements or carries an invalid UUID.
    #[error("malformed {FWD_SRC_TAG} tag: {reason}")]
    MalformedSource {
        /// What was wrong with the tag.
        reason: String,
    },

    /// The `fwd-src` type label is not a known source class.
    #[error("unknown {FWD_SRC_TAG} type {label:?}")]
    UnknownSourceType {
        /// The unrecognized label.
        label: String,
    },

    /// No destination `h` tag on the outer event.
    #[error("missing destination h tag")]
    MissingDestination,

    /// More than one destination `h` tag; exactly one is allowed so generic
    /// NIP-29 consumers cannot scope the same signed event differently.
    #[error("expected exactly one destination h tag, found {count}")]
    DuplicateDestination {
        /// Number of outer `h` tags seen.
        count: usize,
    },

    /// The destination `h` tag is not the canonical two-element shape.
    #[error("malformed destination h tag: {reason}")]
    MalformedDestination {
        /// What was wrong with the tag.
        reason: String,
    },

    /// The embedded original carries no `h` tag, or it is not a UUID.
    #[error("embedded event has no valid h tag")]
    EmbeddedMissingChannel,

    /// More than one `h` tag on the embedded original; its source channel must
    /// be unambiguous.
    #[error("expected exactly one h tag on the embedded original, found {count}")]
    DuplicateEmbeddedChannel {
        /// Number of embedded `h` tags seen.
        count: usize,
    },

    /// The embedded original's `h` tag is not the canonical two-element shape.
    #[error("malformed h tag on the embedded original: {reason}")]
    MalformedEmbeddedChannel {
        /// What was wrong with the tag.
        reason: String,
    },

    /// The `fwd-src` UUID disagrees with the embedded original's `h` tag.
    #[error("{FWD_SRC_TAG} channel {declared} does not match embedded h tag {embedded}")]
    SourceChannelMismatch {
        /// UUID declared in `fwd-src`.
        declared: Uuid,
        /// UUID found on the embedded original.
        embedded: Uuid,
    },

    /// A `q` tag was present for a non-open source.
    #[error("{FWD_QUOTE_TAG} tag is only allowed for {FWD_SRC_TYPE_CHANNEL} sources")]
    QuoteNotAllowed,

    /// More than one `q` tag; at most one is allowed.
    #[error("expected at most one {FWD_QUOTE_TAG} tag, found {count}")]
    DuplicateQuote {
        /// Number of `q` tags seen.
        count: usize,
    },

    /// The `q` tag is missing its id or author element.
    #[error("malformed {FWD_QUOTE_TAG} tag: {reason}")]
    MalformedQuote {
        /// What was wrong with the tag.
        reason: String,
    },

    /// The `q` tag id or author does not match the embedded original.
    #[error("{FWD_QUOTE_TAG} tag {field} {declared:?} does not match embedded {expected:?}")]
    QuoteMismatch {
        /// Which element mismatched (`id` or `pubkey`).
        field: &'static str,
        /// Value found in the `q` tag.
        declared: String,
        /// Value on the embedded original.
        expected: String,
    },

    /// A marked (`root`/`reply`) `e` tag was present. Forwards are always roots.
    #[error("marked e tags are not allowed on a forward")]
    MarkedETag,

    /// The outer event's `imeta` tags are not a verbatim copy of the embedded
    /// original's. Divergent outer attachments would render differently from
    /// the embedded copy clients actually display.
    #[error("imeta tags must be copied verbatim from the embedded original (outer {outer}, embedded {embedded})")]
    ImetaMismatch {
        /// Number of `imeta` tags on the outer event.
        outer: usize,
        /// Number of `imeta` tags on the embedded original.
        embedded: usize,
    },
}

impl ForwardEnvelope {
    /// Parses the forward envelope out of a kind 40009 event's tags.
    ///
    /// Callers dispatch on the outer kind; this only reads the envelope tags.
    /// Purely structural — the embedded signature and the existence/visibility
    /// of the source channel are checked separately (see
    /// [`ForwardEnvelope::verify_embedded`] and the relay's channel lookup).
    pub fn parse(event: &Event) -> Result<Self, ForwardError> {
        reject_marked_e_tags(event)?;
        check_destination_tag(event)?;

        match tag_count(event, FWD_TAG) {
            0 => return Err(ForwardError::MissingFwd),
            1 => {}
            count => return Err(ForwardError::DuplicateFwd { count }),
        }
        let fwd = single_tag_value(event, FWD_TAG).ok_or(ForwardError::MissingFwd)?;

        if fwd.len() > FWD_MAX_BYTES {
            return Err(ForwardError::FwdTooLarge { bytes: fwd.len() });
        }

        let original = Event::from_json(fwd).map_err(|e| ForwardError::UnparseableEmbedded {
            reason: e.to_string(),
        })?;

        let embedded_kind = event_kind_u32(&original);
        if !FORWARDABLE_KINDS.contains(&embedded_kind) {
            return Err(ForwardError::KindNotForwardable {
                kind: embedded_kind,
            });
        }

        let declared_kind =
            single_tag_value(event, FWD_KIND_TAG).ok_or(ForwardError::MissingKind)?;
        if declared_kind.parse::<u32>() != Ok(embedded_kind) {
            return Err(ForwardError::KindMismatch {
                declared: declared_kind.to_string(),
                embedded: embedded_kind,
            });
        }

        let (source_channel_id, source_type) = parse_source_tag(event)?;
        let embedded_channel_id = embedded_channel_id(&original)?;
        if source_channel_id != embedded_channel_id {
            return Err(ForwardError::SourceChannelMismatch {
                declared: source_channel_id,
                embedded: embedded_channel_id,
            });
        }

        let quote = parse_quote_tag(event, &original, source_type)?;

        // The outer copy is what generic NIP-92 consumers render, so it must be
        // byte-identical to the embedded original's — no extra, missing, or
        // rewritten attachment metadata.
        let imeta = imeta_tags(&original);
        let outer_imeta = imeta_tags(event);
        if outer_imeta != imeta {
            return Err(ForwardError::ImetaMismatch {
                outer: outer_imeta.len(),
                embedded: imeta.len(),
            });
        }

        Ok(Self {
            original,
            source_channel_id,
            source_type,
            quote,
            imeta,
        })
    }

    /// Re-derives the embedded original's NIP-01 id and verifies its Schnorr
    /// signature, reusing [`verify_event`].
    ///
    /// CPU-bound — call via `tokio::task::spawn_blocking` in async contexts.
    pub fn verify_embedded(&self) -> Result<(), VerificationError> {
        verify_event(&self.original)
    }
}

/// Builds the canonical tag set for a kind 40009 forward.
///
/// Emits, in order: `h` (destination), `fwd` (complete original JSON), `k`
/// (original kind), `fwd-src` (source channel + type), `q` (open sources only),
/// then the original's `imeta` tags verbatim. `p` tags for mentions the
/// forwarder writes in the note are the caller's business — a forward never
/// p-tags the original author.
///
/// `source_channel_id` must equal the original's own `h` tag.
pub fn forward_tags(
    destination_channel_id: Uuid,
    original: &Event,
    source_channel_id: Uuid,
    source_type: ForwardSourceType,
) -> Result<Vec<Vec<String>>, ForwardError> {
    let embedded_kind = event_kind_u32(original);
    if !FORWARDABLE_KINDS.contains(&embedded_kind) {
        return Err(ForwardError::KindNotForwardable {
            kind: embedded_kind,
        });
    }

    let embedded_channel_id = embedded_channel_id(original)?;
    if source_channel_id != embedded_channel_id {
        return Err(ForwardError::SourceChannelMismatch {
            declared: source_channel_id,
            embedded: embedded_channel_id,
        });
    }

    let embedded_json = original.as_json();
    if embedded_json.len() > FWD_MAX_BYTES {
        return Err(ForwardError::FwdTooLarge {
            bytes: embedded_json.len(),
        });
    }

    let mut tags = vec![
        vec!["h".to_string(), destination_channel_id.to_string()],
        vec![FWD_TAG.to_string(), embedded_json],
        vec![FWD_KIND_TAG.to_string(), embedded_kind.to_string()],
        vec![
            FWD_SRC_TAG.to_string(),
            source_channel_id.to_string(),
            source_type.as_str().to_string(),
        ],
    ];

    if source_type.allows_quote() {
        tags.push(vec![
            FWD_QUOTE_TAG.to_string(),
            original.id.to_hex(),
            String::new(),
            original.pubkey.to_hex(),
        ]);
    }

    tags.extend(imeta_tags(original));

    Ok(tags)
}

/// Collects an event's `imeta` tags verbatim as raw string vectors.
fn imeta_tags(event: &Event) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(|parts| parts.first().is_some_and(|name| name == "imeta"))
        .map(<[String]>::to_vec)
        .collect()
}

fn tag_count(event: &Event, name: &str) -> usize {
    event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().is_some_and(|n| n == name))
        .count()
}

/// Returns the second element of the sole tag with `name`, or `None` when the
/// tag is absent, duplicated, or has no value element.
fn single_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut matches = event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(|parts| parts.first().is_some_and(|n| n == name));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    first.get(1).map(String::as_str)
}

fn reject_marked_e_tags(event: &Event) -> Result<(), ForwardError> {
    let marked = event.tags.iter().any(|t| {
        let parts = t.as_slice();
        parts.first().is_some_and(|n| n == "e")
            && parts
                .get(3)
                .is_some_and(|marker| marker == "root" || marker == "reply")
    });
    if marked {
        return Err(ForwardError::MarkedETag);
    }
    Ok(())
}

/// Returns the elements of the first `h` tag, or `None` when there is none.
fn first_h_tag(event: &Event) -> Option<&[String]> {
    event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .find(|parts| parts.first().is_some_and(|n| n == "h"))
}

/// A forward names exactly one destination, in the canonical `["h", <value>]`
/// shape: exactly two elements, so a trailing element cannot smuggle a second
/// reading of the same scope past consumers that only look at element 1. Scoped
/// to kind 40009 — the generic ingest `h` handling for other kinds is untouched.
fn check_destination_tag(event: &Event) -> Result<(), ForwardError> {
    match tag_count(event, "h") {
        0 => return Err(ForwardError::MissingDestination),
        1 => {}
        count => return Err(ForwardError::DuplicateDestination { count }),
    }

    let parts = first_h_tag(event).ok_or(ForwardError::MissingDestination)?;
    if parts.len() != 2 {
        return Err(ForwardError::MalformedDestination {
            reason: format!("expected exactly 2 elements, found {}", parts.len()),
        });
    }
    Ok(())
}

/// Whether `value` is a 64-character hex string (either case).
fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_source_tag(event: &Event) -> Result<(Uuid, ForwardSourceType), ForwardError> {
    let count = tag_count(event, FWD_SRC_TAG);
    match count {
        0 => return Err(ForwardError::MissingSource),
        1 => {}
        count => return Err(ForwardError::DuplicateSource { count }),
    }

    let parts = event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .find(|parts| parts.first().is_some_and(|n| n == FWD_SRC_TAG))
        .ok_or(ForwardError::MissingSource)?;

    let uuid = parts.get(1).ok_or_else(|| ForwardError::MalformedSource {
        reason: "missing source channel uuid".to_string(),
    })?;
    let uuid = Uuid::parse_str(uuid).map_err(|_| ForwardError::MalformedSource {
        reason: format!("invalid source channel uuid {uuid:?}"),
    })?;
    let label = parts.get(2).ok_or_else(|| ForwardError::MalformedSource {
        reason: "missing source type label".to_string(),
    })?;

    Ok((uuid, label.parse()?))
}

/// The embedded original's source channel, held to the same canonical
/// `["h", <uuid>]` shape as the destination tag.
fn embedded_channel_id(original: &Event) -> Result<Uuid, ForwardError> {
    match tag_count(original, "h") {
        0 => return Err(ForwardError::EmbeddedMissingChannel),
        1 => {}
        count => return Err(ForwardError::DuplicateEmbeddedChannel { count }),
    }

    let parts = first_h_tag(original).ok_or(ForwardError::EmbeddedMissingChannel)?;
    if parts.len() != 2 {
        return Err(ForwardError::MalformedEmbeddedChannel {
            reason: format!("expected exactly 2 elements, found {}", parts.len()),
        });
    }

    Uuid::parse_str(&parts[1]).map_err(|_| ForwardError::EmbeddedMissingChannel)
}

fn parse_quote_tag(
    event: &Event,
    original: &Event,
    source_type: ForwardSourceType,
) -> Result<Option<ForwardQuote>, ForwardError> {
    let count = tag_count(event, FWD_QUOTE_TAG);
    match count {
        0 => return Ok(None),
        1 => {}
        count => return Err(ForwardError::DuplicateQuote { count }),
    }
    if !source_type.allows_quote() {
        return Err(ForwardError::QuoteNotAllowed);
    }

    let parts = event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .find(|parts| parts.first().is_some_and(|n| n == FWD_QUOTE_TAG))
        .ok_or_else(|| ForwardError::MalformedQuote {
            reason: "missing quote tag".to_string(),
        })?;

    // NIP-FW pins the shape to exactly `["q", <id>, "", <pubkey>]`: no relay
    // hint (the source is same-relay by construction) and no trailing
    // elements, so every producer emits the identical tag.
    if parts.len() != 4 {
        return Err(ForwardError::MalformedQuote {
            reason: format!("expected exactly 4 elements, found {}", parts.len()),
        });
    }
    if !parts[2].is_empty() {
        return Err(ForwardError::MalformedQuote {
            reason: format!("relay hint must be empty, found {:?}", parts[2]),
        });
    }

    let event_id = &parts[1];
    let author_pubkey = &parts[3];
    if !is_hex64(event_id) {
        return Err(ForwardError::MalformedQuote {
            reason: format!("event id {event_id:?} is not 64 hex characters"),
        });
    }
    if !is_hex64(author_pubkey) {
        return Err(ForwardError::MalformedQuote {
            reason: format!("author pubkey {author_pubkey:?} is not 64 hex characters"),
        });
    }

    let expected_id = original.id.to_hex();
    if !event_id.eq_ignore_ascii_case(&expected_id) {
        return Err(ForwardError::QuoteMismatch {
            field: "id",
            declared: event_id.to_string(),
            expected: expected_id,
        });
    }
    let expected_pubkey = original.pubkey.to_hex();
    if !author_pubkey.eq_ignore_ascii_case(&expected_pubkey) {
        return Err(ForwardError::QuoteMismatch {
            field: "pubkey",
            declared: author_pubkey.to_string(),
            expected: expected_pubkey,
        });
    }

    Ok(Some(ForwardQuote {
        event_id: expected_id,
        author_pubkey: expected_pubkey,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::KIND_STREAM_MESSAGE_FORWARD;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const SOURCE_CHANNEL: &str = "11111111-1111-4111-8111-111111111111";
    const DEST_CHANNEL: &str = "22222222-2222-4222-8222-222222222222";

    fn source_id() -> Uuid {
        Uuid::parse_str(SOURCE_CHANNEL).expect("source uuid")
    }

    fn dest_id() -> Uuid {
        Uuid::parse_str(DEST_CHANNEL).expect("dest uuid")
    }

    fn sign(kind: u32, content: &str, tags: &[Vec<String>]) -> Event {
        let keys = Keys::generate();
        let tags: Vec<Tag> = tags
            .iter()
            .map(|parts| Tag::parse(parts.iter().map(String::as_str)).expect("tag"))
            .collect();
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign")
    }

    fn owned(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_string()).collect()
    }

    /// A kind 9 original in the source channel with one imeta tag.
    fn original() -> Event {
        sign(
            KIND_STREAM_MESSAGE,
            "original text",
            &[
                owned(&["h", SOURCE_CHANNEL]),
                owned(&["imeta", "url https://example.test/a.png", "x abc123"]),
            ],
        )
    }

    fn forward_from(tags: &[Vec<String>]) -> Event {
        sign(KIND_STREAM_MESSAGE_FORWARD, "note", tags)
    }

    fn open_tags(original: &Event) -> Vec<Vec<String>> {
        forward_tags(dest_id(), original, source_id(), ForwardSourceType::Channel)
            .expect("build tags")
    }

    fn replace_tag(tags: &[Vec<String>], name: &str, replacement: Vec<String>) -> Vec<Vec<String>> {
        tags.iter()
            .map(|parts| {
                if parts.first().map(String::as_str) == Some(name) {
                    replacement.clone()
                } else {
                    parts.clone()
                }
            })
            .collect()
    }

    fn drop_tag(tags: &[Vec<String>], name: &str) -> Vec<Vec<String>> {
        tags.iter()
            .filter(|parts| parts.first().map(String::as_str) != Some(name))
            .cloned()
            .collect()
    }

    #[test]
    fn happy_path_round_trips_and_verifies() {
        let original = original();
        let tags = open_tags(&original);

        assert_eq!(tags[0], owned(&["h", DEST_CHANNEL]));
        assert_eq!(tags[2], owned(&["k", "9"]));
        assert_eq!(
            tags[3],
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_CHANNEL])
        );
        assert!(tags.iter().all(|parts| parts[0] != "p"));

        let forward = forward_from(&tags);
        let envelope = ForwardEnvelope::parse(&forward).expect("parse");

        assert_eq!(envelope.original.id, original.id);
        assert_eq!(envelope.source_channel_id, source_id());
        assert_eq!(envelope.source_type, ForwardSourceType::Channel);
        assert_eq!(
            envelope.quote,
            Some(ForwardQuote {
                event_id: original.id.to_hex(),
                author_pubkey: original.pubkey.to_hex(),
            })
        );
        assert_eq!(envelope.imeta.len(), 1);
        assert_eq!(envelope.imeta[0][0], "imeta");
        assert!(envelope.verify_embedded().is_ok());
    }

    #[test]
    fn private_and_dm_sources_omit_the_quote_tag() {
        let original = original();
        for source_type in [ForwardSourceType::Private, ForwardSourceType::Dm] {
            let tags =
                forward_tags(dest_id(), &original, source_id(), source_type).expect("build tags");
            assert!(tags.iter().all(|parts| parts[0] != FWD_QUOTE_TAG));

            let envelope = ForwardEnvelope::parse(&forward_from(&tags)).expect("parse");
            assert_eq!(envelope.source_type, source_type);
            assert_eq!(envelope.quote, None);
        }
    }

    #[test]
    fn rejects_missing_and_duplicate_fwd_tags() {
        let original = original();
        let tags = open_tags(&original);

        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&drop_tag(&tags, FWD_TAG))),
            Err(ForwardError::MissingFwd)
        );

        let mut doubled = tags.clone();
        doubled.push(
            tags.iter()
                .find(|parts| parts[0] == FWD_TAG)
                .expect("fwd tag")
                .clone(),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&doubled)),
            Err(ForwardError::DuplicateFwd { count: 2 })
        );
    }

    #[test]
    fn rejects_oversized_fwd_tag() {
        let big = sign(
            KIND_STREAM_MESSAGE,
            &"x".repeat(FWD_MAX_BYTES + 16),
            &[owned(&["h", SOURCE_CHANNEL])],
        );

        assert!(matches!(
            forward_tags(dest_id(), &big, source_id(), ForwardSourceType::Channel),
            Err(ForwardError::FwdTooLarge { .. })
        ));

        let tags = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &big.as_json()]),
            owned(&["k", "9"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
        ];
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::FwdTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_unparseable_embedded_json() {
        let original = original();
        let tags = replace_tag(
            &open_tags(&original),
            FWD_TAG,
            owned(&["fwd", "{not an event}"]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::UnparseableEmbedded { .. })
        ));
    }

    #[test]
    fn rejects_non_forwardable_embedded_kind() {
        let inner_forward = sign(
            KIND_STREAM_MESSAGE_FORWARD,
            "note",
            &[owned(&["h", SOURCE_CHANNEL])],
        );

        assert_eq!(
            forward_tags(
                dest_id(),
                &inner_forward,
                source_id(),
                ForwardSourceType::Channel
            ),
            Err(ForwardError::KindNotForwardable {
                kind: KIND_STREAM_MESSAGE_FORWARD
            })
        );

        let tags = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &inner_forward.as_json()]),
            owned(&["k", "40009"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_CHANNEL]),
        ];
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::KindNotForwardable {
                kind: KIND_STREAM_MESSAGE_FORWARD
            })
        );
    }

    #[test]
    fn rejects_missing_or_mismatched_k_tag() {
        let original = original();
        let tags = open_tags(&original);

        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&drop_tag(&tags, FWD_KIND_TAG))),
            Err(ForwardError::MissingKind)
        );

        let mismatched = replace_tag(&tags, FWD_KIND_TAG, owned(&["k", "40002"]));
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&mismatched)),
            Err(ForwardError::KindMismatch {
                declared: "40002".to_string(),
                embedded: KIND_STREAM_MESSAGE,
            })
        );
    }

    #[test]
    fn rejects_malformed_fwd_src_tags() {
        let original = original();
        let tags = open_tags(&original);

        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&drop_tag(&tags, FWD_SRC_TAG))),
            Err(ForwardError::MissingSource)
        );

        let no_label = replace_tag(&tags, FWD_SRC_TAG, owned(&["fwd-src", SOURCE_CHANNEL]));
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&no_label)),
            Err(ForwardError::MalformedSource { .. })
        ));

        let bad_uuid = replace_tag(
            &tags,
            FWD_SRC_TAG,
            owned(&["fwd-src", "not-a-uuid", FWD_SRC_TYPE_CHANNEL]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&bad_uuid)),
            Err(ForwardError::MalformedSource { .. })
        ));

        let bad_label = replace_tag(
            &tags,
            FWD_SRC_TAG,
            owned(&["fwd-src", SOURCE_CHANNEL, "secret"]),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&bad_label)),
            Err(ForwardError::UnknownSourceType {
                label: "secret".to_string()
            })
        );
    }

    #[test]
    fn rejects_fwd_src_uuid_that_differs_from_embedded_h_tag() {
        let original = original();
        let tags = replace_tag(
            &open_tags(&original),
            FWD_SRC_TAG,
            owned(&["fwd-src", DEST_CHANNEL, FWD_SRC_TYPE_CHANNEL]),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::SourceChannelMismatch {
                declared: dest_id(),
                embedded: source_id(),
            })
        );

        assert_eq!(
            forward_tags(dest_id(), &original, dest_id(), ForwardSourceType::Channel),
            Err(ForwardError::SourceChannelMismatch {
                declared: dest_id(),
                embedded: source_id(),
            })
        );
    }

    #[test]
    fn rejects_embedded_original_without_h_tag() {
        let orphan = sign(KIND_STREAM_MESSAGE, "no channel", &[]);
        assert_eq!(
            forward_tags(dest_id(), &orphan, source_id(), ForwardSourceType::Channel),
            Err(ForwardError::EmbeddedMissingChannel)
        );
    }

    #[test]
    fn rejects_quote_mismatch_and_disallowed_quote() {
        let original = original();
        let tags = open_tags(&original);

        let bad_id = replace_tag(
            &tags,
            FWD_QUOTE_TAG,
            owned(&["q", &"a".repeat(64), "", &original.pubkey.to_hex()]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&bad_id)),
            Err(ForwardError::QuoteMismatch { field: "id", .. })
        ));

        let bad_pubkey = replace_tag(
            &tags,
            FWD_QUOTE_TAG,
            owned(&["q", &original.id.to_hex(), "", &"b".repeat(64)]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&bad_pubkey)),
            Err(ForwardError::QuoteMismatch {
                field: "pubkey",
                ..
            })
        ));

        let short = replace_tag(&tags, FWD_QUOTE_TAG, owned(&["q", &original.id.to_hex()]));
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&short)),
            Err(ForwardError::MalformedQuote { .. })
        ));

        let mut doubled = tags.clone();
        doubled.push(
            tags.iter()
                .find(|parts| parts[0] == FWD_QUOTE_TAG)
                .expect("q tag")
                .clone(),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&doubled)),
            Err(ForwardError::DuplicateQuote { count: 2 })
        );

        let private = replace_tag(
            &tags,
            FWD_SRC_TAG,
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&private)),
            Err(ForwardError::QuoteNotAllowed)
        );
    }

    #[test]
    fn rejects_marked_e_tags() {
        let original = original();
        for marker in ["root", "reply"] {
            let mut tags = open_tags(&original);
            tags.push(owned(&["e", &"c".repeat(64), "", marker]));
            assert_eq!(
                ForwardEnvelope::parse(&forward_from(&tags)),
                Err(ForwardError::MarkedETag)
            );
        }

        // Unmarked e tags (quote-style references) are untouched.
        let mut tags = open_tags(&original);
        tags.push(owned(&["e", &"c".repeat(64)]));
        assert!(ForwardEnvelope::parse(&forward_from(&tags)).is_ok());
    }

    /// The outer `imeta` copy is what generic NIP-92 consumers render, so it may
    /// not diverge from the embedded original's.
    #[test]
    fn accepts_matching_imeta_and_rejects_divergence() {
        let original = original();
        let tags = open_tags(&original);
        assert!(ForwardEnvelope::parse(&forward_from(&tags)).is_ok());

        let mut extra = tags.clone();
        extra.push(owned(&["imeta", "url https://example.test/b.png"]));
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&extra)),
            Err(ForwardError::ImetaMismatch {
                outer: 2,
                embedded: 1
            })
        );

        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&drop_tag(&tags, "imeta"))),
            Err(ForwardError::ImetaMismatch {
                outer: 0,
                embedded: 1
            })
        );

        let divergent = replace_tag(
            &tags,
            "imeta",
            owned(&["imeta", "url https://evil.test/a.png", "x abc123"]),
        );
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&divergent)),
            Err(ForwardError::ImetaMismatch {
                outer: 1,
                embedded: 1
            })
        );
    }

    /// Exactly one destination and exactly one source channel — otherwise a
    /// generic NIP-29 consumer could scope the same signed event differently.
    #[test]
    fn rejects_ambiguous_h_tags_on_the_outer_event() {
        let original = original();
        let tags = open_tags(&original);

        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&drop_tag(&tags, "h"))),
            Err(ForwardError::MissingDestination)
        );

        let mut doubled = tags.clone();
        doubled.push(owned(&["h", SOURCE_CHANNEL]));
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&doubled)),
            Err(ForwardError::DuplicateDestination { count: 2 })
        );
    }

    #[test]
    fn rejects_ambiguous_h_tags_on_the_embedded_original() {
        let two_channels = sign(
            KIND_STREAM_MESSAGE,
            "which channel?",
            &[owned(&["h", SOURCE_CHANNEL]), owned(&["h", DEST_CHANNEL])],
        );
        assert_eq!(
            forward_tags(
                dest_id(),
                &two_channels,
                source_id(),
                ForwardSourceType::Channel
            ),
            Err(ForwardError::DuplicateEmbeddedChannel { count: 2 })
        );

        let tags = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &two_channels.as_json()]),
            owned(&["k", "9"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
        ];
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::DuplicateEmbeddedChannel { count: 2 })
        );

        let no_channel = sign(KIND_STREAM_MESSAGE, "orphan", &[]);
        let tags = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &no_channel.as_json()]),
            owned(&["k", "9"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
        ];
        assert_eq!(
            ForwardEnvelope::parse(&forward_from(&tags)),
            Err(ForwardError::EmbeddedMissingChannel)
        );
    }

    /// The `h` shape is pinned to exactly `["h", <uuid>]` on both the outer
    /// event and the embedded original. A trailing element is otherwise
    /// invisible to consumers that only read element 1, so two readers could
    /// disagree about what the same signed event is scoped to.
    #[test]
    fn rejects_h_tags_with_trailing_elements() {
        let original = original();
        let tags = open_tags(&original);

        // The canonical builder output still round-trips.
        assert!(ForwardEnvelope::parse(&forward_from(&tags)).is_ok());

        let trailing_destination = replace_tag(&tags, "h", owned(&["h", DEST_CHANNEL, "extra"]));
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&trailing_destination)),
            Err(ForwardError::MalformedDestination { .. })
        ));

        let trailing_embedded = sign(
            KIND_STREAM_MESSAGE,
            "original text",
            &[
                owned(&["h", SOURCE_CHANNEL, "extra"]),
                owned(&["imeta", "url https://example.test/a.png", "x abc123"]),
            ],
        );
        assert!(matches!(
            forward_tags(
                dest_id(),
                &trailing_embedded,
                source_id(),
                ForwardSourceType::Channel
            ),
            Err(ForwardError::MalformedEmbeddedChannel { .. })
        ));

        let embedded = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &trailing_embedded.as_json()]),
            owned(&["k", "9"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
            owned(&["imeta", "url https://example.test/a.png", "x abc123"]),
        ];
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&embedded)),
            Err(ForwardError::MalformedEmbeddedChannel { .. })
        ));
    }

    /// The `q` tag shape is pinned: exactly 4 elements, empty relay hint, hex
    /// id and pubkey.
    #[test]
    fn rejects_q_tags_with_the_wrong_shape() {
        let original = original();
        let tags = open_tags(&original);
        let id = original.id.to_hex();
        let pubkey = original.pubkey.to_hex();

        let trailing = replace_tag(
            &tags,
            FWD_QUOTE_TAG,
            owned(&["q", &id, "", &pubkey, "extra"]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&trailing)),
            Err(ForwardError::MalformedQuote { .. })
        ));

        let hinted = replace_tag(
            &tags,
            FWD_QUOTE_TAG,
            owned(&["q", &id, "wss://elsewhere.test", &pubkey]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&hinted)),
            Err(ForwardError::MalformedQuote { .. })
        ));

        let not_hex = replace_tag(
            &tags,
            FWD_QUOTE_TAG,
            owned(&["q", &"z".repeat(64), "", &pubkey]),
        );
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&not_hex)),
            Err(ForwardError::MalformedQuote { .. })
        ));

        let short_pubkey = replace_tag(&tags, FWD_QUOTE_TAG, owned(&["q", &id, "", "abcd"]));
        assert!(matches!(
            ForwardEnvelope::parse(&forward_from(&short_pubkey)),
            Err(ForwardError::MalformedQuote { .. })
        ));

        // The canonical builder output still round-trips.
        assert!(ForwardEnvelope::parse(&forward_from(&tags)).is_ok());
    }

    #[test]
    fn verify_embedded_rejects_a_tampered_original() {
        let original = original();
        let mut json: serde_json::Value =
            serde_json::from_str(&original.as_json()).expect("parse json");
        json["content"] = serde_json::Value::String("tampered".to_string());
        let tampered = Event::from_json(json.to_string()).expect("parse event");

        let tags = vec![
            owned(&["h", DEST_CHANNEL]),
            owned(&["fwd", &tampered.as_json()]),
            owned(&["k", "9"]),
            owned(&["fwd-src", SOURCE_CHANNEL, FWD_SRC_TYPE_PRIVATE]),
            owned(&["imeta", "url https://example.test/a.png", "x abc123"]),
        ];
        let envelope = ForwardEnvelope::parse(&forward_from(&tags)).expect("parse");
        assert!(matches!(
            envelope.verify_embedded(),
            Err(VerificationError::InvalidId { .. })
        ));
    }
}
