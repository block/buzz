//! Buzz-specific execution domains and broker checks built on `ifc-core`.
//!
//! The trusted broker verifies events, conversation metadata, membership, and
//! requester identities before constructing [`DomainFacts`]. This crate
//! deterministically derives an [`ExecutionDomain`] from those facts. The
//! domain binds the executing agent, authorized audience, retained context,
//! membership epoch, and effective capabilities into one stable routing key.
//! [`IfcSession`] then gives the broker one small checked surface for reads,
//! non-egressing calls, and publication.

#![forbid(unsafe_code)]

mod domain;
mod label;
mod session;

pub use domain::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, ConversationKind, DerivationError,
    DomainFacts, DomainKey, ExecutionDomain, MembershipEpoch, OperationEffect,
};
pub use label::{CommunityId, ConfidentialityLabel, LabelError, Principal, PrincipalError};
pub use session::{
    AuthorizedPublication, IfcError, IfcSession, PublicationRequest, PublicationTarget,
    ResourceLabel,
};

#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;
