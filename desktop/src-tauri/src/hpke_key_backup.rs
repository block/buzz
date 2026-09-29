//! HPKE encryption for a future enterprise identity-backup service.
//!
//! This module only creates the opaque client envelope. Recipient enrollment,
//! upload, authorization, storage, and recovery are deliberately outside its
//! scope. In particular, this is a native Rust API and is not registered as a
//! Tauri command: renderer-controlled recipient keys would turn encryption into
//! an nsec export path.
//!
//! This is a pure sealing component, not an active [`crate::app_state::AppState`]
//! backup operation. A future enrollment/upload path must serialize identity
//! selection and sealing with `AppState::identity_mutation`, obtain the live
//! identity through `AppState::signing_keys()`, and bind its public key to the
//! authenticated enrollment before upload. This API alone does not protect
//! against identity lock, loss, or concurrent rotation, and its tests do not
//! claim that runtime integration.

use std::fmt;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::{PublicKey, SecretKey};
use rustls::crypto::{
    aws_lc_rs::hpke::DH_KEM_P256_HKDF_SHA256_AES_256,
    hpke::{Hpke, HpkePublicKey},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

/// HPKE `info` bytes for every v1 nsec-backup context.
pub const HPKE_INFO_V1: &[u8] = b"buzz/nsec-backup/v1";

/// Version of the envelope and associated-data contract.
pub const ENVELOPE_VERSION_V1: u8 = 1;

/// RFC 9180 KEM ID for DHKEM(P-256, HKDF-SHA256).
pub const KEM_ID_P256_HKDF_SHA256: u16 = 0x0010;

/// RFC 9180 KDF ID for HKDF-SHA256.
pub const KDF_ID_HKDF_SHA256: u16 = 0x0001;

/// RFC 9180 AEAD ID for AES-256-GCM.
pub const AEAD_ID_AES_256_GCM: u16 = 0x0002;

/// Encoded length of an uncompressed SEC1 P-256 public key.
pub const P256_PUBLIC_KEY_LEN: usize = 65;

/// Encoded length of a v1 ciphertext: 32 secret bytes plus a 16-byte GCM tag.
pub const CIPHERTEXT_LEN: usize = 48;

const AAD_DOMAIN_V1: &[u8] = b"buzz/nsec-backup/aad/v1";
const MAX_CONTEXT_FIELD_LEN: usize = 255;
const P256_PUBLIC_KEY_BASE64URL_LEN: usize = 87;
const CIPHERTEXT_BASE64URL_LEN: usize = 64;

/// Errors produced while validating or sealing an HPKE nsec-backup envelope.
///
/// Error values never contain secret-key bytes or plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HpkeBackupError {
    /// An enrollment or envelope field did not satisfy the v1 wire contract.
    InvalidField {
        /// Name of the invalid field.
        field: &'static str,
        /// Non-sensitive validation failure.
        reason: &'static str,
    },
    /// The recipient key is not an uncompressed P-256 SEC1 point.
    InvalidRecipientKey(&'static str),
    /// An envelope is inconsistent with the v1 wire contract.
    InvalidEnvelope(&'static str),
    /// HPKE rejected the recipient key or failed to seal the plaintext.
    EncryptionFailed,
}

impl fmt::Display for HpkeBackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField { field, reason } => {
                write!(formatter, "invalid backup {field}: {reason}")
            }
            Self::InvalidRecipientKey(reason) => {
                write!(formatter, "invalid HPKE recipient public key: {reason}")
            }
            Self::InvalidEnvelope(reason) => write!(formatter, "invalid backup envelope: {reason}"),
            Self::EncryptionFailed => {
                formatter.write_str("HPKE backup encryption failed; recipient key may be invalid")
            }
        }
    }
}

impl std::error::Error for HpkeBackupError {}

/// Fixed algorithm identifiers carried by a v1 envelope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HpkeSuiteIds {
    kem_id: u16,
    kdf_id: u16,
    aead_id: u16,
}

impl HpkeSuiteIds {
    const V1: Self = Self {
        kem_id: KEM_ID_P256_HKDF_SHA256,
        kdf_id: KDF_ID_HKDF_SHA256,
        aead_id: AEAD_ID_AES_256_GCM,
    };

    fn is_v1(self) -> bool {
        self == Self::V1
    }
}

/// Enrollment values authenticated into one backup envelope.
#[derive(Clone, Debug)]
pub struct HpkeBackupEnrollment {
    recipient_key_id: String,
    service_namespace: String,
    owner_id: String,
    backup_id: Uuid,
    recipient_public_key: [u8; P256_PUBLIC_KEY_LEN],
}

impl HpkeBackupEnrollment {
    /// Validate enrollment metadata supplied by a trusted native integration.
    ///
    /// Identifier strings are encoded as UTF-8 bytes without normalization in
    /// AAD. Each must be 1 through 255 bytes and must not contain control
    /// characters. The backup ID is encoded as its 16 raw UUID bytes.
    pub fn new(
        recipient_key_id: impl Into<String>,
        service_namespace: impl Into<String>,
        owner_id: impl Into<String>,
        backup_id: Uuid,
        recipient_public_key_sec1: &[u8],
    ) -> Result<Self, HpkeBackupError> {
        let recipient_key_id = recipient_key_id.into();
        let service_namespace = service_namespace.into();
        let owner_id = owner_id.into();
        validate_context_field("recipient key ID", &recipient_key_id)?;
        validate_context_field("service namespace", &service_namespace)?;
        validate_context_field("owner ID", &owner_id)?;

        let recipient_public_key: [u8; P256_PUBLIC_KEY_LEN] = recipient_public_key_sec1
            .try_into()
            .map_err(|_| HpkeBackupError::InvalidRecipientKey("expected exactly 65 SEC1 bytes"))?;
        if recipient_public_key[0] != 0x04 {
            return Err(HpkeBackupError::InvalidRecipientKey(
                "expected uncompressed SEC1 prefix 0x04",
            ));
        }

        Ok(Self {
            recipient_key_id,
            service_namespace,
            owner_id,
            backup_id,
            recipient_public_key,
        })
    }
}

/// Opaque v1 backup envelope produced by [`seal_nostr_secret`].
///
/// JSON serialization is the wire representation. Binary HPKE values use
/// URL-safe base64 with no padding. This type contains no plaintext or raw nsec
/// material.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HpkeBackupEnvelope {
    version: u8,
    suite: HpkeSuiteIds,
    recipient_key_id: String,
    service_namespace: String,
    owner_id: String,
    backup_id: String,
    nostr_pubkey: String,
    enc: String,
    ciphertext: String,
}

impl HpkeBackupEnvelope {
    /// Reconstruct the v1 associated-data bytes from envelope metadata.
    ///
    /// This validates fixed suite/version values and canonical metadata
    /// encodings before returning bytes. Receivers must separately validate
    /// `enc` and ciphertext through the byte accessors before HPKE open.
    pub fn associated_data(&self) -> Result<Vec<u8>, HpkeBackupError> {
        if self.version != ENVELOPE_VERSION_V1 {
            return Err(HpkeBackupError::InvalidEnvelope("unsupported version"));
        }
        if !self.suite.is_v1() {
            return Err(HpkeBackupError::InvalidEnvelope("unsupported HPKE suite"));
        }
        validate_context_field("recipient key ID", &self.recipient_key_id)?;
        validate_context_field("service namespace", &self.service_namespace)?;
        validate_context_field("owner ID", &self.owner_id)?;

        let backup_id = Uuid::parse_str(&self.backup_id)
            .map_err(|_| HpkeBackupError::InvalidEnvelope("backup ID is not a UUID"))?;
        if backup_id.hyphenated().to_string() != self.backup_id {
            return Err(HpkeBackupError::InvalidEnvelope(
                "backup ID is not canonical lowercase hyphenated UUID",
            ));
        }

        let nostr_pubkey = PublicKey::from_hex(&self.nostr_pubkey)
            .map_err(|_| HpkeBackupError::InvalidEnvelope("invalid Nostr public key"))?;
        if nostr_pubkey.to_hex() != self.nostr_pubkey {
            return Err(HpkeBackupError::InvalidEnvelope(
                "Nostr public key is not canonical lowercase hex",
            ));
        }

        Ok(build_aad(
            &self.recipient_key_id,
            &self.service_namespace,
            &self.owner_id,
            backup_id.as_bytes(),
            &nostr_pubkey.to_bytes(),
        ))
    }

    /// Decode and validate the RFC 9180 encapsulated P-256 public-key shape.
    ///
    /// This enforces the 65-byte uncompressed SEC1 representation and its
    /// `0x04` prefix. The HPKE provider validates P-256 curve membership when
    /// a recipient uses these bytes to open the envelope.
    pub fn encapsulated_key_bytes(&self) -> Result<Vec<u8>, HpkeBackupError> {
        let decoded = decode_canonical_base64(
            &self.enc,
            P256_PUBLIC_KEY_BASE64URL_LEN,
            P256_PUBLIC_KEY_LEN,
            "enc",
        )?;
        if decoded.first() != Some(&0x04) {
            return Err(HpkeBackupError::InvalidField {
                field: "enc",
                reason: "expected uncompressed SEC1 prefix 0x04",
            });
        }
        Ok(decoded)
    }

    /// Decode and validate the 32-byte-secret AES-256-GCM ciphertext.
    pub fn ciphertext_bytes(&self) -> Result<Vec<u8>, HpkeBackupError> {
        decode_canonical_base64(
            &self.ciphertext,
            CIPHERTEXT_BASE64URL_LEN,
            CIPHERTEXT_LEN,
            "ciphertext",
        )
    }
}

/// Seal one native Nostr secret key to a trusted HPKE backup enrollment.
///
/// The function derives the Nostr public key from `secret_key` and encrypts its
/// raw 32-byte secret representation in RFC 9180 Base mode. `rustls` creates a
/// fresh ephemeral P-256 key for every call. HPKE Base mode does not
/// authenticate the sender; future upload authorization or a separate signature
/// must provide that property.
pub fn seal_nostr_secret(
    secret_key: &SecretKey,
    enrollment: &HpkeBackupEnrollment,
) -> Result<HpkeBackupEnvelope, HpkeBackupError> {
    let nostr_pubkey = PublicKey::from(secret_key.x_only_public_key(nostr::SECP256K1).0).to_hex();
    let mut envelope = HpkeBackupEnvelope {
        version: ENVELOPE_VERSION_V1,
        suite: HpkeSuiteIds::V1,
        recipient_key_id: enrollment.recipient_key_id.clone(),
        service_namespace: enrollment.service_namespace.clone(),
        owner_id: enrollment.owner_id.clone(),
        backup_id: enrollment.backup_id.hyphenated().to_string(),
        nostr_pubkey,
        enc: String::new(),
        ciphertext: String::new(),
    };
    let aad = envelope.associated_data()?;
    // This clears our buffer on drop, but not the HPKE provider's internal
    // plaintext copy, which can be freed without zeroization.
    let plaintext = Zeroizing::new(secret_key.to_secret_bytes());

    let recipient = HpkePublicKey(enrollment.recipient_public_key.to_vec());
    let (enc, ciphertext) = DH_KEM_P256_HKDF_SHA256_AES_256
        .seal(HPKE_INFO_V1, &aad, plaintext.as_ref(), &recipient)
        .map_err(|_| HpkeBackupError::EncryptionFailed)?;

    if enc.0.len() != P256_PUBLIC_KEY_LEN || ciphertext.len() != CIPHERTEXT_LEN {
        return Err(HpkeBackupError::EncryptionFailed);
    }
    envelope.enc = URL_SAFE_NO_PAD.encode(enc.0);
    envelope.ciphertext = URL_SAFE_NO_PAD.encode(ciphertext);
    Ok(envelope)
}

fn validate_context_field(field: &'static str, value: &str) -> Result<(), HpkeBackupError> {
    if value.is_empty() {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "must not be empty",
        });
    }
    if value.len() > MAX_CONTEXT_FIELD_LEN {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "must be at most 255 UTF-8 bytes",
        });
    }
    if value.chars().any(char::is_control) {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "must not contain control characters",
        });
    }
    Ok(())
}

fn build_aad(
    recipient_key_id: &str,
    service_namespace: &str,
    owner_id: &str,
    backup_id: &[u8; 16],
    nostr_pubkey: &[u8; 32],
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        AAD_DOMAIN_V1.len()
            + 7
            + 6
            + recipient_key_id.len()
            + service_namespace.len()
            + owner_id.len()
            + backup_id.len()
            + nostr_pubkey.len(),
    );
    aad.extend_from_slice(AAD_DOMAIN_V1);
    aad.push(ENVELOPE_VERSION_V1);
    aad.extend_from_slice(&KEM_ID_P256_HKDF_SHA256.to_be_bytes());
    aad.extend_from_slice(&KDF_ID_HKDF_SHA256.to_be_bytes());
    aad.extend_from_slice(&AEAD_ID_AES_256_GCM.to_be_bytes());
    push_framed(&mut aad, recipient_key_id.as_bytes());
    push_framed(&mut aad, service_namespace.as_bytes());
    push_framed(&mut aad, owner_id.as_bytes());
    aad.extend_from_slice(backup_id);
    aad.extend_from_slice(nostr_pubkey);
    aad
}

fn push_framed(output: &mut Vec<u8>, value: &[u8]) {
    // All callers first apply MAX_CONTEXT_FIELD_LEN, which is below u16::MAX.
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value);
}

fn decode_canonical_base64(
    value: &str,
    expected_encoded_len: usize,
    expected_decoded_len: usize,
    field: &'static str,
) -> Result<Vec<u8>, HpkeBackupError> {
    // `str::len` is the UTF-8 byte length. Check it before decoding so an
    // untrusted JSON string cannot cause an allocation proportional to input.
    if value.len() != expected_encoded_len {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "unexpected encoded length",
        });
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| HpkeBackupError::InvalidField {
            field,
            reason: "invalid base64url",
        })?;
    if decoded.len() != expected_decoded_len {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "unexpected decoded length",
        });
    }
    if URL_SAFE_NO_PAD.encode(&decoded) != value {
        return Err(HpkeBackupError::InvalidField {
            field,
            reason: "base64url is not canonical and unpadded",
        });
    }
    Ok(decoded)
}

#[cfg(test)]
#[path = "hpke_key_backup_tests.rs"]
mod tests;
