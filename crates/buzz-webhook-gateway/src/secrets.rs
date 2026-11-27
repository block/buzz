//! Provider secret custody. Secrets are sealed with AES-256-GCM under the
//! current keyring entry; ciphertexts are self-describing so bounded
//! decrypt-only predecessors keep working during key rotation.
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::collections::HashMap;
use thiserror::Error;

const AAD_PREFIX: &[u8] = b"buzz-webhook-secret-v1:";
pub const MAX_SECRET_BYTES: usize = 1024;
pub const MAX_CIPHERTEXT_BYTES: usize = 4096;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("invalid secret or ciphertext")]
    Invalid,
    #[error("secret keyring is empty or contains duplicate ids")]
    InvalidKeyring,
}

#[derive(Clone)]
pub struct SecretKey {
    id: String,
    cipher: Aes256Gcm,
}

impl SecretKey {
    pub fn new(id: impl Into<String>, key: &[u8]) -> Result<Self, SecretError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(SecretError::Invalid);
        }
        Ok(Self {
            id,
            cipher: Aes256Gcm::new_from_slice(key).map_err(|_| SecretError::Invalid)?,
        })
    }

    fn aad(&self) -> Vec<u8> {
        [AAD_PREFIX, self.id.as_bytes()].concat()
    }
}

/// The first key seals new secrets; the rest only open existing ciphertexts.
#[derive(Clone)]
pub struct SecretKeyring {
    current: SecretKey,
    predecessors: HashMap<String, SecretKey>,
}

impl SecretKeyring {
    pub fn new(keys: Vec<SecretKey>) -> Result<Self, SecretError> {
        let mut keys = keys.into_iter();
        let current = keys.next().ok_or(SecretError::InvalidKeyring)?;
        let mut predecessors = HashMap::new();
        for key in keys {
            if key.id == current.id || predecessors.insert(key.id.clone(), key).is_some() {
                return Err(SecretError::InvalidKeyring);
            }
        }
        Ok(Self {
            current,
            predecessors,
        })
    }

    pub fn from_config(keys: &[crate::config::KeyConfig]) -> Result<Self, SecretError> {
        Self::new(
            keys.iter()
                .map(|k| SecretKey::new(&k.id, &k.key))
                .collect::<Result<_, _>>()?,
        )
    }

    /// Identifier of the key new ciphertexts are sealed with.
    pub fn current_id(&self) -> &str {
        &self.current.id
    }

    pub fn seal(&self, secret: &[u8]) -> Result<Vec<u8>, SecretError> {
        if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
            return Err(SecretError::Invalid);
        }
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut sealed = nonce.to_vec();
        sealed.extend(
            self.current
                .cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    aes_gcm::aead::Payload {
                        msg: secret,
                        aad: &self.current.aad(),
                    },
                )
                .map_err(|_| SecretError::Invalid)?,
        );
        let encoded =
            format!("{}.{}", self.current.id, URL_SAFE_NO_PAD.encode(sealed)).into_bytes();
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(SecretError::Invalid);
        }
        Ok(encoded)
    }

    pub fn open(&self, encoded: &[u8]) -> Result<Vec<u8>, SecretError> {
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(SecretError::Invalid);
        }
        let encoded = std::str::from_utf8(encoded).map_err(|_| SecretError::Invalid)?;
        let (id, body) = encoded.split_once('.').ok_or(SecretError::Invalid)?;
        let key = if id == self.current.id {
            &self.current
        } else {
            self.predecessors.get(id).ok_or(SecretError::Invalid)?
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| SecretError::Invalid)?;
        if bytes.len() < 13 {
            return Err(SecretError::Invalid);
        }
        key.cipher
            .decrypt(
                Nonce::from_slice(&bytes[..12]),
                aes_gcm::aead::Payload {
                    msg: &bytes[12..],
                    aad: &key.aad(),
                },
            )
            .map_err(|_| SecretError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(ids: &[(&str, u8)]) -> SecretKeyring {
        SecretKeyring::new(
            ids.iter()
                .map(|(id, byte)| SecretKey::new(*id, &[*byte; 32]).unwrap())
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn seal_round_trips_and_predecessors_only_open() {
        let old = ring(&[("k1", 1)]);
        let rotated = ring(&[("k2", 2), ("k1", 1)]);
        let sealed = old.seal(b"whsec_example").unwrap();
        assert!(String::from_utf8_lossy(&sealed).starts_with("k1."));
        assert_eq!(rotated.open(&sealed).unwrap(), b"whsec_example");
        let resealed = rotated.seal(b"whsec_example").unwrap();
        assert!(String::from_utf8_lossy(&resealed).starts_with("k2."));
        assert_eq!(old.open(&resealed), Err(SecretError::Invalid));
    }

    #[test]
    fn tampered_or_oversized_inputs_are_rejected() {
        let ring = ring(&[("k1", 1)]);
        let mut sealed = ring.seal(b"secret").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert_eq!(ring.open(&sealed), Err(SecretError::Invalid));
        assert_eq!(ring.seal(b""), Err(SecretError::Invalid));
        assert_eq!(
            ring.seal(&[0u8; MAX_SECRET_BYTES + 1]),
            Err(SecretError::Invalid)
        );
        assert_eq!(ring.open(b"k1.not-base64!"), Err(SecretError::Invalid));
    }

    #[test]
    fn keyring_rejects_duplicate_ids() {
        let keys = vec![
            SecretKey::new("k1", &[1; 32]).unwrap(),
            SecretKey::new("k1", &[2; 32]).unwrap(),
        ];
        assert!(matches!(
            SecretKeyring::new(keys),
            Err(SecretError::InvalidKeyring)
        ));
        assert!(matches!(
            SecretKeyring::new(vec![]),
            Err(SecretError::InvalidKeyring)
        ));
    }
}
