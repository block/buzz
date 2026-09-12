//! Multi-issuer assertion-policy configuration and the two NIP-FI semantic
//! contract identities.
//!
//! Identity is issuer-qualified `(iss, sub)`; there is no single-global-issuer
//! assumption. An [`IssuerRegistry`] selects exactly one [`IssuerPolicy`] by the
//! exact `iss` value returned by JWT decoding; a single-issuer deployment is
//! just a registry of length one.
//!
//! Buzz ships the generic OSS contract only: issuer URLs and audiences are
//! deployment configuration. The identity claim names are fixed — `sub` is the
//! subject coordinate and `nostr_pubkey` the bound key — so no deployment can
//! promote a mutable attribute into identity.
//!
//! Two deployment-local but deterministic identities are defined here
//! ([NIP-FI.md](../../../../docs/nips/NIP-FI.md), "Policy identity and
//! snapshots"):
//!
//! - [`AssertionPolicyId`] `= H(canonical assertion-policy contract)` — changes
//!   when accepted assertion semantics change, never when key or status
//!   snapshot contents rotate.
//! - [`TransportContractId`] `= H(canonical transport contract)` — identifies
//!   the client-attached field, parsing, attachment, no-fallback, and
//!   context-preservation semantics.

use jsonwebtoken::Algorithm;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

use super::jwks::JwksSourceContract;

/// Maximum accepted length of an `iss` or `aud` string.
const MAX_URI_LEN: usize = 2_048;
/// Maximum accepted length of a claim name.
const MAX_CLAIM_NAME_LEN: usize = 128;
/// Maximum accepted length of a configured claim value (subject-class markers).
const MAX_CLAIM_VALUE_LEN: usize = 2_048;
/// Maximum accepted clock skew, in seconds.
const MAX_SKEW_SECONDS: u64 = 300;
/// Maximum accepted assertion age, in seconds.
const MAX_ASSERTION_AGE_SECONDS: u64 = 86_400;

// Normative size rules for the assertion the verifier bounds before lookup or
// logging. They live here so they fold into `assertion_policy_id`: a change to
// any bound moves the ID mechanically. The verifier imports them.
/// Maximum accepted compact-JWS length, in bytes.
pub(crate) const MAX_TOKEN_BYTES: usize = 64 * 1024;
/// Maximum accepted `kid` length, in bytes.
pub(crate) const MAX_KID_BYTES: usize = 512;
/// Maximum accepted subject length, in bytes.
pub(crate) const MAX_SUBJECT_BYTES: usize = 2_048;
/// Maximum accepted `client_id` length, in bytes.
pub(crate) const MAX_CLIENT_ID_BYTES: usize = 2_048;
/// Maximum number of keys in one authenticated JWKS snapshot. The verifier
/// scans the snapshot by an attacker-controlled `kid` on every unauthenticated
/// token naming a configured issuer, so the authenticated key set is bounded
/// before lookup (NIP-FI.md "bounds the … authenticated key set before
/// lookup"). Real issuer JWKS carry a handful of keys even across rotation;
/// this cap blocks an oversized snapshot from turning each lookup into an
/// attacker-driven O(keys) scan.
pub(crate) const MAX_JWKS_KEYS: usize = 64;

/// The compiled-verifier-behavior fingerprint folded into every
/// [`AssertionPolicyId`]. It stands in for the normative semantic inputs that
/// are not otherwise field-encoded: duplicate-member rejection, exact-byte
/// (non-canonicalizing) identity handling, the JWKS-snapshot key-source
/// contract (kid selection, generation versioning, hard deadline), claim
/// capture, and the offline time arithmetic. **Bump on any change to those
/// semantics** so prepared evidence built against an older contract is
/// invalidated. Per-policy fields (issuer, class, bounds, …) are hashed
/// separately and need no bump.
///
/// v2 (PR #7221): `nostr_pubkey` absence now unconditionally rejects — the
/// per-issuer `require_attested_key` knob is removed and the NIP-FI v2 spec
/// requirement is always enforced.
pub(crate) const VERIFIER_CONTRACT_VERSION: u32 = 2;

/// The transport-contract fingerprint folded into [`TransportContractId`].
/// **Bump on any change** to the client-attached parsing, attachment,
/// no-fallback, or context-preservation semantics.
pub(crate) const TRANSPORT_CONTRACT_VERSION: u32 = 1;

/// The fixed name of the Nostr-key claim ([NIP-FI.md](../../../../docs/nips/NIP-FI.md),
/// "Assertion validation"). Not configurable: other encodings and aliases deny.
pub const NOSTR_PUBKEY_CLAIM: &str = "nostr_pubkey";

/// The fixed identity-subject claim. Identity is the exact tuple `(iss, sub)`
/// (NIP-FI.md:35-41), so the subject coordinate is always the JWT `sub` claim
/// and is never deployment-configurable: an operator cannot seal a mutable
/// attribute such as `email` or `display_name` as identity (NIP-FI.md:173-175,
/// :296-298). Attributes other than `sub` may be captured as claims/capabilities
/// but never as the identity coordinate.
pub const SUBJECT_CLAIM: &str = "sub";

/// Stable identifier for the accepted assertion-policy semantics.
///
/// Deliberately excludes key material, snapshot versions, and mutable state:
/// benign JWKS rotation must not change policy lineage.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssertionPolicyId([u8; 32]);

impl AssertionPolicyId {
    /// The stable 32-byte policy digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for AssertionPolicyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertionPolicyId({})", hex::encode(self.0))
    }
}

/// Stable identifier for the client-attached transport contract semantics.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportContractId([u8; 32]);

impl TransportContractId {
    /// The core client-attached transport contract identity.
    ///
    /// Covers the exact field name, `Bearer` parsing, request/upgrade
    /// attachment, no-fallback, and context-preservation semantics of
    /// [`super::CLIENT_ATTACHED_HEADER`]. Changing any of those semantics
    /// changes this constant; request data does not.
    pub fn core_client_attached() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"buzz:nip-fi:transport-contract:v1\0");
        // Explicit contract version: bump on any change to the parsing,
        // attachment, no-fallback, or context-preservation semantics below so
        // prepared evidence bound to an older transport contract is invalidated.
        hasher.update(TRANSPORT_CONTRACT_VERSION.to_be_bytes());
        hash_field(&mut hasher, super::CLIENT_ATTACHED_HEADER.as_bytes());
        hash_field(&mut hasher, b"Bearer");
        // No-fallback, request-attached, one-field, context-preserving.
        hash_field(
            &mut hasher,
            b"no-fallback;single-field;request-attached;server-owned-context",
        );
        Self(hasher.finalize().into())
    }

    /// The stable 32-byte transport-contract digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TransportContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransportContractId({})", hex::encode(self.0))
    }
}

/// The RFC 9068 / OAuth 2.0 access-token claim naming the OAuth client. An
/// `at+jwt` access token MUST carry exactly one non-empty bounded value.
/// Not deployment-configurable.
pub const OAUTH_CLIENT_ID_CLAIM: &str = "client_id";

/// Whether an issuer policy admits tokens whose subject represents the OAuth
/// client (client-credentials or client-subject tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSubjectPosture {
    /// Client-subject tokens are ineligible; only resource-owner tokens admit.
    Reject,
    /// Client-subject tokens are eligible. The issuer has guaranteed their
    /// `(iss, sub)` coordinates cannot collide with resource-owner coordinates
    /// (NIP-FI.md token-class rule); the operator records that guarantee here.
    AcceptNonColliding,
}

impl ClientSubjectPosture {
    const fn tag(self) -> &'static str {
        match self {
            Self::Reject => "client-subject:reject",
            Self::AcceptNonColliding => "client-subject:accept-non-colliding",
        }
    }
}

/// A closed, issuer-configured contract that classifies an access token's
/// subject as resource-owner or OAuth-client from one authenticated marker
/// claim, using mutually exclusive value sets. A token matching both sets or
/// neither is ambiguous and denies — "admits both interpretations" is
/// unrepresentable as an accepted result. When client-subject tokens are
/// admitted, the operator records the non-collision guarantee via
/// [`ClientSubjectPosture::AcceptNonColliding`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectClassContract {
    marker_claim: String,
    resource_owner_values: Vec<String>,
    client_subject_values: Vec<String>,
    posture: ClientSubjectPosture,
}

/// The classification of one token's subject under a [`SubjectClassContract`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectClass {
    /// The subject is the human/resource owner.
    ResourceOwner,
    /// The subject represents the OAuth client.
    ClientSubject,
}

impl SubjectClassContract {
    /// Build and validate a subject-class contract. The two value sets must be
    /// non-empty, bounded, and disjoint, so classification is total and
    /// mutually exclusive. Rejects overlap with [`IssuerPolicyError::NonExclusiveSubjectClass`].
    pub fn new(
        marker_claim: String,
        resource_owner_values: Vec<String>,
        client_subject_values: Vec<String>,
        posture: ClientSubjectPosture,
    ) -> Result<Self, IssuerPolicyError> {
        if marker_claim.is_empty() || marker_claim.len() > MAX_CLAIM_NAME_LEN {
            return Err(IssuerPolicyError::InvalidSubjectClaim);
        }
        let bounded = |vs: &[String]| {
            !vs.is_empty()
                && vs
                    .iter()
                    .all(|v| !v.is_empty() && v.len() <= MAX_CLAIM_VALUE_LEN)
        };
        if !bounded(&resource_owner_values) || !bounded(&client_subject_values) {
            return Err(IssuerPolicyError::NonExclusiveSubjectClass);
        }
        // These value sets are consumed as membership sets during
        // classification, so caller order and duplicates carry no semantics.
        // Canonicalize before storage so the derived policy ID is invariant
        // under permutation and duplication (NIP-FI.md "Policy identity").
        let resource_owner_values = canonical_set(resource_owner_values);
        let client_subject_values = canonical_set(client_subject_values);
        if resource_owner_values
            .iter()
            .any(|v| client_subject_values.contains(v))
        {
            return Err(IssuerPolicyError::NonExclusiveSubjectClass);
        }
        Ok(Self {
            marker_claim,
            resource_owner_values,
            client_subject_values,
            posture,
        })
    }

    /// The authenticated marker claim classified.
    pub fn marker_claim(&self) -> &str {
        &self.marker_claim
    }

    /// Values marking a resource-owner subject.
    pub fn resource_owner_values(&self) -> &[String] {
        &self.resource_owner_values
    }

    /// Values marking an OAuth-client subject.
    pub fn client_subject_values(&self) -> &[String] {
        &self.client_subject_values
    }

    /// The client-subject admission posture.
    pub const fn posture(&self) -> ClientSubjectPosture {
        self.posture
    }

    /// Classify a marker value. Exactly one set matches or the token is
    /// ambiguous. Values are compared by exact bytes.
    pub fn classify(&self, marker_value: Option<&str>) -> Option<SubjectClass> {
        let value = marker_value?;
        let ro = self.resource_owner_values.iter().any(|v| v == value);
        let cs = self.client_subject_values.iter().any(|v| v == value);
        match (ro, cs) {
            (true, false) => Some(SubjectClass::ResourceOwner),
            (false, true) => Some(SubjectClass::ClientSubject),
            // Disjoint sets make (true, true) impossible; (false, false) is an
            // unclassifiable subject.
            _ => None,
        }
    }
}

/// The single token class an issuer policy accepts before parsing claims.
/// Policy selects exactly one; failure under one class never triggers another.
///
/// Only `at+jwt` and `nip-fi+jwt` are offered. There is deliberately no
/// generic/absent-`typ` "named compatibility" variant: such a class cannot be
/// proven disjoint from an OIDC ID token by claim presence alone (an issuer can
/// mint an ID token carrying `client_id`), and the only authenticated
/// discriminator is `typ`, which that mode declines to constrain. Its absence
/// is a live regression — an external crate that names the removed variant
/// fails to compile:
///
/// ```compile_fail
/// use buzz_auth::TokenClass;
/// let _forge = TokenClass::NamedCompatibility {
///     required_claims: vec!["client_id".to_owned()],
///     forbidden_claims: vec![],
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenClass {
    /// RFC 9068 `at+jwt` access token: protected `typ` is exactly `at+jwt`.
    /// Validated under this document's claim contract, not the full RFC 9068
    /// profile. Requires one non-empty bounded `client_id`; its subject is
    /// classified by an authenticated [`SubjectClassContract`].
    AccessTokenAtJwt {
        /// The mutually exclusive resource-owner/client-subject contract.
        subject_class: SubjectClassContract,
    },
    /// A dedicated Buzz assertion: protected `typ` is exactly `nip-fi+jwt`.
    DedicatedNipFi,
}

impl TokenClass {
    fn discriminant(&self) -> &'static str {
        match self {
            Self::AccessTokenAtJwt { .. } => "at+jwt",
            Self::DedicatedNipFi => "nip-fi+jwt",
        }
    }
}

/// The server-owned freshness class an issuer policy declares. Folded into
/// [`AssertionPolicyId`]. The verifier validates the offline portion; a
/// `CurrentStatus` policy additionally requires a runtime status witness
/// (delivered by a later PR), which the verifier does not itself gather.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessClass {
    /// Validates the JWT and authenticated key snapshot only.
    OfflineJwt,
    /// Additionally requires an authenticated current-status witness at runtime.
    CurrentStatus,
}

impl FreshnessClass {
    const fn tag(self) -> &'static str {
        match self {
            Self::OfflineJwt => "offline-jwt",
            Self::CurrentStatus => "current-status",
        }
    }
}

/// One issuer's accepted assertion semantics. Its [`AssertionPolicyId`] is
/// derived from every field below; a semantic change changes the ID.
#[derive(Debug, Clone)]
pub struct IssuerPolicy {
    issuer: String,
    audiences: Vec<String>,
    token_class: TokenClass,
    freshness: FreshnessClass,
    algorithms: Vec<Algorithm>,
    skew_seconds: u64,
    maximum_assertion_age_seconds: u64,
    maximum_status_age_seconds: Option<u64>,
    /// The authenticated key-source contract: validated JWKS URI, refresh
    /// interval, and hard deadline. Included in `derive_assertion_policy_id`
    /// so that a change to the endpoint, refresh schedule, or hard-deadline
    /// rule changes the policy ID and invalidates all prepared evidence.
    jwks_source_contract: JwksSourceContract,
    id: AssertionPolicyId,
}

/// Why an [`IssuerPolicy`] could not be constructed. Independent of any token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IssuerPolicyError {
    /// `iss` was empty or exceeded the length bound.
    #[error("invalid issuer")]
    InvalidIssuer,
    /// The audience set was empty or contained an invalid value.
    #[error("invalid audience set")]
    InvalidAudiences,
    /// The subject claim name was empty or exceeded the length bound.
    #[error("invalid subject claim")]
    InvalidSubjectClaim,
    /// The algorithm set was empty or contained a symmetric or `none` algorithm.
    #[error("invalid algorithm set")]
    InvalidAlgorithms,
    /// A time or size rule was outside its accepted bound.
    #[error("invalid time bounds")]
    InvalidTimeBounds,
    /// `current-status` freshness requires a positive finite `maximum_status_age`.
    #[error("missing maximum status age")]
    MissingMaximumStatusAge,
    /// `offline-jwt` freshness never reads `maximum_status_age`, so accepting a
    /// value would move the policy ID for two semantically identical offline
    /// policies. It is rejected at construction.
    #[error("inapplicable maximum status age")]
    InapplicableMaximumStatusAge,
    /// A `SubjectClassContract`'s value sets were empty, unbounded, or overlapped,
    /// so subject classification could not be total and mutually exclusive.
    #[error("subject class contract is not exclusive")]
    NonExclusiveSubjectClass,
    /// The [`JwksSourceContract`] was not valid — invalid URI, zero or
    /// out-of-range timing, or `refresh_interval >= hard_deadline`.
    #[error("invalid JWKS source contract")]
    InvalidJwksSourceContract,
}

impl IssuerPolicy {
    /// Validate policy fields and derive its stable [`AssertionPolicyId`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer: String,
        audiences: Vec<String>,
        token_class: TokenClass,
        freshness: FreshnessClass,
        algorithms: Vec<Algorithm>,
        skew_seconds: u64,
        maximum_assertion_age_seconds: u64,
        maximum_status_age_seconds: Option<u64>,
        jwks_source_contract: JwksSourceContract,
    ) -> Result<Self, IssuerPolicyError> {
        // Identity-bearing strings are validated for bounds but never mutated:
        // exact `iss`/`aud`/`sub` bytes select policies and form the identity
        // tuple (NIP-FI.md, "Terms and identifier classes"). The subject
        // coordinate is the fixed `sub` claim, not a configurable name.
        if issuer.is_empty() || issuer.len() > MAX_URI_LEN {
            return Err(IssuerPolicyError::InvalidIssuer);
        }
        if audiences.is_empty()
            || audiences
                .iter()
                .any(|a| a.is_empty() || a.len() > MAX_URI_LEN)
        {
            return Err(IssuerPolicyError::InvalidAudiences);
        }
        if algorithms.is_empty() || !algorithms.iter().copied().all(is_asymmetric_algorithm) {
            return Err(IssuerPolicyError::InvalidAlgorithms);
        }
        if skew_seconds > MAX_SKEW_SECONDS
            || maximum_assertion_age_seconds == 0
            || maximum_assertion_age_seconds > MAX_ASSERTION_AGE_SECONDS
        {
            return Err(IssuerPolicyError::InvalidTimeBounds);
        }
        // `maximum_status_age` is read only by `current-status` verification.
        // Tie its applicability to the freshness class so semantically
        // identical offline policies always derive one ID: `current-status`
        // requires a positive finite value; `offline-jwt` must omit it. Both
        // rejects fail closed at construction, keeping the canonical ID
        // encoding total over valid configs (NIP-FI.md:176-181, :219-237).
        match (freshness, maximum_status_age_seconds) {
            (FreshnessClass::CurrentStatus, None) => {
                return Err(IssuerPolicyError::MissingMaximumStatusAge);
            }
            (FreshnessClass::CurrentStatus, Some(0)) => {
                return Err(IssuerPolicyError::InvalidTimeBounds);
            }
            (FreshnessClass::OfflineJwt, Some(_)) => {
                return Err(IssuerPolicyError::InapplicableMaximumStatusAge);
            }
            (FreshnessClass::CurrentStatus, Some(_)) | (FreshnessClass::OfflineJwt, None) => {}
        }

        // The verifier consumes audiences and algorithms as membership sets, so
        // caller order and duplicates carry no accepted-assertion semantics.
        // Canonicalize before storage and ID derivation so the policy ID is
        // invariant under permutation and duplication (NIP-FI.md "Policy
        // identity and snapshots"). Subject-class value sets are already
        // canonicalized in `SubjectClassContract::new`.
        let audiences = canonical_set(audiences);
        let algorithms = canonical_algorithm_set(algorithms);

        let id = derive_assertion_policy_id(
            &issuer,
            &audiences,
            &token_class,
            freshness,
            &algorithms,
            skew_seconds,
            maximum_assertion_age_seconds,
            maximum_status_age_seconds,
            &jwks_source_contract,
        );

        Ok(Self {
            issuer,
            audiences,
            token_class,
            freshness,
            algorithms,
            skew_seconds,
            maximum_assertion_age_seconds,
            maximum_status_age_seconds,
            jwks_source_contract,
            id,
        })
    }

    /// The exact `iss` value this policy is selected by.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The configured audiences; at least one must match the token `aud`.
    pub fn audiences(&self) -> &[String] {
        &self.audiences
    }

    /// The single accepted token class.
    pub fn token_class(&self) -> &TokenClass {
        &self.token_class
    }

    /// The declared freshness class.
    pub const fn freshness(&self) -> FreshnessClass {
        self.freshness
    }

    /// The accepted asymmetric algorithms.
    pub fn algorithms(&self) -> &[Algorithm] {
        &self.algorithms
    }

    /// The accepted clock skew, in seconds.
    pub const fn skew_seconds(&self) -> u64 {
        self.skew_seconds
    }

    /// The maximum assertion age, in seconds.
    pub const fn maximum_assertion_age_seconds(&self) -> u64 {
        self.maximum_assertion_age_seconds
    }

    /// The maximum status age, in seconds, when `current-status` is declared.
    pub const fn maximum_status_age_seconds(&self) -> Option<u64> {
        self.maximum_status_age_seconds
    }

    /// The stable policy identity.
    pub const fn id(&self) -> AssertionPolicyId {
        self.id
    }

    /// The authenticated key-source contract for this policy's JWKS endpoint.
    pub fn jwks_source_contract(&self) -> &JwksSourceContract {
        &self.jwks_source_contract
    }
}

/// A closed set of issuer policies keyed by exact `iss`. Selection preserves
/// every tuple component: equal `sub` under different `iss` are distinct
/// identities.
#[derive(Debug, Clone, Default)]
pub struct IssuerRegistry {
    policies: BTreeMap<String, IssuerPolicy>,
}

impl IssuerRegistry {
    /// An empty registry accepting no issuers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a policy. Returns the previous policy for the same `iss`, if any.
    pub fn insert(&mut self, policy: IssuerPolicy) -> Option<IssuerPolicy> {
        self.policies.insert(policy.issuer.clone(), policy)
    }

    /// Select the policy for an exact `iss`. No prefix, suffix, or normalization
    /// match is performed.
    pub fn policy_for_issuer(&self, issuer: &str) -> Option<&IssuerPolicy> {
        self.policies.get(issuer)
    }

    /// The number of registered issuers.
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Iteration order is deliberately unspecified; callers must not depend on
    /// registration order.
    pub fn all_policies(&self) -> impl Iterator<Item = &IssuerPolicy> {
        self.policies.values()
    }
}

/// Sort and deduplicate a set-valued list of strings into its canonical form.
/// Membership-set fields (audiences, subject-class values, compatibility claim
/// names) hash and compare identically under any caller permutation or
/// duplication once canonicalized.
fn canonical_set(mut values: Vec<String>) -> Vec<String> {
    values.sort_unstable();
    values.dedup();
    values
}

/// Canonicalize a set-valued algorithm list, ordered by its stable wire tag so
/// the derived policy ID is invariant under permutation and duplication.
fn canonical_algorithm_set(mut algorithms: Vec<Algorithm>) -> Vec<Algorithm> {
    algorithms.sort_unstable_by_key(|a| algorithm_tag(*a));
    algorithms.dedup();
    algorithms
}

/// Whether an algorithm is an accepted asymmetric signature algorithm.
/// `alg=none` and symmetric (HMAC) algorithms are always rejected.
pub(crate) fn is_asymmetric_algorithm(algorithm: Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

fn algorithm_tag(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::HS256 => "HS256",
        Algorithm::HS384 => "HS384",
        Algorithm::HS512 => "HS512",
        Algorithm::RS256 => "RS256",
        Algorithm::RS384 => "RS384",
        Algorithm::RS512 => "RS512",
        Algorithm::ES256 => "ES256",
        Algorithm::ES384 => "ES384",
        Algorithm::PS256 => "PS256",
        Algorithm::PS384 => "PS384",
        Algorithm::PS512 => "PS512",
        Algorithm::EdDSA => "EdDSA",
    }
}

#[allow(clippy::too_many_arguments)]
fn derive_assertion_policy_id(
    issuer: &str,
    audiences: &[String],
    token_class: &TokenClass,
    freshness: FreshnessClass,
    algorithms: &[Algorithm],
    skew_seconds: u64,
    maximum_assertion_age_seconds: u64,
    maximum_status_age_seconds: Option<u64>,
    jwks_source_contract: &JwksSourceContract,
) -> AssertionPolicyId {
    let mut hasher = Sha256::new();
    hasher.update(b"buzz:nip-fi:assertion-policy:v1\0");
    // Compiled-verifier-behavior fingerprint: covers duplicate-member
    // rejection, exact-byte identity handling, the key-source contract, claim
    // capture, and time arithmetic — the normative semantics not otherwise
    // field-encoded. A change to any of them bumps VERIFIER_CONTRACT_VERSION and
    // moves every policy ID.
    hasher.update(VERIFIER_CONTRACT_VERSION.to_be_bytes());
    // Normative size rules (NIP-FI.md "bounds the assertion, headers, claims,
    // subject, key identifiers, and authenticated key set … before lookup").
    for bound in [
        MAX_TOKEN_BYTES,
        MAX_KID_BYTES,
        MAX_SUBJECT_BYTES,
        MAX_CLIENT_ID_BYTES,
        MAX_JWKS_KEYS,
    ] {
        hasher.update((bound as u64).to_be_bytes());
    }
    hash_field(&mut hasher, issuer.as_bytes());
    hash_seq(&mut hasher, audiences.iter().map(String::as_bytes));
    hash_field(&mut hasher, token_class.discriminant().as_bytes());
    match token_class {
        TokenClass::AccessTokenAtJwt { subject_class } => {
            hash_field(&mut hasher, subject_class.marker_claim().as_bytes());
            hash_seq(
                &mut hasher,
                subject_class
                    .resource_owner_values()
                    .iter()
                    .map(String::as_bytes),
            );
            hash_seq(
                &mut hasher,
                subject_class
                    .client_subject_values()
                    .iter()
                    .map(String::as_bytes),
            );
            hash_field(&mut hasher, subject_class.posture().tag().as_bytes());
        }
        TokenClass::DedicatedNipFi => {}
    }
    hash_field(&mut hasher, freshness.tag().as_bytes());
    hash_field(&mut hasher, SUBJECT_CLAIM.as_bytes());
    hash_field(&mut hasher, NOSTR_PUBKEY_CLAIM.as_bytes());
    hash_seq(
        &mut hasher,
        algorithms.iter().map(|a| algorithm_tag(*a).as_bytes()),
    );
    hasher.update(skew_seconds.to_be_bytes());
    hasher.update(maximum_assertion_age_seconds.to_be_bytes());
    hasher.update(maximum_status_age_seconds.unwrap_or(0).to_be_bytes());
    // Authenticated key-source contract (NIP-FI.md, "Policy identity and
    // snapshots"): URI selects the authenticated source; interval defines
    // bounded refresh; hard deadline defines the accepted time rule. These are
    // contract, not mutable state — key rotation (JWKS content change) leaves
    // all three unchanged and must not move the ID.
    hasher.update(b"jwks-source-contract\0");
    hash_field(&mut hasher, jwks_source_contract.jwks_uri().as_bytes());
    hasher.update(
        jwks_source_contract
            .refresh_interval_seconds()
            .to_be_bytes(),
    );
    hasher.update(
        jwks_source_contract
            .key_snapshot_hard_deadline_seconds()
            .to_be_bytes(),
    );
    AssertionPolicyId(hasher.finalize().into())
}

/// Length-prefix one field so distinct field boundaries cannot collide.
fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

/// Length-prefix a sequence: element count, then each length-prefixed element.
fn hash_seq<'a>(hasher: &mut Sha256, items: impl ExactSizeIterator<Item = &'a [u8]>) {
    hasher.update((items.len() as u64).to_be_bytes());
    for item in items {
        hash_field(hasher, item);
    }
}
