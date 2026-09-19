pub use buzz_core::CommunityId;
pub use ifc_core::LabelError;
use nostr::{secp256k1::XOnlyPublicKey, PublicKey};
use serde::Serialize;

/// A person, agent, or relay identified by a valid Nostr public key.
///
/// The key is validated and stored in binary form. Hexadecimal case does not
/// affect equality or domain keys.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Principal(XOnlyPublicKey);

impl Principal {
    /// Parse a hexadecimal Nostr public key, rejecting invalid curve points.
    pub fn from_hex(value: &str) -> Result<Self, PrincipalError> {
        value
            .parse::<XOnlyPublicKey>()
            .map(Self)
            .map_err(|_| PrincipalError::InvalidPublicKey)
    }

    /// Convert a Nostr key after checking that it is a valid curve point.
    ///
    /// `PublicKey` can hold 32 bytes that do not represent an x-only secp256k1
    /// point. Reject those values before using them as reader identities.
    pub fn from_public_key(value: &PublicKey) -> Result<Self, PrincipalError> {
        let key = value
            .xonly()
            .map_err(|_| PrincipalError::InvalidPublicKey)?;
        Ok(Self(key))
    }

    /// Return the public key as lowercase hexadecimal.
    pub fn to_hex(&self) -> String {
        self.0.to_string()
    }

    pub(crate) fn to_bytes(self) -> [u8; 32] {
        self.0.serialize()
    }
}

/// Invalid Nostr public-key input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PrincipalError {
    /// The value is not a valid Nostr public key.
    #[error("invalid Nostr public key")]
    InvalidPublicKey,
}

/// Who may read a value in one Buzz community.
///
/// A public label allows everyone in that community; a restricted label names
/// the allowed readers. Labels from different communities cannot be combined.
pub type ConfidentialityLabel = ifc_core::ConfidentialityLabel<CommunityId, Principal>;

pub(crate) type ReaderSet = ifc_core::ReaderSet<Principal>;
