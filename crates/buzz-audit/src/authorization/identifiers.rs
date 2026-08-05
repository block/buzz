use std::fmt;

use buzz_core::CommunityId;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroize;

use super::AuthorizationEvidenceError;

macro_rules! uuid_identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Validate and preserve an existing identifier.
            pub fn from_uuid(value: Uuid) -> Result<Self, AuthorizationEvidenceError> {
                if value.is_nil() {
                    return Err(AuthorizationEvidenceError::NilIdentifier);
                }
                Ok(Self(value))
            }

            /// Allocate a fresh random identifier.
            pub fn generate() -> Self {
                Self(Uuid::new_v4())
            }

            /// Borrow the exact UUID for storage and comparison.
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&"[redacted]")
                    .finish()
            }
        }
    };
}

uuid_identifier!(OperationId, "Stable identity for one semantic operation.");
uuid_identifier!(
    CorrelationId,
    "Cross-component correlation identity for one attempt."
);
uuid_identifier!(AttemptId, "Identity for one invocation attempt.");
uuid_identifier!(EventId, "Identity for one logical evidence event.");
uuid_identifier!(StreamId, "Identity for one durable evidence stream.");
uuid_identifier!(ReceiptId, "Identity for one immutable operation receipt.");
uuid_identifier!(
    EffectId,
    "Identity for one immutable post-commit effect intent."
);
uuid_identifier!(
    AuthorityEvidenceId,
    "Single-use identity for verified authority evidence."
);
uuid_identifier!(
    ApprovalEvidenceId,
    "Single-use identity for independently verified approval evidence."
);
uuid_identifier!(
    DeliveryAttemptId,
    "Identity for one exporter delivery attempt."
);
uuid_identifier!(ExporterId, "Identity for one exporter worker.");

/// Closed identity classes accepted by authorization evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ReferenceKind {
    /// Authenticated operator or request actor.
    Actor = 1,
    /// Independently authenticated approver.
    Approver = 2,
    /// Issuer-qualified principal.
    Principal = 3,
    /// Nostr or other public key.
    Key = 4,
    /// Stable identity-binding record.
    Binding = 5,
    /// Authorization lease.
    Lease = 6,
    /// Runtime session.
    Session = 7,
    /// Exact delegated relationship.
    DelegatedRelationship = 8,
    /// Authorization policy revision.
    Policy = 9,
}

/// Secret key dedicated to the authorization-audit pseudonym namespace.
pub struct PseudonymKey([u8; 32]);

impl PseudonymKey {
    /// Construct a key from already generated secret bytes.
    pub fn new(bytes: [u8; 32]) -> Result<Self, AuthorizationEvidenceError> {
        if bytes == [0; 32] {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Debug for PseudonymKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PseudonymKey([redacted])")
    }
}

impl Drop for PseudonymKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Domain- and kind-separated pseudonymous reference.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PseudonymousReference {
    kind: ReferenceKind,
    key_epoch: u32,
    digest: [u8; 32],
}

impl PseudonymousReference {
    /// Reference class.
    pub const fn kind(self) -> ReferenceKind {
        self.kind
    }

    /// Pseudonymization key epoch.
    pub const fn key_epoch(self) -> u32 {
        self.key_epoch
    }

    /// Stable digest within this domain, class, and key epoch.
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

impl fmt::Debug for PseudonymousReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PseudonymousReference")
            .field("kind", &self.kind)
            .field("key_epoch", &self.key_epoch)
            .field("digest", &"[redacted]")
            .finish()
    }
}

/// Derives audit-only pseudonyms without retaining raw identifiers.
pub struct Pseudonymizer {
    key: PseudonymKey,
    key_epoch: u32,
}

impl Pseudonymizer {
    /// Bind a dedicated audit key to its rotation epoch.
    pub const fn new(key: PseudonymKey, key_epoch: u32) -> Self {
        Self { key, key_epoch }
    }

    /// Derive a domain- and reference-kind-separated pseudonym.
    pub fn derive(
        &self,
        domain: CommunityId,
        kind: ReferenceKind,
        raw: &[u8],
    ) -> Result<PseudonymousReference, AuthorizationEvidenceError> {
        if self.key_epoch == 0 || raw.is_empty() || raw.len() > 4096 {
            return Err(AuthorizationEvidenceError::InvalidPseudonymInput);
        }
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.key.0)
            .map_err(|_| AuthorizationEvidenceError::InvalidPseudonymInput)?;
        Mac::update(&mut mac, b"buzz-authorization-audit-pseudonym-v1");
        Mac::update(&mut mac, domain.as_uuid().as_bytes());
        Mac::update(&mut mac, &[kind as u8]);
        Mac::update(&mut mac, &self.key_epoch.to_be_bytes());
        Mac::update(&mut mac, &(raw.len() as u64).to_be_bytes());
        Mac::update(&mut mac, raw);
        Ok(PseudonymousReference {
            kind,
            key_epoch: self.key_epoch,
            digest: mac.finalize().into_bytes().into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_nil_and_debug_redacts() {
        assert_eq!(
            EventId::from_uuid(Uuid::nil()),
            Err(AuthorizationEvidenceError::NilIdentifier)
        );
        assert!(format!("{:?}", EventId::generate()).contains("[redacted]"));
    }

    #[test]
    fn pseudonyms_are_domain_kind_and_epoch_separated() {
        let domain_a = CommunityId::from_uuid(Uuid::new_v4());
        let domain_b = CommunityId::from_uuid(Uuid::new_v4());
        let input = b"synthetic-principal";
        let a = Pseudonymizer::new(PseudonymKey::new([7; 32]).unwrap(), 1);
        let b = Pseudonymizer::new(PseudonymKey::new([7; 32]).unwrap(), 2);
        assert_ne!(
            a.derive(domain_a, ReferenceKind::Actor, input).unwrap(),
            a.derive(domain_b, ReferenceKind::Actor, input).unwrap()
        );
        assert_ne!(
            a.derive(domain_a, ReferenceKind::Actor, input).unwrap(),
            a.derive(domain_a, ReferenceKind::Principal, input).unwrap()
        );
        assert_ne!(
            a.derive(domain_a, ReferenceKind::Actor, input).unwrap(),
            b.derive(domain_a, ReferenceKind::Actor, input).unwrap()
        );
        assert_eq!(
            Pseudonymizer::new(PseudonymKey::new([7; 32]).unwrap(), 0).derive(
                domain_a,
                ReferenceKind::Actor,
                input
            ),
            Err(AuthorizationEvidenceError::InvalidPseudonymInput)
        );
        assert_eq!(
            PseudonymKey::new([0; 32]).unwrap_err(),
            AuthorizationEvidenceError::InvalidPseudonymInput
        );
    }
}
