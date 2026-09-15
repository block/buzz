//! Two-secret key derivation for portable identity recovery.
//!
//! A Buzz identity is encrypted with a key assembled from two independent
//! halves: an Argon2id-derived passphrase half and an HKDF-derived random
//! recovery-secret half. The encrypted backup is safe to store separately
//! from the recovery secret; neither the backup nor the secret is useful on
//! its own.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use nostr::{Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2_legacy::Sha256;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

pub const BACKUP_PREFIX: &str = "buzz2skd1:";
pub const RECOVERY_SECRET_PREFIX: &str = "buzz-recovery-v1-";
pub const BACKUP_FILE_NAME: &str = "identity.buzzbackup";

const FORMAT_VERSION: u8 = 1;
const RECOVERY_SECRET_BYTES: usize = 16;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;
const CIPHERTEXT_BYTES: usize = KEY_BYTES + 16;
const INFO: &[u8] = b"Buzz 2SKD v1";
const MAX_BACKUP_BYTES: usize = 4096;

/// RFC 9106 section 4's first recommended Argon2id profile: 2 GiB, one pass,
/// four lanes. The serialized artifact records these values so a future format
/// can change them without making existing backups unreadable.
pub const ARGON2_MEMORY_KIB: u32 = 1 << 21;
pub const ARGON2_ITERATIONS: u32 = 1;
pub const ARGON2_PARALLELISM: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub struct Argon2Cost {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Argon2Cost {
    pub const PRODUCTION: Self = Self {
        memory_kib: ARGON2_MEMORY_KIB,
        iterations: ARGON2_ITERATIONS,
        parallelism: ARGON2_PARALLELISM,
    };
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedBackup {
    pub backup: String,
    pub recovery_secret: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackupPayload {
    version: u8,
    pubkey: String,
    salt: String,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    nonce: String,
    ciphertext: String,
}

impl BackupPayload {
    fn cost(&self) -> Argon2Cost {
        Argon2Cost {
            memory_kib: self.memory_kib,
            iterations: self.iterations,
            parallelism: self.parallelism,
        }
    }

    fn aad(&self) -> Vec<u8> {
        format!(
            "Buzz 2SKD v1|{}|{}|{}|{}|{}|{}",
            self.pubkey, self.salt, self.memory_kib, self.iterations, self.parallelism, self.nonce
        )
        .into_bytes()
    }
}

pub(crate) fn is_two_skd_backup(input: &str) -> bool {
    input.trim().starts_with(BACKUP_PREFIX)
}

/// Encrypt `keys` under a fresh recovery secret and the user passphrase.
pub(crate) fn create_backup(keys: &Keys, passphrase: &str) -> Result<CreatedBackup, String> {
    create_backup_with_cost(keys, passphrase, Argon2Cost::PRODUCTION)
}

pub(crate) fn create_backup_with_cost(
    keys: &Keys,
    passphrase: &str,
    cost: Argon2Cost,
) -> Result<CreatedBackup, String> {
    validate_cost(cost)?;

    let mut recovery_secret = Zeroizing::new([0u8; RECOVERY_SECRET_BYTES]);
    let mut salt = [0u8; SALT_BYTES];
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::getrandom(recovery_secret.as_mut())
        .map_err(|error| format!("generate recovery secret: {error}"))?;
    getrandom::getrandom(&mut salt).map_err(|error| format!("generate backup salt: {error}"))?;
    getrandom::getrandom(&mut nonce).map_err(|error| format!("generate backup nonce: {error}"))?;

    // Buzz does not use a passkey in this flow, so the stable public Nostr
    // identity fills the public-key binding role from the 2SKD sketch.
    let pubkey_bytes = keys.public_key().to_bytes();
    let encryption_key =
        derive_encryption_key(passphrase, &salt, &recovery_secret, &pubkey_bytes, cost)?;

    let mut payload = BackupPayload {
        version: FORMAT_VERSION,
        pubkey: keys.public_key().to_hex(),
        salt: URL_SAFE_NO_PAD.encode(salt),
        memory_kib: cost.memory_kib,
        iterations: cost.iterations,
        parallelism: cost.parallelism,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
        ciphertext: String::new(),
    };

    let cipher = ChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "initialize backup cipher".to_string())?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: keys.secret_key().as_secret_bytes(),
                aad: &payload.aad(),
            },
        )
        .map_err(|_| "encrypt identity backup".to_string())?;
    payload.ciphertext = URL_SAFE_NO_PAD.encode(ciphertext);

    let encoded =
        serde_json::to_vec(&payload).map_err(|error| format!("encode identity backup: {error}"))?;
    let backup = format!("{BACKUP_PREFIX}{}", URL_SAFE_NO_PAD.encode(encoded));
    let recovery_secret = format!(
        "{RECOVERY_SECRET_PREFIX}{}",
        hex::encode(recovery_secret.as_ref())
    );

    // Refuse to hand a fresh backup to the UI unless all three inputs recover
    // this exact identity.
    let recovered = decrypt_backup(&backup, passphrase, &recovery_secret)?;
    if recovered.public_key() != keys.public_key() {
        return Err("verify identity backup: recovered identity did not match".to_string());
    }

    Ok(CreatedBackup {
        backup,
        recovery_secret,
    })
}

/// Recover identity keys from a 2SKD backup, passphrase, and recovery secret.
pub(crate) fn decrypt_backup(
    input: &str,
    passphrase: &str,
    recovery_secret: &str,
) -> Result<Keys, String> {
    let payload = parse_backup(input)?;
    let cost = payload.cost();
    validate_cost(cost)?;

    let salt = decode_array::<SALT_BYTES>(&payload.salt, "backup salt")?;
    let nonce = decode_array::<NONCE_BYTES>(&payload.nonce, "backup nonce")?;
    let pubkey_bytes = hex::decode(&payload.pubkey)
        .map_err(|_| "invalid backup public key".to_string())?
        .try_into()
        .map_err(|_| "invalid backup public key".to_string())?;
    let recovery_secret = parse_recovery_secret(recovery_secret)?;
    let encryption_key =
        derive_encryption_key(passphrase, &salt, &recovery_secret, &pubkey_bytes, cost)?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(&payload.ciphertext)
        .map_err(|_| "invalid backup ciphertext".to_string())?;
    let cipher = ChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "initialize backup cipher".to_string())?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &payload.aad(),
                },
            )
            .map_err(|_| "wrong backup password, recovery code, or damaged backup".to_string())?,
    );
    if plaintext.len() != KEY_BYTES {
        return Err("damaged identity backup".to_string());
    }
    let secret =
        SecretKey::from_slice(&plaintext).map_err(|_| "damaged identity backup".to_string())?;
    let keys = Keys::new(secret);
    if keys.public_key().to_bytes() != pubkey_bytes {
        return Err("damaged identity backup".to_string());
    }
    Ok(keys)
}

fn parse_backup(input: &str) -> Result<BackupPayload, String> {
    let trimmed = input.trim();
    if trimmed.len() > MAX_BACKUP_BYTES {
        return Err("identity backup is too large".to_string());
    }
    let encoded = trimmed
        .strip_prefix(BACKUP_PREFIX)
        .ok_or_else(|| "invalid 2SKD identity backup".to_string())?;
    let json = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "invalid 2SKD identity backup".to_string())?;
    let payload: BackupPayload =
        serde_json::from_slice(&json).map_err(|_| "invalid 2SKD identity backup".to_string())?;
    if payload.version != FORMAT_VERSION {
        return Err(format!(
            "unsupported 2SKD backup version: {}",
            payload.version
        ));
    }
    validate_payload_shape(&payload)?;
    Ok(payload)
}

pub(crate) fn validate_backup(input: &str) -> Result<(), String> {
    parse_backup(input).map(|_| ())
}

pub(crate) fn backup_public_key(input: &str) -> Result<PublicKey, String> {
    let payload = parse_backup(input)?;
    PublicKey::parse(&payload.pubkey).map_err(|_| "invalid backup public key".to_string())
}

fn validate_payload_shape(payload: &BackupPayload) -> Result<(), String> {
    validate_cost(payload.cost())?;
    decode_array::<SALT_BYTES>(&payload.salt, "backup salt")?;
    decode_array::<NONCE_BYTES>(&payload.nonce, "backup nonce")?;
    let _: [u8; KEY_BYTES] = hex::decode(&payload.pubkey)
        .map_err(|_| "invalid backup public key".to_string())?
        .try_into()
        .map_err(|_| "invalid backup public key".to_string())?;
    decode_array::<CIPHERTEXT_BYTES>(&payload.ciphertext, "backup ciphertext")?;
    Ok(())
}

fn parse_recovery_secret(input: &str) -> Result<Zeroizing<[u8; RECOVERY_SECRET_BYTES]>, String> {
    let encoded = input
        .trim()
        .strip_prefix(RECOVERY_SECRET_PREFIX)
        .ok_or_else(|| "invalid recovery code".to_string())?;
    let bytes = hex::decode(encoded).map_err(|_| "invalid recovery code".to_string())?;
    let secret = bytes
        .try_into()
        .map_err(|_| "invalid recovery code".to_string())?;
    Ok(Zeroizing::new(secret))
}

pub(crate) fn validate_recovery_secret(input: &str) -> Result<(), String> {
    parse_recovery_secret(input).map(|_| ())
}

fn decode_array<const N: usize>(encoded: &str, label: &str) -> Result<[u8; N], String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| format!("invalid {label}"))?;
    bytes.try_into().map_err(|_| format!("invalid {label}"))
}

fn derive_encryption_key(
    passphrase: &str,
    salt: &[u8; SALT_BYTES],
    recovery_secret: &[u8; RECOVERY_SECRET_BYTES],
    pubkey: &[u8; KEY_BYTES],
    cost: Argon2Cost,
) -> Result<Zeroizing<[u8; KEY_BYTES]>, String> {
    let params = Params::new(
        cost.memory_kib,
        cost.iterations,
        cost.parallelism,
        Some(KEY_BYTES),
    )
    .map_err(|error| format!("invalid backup KDF parameters: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let normalized = Zeroizing::new(passphrase.nfkc().collect::<String>());
    let mut passphrase_half = Zeroizing::new([0u8; KEY_BYTES]);
    argon2
        .hash_password_into(normalized.as_bytes(), salt, passphrase_half.as_mut())
        .map_err(|error| format!("derive backup key from password: {error}"))?;

    let mut secret_half = Zeroizing::new([0u8; KEY_BYTES]);
    Hkdf::<Sha256>::new(Some(pubkey), recovery_secret)
        .expand(INFO, secret_half.as_mut())
        .map_err(|_| "derive backup key from recovery secret".to_string())?;

    let mut combined = Zeroizing::new([0u8; KEY_BYTES]);
    for index in 0..KEY_BYTES {
        combined[index] = passphrase_half[index] ^ secret_half[index];
    }
    Ok(combined)
}

fn validate_cost(cost: Argon2Cost) -> Result<(), String> {
    if cost.memory_kib == 0
        || cost.memory_kib > ARGON2_MEMORY_KIB
        || cost.iterations == 0
        || cost.iterations > 4
        || cost.parallelism == 0
        || cost.parallelism > ARGON2_PARALLELISM
    {
        return Err("unsupported backup KDF parameters".to_string());
    }
    Params::new(
        cost.memory_kib,
        cost.iterations,
        cost.parallelism,
        Some(KEY_BYTES),
    )
    .map_err(|_| "unsupported backup KDF parameters".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST_COST: Argon2Cost = Argon2Cost {
        memory_kib: 8 * 1024,
        iterations: 1,
        parallelism: 1,
    };

    #[test]
    fn round_trip_requires_both_secrets() {
        let keys = Keys::generate();
        let created = create_backup_with_cost(&keys, "correct horse battery", FAST_COST).unwrap();
        assert!(created.backup.starts_with(BACKUP_PREFIX));
        assert!(created.recovery_secret.starts_with(RECOVERY_SECRET_PREFIX));
        assert!(!created.backup.contains(&created.recovery_secret));

        let recovered = decrypt_backup(
            &created.backup,
            "correct horse battery",
            &created.recovery_secret,
        )
        .unwrap();
        assert_eq!(recovered.public_key(), keys.public_key());

        assert_eq!(
            decrypt_backup(&created.backup, "wrong password", &created.recovery_secret,)
                .unwrap_err(),
            "wrong backup password, recovery code, or damaged backup"
        );
        let other_secret = format!("{RECOVERY_SECRET_PREFIX}{}", "00".repeat(16));
        assert_eq!(
            decrypt_backup(&created.backup, "correct horse battery", &other_secret).unwrap_err(),
            "wrong backup password, recovery code, or damaged backup"
        );
    }

    #[test]
    fn normalized_passphrases_round_trip() {
        let keys = Keys::generate();
        let created =
            create_backup_with_cost(&keys, "caf\u{00e9} recovery phrase", FAST_COST).unwrap();
        let recovered = decrypt_backup(
            &created.backup,
            "cafe\u{0301} recovery phrase",
            &created.recovery_secret,
        )
        .unwrap();
        assert_eq!(recovered.public_key(), keys.public_key());
    }

    #[test]
    fn tampered_public_metadata_fails_authentication() {
        let keys = Keys::generate();
        let created = create_backup_with_cost(&keys, "correct horse battery", FAST_COST).unwrap();
        let mut payload = parse_backup(&created.backup).unwrap();
        payload.pubkey = Keys::generate().public_key().to_hex();
        let encoded = serde_json::to_vec(&payload).unwrap();
        let tampered = format!("{BACKUP_PREFIX}{}", URL_SAFE_NO_PAD.encode(encoded));
        assert!(
            decrypt_backup(&tampered, "correct horse battery", &created.recovery_secret).is_err()
        );
    }

    #[test]
    fn untrusted_cost_is_bounded_before_derivation() {
        let keys = Keys::generate();
        let created = create_backup_with_cost(&keys, "correct horse battery", FAST_COST).unwrap();
        let mut payload = parse_backup(&created.backup).unwrap();
        payload.memory_kib = ARGON2_MEMORY_KIB + 1;
        let encoded = serde_json::to_vec(&payload).unwrap();
        let oversized = format!("{BACKUP_PREFIX}{}", URL_SAFE_NO_PAD.encode(encoded));
        assert_eq!(
            decrypt_backup(
                &oversized,
                "correct horse battery",
                &created.recovery_secret,
            )
            .unwrap_err(),
            "unsupported backup KDF parameters"
        );
    }
}
