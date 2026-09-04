pub use buzz_core::CommunityId;
pub use ifc_core::LabelError;
use nostr::{secp256k1::XOnlyPublicKey, PublicKey};
use serde::Serialize;

/// A Buzz principal represented by a validated, normalized Nostr public key.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Principal(XOnlyPublicKey);

impl Principal {
    /// Parse and normalize a hexadecimal Nostr public key.
    pub fn from_hex(value: &str) -> Result<Self, PrincipalError> {
        value
            .parse::<XOnlyPublicKey>()
            .map(Self)
            .map_err(|_| PrincipalError::InvalidPublicKey)
    }

    /// Validate and convert a Nostr public key.
    ///
    /// PublicKey can hold any 32-byte value, including values that are not
    /// valid x-only secp256k1 points. IFC identities must reject those values
    /// before they enter reader sets or domain keys.
    pub fn from_public_key(value: &PublicKey) -> Result<Self, PrincipalError> {
        let key = value
            .xonly()
            .map_err(|_| PrincipalError::InvalidPublicKey)?;
        Ok(Self(key))
    }

    /// Return the normalized hexadecimal public key.
    pub fn to_hex(&self) -> String {
        self.0.to_string()
    }

    pub(crate) fn to_bytes(self) -> [u8; 32] {
        self.0.serialize()
    }
}

/// A principal could not be constructed from the supplied key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PrincipalError {
    /// The value is not a valid Nostr public key.
    #[error("invalid Nostr public key")]
    InvalidPublicKey,
}

/// A reader-set confidentiality label within one Buzz community.
pub type ConfidentialityLabel = ifc_core::ConfidentialityLabel<CommunityId, Principal>;

pub(crate) type ReaderSet = ifc_core::ReaderSet<Principal>;
