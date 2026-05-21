//! Agent observer frame helpers.
//!
//! Observer frames are transient, owner-scoped agent telemetry/control messages.
//! They use a Sprout ephemeral event kind and carry NIP-44 encrypted JSON in the
//! event content so relays can route frames without reading ACP internals.

use nostr::{nips::nip44, Event, Keys, PublicKey};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

/// Tag name that identifies the agent pubkey the observer frame belongs to.
pub const OBSERVER_AGENT_TAG: &str = "agent";
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
}

/// Returns true when `content` fits the NIP-44 v2 ciphertext length envelope.
pub fn content_looks_like_nip44(content: &str) -> bool {
    (NIP44_MIN_CONTENT_LEN..=NIP44_MAX_CONTENT_LEN).contains(&content.len())
}

/// Serialize and NIP-44 encrypt an observer payload for `recipient`.
pub fn encrypt_observer_payload<T: Serialize>(
    sender_keys: &Keys,
    recipient: &PublicKey,
    payload: &T,
) -> Result<String, ObserverPayloadError> {
    let mut plaintext = serde_json::to_string(payload)?;
    if plaintext.len() > OBSERVER_MAX_PLAINTEXT_LEN {
        let got = plaintext.len();
        plaintext.zeroize();
        return Err(ObserverPayloadError::PlaintextTooLarge {
            max: OBSERVER_MAX_PLAINTEXT_LEN,
            got,
        });
    }

    let encrypted = nip44::encrypt(
        sender_keys.secret_key(),
        recipient,
        &plaintext,
        nip44::Version::V2,
    )?;
    plaintext.zeroize();
    Ok(encrypted)
}

/// NIP-44 decrypt and deserialize an observer payload from `event`.
pub fn decrypt_observer_payload<T: DeserializeOwned>(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<T, ObserverPayloadError> {
    if !content_looks_like_nip44(&event.content) {
        return Err(ObserverPayloadError::InvalidCiphertextLength(
            event.content.len(),
        ));
    }

    let mut plaintext = nip44::decrypt(
        recipient_keys.secret_key(),
        &event.pubkey,
        event.content.as_str(),
    )?;
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
