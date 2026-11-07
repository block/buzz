//! Derives an agent's execution domain from Buzz membership and capability policy.
//!
//! An [`ExecutionDomain`] records which agent is running, who may receive its
//! output, which conversations may share its saved state, and which operations
//! it may use. Its [`DomainKey`] includes all of those decisions, plus the owner
//! and membership version, so a change produces a different key.
//!
//! The broker must verify events and membership before supplying [`DomainFacts`].
//! It must also use the resulting key when selecting a session. This crate
//! computes the domain; it does not verify signatures, manage sessions, or
//! enforce tool calls.

mod domain;
mod label;

pub use domain::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, ConversationKind, DerivationError,
    DomainFacts, DomainKey, ExecutionDomain, MembershipEpoch, OperationEffect,
};
pub use label::{CommunityId, ConfidentialityLabel, LabelError, Principal, PrincipalError};

#[cfg(test)]
mod tests;
