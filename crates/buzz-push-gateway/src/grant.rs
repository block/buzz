//! Authenticated, expiring endpoint grants. The gateway is stateless: relays
//! retain the opaque ciphertext and present it on each delivery attempt.
use std::collections::{HashMap, HashSet};

use crate::model::{EndpointGrant, MAX_GRANT_BYTES};
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use thiserror::Error;

const AAD_PREFIX: &[u8] = b"buzz-stateful-delivery-capability-v1:";
const MAX_KEY_ID_BYTES: usize = 32;

#[derive(Clone)]
pub struct GrantKey {
    id: String,
    cipher: Aes256Gcm,
}

#[derive(Clone)]
pub struct GrantKeyring {
    current: GrantKey,
    predecessors: HashMap<String, GrantKey>,
}

#[derive(Debug, Error)]
pub enum GrantError {
    #[error("invalid endpoint grant")]
    Invalid,
    #[error("duplicate grant key id")]
    DuplicateKeyId,
    #[error("grant keyring is empty")]
    EmptyKeyring,
}

impl GrantKey {
    pub fn new(id: impl Into<String>, key: &[u8]) -> Result<Self, GrantError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > MAX_KEY_ID_BYTES
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(GrantError::Invalid);
        }
        Ok(Self {
            id,
            cipher: Aes256Gcm::new_from_slice(key).map_err(|_| GrantError::Invalid)?,
        })
    }

    fn aad(&self) -> Vec<u8> {
        [AAD_PREFIX, self.id.as_bytes()].concat()
    }

    fn seal(&self, grant: &EndpointGrant) -> Result<String, GrantError> {
        let plaintext = serde_json::to_vec(grant).map_err(|_| GrantError::Invalid)?;
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut encrypted = nonce.to_vec();
        encrypted.extend(
            self.cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    aes_gcm::aead::Payload {
                        msg: &plaintext,
                        aad: &self.aad(),
                    },
                )
                .map_err(|_| GrantError::Invalid)?,
        );
        let encoded = format!("{}.{}", self.id, URL_SAFE_NO_PAD.encode(encrypted));
        if encoded.len() > MAX_GRANT_BYTES {
            return Err(GrantError::Invalid);
        }
        Ok(encoded)
    }

    fn open(&self, encoded: &str) -> Result<EndpointGrant, GrantError> {
        let (id, payload) = encoded.split_once('.').ok_or(GrantError::Invalid)?;
        if id != self.id {
            return Err(GrantError::Invalid);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| GrantError::Invalid)?;
        if bytes.len() < 13 {
            return Err(GrantError::Invalid);
        }
        let plaintext = self
            .cipher
            .decrypt(
                Nonce::from_slice(&bytes[..12]),
                aes_gcm::aead::Payload {
                    msg: &bytes[12..],
                    aad: &self.aad(),
                },
            )
            .map_err(|_| GrantError::Invalid)?;
        crate::strict_json::from_slice(&plaintext).map_err(|_| GrantError::Invalid)
    }
}

impl GrantKeyring {
    /// Build a keyring ordered current key first, then decrypt-only predecessors.
    pub fn new(keys: Vec<GrantKey>) -> Result<Self, GrantError> {
        let mut keys = keys.into_iter();
        let current = keys.next().ok_or(GrantError::EmptyKeyring)?;
        let predecessor_keys: Vec<_> = keys.collect();
        let mut ids = HashSet::with_capacity(predecessor_keys.len() + 1);
        if !ids.insert(current.id.as_str())
            || predecessor_keys
                .iter()
                .any(|key| !ids.insert(key.id.as_str()))
        {
            return Err(GrantError::DuplicateKeyId);
        }
        let predecessors = predecessor_keys
            .into_iter()
            .map(|key| (key.id.clone(), key))
            .collect();
        Ok(Self {
            current,
            predecessors,
        })
    }

    /// Mint with the current key only.
    pub fn issue(&self, grant: &EndpointGrant) -> Result<String, GrantError> {
        self.current.seal(grant)
    }

    /// Open with the key selected by the authenticated envelope key id.
    pub fn open(&self, encoded: &str) -> Result<EndpointGrant, GrantError> {
        if encoded.len() > MAX_GRANT_BYTES {
            return Err(GrantError::Invalid);
        }
        let (id, _) = encoded.split_once('.').ok_or(GrantError::Invalid)?;
        if id == self.current.id {
            return self.current.open(encoded);
        }
        self.predecessors
            .get(id)
            .ok_or(GrantError::Invalid)?
            .open(encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn grant() -> EndpointGrant {
        EndpointGrant {
            v: 1,
            delegation_id: uuid::Uuid::nil(),
            relay_pubkey: "11".repeat(32),
            app_profile: AppProfile::BuzzIosProduction,
            endpoint_epoch: 1,
            generation: 2,
            expires_at: 99,
        }
    }

    #[test]
    fn current_issues_and_predecessor_opens_after_rotation() {
        let old = GrantKeyring::new(vec![GrantKey::new("old", &[7; 32]).unwrap()]).unwrap();
        let sealed_old = old.issue(&grant()).unwrap();
        let without_old =
            GrantKeyring::new(vec![GrantKey::new("current", &[8; 32]).unwrap()]).unwrap();
        assert!(without_old.open(&sealed_old).is_err());
        let rotated = GrantKeyring::new(vec![
            GrantKey::new("current", &[8; 32]).unwrap(),
            GrantKey::new("old", &[7; 32]).unwrap(),
        ])
        .unwrap();
        let sealed_current = rotated.issue(&grant()).unwrap();
        assert!(sealed_current.starts_with("current."));
        assert_eq!(rotated.open(&sealed_old).unwrap(), grant());
        assert_eq!(rotated.open(&sealed_current).unwrap(), grant());
    }

    #[test]
    fn configured_route_id_is_authenticated_even_when_keys_match() {
        let ring = GrantKeyring::new(vec![
            GrantKey::new("current", &[7; 32]).unwrap(),
            GrantKey::new("previous", &[7; 32]).unwrap(),
        ])
        .unwrap();
        let route_tampered = ring
            .issue(&grant())
            .unwrap()
            .replacen("current.", "previous.", 1);
        assert!(ring.open(&route_tampered).is_err());
    }

    #[test]
    fn complete_envelope_length_is_bounded_on_issue_and_open() {
        let ring = GrantKeyring::new(vec![GrantKey::new("current", &[7; 32]).unwrap()]).unwrap();
        let mut oversized = grant();
        oversized.relay_pubkey = "a".repeat(MAX_GRANT_BYTES * 2);
        assert!(ring.issue(&oversized).is_err());

        let at_limit = "a".repeat(MAX_GRANT_BYTES);
        let over_limit = "a".repeat(MAX_GRANT_BYTES + 1);
        assert!(ring.open(&at_limit).is_err());
        assert!(ring.open(&over_limit).is_err());
    }

    #[test]
    fn tampering_unknown_ids_and_duplicate_configuration_fail() {
        let ring = GrantKeyring::new(vec![GrantKey::new("current", &[7; 32]).unwrap()]).unwrap();
        let sealed = ring.issue(&grant()).unwrap();
        let mut bad = sealed.into_bytes();
        let n = bad.len() - 1;
        bad[n] = if bad[n] == b'A' { b'B' } else { b'A' };
        assert!(ring.open(std::str::from_utf8(&bad).unwrap()).is_err());
        let route_tampered = ring
            .issue(&grant())
            .unwrap()
            .replacen("current.", "other.", 1);
        assert!(ring.open(&route_tampered).is_err());
        assert!(ring.open("unknown.AAAA").is_err());
        assert!(matches!(
            GrantKeyring::new(vec![
                GrantKey::new("same", &[1; 32]).unwrap(),
                GrantKey::new("same", &[2; 32]).unwrap(),
            ]),
            Err(GrantError::DuplicateKeyId)
        ));
        assert!(matches!(
            GrantKeyring::new(Vec::new()),
            Err(GrantError::EmptyKeyring)
        ));
    }
}
