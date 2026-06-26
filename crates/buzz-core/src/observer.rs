//! Agent observer frame helpers.
//!
//! Observer frames are transient, owner-scoped agent telemetry/control messages.
//! They use a Buzz ephemeral event kind and carry NIP-44 encrypted JSON in the
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

/// Strong syntactic validation that `content` is a plausible NIP-44 v2
/// ciphertext payload, beyond the length-envelope check of
/// [`content_looks_like_nip44`].
///
/// Checks:
/// - Standard base64 alphabet only (`A-Z`, `a-z`, `0-9`, `+`, `/`, `=`), with
///   padding only at the end and total length a multiple of 4.
/// - Decoded length >= 99 bytes (1 version + 32 nonce + 32 MAC + minimum 34
///   bytes of length-prefixed padded ciphertext required by NIP-44 v2).
/// - First decoded byte is `0x02` (NIP-44 version 2).
///
/// This is an envelope sanity check, not full validation: the MAC and actual
/// decryption happen at the reader. The relay cannot decrypt, so this is the
/// strongest guard it can apply to refuse a plaintext-leak ("client forgot to
/// encrypt") without holding any key. Use this — not the length-only
/// [`content_looks_like_nip44`] — anywhere a fail-visible "must be encrypted"
/// boundary is enforced.
pub fn validate_nip44_v2(content: &str) -> Result<(), Nip44SyntaxError> {
    use Nip44SyntaxError::*;
    if content.is_empty() {
        return Err(Empty);
    }
    let bytes = content.as_bytes();
    let len = bytes.len();
    if !len.is_multiple_of(4) {
        return Err(NotBase64);
    }
    let mut pad_count = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                if pad_count > 0 {
                    return Err(NotBase64);
                }
            }
            b'=' => {
                if i < len - 2 {
                    return Err(NotBase64);
                }
                pad_count += 1;
                if pad_count > 2 {
                    return Err(NotBase64);
                }
            }
            _ => return Err(NotBase64),
        }
    }
    let decoded_len = (len / 4) * 3 - pad_count;
    if decoded_len < 99 {
        return Err(TooShort);
    }
    let b64_val = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let v0 = b64_val(bytes[0]).ok_or(NotBase64)?;
    let v1 = b64_val(bytes[1]).ok_or(NotBase64)?;
    let first_byte = (v0 << 2) | (v1 >> 4);
    if first_byte != 0x02 {
        return Err(WrongVersion);
    }
    Ok(())
}

/// Reasons [`validate_nip44_v2`] rejects content as non-NIP-44-v2 ciphertext.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Nip44SyntaxError {
    /// Content was empty.
    #[error("content must not be empty (NIP-44 ciphertext)")]
    Empty,
    /// Content was not valid standard base64.
    #[error("content is not valid base64")]
    NotBase64,
    /// Decoded content was shorter than the NIP-44 v2 minimum (99 bytes).
    #[error("content too short for NIP-44 v2")]
    TooShort,
    /// First decoded byte was not the NIP-44 version-2 prefix (`0x02`).
    #[error("content is not NIP-44 v2 (expected 0x02 version prefix)")]
    WrongVersion,
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

    #[test]
    fn test_validate_nip44_v2_accepts_real_ciphertext() {
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let encrypted = nip44::encrypt(
            sender.secret_key(),
            &recipient.public_key(),
            "hello over an encrypted channel",
            nip44::Version::V2,
        )
        .expect("encrypt");
        assert_eq!(validate_nip44_v2(&encrypted), Ok(()));
    }

    #[test]
    fn test_validate_nip44_v2_rejects_long_plaintext_in_length_envelope() {
        // A long plaintext message passes the length-only `content_looks_like_nip44`
        // check but must be rejected by the strong validator (D2 — the whole point).
        // Spaces are outside the base64 alphabet, so a human-readable message fails fast.
        let plaintext =
            "this is a long plaintext message a careless client forgot to encrypt ".repeat(2);
        assert!(content_looks_like_nip44(&plaintext));
        assert_eq!(
            validate_nip44_v2(&plaintext),
            Err(Nip44SyntaxError::NotBase64)
        );
    }

    #[test]
    fn test_validate_nip44_v2_rejects_empty() {
        assert_eq!(validate_nip44_v2(""), Err(Nip44SyntaxError::Empty));
    }

    #[test]
    fn test_validate_nip44_v2_rejects_wrong_version_byte() {
        // Take a real v2 ciphertext (first decoded byte 0x02 -> base64 starts "Ag")
        // and flip the leading base64 char so the decoded version byte is no longer
        // 0x02, while keeping valid base64 and length. 'A'(0) -> 'B'(1) shifts the
        // first decoded byte off 0x02 without changing any other property.
        let sender = Keys::generate();
        let recipient = Keys::generate();
        let ct = nip44::encrypt(
            sender.secret_key(),
            &recipient.public_key(),
            "payload",
            nip44::Version::V2,
        )
        .expect("encrypt");
        assert!(
            ct.starts_with('A'),
            "v2 ciphertext base64 should start with 'A'"
        );
        let mutated = format!("B{}", &ct[1..]);
        assert_eq!(
            validate_nip44_v2(&mutated),
            Err(Nip44SyntaxError::WrongVersion)
        );
    }

    #[test]
    fn test_validate_nip44_v2_rejects_too_short() {
        // "AgAA" decodes to [0x02, 0x00, 0x00] — correct version prefix, only 3
        // bytes, under the 99-byte NIP-44 v2 minimum.
        assert_eq!(validate_nip44_v2("AgAA"), Err(Nip44SyntaxError::TooShort));
    }
}
