//! APNs token custody. Ciphertexts are self-describing and can be decrypted by
//! the current key or bounded decrypt-only predecessors during key rotation.
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const AAD_PREFIX: &[u8] = b"buzz-apns-token-v1:";
const MAX_KEY_ID_BYTES: usize = 32;
const MAX_CIPHERTEXT_BYTES: usize = 2048;

#[derive(Clone)]
pub struct TokenKey {
    id: String,
    cipher: Aes256Gcm,
}
#[derive(Clone)]
pub struct TokenKeyring {
    current: TokenKey,
    predecessors: HashMap<String, TokenKey>,
}

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("invalid token ciphertext")]
    Invalid,
    #[error("token keyring is empty or contains duplicate ids")]
    InvalidKeyring,
}

impl TokenKey {
    pub fn new(id: impl Into<String>, key: &[u8]) -> Result<Self, TokenError> {
        let id = id.into();
        if id.is_empty()
            || id.len() > MAX_KEY_ID_BYTES
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(TokenError::Invalid);
        }
        Ok(Self {
            id,
            cipher: Aes256Gcm::new_from_slice(key).map_err(|_| TokenError::Invalid)?,
        })
    }
    fn aad(&self) -> Vec<u8> {
        [AAD_PREFIX, self.id.as_bytes()].concat()
    }
}

impl TokenKeyring {
    pub fn new(keys: Vec<TokenKey>) -> Result<Self, TokenError> {
        let mut keys = keys.into_iter();
        let current = keys.next().ok_or(TokenError::InvalidKeyring)?;
        let rest: Vec<_> = keys.collect();
        let mut ids = HashSet::new();
        if !ids.insert(current.id.clone()) || rest.iter().any(|k| !ids.insert(k.id.clone())) {
            return Err(TokenError::InvalidKeyring);
        }
        Ok(Self {
            current,
            predecessors: rest.into_iter().map(|k| (k.id.clone(), k)).collect(),
        })
    }
    pub fn seal(&self, token: &[u8]) -> Result<Vec<u8>, TokenError> {
        if token.is_empty() || token.len() > crate::model::MAX_ENDPOINT_HEX_BYTES {
            return Err(TokenError::Invalid);
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
                        msg: token,
                        aad: &self.current.aad(),
                    },
                )
                .map_err(|_| TokenError::Invalid)?,
        );
        let encoded =
            format!("{}.{}", self.current.id, URL_SAFE_NO_PAD.encode(sealed)).into_bytes();
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(TokenError::Invalid);
        }
        Ok(encoded)
    }
    pub fn open(&self, encoded: &[u8]) -> Result<Vec<u8>, TokenError> {
        if encoded.len() > MAX_CIPHERTEXT_BYTES {
            return Err(TokenError::Invalid);
        }
        let encoded = std::str::from_utf8(encoded).map_err(|_| TokenError::Invalid)?;
        let (id, body) = encoded.split_once('.').ok_or(TokenError::Invalid)?;
        let key = if id == self.current.id {
            &self.current
        } else {
            self.predecessors.get(id).ok_or(TokenError::Invalid)?
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| TokenError::Invalid)?;
        if bytes.len() < 13 {
            return Err(TokenError::Invalid);
        }
        key.cipher
            .decrypt(
                Nonce::from_slice(&bytes[..12]),
                aes_gcm::aead::Payload {
                    msg: &bytes[12..],
                    aad: &key.aad(),
                },
            )
            .map_err(|_| TokenError::Invalid)
    }
}
