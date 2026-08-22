//! NIP-AH: encrypted agent-to-agent handoff records.
//!
//! A handoff is a durable, sender-authored record encrypted specifically for
//! one receiving agent. It carries a curated task transcript and transition
//! notes, not hidden model reasoning or an open-ended grant to future activity.

use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};

use crate::kind::KIND_AGENT_HANDOFF;
use crate::observer::{
    content_looks_like_nip44, decrypt_observer_payload, encrypt_observer_payload,
    ObserverPayloadError,
};

/// Current handoff payload schema version.
pub const HANDOFF_VERSION: u8 = 1;
/// Maximum UTF-8 bytes accepted for a handoff title.
pub const MAX_TITLE_BYTES: usize = 200;
/// Maximum UTF-8 bytes accepted for the optional summary.
pub const MAX_SUMMARY_BYTES: usize = 4_000;
/// Maximum UTF-8 bytes accepted for the curated history body.
pub const MAX_HISTORY_BYTES: usize = 56_000;

/// Decrypted handoff content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffPayload {
    /// Payload schema version.
    pub version: u8,
    /// Short human-readable task name.
    pub title: String,
    /// Optional executive summary for quick scanning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Curated Markdown transcript and transition notes.
    pub history: String,
}

impl AgentHandoffPayload {
    /// Validate payload version and bounded text fields.
    pub fn validate(&self) -> Result<(), ObserverPayloadError> {
        if self.version != HANDOFF_VERSION {
            return Err(ObserverPayloadError::InvalidPayload(format!(
                "unsupported handoff version {}",
                self.version
            )));
        }
        validate_text("title", &self.title, 1, MAX_TITLE_BYTES)?;
        if let Some(summary) = &self.summary {
            validate_text("summary", summary, 1, MAX_SUMMARY_BYTES)?;
        }
        validate_text("history", &self.history, 1, MAX_HISTORY_BYTES)
    }
}

fn validate_text(
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), ObserverPayloadError> {
    let bytes = value.as_bytes().len();
    if value.trim().is_empty() || bytes < min || bytes > max {
        return Err(ObserverPayloadError::InvalidPayload(format!(
            "{field} must contain {min}..={max} UTF-8 bytes (got {bytes})"
        )));
    }
    Ok(())
}

/// Build and sign a handoff event encrypted to `recipient`.
pub fn build_agent_handoff_event(
    sender_keys: &Keys,
    recipient: &PublicKey,
    payload: &AgentHandoffPayload,
) -> Result<Event, ObserverPayloadError> {
    payload.validate()?;
    let ciphertext = encrypt_observer_payload(sender_keys, recipient, payload)?;
    EventBuilder::new(Kind::Custom(KIND_AGENT_HANDOFF as u16), ciphertext)
        .tags([
            Tag::public_key(*recipient),
            Tag::parse(["handoff", &HANDOFF_VERSION.to_string()]).map_err(|error| {
                ObserverPayloadError::InvalidPayload(format!("invalid handoff tag: {error}"))
            })?,
        ])
        .sign_with_keys(sender_keys)
        .map_err(|error| {
            ObserverPayloadError::InvalidPayload(format!("failed to sign handoff: {error}"))
        })
}

/// Decrypt and validate a handoff addressed to `recipient_keys`.
pub fn decrypt_agent_handoff(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<AgentHandoffPayload, ObserverPayloadError> {
    validate_agent_handoff_envelope(event)?;
    let payload: AgentHandoffPayload = decrypt_observer_payload(recipient_keys, event)?;
    payload.validate()?;
    Ok(payload)
}

/// Validate the public envelope without decrypting its content.
pub fn validate_agent_handoff_envelope(event: &Event) -> Result<(), ObserverPayloadError> {
    if event.kind.as_u16() as u32 != KIND_AGENT_HANDOFF {
        return Err(ObserverPayloadError::InvalidPayload(
            "not an agent handoff event".to_string(),
        ));
    }
    if !content_looks_like_nip44(&event.content) {
        return Err(ObserverPayloadError::InvalidCiphertextLength(
            event.content.len(),
        ));
    }
    let p_tags = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "p")
        .count();
    if p_tags != 1 {
        return Err(ObserverPayloadError::InvalidPayload(format!(
            "handoff requires exactly one p tag (got {p_tags})"
        )));
    }
    let version_ok = event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() == 2
            && parts[0].as_str() == "handoff"
            && parts[1].as_str() == HANDOFF_VERSION.to_string()
    });
    if !version_ok {
        return Err(ObserverPayloadError::InvalidPayload(
            "handoff version tag is missing or unsupported".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentHandoffPayload {
        AgentHandoffPayload {
            version: HANDOFF_VERSION,
            title: "Continue attachment previews".to_string(),
            summary: Some("Markdown and PDF work; CSV needs tests.".to_string()),
            history: "## Completed\n- Added preview routing\n\n## Next\n- Test CSV".to_string(),
        }
    }

    #[test]
    fn handoff_round_trips_only_for_recipient() {
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let unrelated = Keys::generate();
        let event = build_agent_handoff_event(&sender, &recipient.public_key(), &sample())
            .expect("build handoff");

        assert_eq!(decrypt_agent_handoff(&recipient, &event).unwrap(), sample());
        assert!(decrypt_agent_handoff(&unrelated, &event).is_err());
    }

    #[test]
    fn rejects_blank_or_oversized_history() {
        let mut payload = sample();
        payload.history = "   ".to_string();
        assert!(payload.validate().is_err());
        payload.history = "x".repeat(MAX_HISTORY_BYTES + 1);
        assert!(payload.validate().is_err());
    }

    #[test]
    fn envelope_requires_exactly_one_recipient() {
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let event = EventBuilder::new(
            Kind::Custom(KIND_AGENT_HANDOFF as u16),
            encrypt_observer_payload(&sender, &recipient.public_key(), &sample()).unwrap(),
        )
        .tags([Tag::parse(["handoff", "1"]).unwrap()])
        .sign_with_keys(&sender)
        .unwrap();
        assert!(validate_agent_handoff_envelope(&event).is_err());
    }
}
