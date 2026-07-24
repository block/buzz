//! NIP-CB encrypted, owner-only Daily Command Brief lifecycle records.
//!
//! The relay can inspect only the fixed public envelope. The owner signs and
//! NIP-44-v2 encrypts the bounded payload to the same keypair.

use chrono::DateTime;
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, Tag};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kind::KIND_COMMAND_BRIEF;

/// Current encrypted payload schema version.
pub const COMMAND_BRIEF_PAYLOAD_VERSION: u32 = 1;
/// Maximum bytes in an identifier or failure code.
pub const MAX_COMMAND_BRIEF_ID_BYTES: usize = 256;
/// Maximum serialized plaintext size.
pub const MAX_COMMAND_BRIEF_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_VALUE_DEPTH: usize = 16;
const MAX_VALUE_ARRAY_ITEMS: usize = 320;
const MAX_VALUE_OBJECT_FIELDS: usize = 64;
const MAX_VALUE_STRING_BYTES: usize = 4096;

/// Closed terminal lifecycle vocabulary persisted by NIP-CB.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandBriefLifecycleState {
    /// A healthy brief completed.
    Completed,
    /// A brief completed with visible degraded inputs.
    Degraded,
    /// Generation was cancelled before a final brief existed.
    Cancelled,
    /// Generation failed with a redacted code.
    Failed,
}

impl CommandBriefLifecycleState {
    fn as_tag(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Degraded => "degraded",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn is_success(self) -> bool {
        matches!(self, Self::Completed | Self::Degraded)
    }
}

/// Redacted terminal failure metadata.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandBriefFailure {
    /// Stable bounded error code; never a provider body or secret.
    pub code: String,
}

/// Decrypted NIP-CB lifecycle payload.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandBriefEventPayload {
    /// Schema version, currently `1`.
    pub version: u32,
    /// Always the literal `OFFICIAL`.
    pub classification: String,
    /// Unique run identity.
    pub run_id: String,
    /// Schedule identity.
    pub schedule_id: String,
    /// Terminal lifecycle state.
    pub lifecycle_state: CommandBriefLifecycleState,
    /// RFC3339 occurrence timestamp.
    pub occurred_at: String,
    /// Frozen signed knowledge snapshot identity.
    pub frozen_snapshot_id: String,
    /// Validated final brief for completed/degraded states.
    pub final_brief: Option<Value>,
    /// Redacted metadata for failed/cancelled states.
    pub failure: Option<CommandBriefFailure>,
    /// Exact preceding lifecycle event ID, when one exists.
    pub previous_lifecycle_event_id: Option<String>,
}

/// Fail-closed NIP-CB build/decrypt error without sensitive payload detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CommandBriefEventError {
    /// Public envelope or decrypted contract was invalid.
    #[error("invalid command brief event")]
    Invalid,
    /// Cryptographic processing failed.
    #[error("command brief cryptography failed")]
    Cryptography,
}

impl CommandBriefEventPayload {
    /// Validate the bounded terminal payload and predecessor relationship.
    pub fn validate(&self) -> Result<(), CommandBriefEventError> {
        if self.version != COMMAND_BRIEF_PAYLOAD_VERSION
            || self.classification != "OFFICIAL"
            || !valid_identifier(&self.run_id)
            || !valid_identifier(&self.schedule_id)
            || !valid_identifier(&self.frozen_snapshot_id)
            || DateTime::parse_from_rfc3339(&self.occurred_at).is_err()
            || self
                .previous_lifecycle_event_id
                .as_deref()
                .is_some_and(|id| !is_event_id(id))
        {
            return Err(CommandBriefEventError::Invalid);
        }
        if self.lifecycle_state.is_success() {
            if self.failure.is_some() || self.final_brief.is_none() {
                return Err(CommandBriefEventError::Invalid);
            }
        } else if self.final_brief.is_some() || self.failure.is_none() {
            return Err(CommandBriefEventError::Invalid);
        }
        if let Some(failure) = &self.failure {
            if !valid_identifier(&failure.code) {
                return Err(CommandBriefEventError::Invalid);
            }
        }
        if let Some(brief) = &self.final_brief {
            validate_json_value(brief, 0)?;
        }
        let serialized = serde_json::to_vec(self).map_err(|_| CommandBriefEventError::Invalid)?;
        if serialized.len() > MAX_COMMAND_BRIEF_PAYLOAD_BYTES {
            return Err(CommandBriefEventError::Invalid);
        }
        Ok(())
    }
}

/// Encrypt, envelope, and sign one append-only owner-to-self lifecycle event.
pub fn build_command_brief_event(
    owner_keys: &Keys,
    payload: &CommandBriefEventPayload,
) -> Result<Event, CommandBriefEventError> {
    payload.validate()?;
    let plaintext = serde_json::to_string(payload).map_err(|_| CommandBriefEventError::Invalid)?;
    let owner = owner_keys.public_key();
    let ciphertext = nip44::encrypt(
        owner_keys.secret_key(),
        &owner,
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|_| CommandBriefEventError::Cryptography)?;
    let mut tags = vec![
        Tag::public_key(owner),
        parse_tag(["d", payload.run_id.as_str()])?,
        parse_tag(["status", payload.lifecycle_state.as_tag()])?,
    ];
    if let Some(previous) = &payload.previous_lifecycle_event_id {
        tags.push(parse_tag(["previous", previous.as_str()])?);
    }
    EventBuilder::new(Kind::Custom(KIND_COMMAND_BRIEF as u16), ciphertext)
        .tags(tags)
        .allow_self_tagging()
        .sign_with_keys(owner_keys)
        .map_err(|_| CommandBriefEventError::Cryptography)
}

/// Prove current-key ownership, decrypt, and validate a signed NIP-CB event.
pub fn decrypt_command_brief_event(
    owner_keys: &Keys,
    event: &Event,
) -> Result<CommandBriefEventPayload, CommandBriefEventError> {
    let envelope = validate_public_envelope(event)?;
    let owner = owner_keys.public_key();
    if event.pubkey != owner || envelope.owner != owner.to_hex() {
        return Err(CommandBriefEventError::Invalid);
    }
    let plaintext = nip44::decrypt(owner_keys.secret_key(), &owner, &event.content)
        .map_err(|_| CommandBriefEventError::Cryptography)?;
    if plaintext.len() > MAX_COMMAND_BRIEF_PAYLOAD_BYTES {
        return Err(CommandBriefEventError::Invalid);
    }
    let payload: CommandBriefEventPayload =
        serde_json::from_str(&plaintext).map_err(|_| CommandBriefEventError::Invalid)?;
    payload.validate()?;
    if payload.run_id != envelope.run_id
        || payload.lifecycle_state.as_tag() != envelope.status
        || payload.previous_lifecycle_event_id.as_deref() != envelope.previous.as_deref()
    {
        return Err(CommandBriefEventError::Invalid);
    }
    Ok(payload)
}

struct PublicEnvelope {
    owner: String,
    run_id: String,
    status: String,
    previous: Option<String>,
}

fn validate_public_envelope(event: &Event) -> Result<PublicEnvelope, CommandBriefEventError> {
    if event.kind != Kind::Custom(KIND_COMMAND_BRIEF as u16) {
        return Err(CommandBriefEventError::Invalid);
    }
    let mut owner = None;
    let mut run_id = None;
    let mut status = None;
    let mut previous = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() != 2 {
            return Err(CommandBriefEventError::Invalid);
        }
        let target = match parts[0].as_str() {
            "p" => &mut owner,
            "d" => &mut run_id,
            "status" => &mut status,
            "previous" => &mut previous,
            _ => return Err(CommandBriefEventError::Invalid),
        };
        if target.replace(parts[1].clone()).is_some() {
            return Err(CommandBriefEventError::Invalid);
        }
    }
    let owner = owner.ok_or(CommandBriefEventError::Invalid)?;
    let run_id = run_id.ok_or(CommandBriefEventError::Invalid)?;
    let status = status.ok_or(CommandBriefEventError::Invalid)?;
    if !is_event_id(&owner)
        || !valid_identifier(&run_id)
        || !matches!(
            status.as_str(),
            "completed" | "degraded" | "cancelled" | "failed"
        )
        || previous.as_deref().is_some_and(|id| !is_event_id(id))
    {
        return Err(CommandBriefEventError::Invalid);
    }
    Ok(PublicEnvelope {
        owner,
        run_id,
        status,
        previous,
    })
}

fn parse_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, CommandBriefEventError> {
    Tag::parse(parts).map_err(|_| CommandBriefEventError::Invalid)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_COMMAND_BRIEF_ID_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn is_event_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_json_value(value: &Value, depth: usize) -> Result<(), CommandBriefEventError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(CommandBriefEventError::Invalid);
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(value) if value.len() <= MAX_VALUE_STRING_BYTES => Ok(()),
        Value::String(_) => Err(CommandBriefEventError::Invalid),
        Value::Array(values) if values.len() <= MAX_VALUE_ARRAY_ITEMS => values
            .iter()
            .try_for_each(|value| validate_json_value(value, depth + 1)),
        Value::Array(_) => Err(CommandBriefEventError::Invalid),
        Value::Object(values) if values.len() <= MAX_VALUE_OBJECT_FIELDS => {
            for (key, value) in values {
                let normalized: String = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                if matches!(
                    normalized.as_str(),
                    "prompt"
                        | "systemprompt"
                        | "reasoning"
                        | "credentials"
                        | "credential"
                        | "bearertoken"
                        | "accesstoken"
                        | "apikey"
                        | "secret"
                ) || key.len() > MAX_VALUE_STRING_BYTES
                {
                    return Err(CommandBriefEventError::Invalid);
                }
                validate_json_value(value, depth + 1)?;
            }
            Ok(())
        }
        Value::Object(_) => Err(CommandBriefEventError::Invalid),
    }
}
