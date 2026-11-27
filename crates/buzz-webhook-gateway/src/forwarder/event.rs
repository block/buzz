//! Nostr events for queued messages, thread replies and edits.
use crate::config::RelayScheme;
use crate::store::{ClaimedMessage, GroupKeyAction};
use nostr::Url;
use nostr::{Event, EventBuilder, EventId, Kind, NostrSigner, PublicKey, Tag, TagKind, Timestamp};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// NIP-29 channel message kind.
pub const CHANNEL_MESSAGE_KIND: Kind = Kind::Custom(9);

/// Tag carrying [`message_tag`]. Multi-letter, so relays do not index it.
pub const MESSAGE_TAG: &str = "webhook";

/// An opaque, stable value unique to one outbox row: a hash of its key.
///
/// Two messages can share their author, channel, text and timestamp, such as
/// two deliveries of one event type in the same second rendered with the
/// default message. The event id covers this tag, so each row still gets its
/// own event instead of the relay storing one and treating the other as its
/// duplicate. Hashing keeps the provider's delivery id out of the channel.
pub fn message_tag(message: &ClaimedMessage) -> String {
    let mut hash = Sha256::new();
    hash.update(b"buzz-webhook-gateway/message\0");
    hash.update(message.key.integration_id.as_bytes());
    hash.update(message.key.subscription_id.as_bytes());
    hash.update(message.key.delivery_id.as_bytes());
    hex::encode(hash.finalize())
}

fn builder(message: &ClaimedMessage) -> EventBuilder {
    let created_at = u64::try_from(message.signed_at.timestamp()).unwrap_or(0);
    let event = match &message.target {
        None => EventBuilder::new(CHANNEL_MESSAGE_KIND, &message.content),
        Some((GroupKeyAction::ReplyInThread, root)) => {
            EventBuilder::new(CHANNEL_MESSAGE_KIND, &message.content)
                .tag(Tag::custom(TagKind::e(), [root.as_str(), "", "reply"]))
        }
        Some((GroupKeyAction::UpdateMessage, root)) => {
            EventBuilder::new(Kind::Custom(40003), &message.content)
                .tag(Tag::custom(TagKind::e(), [root.as_str()]))
        }
    };
    event
        .tag(Tag::custom(TagKind::h(), [message.channel_id.to_string()]))
        .tag(Tag::custom(
            TagKind::custom(MESSAGE_TAG),
            [message_tag(message)],
        ))
        .custom_created_at(Timestamp::from_secs(created_at))
}

/// Build the signed relay event for a claimed message.
///
/// `created_at` is the message's `signed_at`, so every attempt by one author
/// within a stamp produces the same event id, and the relay treats a resend
/// after a lost acknowledgement as a duplicate rather than a second message.
pub async fn build_message_event(
    signer: &Arc<dyn NostrSigner>,
    message: &ClaimedMessage,
) -> Result<Event, nostr::event::builder::Error> {
    builder(message).sign(signer).await
}

/// The id [`build_message_event`] gives `message` when `author` signs it.
pub fn message_event_id(author: PublicKey, message: &ClaimedMessage) -> EventId {
    builder(message).build(author).id()
}

/// A relay endpoint, such as `events` or `query`, of a community.
pub fn relay_url(scheme: RelayScheme, community_host: &str, endpoint: &str) -> Option<Url> {
    let url = Url::parse(&format!(
        "{}://{community_host}/{endpoint}",
        scheme.as_str()
    ))
    .ok()?;
    (url.host_str() == Some(community_host)).then_some(url)
}
