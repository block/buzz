//! Derives Buzz execution domains and checks broker reads, calls, and publications.
//!
//! An [`ExecutionDomain`] records which agent is running, who may receive its
//! output, which conversations may share its saved state, and which operations
//! it may use. Structural equality compares all of those decisions, plus the
//! owner and community. Membership determines restricted audiences; reissuing
//! the same policy in a different event does not change the domain.
//!
//! The broker must verify events and membership before supplying [`DomainFacts`].
//! Domain equality permits policy-compatible reuse. The broker also keeps
//! conversation histories separate and manages lifecycle generations; equality
//! alone provides no epoch revocation. Keep the matching [`IfcSession`] for as
//! long as retained state survives: recreating it would forget whether unlabeled
//! input had reached the agent. Label raw conversation data with
//! [`ResourceLabel::from_conversation`] and derived state with
//! [`ResourceLabel::from_domain`].
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
    DomainFacts, ExecutionDomain, OperationEffect,
};
pub use label::{CommunityId, ConfidentialityLabel, LabelError, Principal, PrincipalError};
pub use session::{AuthorizedPublication, IfcError, IfcSession, ResourceLabel};

#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;
