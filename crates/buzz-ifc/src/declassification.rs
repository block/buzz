use std::marker::PhantomData;

use crate::domain::DomainContext;
use crate::label::{ConfidentialityLabel, Principal};

/// Typestate marker for a grant whose owner signature has not been checked.
pub struct PendingGrant;

/// Typestate marker for a grant authenticated by an external verifier.
pub struct VerifiedGrant;

/// Paper: "Declassification." A content- and destination-specific grant.
pub struct DeclassificationGrant<State> {
    approver: Principal,
    source_domain_id: String,
    destination: ConfidentialityLabel,
    destination_context: DomainContext,
    content_digest: [u8; 32],
    _state: PhantomData<State>,
}

/// Verifies the owner signature over a pending grant's canonical payload.
pub trait GrantSignatureVerifier {
    /// Return true only when the grant bears an authentic owner signature.
    fn verifies(&self, grant: &DeclassificationGrant<PendingGrant>) -> bool;
}

impl DeclassificationGrant<PendingGrant> {
    /// Construct an unverified grant from signed-event fields.
    pub fn pending(
        approver: Principal,
        source_domain_id: String,
        destination: ConfidentialityLabel,
        destination_context: DomainContext,
        content_digest: [u8; 32],
    ) -> Self {
        Self {
            approver,
            source_domain_id,
            destination,
            destination_context,
            content_digest,
            _state: PhantomData,
        }
    }

    /// Authenticate the owner and move the grant into the verified typestate.
    pub fn verify<V: GrantSignatureVerifier>(
        self,
        expected_owner: &Principal,
        verifier: &V,
    ) -> Result<DeclassificationGrant<VerifiedGrant>, GrantError> {
        if &self.approver != expected_owner {
            return Err(GrantError::WrongApprover);
        }
        if !verifier.verifies(&self) {
            return Err(GrantError::InvalidSignature);
        }
        Ok(DeclassificationGrant {
            approver: self.approver,
            source_domain_id: self.source_domain_id,
            destination: self.destination,
            destination_context: self.destination_context,
            content_digest: self.content_digest,
            _state: PhantomData,
        })
    }
}

impl DeclassificationGrant<VerifiedGrant> {
    pub(crate) fn matches(
        &self,
        source_domain_id: &str,
        destination: &ConfidentialityLabel,
        destination_context: &DomainContext,
        content_digest: &[u8; 32],
    ) -> bool {
        self.source_domain_id == source_domain_id
            && &self.destination == destination
            && &self.destination_context == destination_context
            && &self.content_digest == content_digest
    }
}

/// A declassification grant failed owner authentication.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GrantError {
    /// The signer is not the expected bot owner.
    #[error("declassification grant was signed by the wrong principal")]
    WrongApprover,
    /// The supplied signature did not authenticate the grant.
    #[error("declassification grant signature is invalid")]
    InvalidSignature,
}
