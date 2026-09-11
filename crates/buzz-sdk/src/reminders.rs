//! Encrypted, author-owned event reminders (NIP-ER).

use std::collections::HashMap;

use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{kind::KIND_EVENT_REMINDER, SdkError};

#[path = "reminders_json.rs"]
mod strict_json;

/// An author's disposition of a reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Awaiting the author's follow-up.
    Pending,
    /// Acknowledged or completed by its author.
    Done,
    /// No longer wanted by its author.
    Cancelled,
}

/// Private reminder content. Unknown fields survive updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    /// Current author-owned disposition.
    pub status: Status,
    /// Optional NIP-ER target; also accepts Buzz Desktop's eventId/channelId shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
    /// Context and reason for returning to this work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Forward-compatible private fields.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

/// A validated reminder head, decrypted for its owner.
#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    /// The opaque address identifier (`d`), stable across replacements.
    pub id: String,
    /// Signed event ID for this particular version.
    pub event_id: String,
    /// Version timestamp, not the due time.
    pub created_at: u64,
    /// Earliest permissible delivery time, in Unix seconds.
    pub not_before: Option<u64>,
    /// Optional NIP-40 expiration; expired reminders must not wake their owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<u64>,
    /// Decrypted private content.
    pub content: Content,
}

fn invalid(message: &str) -> SdkError {
    SdkError::InvalidInput(message.into())
}

/// Whether a NIP-11 document advertises the private reminder read contract.
pub fn relay_supports_private_reminders(info: &Value) -> bool {
    info.get("supported_extensions")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item == "nip-er"))
        && info
            .get("supported_nips")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_u64() == Some(42)))
}

fn hex_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn validate_content(content: &Content) -> Result<(), SdkError> {
    if ["status", "target", "note"]
        .iter()
        .any(|field| content.extra.contains_key(*field))
    {
        return Err(invalid(
            "extra reminder fields conflict with known content fields",
        ));
    }
    let mut has_reference = false;
    if let Some(target) = &content.target {
        let target = target
            .as_object()
            .ok_or_else(|| invalid("reminder target must be an object"))?;
        for field in ["id", "eventId"] {
            if let Some(value) = target.get(field) {
                if !value.as_str().is_some_and(hex_id) {
                    return Err(invalid("invalid reminder target event ID"));
                }
                has_reference = true;
            }
        }
        if let (Some(id), Some(legacy)) = (target.get("id"), target.get("eventId")) {
            if id != legacy {
                return Err(invalid("conflicting reminder target IDs"));
            }
        }
        if let Some(address) = target.get("a") {
            let address = address
                .as_str()
                .ok_or_else(|| invalid("invalid reminder target address"))?;
            nostr::nips::nip01::Coordinate::parse(address)
                .map_err(|_| invalid("invalid reminder target address"))?;
            has_reference = true;
        }
        for field in ["preview", "channelId", "authorPubkey"] {
            if target.get(field).is_some_and(|value| !value.is_string()) {
                return Err(invalid("invalid reminder target text"));
            }
        }
        if target.get("relays").is_some_and(|value| !value.is_array()) {
            return Err(invalid("reminder relays must be an array"));
        }
    }
    if content.status == Status::Pending
        && !has_reference
        && content.note.as_deref().is_none_or(|n| n.trim().is_empty())
    {
        return Err(invalid(
            "a pending reminder needs a target or a non-empty note",
        ));
    }
    Ok(())
}

fn single_tag<'a>(event: &'a Event, name: &str) -> Result<Option<&'a str>, SdkError> {
    let mut tags = event.tags.iter().filter(|tag| tag.kind().as_str() == name);
    let value = tags.next();
    if tags.next().is_some() {
        return Err(invalid("duplicate reminder tag"));
    }
    value
        .map(|tag| tag.content().ok_or_else(|| invalid("empty reminder tag")))
        .transpose()
}

/// Parse the exact integer format required for NIP-ER due times.
pub fn parse_not_before(raw: &str) -> Result<u64, SdkError> {
    if raw.is_empty()
        || !raw.bytes().all(|c| c.is_ascii_digit())
        || (raw.len() > 1 && raw.starts_with('0'))
    {
        return Err(invalid("malformed not_before"));
    }
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or_else(|| invalid("not_before out of range"))
}

impl Reminder {
    /// Verify the signature and owner before decrypting a reminder.
    pub fn decrypt(event: &Event, keys: &Keys) -> Result<Self, SdkError> {
        if event.kind.as_u16() != KIND_EVENT_REMINDER as u16 || event.pubkey != keys.public_key() {
            return Err(invalid("reminder kind or owner mismatch"));
        }
        event
            .verify()
            .map_err(|_| invalid("invalid reminder signature"))?;
        let id = single_tag(event, "d")?
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("reminder needs one non-empty d tag"))?;
        let not_before = single_tag(event, "not_before")?
            .map(parse_not_before)
            .transpose()?;
        let plaintext = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
            .map_err(|_| invalid("cannot decrypt reminder"))?;
        let value = strict_json::parse(&plaintext).map_err(|_| invalid("invalid reminder JSON"))?;
        if value.get("note").is_some_and(|note| !note.is_string())
            || value
                .get("target")
                .is_some_and(|target| !target.is_object())
        {
            return Err(invalid("invalid reminder note or target"));
        }
        let content: Content =
            serde_json::from_value(value).map_err(|_| invalid("invalid reminder content"))?;
        validate_content(&content)?;
        Ok(Self {
            id: id.into(),
            event_id: event.id.to_hex(),
            created_at: event.created_at.as_secs(),
            not_before,
            expiration: single_tag(event, "expiration")?
                .map(parse_not_before)
                .transpose()?,
            content,
        })
    }

    /// Whether this pending head may be delivered at `now`.
    pub fn is_due(&self, now: u64) -> bool {
        self.content.status == Status::Pending
            && self.not_before.is_some_and(|time| time <= now)
            && self.expiration.is_none_or(|time| time > now)
    }
}

/// Select signed current heads before decrypting, so an invalid newer version
/// cannot resurrect an older pending reminder.
pub fn current_heads(events: Vec<Event>, keys: &Keys) -> Vec<Reminder> {
    let mut heads: HashMap<String, Event> = HashMap::new();
    for event in events {
        if event.pubkey != keys.public_key()
            || event.kind.as_u16() != KIND_EVENT_REMINDER as u16
            || event.verify().is_err()
        {
            continue;
        }
        let Ok(Some(id)) = single_tag(&event, "d") else {
            continue;
        };
        let replace = heads.get(id).is_none_or(|previous| {
            event.created_at > previous.created_at
                || (event.created_at == previous.created_at && event.id < previous.id)
        });
        if replace {
            heads.insert(id.into(), event);
        }
    }
    let mut reminders: Vec<_> = heads
        .values()
        .filter_map(|event| Reminder::decrypt(event, keys).ok())
        .collect();
    reminders.sort_by_key(|reminder| (reminder.not_before, reminder.id.clone()));
    reminders
}

/// Generate an opaque address with more than 128 bits of random entropy.
pub fn new_id() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Build a self-encrypted reminder; callers add community/auth tags and sign.
/// Terminal updates omit `not_before` and expire after 30–90 days.
pub fn build(
    keys: &Keys,
    id: &str,
    content: &Content,
    not_before: Option<u64>,
    created_at: u64,
) -> Result<EventBuilder, SdkError> {
    validate_content(content)?;
    if id.is_empty() {
        return Err(invalid("empty reminder ID"));
    }
    if content.status != Status::Pending && not_before.is_some() {
        return Err(invalid("terminal reminders must omit not_before"));
    }
    let plaintext =
        serde_json::to_string(content).map_err(|_| invalid("cannot serialize reminder"))?;
    let encrypted = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|_| invalid("cannot encrypt reminder"))?;
    let mut tags = vec![Tag::identifier(id), Tag::alt("Encrypted reminder")];
    if let Some(time) = not_before {
        parse_not_before(&time.to_string())?;
        tags.push(
            Tag::parse(["not_before", &time.to_string()])
                .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
        );
    } else if content.status != Status::Pending {
        let jitter = uuid::Uuid::new_v4().as_u128() % (60 * 86_400);
        tags.push(Tag::expiration(Timestamp::from(
            created_at.saturating_add(30 * 86_400 + jitter as u64),
        )));
    }
    Ok(
        EventBuilder::new(Kind::Custom(KIND_EVENT_REMINDER as u16), encrypted)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at)),
    )
}

#[cfg(test)]
#[path = "reminders_tests.rs"]
mod tests;
