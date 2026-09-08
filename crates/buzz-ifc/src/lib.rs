//! Derives Buzz execution domains and checks broker reads, calls, and publications.
//!
//! An [`ExecutionDomain`] records which agent is running, who may receive its
//! output, which conversations may share its saved state, and which operations
//! it may use. Its [`DomainKey`] includes all of those decisions, plus the owner
//! and membership version, so a change produces a different key.
//!
//! The broker must verify events and membership before supplying [`DomainFacts`].
//! It uses the resulting key to select an [`IfcSession`], then checks reads and
//! calls before executing them. Publications must pass [`IfcSession::publish`]
//! before reaching a sink. This crate does not verify signatures, load saved
//! sessions, or execute operations itself.

#![forbid(unsafe_code)]

mod domain;
mod label;
mod session;

pub use domain::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, ConversationKind, DerivationError,
    DomainFacts, DomainKey, ExecutionDomain, MembershipEpoch, OperationEffect,
};
pub use label::{CommunityId, ConfidentialityLabel, LabelError, Principal, PrincipalError};
pub use session::{AuthorizedPublication, IfcError, IfcSession, ResourceLabel};

#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;
