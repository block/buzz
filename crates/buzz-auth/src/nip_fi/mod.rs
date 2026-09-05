//! NIP-FI federated-identity authorization — assertion verifier, JWKS runtime,
//! startup validation, and discovery.

/// The client-attached transport header for federated-identity assertions.
///
/// `Authorization` remains reserved for NIP-98; this separate header avoids
/// conflating authentication schemes at the relay ingress.
/// ([NIP-FI.md](../../../docs/nips/NIP-FI.md), "Client-attached transport")
pub const CLIENT_ATTACHED_HEADER: &str = "Nostr-Federated-Identity";

pub mod assertion;
pub mod command;
pub mod config;
pub mod denial;
pub mod deny_map;
pub mod discovery;
pub mod jwks;
pub mod startup;
pub mod verifier;

pub use assertion::{
    CanonicalCapabilities, ConfidentialAssertion, FederatedIdentity, RevalidationDependencies,
    VerifiedAssertion,
};
pub use command::{
    CommandError, CommandIssuerPolicy, CommandPolicyError, CommandResult, CommandVerifier,
    COMMAND_JWT_TYP, MAX_COMMAND_AGE_SECONDS,
};
pub use config::{
    AssertionPolicyId, ClientSubjectPosture, FreshnessClass, IssuerPolicy, IssuerPolicyError,
    IssuerRegistry, SubjectClass, SubjectClassContract, TokenClass, TransportContractId,
    NOSTR_PUBKEY_CLAIM, OAUTH_CLIENT_ID_CLAIM,
};
pub use denial::DenialClass;
pub use deny_map::{CrossPodMergeResult, DenySetFull, IssuerCapacity, NipFiDenyMap};
pub use discovery::{
    AssertionFreshnessDiscovery, FederatedIdentityDiscovery, FreshnessClassDiscovery,
};
pub use jwks::{
    HttpJwksFetcher, IssuerJwksConfig, JwksFetchError, JwksFetcher, JwksSourceContract,
    ProductionJwksSource,
};
pub use startup::{validate_nip_fi_config, NipFiMode, NipFiStartupError};
pub use verifier::{AssertionKeySet, FederatedAssertionVerifier, IssuerKeySource, VerifierError};
