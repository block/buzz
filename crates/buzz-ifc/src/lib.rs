//! Buzz-specific execution domains built on `ifc-core`.
//!
//! The trusted broker verifies events, conversation metadata, membership, and
//! requester identities before constructing [`DomainFacts`]. This crate
//! deterministically derives an [`ExecutionDomain`] from those facts. The
//! domain binds the executing agent, authorized audience, retained context,
//! membership epoch, and effective capabilities into one stable routing key.

mod domain;
mod label;

pub use domain::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, ConversationKind, DerivationError,
    DomainFacts, DomainKey, ExecutionDomain, MembershipEpoch, OperationEffect,
};
pub use label::{CommunityId, ConfidentialityLabel, LabelError, Principal, PrincipalError};

#[cfg(test)]
mod tests;
