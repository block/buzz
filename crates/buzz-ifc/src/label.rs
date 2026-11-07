use std::collections::BTreeSet;

use nostr::PublicKey;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::hash::hash_field;

/// A Buzz principal represented by a validated, normalized Nostr public key.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Principal(pub(crate) String);

impl Principal {
    /// Parse and normalize a hexadecimal Nostr public key.
    pub fn from_hex(value: &str) -> Result<Self, PrincipalError> {
        let key = PublicKey::from_hex(value).map_err(|_| PrincipalError::InvalidPublicKey)?;
        Self::from_public_key(&key)
    }

    /// Validate and convert a Nostr public key.
    ///
    /// PublicKey can hold any 32-byte value, including values that are not
    /// valid x-only secp256k1 points. IFC identities must reject those values
    /// before they enter reader sets or domain keys.
    pub fn from_public_key(value: &PublicKey) -> Result<Self, PrincipalError> {
        value
            .xonly()
            .map_err(|_| PrincipalError::InvalidPublicKey)?;
        Ok(Self(value.to_hex().to_ascii_lowercase()))
    }

    /// Return the normalized hexadecimal public key.
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

/// A principal could not be constructed from the supplied key.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PrincipalError {
    /// The value is not a valid Nostr public key.
    #[error("invalid Nostr public key")]
    InvalidPublicKey,
}

/// A confidentiality universe derived from one canonical Buzz relay URL.
///
/// Public data in one community is not public in another, so labels from
/// different realms never flow to one another.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RealmId(pub(crate) [u8; 32]);

impl RealmId {
    /// Derive a realm identifier from the relay URL selected by Buzz.
    pub fn from_relay_url(relay_url: &str) -> Self {
        Self(Sha256::digest(relay_url.as_bytes()).into())
    }

    /// Return a short identifier suitable for structured logs.
    pub fn fingerprint(&self) -> String {
        hex::encode(&self.0[..6])
    }

    pub(crate) fn stable_hash(&self, hasher: &mut Sha256) {
        hasher.update(self.0);
    }
}

/// The authorized readers of a value.
///
/// Paper: "Appendix: Security labels as a lattice — Labels and ordering."
/// `Everyone` is the public lattice element. An explicit set becomes more
/// restrictive as principals are removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReaderSet {
    Everyone,
    Only(BTreeSet<Principal>),
}

impl ReaderSet {
    /// The IFC ordering is reverse set inclusion: every reader at the
    /// destination must already be authorized to read the source.
    fn can_flow_to(&self, destination: &Self) -> bool {
        match (self, destination) {
            (Self::Everyone, _) => true,
            (Self::Only(_), Self::Everyone) => false,
            (Self::Only(source), Self::Only(destination)) => destination.is_subset(source),
        }
    }

    /// Paper: "Combining information." Inputs are joined by intersecting their
    /// authorized reader sets.
    pub(crate) fn join(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everyone, value) | (value, Self::Everyone) => value.clone(),
            (Self::Only(left), Self::Only(right)) => {
                Self::Only(left.intersection(right).cloned().collect())
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn meet(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Everyone, _) | (_, Self::Everyone) => Self::Everyone,
            (Self::Only(left), Self::Only(right)) => {
                Self::Only(left.union(right).cloned().collect())
            }
        }
    }

    fn explicit_count(&self) -> Option<usize> {
        match self {
            Self::Everyone => None,
            Self::Only(readers) => Some(readers.len()),
        }
    }

    pub(crate) fn stable_hash(&self, hasher: &mut Sha256) {
        match self {
            Self::Everyone => hasher.update(b"everyone"),
            Self::Only(readers) => {
                hasher.update(b"only");
                for reader in readers {
                    hash_field(hasher, reader.0.as_bytes());
                }
            }
        }
    }
}

/// A reader-set label within one Buzz community.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfidentialityLabel {
    pub(crate) realm: RealmId,
    pub(crate) readers: ReaderSet,
}

impl ConfidentialityLabel {
    /// Label data that every member of the realm may read.
    pub fn public(realm: RealmId) -> Self {
        Self {
            realm,
            readers: ReaderSet::Everyone,
        }
    }

    /// Label data with an explicit non-empty authorized reader set.
    pub fn restricted(realm: RealmId, readers: BTreeSet<Principal>) -> Result<Self, LabelError> {
        if readers.is_empty() {
            return Err(LabelError::EmptyReaderSet);
        }
        Ok(Self {
            realm,
            readers: ReaderSet::Only(readers),
        })
    }

    /// Return the realm in which this label is meaningful.
    pub fn realm(&self) -> &RealmId {
        &self.realm
    }

    /// Whether the label permits every member of the realm to read the value.
    pub fn is_public(&self) -> bool {
        matches!(self.readers, ReaderSet::Everyone)
    }

    /// Return the explicit number of readers, or `None` for public data.
    pub fn reader_count(&self) -> Option<usize> {
        self.readers.explicit_count()
    }

    /// Whether information with this label may flow to `destination`.
    pub fn can_flow_to(&self, destination: &Self) -> bool {
        self.realm == destination.realm && self.readers.can_flow_to(&destination.readers)
    }

    /// Combine the influence of two inputs.
    pub fn join(&self, other: &Self) -> Result<Self, LabelError> {
        if self.realm != other.realm {
            return Err(LabelError::CrossRealm);
        }
        Ok(Self {
            realm: self.realm.clone(),
            readers: self.readers.join(&other.readers),
        })
    }
}

/// A confidentiality label violates the reader-set lattice invariants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LabelError {
    /// Restricted information must name at least one authorized reader.
    #[error("restricted label has no authorized readers")]
    EmptyReaderSet,
    /// Labels from different Buzz communities cannot be combined.
    #[error("labels belong to different Buzz realms")]
    CrossRealm,
}
