//! The closed, provider-neutral normalized result of assertion validation
//! (`FI-INV-16`, canonical verifier).
//!
//! [`VerifiedAssertion`] is an origin-sealed value: its constructor is
//! crate-private, so an unverified claim set cannot be promoted into authority.
//! Every assertion transport feeds this one contract and none can fork final
//! admission.
//!
//! Per the settled spec ([NIP-FI.md](../../../../docs/nips/NIP-FI.md),
//! "Assertion validation"), the result carries the issuer-qualified identity,
//! the optional asserted key, the canonical claims/capabilities, the non-empty
//! `authority_deadlines`, both semantic contract identities, and the
//! `revalidation_dependencies`. Request/connection binding is *not* part of this
//! value: the actor comes from fresh Nostr proof and the request is sealed
//! separately during preparation.

use super::config::{AssertionPolicyId, TransportContractId};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use std::fmt;

/// The issuer-qualified identity `(iss, sub)` returned by validation. Email,
/// display name, employee number, and a bare `sub` are not identities. Equal
/// `sub` under different `iss` are distinct identities.
#[derive(Clone, PartialEq, Eq)]
pub struct FederatedIdentity {
    issuer: String,
    subject: String,
}

impl FederatedIdentity {
    /// The exact issuer.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The exact opaque subject.
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

impl fmt::Debug for FederatedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redacted: identity is a private per-principal fact.
        f.write_str("FederatedIdentity([REDACTED])")
    }
}

/// A confidential handle to the exact compact JWS that produced a
/// [`VerifiedAssertion`]. Final admission revalidates the byte-identical
/// assertion against current state, so carrying the exact token lets a changed
/// key snapshot re-verify the same evidence and a removed key deny
/// (NIP-FI.md:240-249, :371-395). The handle is confidential: it deliberately
/// has no `Debug`, `Display`, or `serde` implementation, so the token cannot
/// leak through a formatting, logging, or serialization path. The exact bytes
/// are exposed only through [`Self::compact_jws`] for in-deployment
/// revalidation. Equality is by exact bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfidentialAssertion {
    compact_jws: String,
}

impl ConfidentialAssertion {
    /// The exact compact JWS, for final-admission revalidation only. This is
    /// the sole read path; there is no `Debug`/`Display`/`serde` exposure.
    pub fn compact_jws(&self) -> &str {
        &self.compact_jws
    }
}

/// The exact key-snapshot member of `revalidation_dependencies`: the
/// verification-key identity, the snapshot generation that authenticated the
/// assertion, the key-snapshot hard deadline, and a confidential handle to the
/// exact compact JWS. A changed generation requires revalidation; a removed key
/// denies (NIP-FI.md:240-249).
#[derive(Clone, PartialEq, Eq)]
pub struct RevalidationDependencies {
    verification_key_id: String,
    key_snapshot_generation: u64,
    key_snapshot_hard_deadline: DateTime<Utc>,
    confidential_assertion: ConfidentialAssertion,
}

impl RevalidationDependencies {
    /// The `kid` of the JWK that verified the signature.
    pub fn verification_key_id(&self) -> &str {
        &self.verification_key_id
    }

    /// The generation of the key snapshot used for verification.
    pub const fn key_snapshot_generation(&self) -> u64 {
        self.key_snapshot_generation
    }

    /// The hard deadline of the key snapshot that authenticated the assertion.
    /// A bounds-class dependency: the sealed authority ends no later than this.
    pub const fn key_snapshot_hard_deadline(&self) -> DateTime<Utc> {
        self.key_snapshot_hard_deadline
    }

    /// The confidential handle to the exact compact JWS, for revalidation.
    pub const fn confidential_assertion(&self) -> &ConfidentialAssertion {
        &self.confidential_assertion
    }
}

impl fmt::Debug for RevalidationDependencies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RevalidationDependencies([REDACTED])")
    }
}

/// The closed normalized result of a successful assertion validation.
///
/// Origin-sealed: only [`super::verifier`] can construct one.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedAssertion {
    identity: FederatedIdentity,
    asserted_key: Option<PublicKey>,
    capabilities: CanonicalCapabilities,
    authority_deadlines: Vec<DateTime<Utc>>,
    assertion_policy_id: AssertionPolicyId,
    transport_contract_id: TransportContractId,
    revalidation_dependencies: RevalidationDependencies,
}

impl VerifiedAssertion {
    /// Crate-private constructor invoked only by the verifier after every check
    /// has passed. `authority_deadlines` must be non-empty.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn seal(
        issuer: String,
        subject: String,
        asserted_key: Option<PublicKey>,
        capabilities: CanonicalCapabilities,
        authority_deadlines: Vec<DateTime<Utc>>,
        assertion_policy_id: AssertionPolicyId,
        transport_contract_id: TransportContractId,
        revalidation_dependencies: RevalidationDependencies,
    ) -> Self {
        debug_assert!(
            !authority_deadlines.is_empty(),
            "authority_deadlines must be non-empty"
        );
        Self {
            identity: FederatedIdentity { issuer, subject },
            asserted_key,
            capabilities,
            authority_deadlines,
            assertion_policy_id,
            transport_contract_id,
            revalidation_dependencies,
        }
    }

    /// The issuer-qualified identity.
    pub fn identity(&self) -> &FederatedIdentity {
        &self.identity
    }

    /// The key the assertion attests, when present. In attested-key enrollment
    /// this must equal the proven actor.
    pub const fn asserted_key(&self) -> Option<PublicKey> {
        self.asserted_key
    }

    /// The canonical closed claims/capabilities carried by the assertion.
    pub const fn capabilities(&self) -> &CanonicalCapabilities {
        &self.capabilities
    }

    /// The non-empty set of authority deadlines. Every member bounds a lease.
    pub fn authority_deadlines(&self) -> &[DateTime<Utc>] {
        &self.authority_deadlines
    }

    /// The earliest offline authority deadline — the `upstream_authority_deadline`
    /// for `offline-jwt`, before any status witness is applied.
    pub fn upstream_authority_deadline(&self) -> DateTime<Utc> {
        self.authority_deadlines
            .iter()
            .copied()
            .min()
            .expect("authority_deadlines is non-empty by construction")
    }

    /// The stable assertion-policy identity.
    pub const fn assertion_policy_id(&self) -> AssertionPolicyId {
        self.assertion_policy_id
    }

    /// The stable transport-contract identity.
    pub const fn transport_contract_id(&self) -> TransportContractId {
        self.transport_contract_id
    }

    /// The mutable dependencies that must be revalidated under current state.
    pub const fn revalidation_dependencies(&self) -> &RevalidationDependencies {
        &self.revalidation_dependencies
    }
}

impl fmt::Debug for VerifiedAssertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VerifiedAssertion([REDACTED])")
    }
}

impl RevalidationDependencies {
    pub(super) fn new(
        verification_key_id: String,
        key_snapshot_generation: u64,
        key_snapshot_hard_deadline: DateTime<Utc>,
        compact_jws: String,
    ) -> Self {
        Self {
            verification_key_id,
            key_snapshot_generation,
            key_snapshot_hard_deadline,
            confidential_assertion: ConfidentialAssertion { compact_jws },
        }
    }
}

/// A closed, deterministically encoded set of authorization claims/capabilities
/// captured from the assertion. Only claim names the policy explicitly reads
/// enter it; unchecked claims never do. The canonical encoding sorts by
/// `(name, value)` and deduplicates so equal authoritative input yields
/// byte-equal capabilities regardless of token order or repetition.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CanonicalCapabilities {
    // Sorted by (key, value) and deduplicated for a deterministic canonical
    // encoding.
    entries: Vec<(String, String)>,
}

impl CanonicalCapabilities {
    /// Build from a set of `(claim_name, value)` pairs, canonicalized by
    /// `(name, value)` order with duplicates removed. Membership-set semantics:
    /// a repeated pair carries no more authority than a single occurrence.
    pub(super) fn from_pairs(mut entries: Vec<(String, String)>) -> Self {
        entries.sort();
        entries.dedup();
        Self { entries }
    }

    /// The canonical `(name, value)` entries in sorted order.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// Whether any capability claim was captured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for CanonicalCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CanonicalCapabilities([REDACTED])")
    }
}
