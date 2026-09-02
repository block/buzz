//! Agent observer frame helpers.
//!
//! Observer frames are transient encrypted agent telemetry/control messages.
//! Owner-only frames use direct NIP-44. Shared telemetry encrypts the payload
//! once to an ephemeral key and NIP-44-wraps that key for each authorized
//! channel member, so adding viewers does not duplicate the full payload.

use std::collections::HashSet;

use nostr::{nips::nip44, Event, Keys, PublicKey, SecretKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

/// Tag name that identifies the agent pubkey the observer frame belongs to.
pub const OBSERVER_AGENT_TAG: &str = "agent";
/// Tag name that identifies the managed agent's controlling owner.
pub const OBSERVER_OWNER_TAG: &str = "owner";
/// Tag name that identifies the cleartext frame direction.
pub const OBSERVER_FRAME_TAG: &str = "frame";
/// Frame value for agent-to-owner observer telemetry.
pub const OBSERVER_FRAME_TELEMETRY: &str = "telemetry";
/// Frame value for owner-to-agent observer control commands.
pub const OBSERVER_FRAME_CONTROL: &str = "control";
/// Minimum plausible NIP-44 v2 ciphertext length.
pub const NIP44_MIN_CONTENT_LEN: usize = 132;
/// Maximum NIP-44 v2 ciphertext length.
pub const NIP44_MAX_CONTENT_LEN: usize = 87_472;
/// Maximum observer plaintext JSON size accepted by helpers.
pub const OBSERVER_MAX_PLAINTEXT_LEN: usize = 65_535;
/// Maximum recipients carried by one shared observer frame.
pub const OBSERVER_MAX_RECIPIENTS: usize = 128;
const OBSERVER_ENVELOPE_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObserverPayloadEnvelope {
    version: u8,
    ciphertext: String,
    wrapped_keys: Vec<String>,
}

/// Errors returned by observer payload encryption/decryption helpers.
#[derive(Debug, Error)]
pub enum ObserverPayloadError {
    /// NIP-44 encryption or decryption failed.
    #[error("NIP-44 error: {0}")]
    Nip44(#[from] nip44::Error),
    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Ciphertext did not fit the expected NIP-44 v2 length envelope.
    #[error("invalid NIP-44 ciphertext length: {0}")]
    InvalidCiphertextLength(usize),
    /// Decrypted JSON exceeded the observer plaintext size limit.
    #[error("observer plaintext exceeds {max} bytes (got {got})")]
    PlaintextTooLarge {
        /// Maximum accepted plaintext bytes.
        max: usize,
        /// Actual plaintext byte count.
        got: usize,
    },
    /// A payload field violated a NIP-AM numeric constraint.
    #[error("invalid payload field: {0}")]
    InvalidPayload(String),
}

/// Returns true when `content` fits the NIP-44 v2 ciphertext length envelope.
pub fn content_looks_like_nip44(content: &str) -> bool {
    !content.starts_with('{')
        && (NIP44_MIN_CONTENT_LEN..=NIP44_MAX_CONTENT_LEN).contains(&content.len())
}

fn parse_observer_envelope(content: &str) -> Result<ObserverPayloadEnvelope, ObserverPayloadError> {
    let envelope: ObserverPayloadEnvelope = serde_json::from_str(content)?;
    if envelope.version != OBSERVER_ENVELOPE_VERSION {
        return Err(ObserverPayloadError::InvalidPayload(format!(
            "unsupported observer envelope version {}",
            envelope.version
        )));
    }
    if envelope.wrapped_keys.is_empty() || envelope.wrapped_keys.len() > OBSERVER_MAX_RECIPIENTS {
        return Err(ObserverPayloadError::InvalidPayload(format!(
            "observer envelope must carry 1..={OBSERVER_MAX_RECIPIENTS} wrapped keys"
        )));
    }
    if !content_looks_like_nip44(&envelope.ciphertext)
        || envelope
            .wrapped_keys
            .iter()
            .any(|wrapped| !content_looks_like_nip44(wrapped))
    {
        return Err(ObserverPayloadError::InvalidPayload(
            "observer envelope contains invalid NIP-44 ciphertext".into(),
        ));
    }
    Ok(envelope)
}

/// Return how many recipients an encrypted observer payload targets.
///
/// Direct NIP-44 payloads target one recipient. Shared envelopes carry one
/// wrapped content key per ordered `p` tag.
pub fn observer_payload_recipient_count(content: &str) -> Result<usize, ObserverPayloadError> {
    if content.starts_with('{') {
        return Ok(parse_observer_envelope(content)?.wrapped_keys.len());
    }
    if content_looks_like_nip44(content) {
        return Ok(1);
    }
    Err(ObserverPayloadError::InvalidCiphertextLength(content.len()))
}

/// Returns true for either direct NIP-44 or a valid shared observer envelope.
pub fn content_looks_like_observer_payload(content: &str) -> bool {
    observer_payload_recipient_count(content).is_ok()
}

fn serialize_observer_payload<T: Serialize>(payload: &T) -> Result<String, ObserverPayloadError> {
    let plaintext = serde_json::to_string(payload)?;
    if plaintext.len() > OBSERVER_MAX_PLAINTEXT_LEN {
        return Err(ObserverPayloadError::PlaintextTooLarge {
            max: OBSERVER_MAX_PLAINTEXT_LEN,
            got: plaintext.len(),
        });
    }
    Ok(plaintext)
}

/// Serialize and NIP-44 encrypt an observer payload for `recipient`.
pub fn encrypt_observer_payload<T: Serialize>(
    sender_keys: &Keys,
    recipient: &PublicKey,
    payload: &T,
) -> Result<String, ObserverPayloadError> {
    let mut plaintext = serialize_observer_payload(payload)?;
    let encrypted = nip44::encrypt(
        sender_keys.secret_key(),
        recipient,
        &plaintext,
        nip44::Version::V2,
    );
    plaintext.zeroize();
    Ok(encrypted?)
}

/// Encrypt one observer payload for multiple ordered recipients.
///
/// The payload is encrypted once to a fresh ephemeral key. That key is then
/// NIP-44 encrypted independently for each recipient, in the same order as the
/// frame's `p` tags.
pub fn encrypt_observer_payload_for_recipients<T: Serialize>(
    sender_keys: &Keys,
    recipients: &[PublicKey],
    payload: &T,
) -> Result<String, ObserverPayloadError> {
    if recipients.is_empty() || recipients.len() > OBSERVER_MAX_RECIPIENTS {
        return Err(ObserverPayloadError::InvalidPayload(format!(
            "observer recipient count must be 1..={OBSERVER_MAX_RECIPIENTS}"
        )));
    }
    let mut seen = HashSet::with_capacity(recipients.len());
    if recipients
        .iter()
        .any(|recipient| !seen.insert(recipient.to_bytes()))
    {
        return Err(ObserverPayloadError::InvalidPayload(
            "observer recipients must be unique".into(),
        ));
    }

    let mut plaintext = serialize_observer_payload(payload)?;
    let ephemeral_keys = Keys::generate();
    let ciphertext = nip44::encrypt(
        sender_keys.secret_key(),
        &ephemeral_keys.public_key(),
        &plaintext,
        nip44::Version::V2,
    );
    plaintext.zeroize();
    let ciphertext = ciphertext?;

    let mut ephemeral_secret = ephemeral_keys.secret_key().to_secret_hex();
    let mut wrapped_keys = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        match nip44::encrypt(
            sender_keys.secret_key(),
            recipient,
            &ephemeral_secret,
            nip44::Version::V2,
        ) {
            Ok(wrapped) => wrapped_keys.push(wrapped),
            Err(error) => {
                ephemeral_secret.zeroize();
                return Err(error.into());
            }
        }
    }
    ephemeral_secret.zeroize();

    Ok(serde_json::to_string(&ObserverPayloadEnvelope {
        version: OBSERVER_ENVELOPE_VERSION,
        ciphertext,
        wrapped_keys,
    })?)
}

fn deserialize_observer_plaintext<T: DeserializeOwned>(
    mut plaintext: String,
) -> Result<T, ObserverPayloadError> {
    if plaintext.len() > OBSERVER_MAX_PLAINTEXT_LEN {
        let got = plaintext.len();
        plaintext.zeroize();
        return Err(ObserverPayloadError::PlaintextTooLarge {
            max: OBSERVER_MAX_PLAINTEXT_LEN,
            got,
        });
    }
    let result = serde_json::from_str(&plaintext);
    plaintext.zeroize();
    Ok(result?)
}

/// NIP-44 decrypt and deserialize a direct or shared observer payload.
pub fn decrypt_observer_payload<T: DeserializeOwned>(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<T, ObserverPayloadError> {
    if event.content.starts_with('{') {
        return decrypt_shared_observer_payload(recipient_keys, event);
    }
    if !content_looks_like_nip44(&event.content) {
        return Err(ObserverPayloadError::InvalidCiphertextLength(
            event.content.len(),
        ));
    }
    let plaintext = nip44::decrypt(
        recipient_keys.secret_key(),
        &event.pubkey,
        event.content.as_str(),
    )?;
    deserialize_observer_plaintext(plaintext)
}

fn decrypt_shared_observer_payload<T: DeserializeOwned>(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<T, ObserverPayloadError> {
    let envelope = parse_observer_envelope(&event.content)?;
    let recipient_hex = recipient_keys.public_key().to_hex();
    let mut recipient_index = None;
    let mut p_count = 0usize;
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("p") {
            continue;
        }
        p_count += 1;
        let tagged = values.get(1).ok_or_else(|| {
            ObserverPayloadError::InvalidPayload("observer frame has an empty p tag".into())
        })?;
        PublicKey::from_hex(tagged).map_err(|_| {
            ObserverPayloadError::InvalidPayload(
                "observer frame p tag must contain a hex pubkey".into(),
            )
        })?;
        if tagged.eq_ignore_ascii_case(&recipient_hex)
            && recipient_index.replace(p_count - 1).is_some()
        {
            return Err(ObserverPayloadError::InvalidPayload(
                "observer frame repeats the recipient p tag".into(),
            ));
        }
    }
    if p_count != envelope.wrapped_keys.len() {
        return Err(ObserverPayloadError::InvalidPayload(
            "observer p-tag count does not match wrapped-key count".into(),
        ));
    }
    let index = recipient_index.ok_or_else(|| {
        ObserverPayloadError::InvalidPayload(
            "observer frame has no wrapped key for this recipient".into(),
        )
    })?;

    let mut ephemeral_secret = nip44::decrypt(
        recipient_keys.secret_key(),
        &event.pubkey,
        &envelope.wrapped_keys[index],
    )?;
    let parsed_secret = SecretKey::from_hex(&ephemeral_secret).map_err(|_| {
        ObserverPayloadError::InvalidPayload(
            "observer envelope wrapped an invalid content key".into(),
        )
    });
    ephemeral_secret.zeroize();
    let ephemeral_keys = Keys::new(parsed_secret?);
    let plaintext = nip44::decrypt(
        ephemeral_keys.secret_key(),
        &event.pubkey,
        &envelope.ciphertext,
    )?;
    deserialize_observer_plaintext(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    #[test]
    fn observer_payload_round_trips_with_nip44() {
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let payload = serde_json::json!({
            "type": "turn_started",
            "turnId": "turn-1"
        });
        let encrypted = encrypt_observer_payload(&sender, &recipient.public_key(), &payload)
            .expect("encrypt payload");
        assert!(content_looks_like_nip44(&encrypted));

        let event = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_AGENT_OBSERVER_FRAME as u16),
            encrypted,
        )
        .tags([Tag::public_key(recipient.public_key())])
        .sign_with_keys(&sender)
        .expect("sign event");
        let decrypted: serde_json::Value =
            decrypt_observer_payload(&recipient, &event).expect("decrypt payload");
        assert_eq!(decrypted, payload);
    }

    #[test]
    fn shared_observer_payload_round_trips_for_each_recipient() {
        let sender = Keys::generate();
        let first = Keys::generate();
        let second = Keys::generate();
        let outsider = Keys::generate();
        let recipients = [first.public_key(), second.public_key()];
        let payload = serde_json::json!({
            "type": "tool_call",
            "channelId": "0dd6ba71-0304-4e50-8100-4e9d1224ebd1"
        });
        let encrypted = encrypt_observer_payload_for_recipients(&sender, &recipients, &payload)
            .expect("encrypt shared payload");
        assert!(!content_looks_like_nip44(&encrypted));
        assert!(content_looks_like_observer_payload(&encrypted));
        assert_eq!(
            observer_payload_recipient_count(&encrypted).expect("recipient count"),
            2
        );

        let event = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_AGENT_OBSERVER_FRAME as u16),
            encrypted,
        )
        .tags([
            Tag::public_key(first.public_key()),
            Tag::public_key(second.public_key()),
        ])
        .sign_with_keys(&sender)
        .expect("sign event");

        for recipient in [&first, &second] {
            let decrypted: serde_json::Value =
                decrypt_observer_payload(recipient, &event).expect("decrypt shared payload");
            assert_eq!(decrypted, payload);
        }
        assert!(matches!(
            decrypt_observer_payload::<serde_json::Value>(&outsider, &event),
            Err(ObserverPayloadError::InvalidPayload(message))
                if message.contains("no wrapped key")
        ));
    }

    #[test]
    fn shared_observer_payload_binds_wrapped_keys_to_ordered_p_tags() {
        let sender = Keys::generate();
        let first = Keys::generate();
        let second = Keys::generate();
        let encrypted = encrypt_observer_payload_for_recipients(
            &sender,
            &[first.public_key(), second.public_key()],
            &serde_json::json!({"type": "turn_started"}),
        )
        .expect("encrypt shared payload");
        let event = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_AGENT_OBSERVER_FRAME as u16),
            encrypted,
        )
        .tags([
            Tag::public_key(second.public_key()),
            Tag::public_key(first.public_key()),
        ])
        .sign_with_keys(&sender)
        .expect("sign event");

        assert!(decrypt_observer_payload::<serde_json::Value>(&first, &event).is_err());
        assert!(decrypt_observer_payload::<serde_json::Value>(&second, &event).is_err());
    }

    #[test]
    fn shared_observer_payload_rejects_duplicate_recipients() {
        let sender = Keys::generate();
        let recipient = Keys::generate().public_key();
        assert!(matches!(
            encrypt_observer_payload_for_recipients(
                &sender,
                &[recipient, recipient],
                &serde_json::json!({"type": "turn_started"}),
            ),
            Err(ObserverPayloadError::InvalidPayload(message))
                if message.contains("unique")
        ));
    }

    #[test]
    fn observer_payload_rejects_short_ciphertext() {
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let event = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_AGENT_OBSERVER_FRAME as u16),
            "not encrypted",
        )
        .tags([Tag::public_key(recipient.public_key())])
        .sign_with_keys(&sender)
        .expect("sign event");

        assert!(matches!(
            decrypt_observer_payload::<serde_json::Value>(&recipient, &event),
            Err(ObserverPayloadError::InvalidCiphertextLength(_))
        ));
    }
}
