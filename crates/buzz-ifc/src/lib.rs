//! Derives Buzz execution domains and checks broker reads, calls, and publications.
//!
//! An [`ExecutionDomain`] records which agent is running, who may receive its
//! output, which conversations may share its saved state, and which operations
//! it may use. Its [`DomainKey`] includes all of those decisions, plus the owner
//! and membership version, so a change produces a different key.
//!
//! The broker must verify events and membership before supplying [`DomainFacts`].
//! It uses the resulting key to select both the agent's saved state and its
//! [`IfcSession`]. Keep that session across turns: recreating it would forget
//! whether unlabeled input had reached the agent.
//!
//! Check reads before delivering data, calls before executing them, and
//! publications with [`IfcSession::publish`] before sending them to a sink.
//! This crate does not verify signatures, check live membership, load saved
//! sessions, isolate agent processes, or execute operations itself.

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
