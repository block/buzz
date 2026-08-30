//! NIP-FI federated-identity authorization — canonical assertion verifier and
//! contracts (Phase A, PR 1).
//!
//! This module is the closed, provider-neutral contract layer at the root of
//! the NIP-FI dependency graph. It defines:
//!
//! - the multi-issuer assertion-policy [`config`] and the two deterministic
//!   semantic contract identities ([`AssertionPolicyId`],
//!   [`TransportContractId`]);
//! - the origin-sealed normalized [`VerifiedAssertion`] result (`FI-INV-16`);
//! - the single [`FederatedAssertionVerifier`] (`FI-INV-16` canonical verifier);
//! - the privacy-preserving four-class [`DenialClass`] wire contract
//!   (`FI-INV-13`).
//!
//! It has no dependencies on other NIP-FI PRs. It defines no database schema,
//! migration, runtime JWKS fetching, binding resolution, enrollment, or
//! request/proof binding — those belong to later PRs. Identity is issuer-
//! qualified `(iss, sub)` throughout: the `sub` claim is the fixed subject
//! coordinate and `nostr_pubkey` is the fixed key claim, never configurable,
//! so no deployment can seal a mutable attribute as identity. Issuer URL and
//! audience remain deployment configuration.

/// The exact client-attached header field ([NIP-FI.md](../../../docs/nips/NIP-FI.md),
/// "Client-attached transport"). `Authorization` remains reserved for NIP-98.
pub const CLIENT_ATTACHED_HEADER: &str = "Nostr-Federated-Identity";

pub mod assertion;
pub mod config;
pub mod denial;
pub mod verifier;

pub use assertion::{
    CanonicalCapabilities, ConfidentialAssertion, FederatedIdentity, RevalidationDependencies,
    VerifiedAssertion,
};
pub use config::{
    AssertionPolicyId, ClientSubjectPosture, FreshnessClass, IssuerPolicy, IssuerPolicyError,
    IssuerRegistry, SubjectClass, SubjectClassContract, TokenClass, TransportContractId,
    NOSTR_PUBKEY_CLAIM, OAUTH_CLIENT_ID_CLAIM,
};
pub use denial::DenialClass;
pub use verifier::{AssertionKeySet, FederatedAssertionVerifier, IssuerKeySource, VerifierError};
