//! Provider-free local authorization composition.
//!
//! Verified assertions and delegations are origin-sealed values: constructors
//! are crate-private so unverified claims cannot be promoted into authority.
//! Storage resolution and finalization are separate phases, and exactly one
//! finalizer creates [`AuthContext`].

use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
    Algorithm, DecodingKey, Validation,
};
use nostr::{FromBech32, PublicKey};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::future::Future;
use thiserror::Error;
use uuid::Uuid;

/// Hard implementation ceiling for one domain's retained audit events.
pub const HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN: u64 = 1_000_000;
/// Hard implementation ceiling for one domain's retained canonical bytes.
pub const HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN: u64 = 4_294_967_296;
/// Hard implementation ceiling for one canonical event envelope.
pub const HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES: u32 = 65_536;

/// Minimal stock NIP-FI operating modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NipFiMode {
    /// NIP-FI protected composition is absent. This is the stock default.
    Off,
    /// Protected routes require complete provider-free local composition.
    Enforce,
    /// Emergency mode denying protected routes before verifier setup.
    DenyProtected,
}

/// Validated immutable-capacity policy required by enabled composition.
///
/// V1 intentionally has no online prune, export, reset, claim, acknowledgement,
/// or retry workflow. Operators must size this policy for the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationEventCapacityPolicy {
    max_events_per_domain: u64,
    max_bytes_per_domain: u64,
    max_envelope_bytes: u32,
}

impl AuthorizationEventCapacityPolicy {
    /// Validate an installation's explicit immutable-capacity bounds.
    pub fn new(
        max_events_per_domain: u64,
        max_bytes_per_domain: u64,
        max_envelope_bytes: u32,
    ) -> Result<Self, AuthorizationEventCapacityPolicyError> {
        if max_events_per_domain == 0
            || max_events_per_domain > HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN
        {
            return Err(AuthorizationEventCapacityPolicyError::EventCount);
        }
        if max_bytes_per_domain == 0
            || max_bytes_per_domain > HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN
        {
            return Err(AuthorizationEventCapacityPolicyError::DomainBytes);
        }
        if max_envelope_bytes == 0
            || max_envelope_bytes > HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES
            || u64::from(max_envelope_bytes) > max_bytes_per_domain
        {
            return Err(AuthorizationEventCapacityPolicyError::EnvelopeBytes);
        }
        Ok(Self {
            max_events_per_domain,
            max_bytes_per_domain,
            max_envelope_bytes,
        })
    }

    /// Maximum retained events in one authorization domain.
    pub const fn max_events_per_domain(self) -> u64 {
        self.max_events_per_domain
    }

    /// Maximum retained canonical envelope bytes in one domain.
    pub const fn max_bytes_per_domain(self) -> u64 {
        self.max_bytes_per_domain
    }

    /// Maximum canonical bytes in one event envelope.
    pub const fn max_envelope_bytes(self) -> u32 {
        self.max_envelope_bytes
    }
}

/// Capacity-policy validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorizationEventCapacityPolicyError {
    /// The per-domain event count was zero or exceeded the implementation cap.
    #[error("invalid per-domain authorization event count")]
    EventCount,
    /// The per-domain byte limit was zero or exceeded the implementation cap.
    #[error("invalid per-domain authorization event byte limit")]
    DomainBytes,
    /// The envelope limit was zero, too large, or larger than domain capacity.
    #[error("invalid authorization event envelope byte limit")]
    EnvelopeBytes,
}

/// Audit-capacity portion of a domain's Base V1 configuration.
///
/// This is deliberately not the complete enabled configuration. Enforce mode
/// additionally requires verifier, audience, transport, enrollment, token-age,
/// clock-skew, and lease bounds owned by the composition layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationAuditConfig {
    mode: NipFiMode,
    event_capacity: Option<AuthorizationEventCapacityPolicy>,
}

impl AuthorizationAuditConfig {
    /// Validate the audit-capacity portion for one closed operating mode.
    pub fn new(
        mode: NipFiMode,
        event_capacity: Option<AuthorizationEventCapacityPolicy>,
    ) -> Result<Self, AuthorizationAuditConfigError> {
        match (mode, event_capacity) {
            (NipFiMode::Enforce, None) => Err(AuthorizationAuditConfigError::MissingEventCapacity),
            (NipFiMode::Off | NipFiMode::DenyProtected, Some(_)) => {
                Err(AuthorizationAuditConfigError::UnexpectedEventCapacity)
            }
            _ => Ok(Self {
                mode,
                event_capacity,
            }),
        }
    }

    /// The domain's closed operating mode.
    pub const fn mode(self) -> NipFiMode {
        self.mode
    }

    /// Enabled-mode audit capacity, absent for Off and emergency denial.
    pub const fn event_capacity(self) -> Option<AuthorizationEventCapacityPolicy> {
        self.event_capacity
    }
}

/// Domain-configuration validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorizationAuditConfigError {
    /// Enforce mode must explicitly size the immutable audit store.
    #[error("enforce mode requires authorization event capacity")]
    MissingEventCapacity,
    /// Disabled and emergency-denial modes do not initialize an audit writer.
    #[error("authorization event capacity is only valid in enforce mode")]
    UnexpectedEventCapacity,
}

/// Transport on which the relay verified the Nostr proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofTransport {
    /// NIP-42 WebSocket AUTH.
    Nip42,
    /// Generic NIP-98 HTTP authorization.
    Nip98,
    /// Git smart-HTTP session authorization.
    GitSmartHttpSession,
    /// Blossom media HTTP authorization.
    Blossom,
}

/// Stable identifier for verifier policy, deliberately excluding key material.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanonicalVerifierPolicyId([u8; 32]);

impl CanonicalVerifierPolicyId {
    /// Return the stable policy digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for CanonicalVerifierPolicyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalVerifierPolicyId([REDACTED])")
    }
}

/// Positive monotonic generation assigned to one fetched verifier key set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VerifierKeyGeneration(u64);

impl VerifierKeyGeneration {
    /// Validate a nonzero key-set generation.
    pub const fn new(generation: u64) -> Option<Self> {
        if generation == 0 {
            None
        } else {
            Some(Self(generation))
        }
    }

    /// Return the positive generation number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Exact stable-policy and rotating-key generation used for verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifierPolicyStamp {
    policy_id: CanonicalVerifierPolicyId,
    key_generation: VerifierKeyGeneration,
}

impl VerifierPolicyStamp {
    /// Construct a stamp from a validated policy and current key generation.
    pub const fn new(
        policy_id: CanonicalVerifierPolicyId,
        key_generation: VerifierKeyGeneration,
    ) -> Self {
        Self {
            policy_id,
            key_generation,
        }
    }

    /// Stable verifier policy identifier.
    pub const fn policy_id(self) -> CanonicalVerifierPolicyId {
        self.policy_id
    }

    /// Rotating key-set generation used for the evidence.
    pub const fn key_generation(self) -> VerifierKeyGeneration {
        self.key_generation
    }
}

/// Provider-free JWT verifier policy whose identifier survives JWKS rotation.
#[derive(Clone)]
pub struct CanonicalVerifierPolicy {
    issuer: String,
    audience: String,
    subject_claim: String,
    event_author_claim: Option<String>,
    clock_skew_seconds: u64,
    maximum_token_lifetime_seconds: u64,
    id: CanonicalVerifierPolicyId,
}

impl CanonicalVerifierPolicy {
    /// Validate policy fields and derive a domain-separated stable identifier.
    pub fn new(
        issuer: String,
        audience: String,
        subject_claim: String,
        event_author_claim: Option<String>,
        clock_skew_seconds: u64,
        maximum_token_lifetime_seconds: u64,
    ) -> Result<Self, CanonicalVerifierError> {
        if issuer.trim().is_empty()
            || audience.trim().is_empty()
            || subject_claim.trim().is_empty()
            || event_author_claim
                .as_ref()
                .is_some_and(|claim| claim.trim().is_empty())
            || issuer.len() > 2_048
            || audience.len() > 2_048
            || subject_claim.len() > 128
            || event_author_claim
                .as_ref()
                .is_some_and(|claim| claim.len() > 128)
            || clock_skew_seconds > 300
            || maximum_token_lifetime_seconds == 0
            || maximum_token_lifetime_seconds > 86_400
        {
            return Err(CanonicalVerifierError::InvalidPolicy);
        }

        let mut hasher = Sha256::new();
        hasher.update(b"buzz:nip-fi:canonical-verifier-policy:v1\0");
        for value in [
            issuer.as_bytes(),
            audience.as_bytes(),
            subject_claim.as_bytes(),
            event_author_claim.as_deref().unwrap_or("").as_bytes(),
        ] {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value);
        }
        hasher.update(clock_skew_seconds.to_be_bytes());
        hasher.update(maximum_token_lifetime_seconds.to_be_bytes());
        let id = CanonicalVerifierPolicyId(hasher.finalize().into());

        Ok(Self {
            issuer,
            audience,
            subject_claim,
            event_author_claim,
            clock_skew_seconds,
            maximum_token_lifetime_seconds,
            id,
        })
    }

    /// Stable identifier excluding JWKS bytes, key ids, and key generation.
    pub const fn id(&self) -> CanonicalVerifierPolicyId {
        self.id
    }
}

impl fmt::Debug for CanonicalVerifierPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalVerifierPolicy([REDACTED])")
    }
}

/// One immutable rotating key-set snapshot admitted by the canonical verifier.
#[derive(Clone)]
pub struct CanonicalVerifierKeySet {
    generation: VerifierKeyGeneration,
    keys: JwkSet,
}

impl CanonicalVerifierKeySet {
    /// Seal a parsed JWKS with its positive cache generation.
    pub const fn new(generation: VerifierKeyGeneration, keys: JwkSet) -> Self {
        Self { generation, keys }
    }

    /// Key generation carried into evidence and the final recheck witness.
    pub const fn generation(&self) -> VerifierKeyGeneration {
        self.generation
    }
}

impl fmt::Debug for CanonicalVerifierKeySet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalVerifierKeySet([REDACTED])")
    }
}

#[derive(Debug, Deserialize)]
struct CanonicalRawJwtClaims {
    #[serde(flatten)]
    claims: Map<String, Value>,
}

/// The sole provider-free JWT verifier used to mint federated evidence.
#[derive(Clone)]
pub struct CanonicalFederatedAssertionVerifier {
    policy: CanonicalVerifierPolicy,
}

impl CanonicalFederatedAssertionVerifier {
    /// Construct a verifier from stable policy. Key material is supplied as an
    /// explicit generation snapshot for every verification operation.
    pub const fn new(policy: CanonicalVerifierPolicy) -> Self {
        Self { policy }
    }

    /// Stable policy identifier, unaffected by JWKS rotation.
    pub const fn policy_id(&self) -> CanonicalVerifierPolicyId {
        self.policy.id()
    }

    /// Verify a JWT and mint origin-sealed evidence carrying the key generation.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        &self,
        token: &str,
        key_set: &CanonicalVerifierKeySet,
        authorization_domain: CommunityId,
        transport: ProofTransport,
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
    ) -> Result<VerifiedFederatedAssertion, CanonicalVerifierError> {
        if token.is_empty()
            || token.len() > 64 * 1024
            || authorization_domain.as_uuid().is_nil()
            || target_fingerprint == [0; 32]
            || request_fingerprint == [0; 32]
            || transport_context_fingerprint == [0; 32]
        {
            return Err(CanonicalVerifierError::MalformedInput);
        }

        let header = decode_header(token).map_err(|_| CanonicalVerifierError::InvalidToken)?;
        if !canonical_algorithm_allowed(header.alg) {
            return Err(CanonicalVerifierError::UnsupportedAlgorithm);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty() && kid.len() <= 512)
            .ok_or(CanonicalVerifierError::MissingKeyId)?;
        let mut matching_keys = key_set
            .keys
            .keys
            .iter()
            .filter(|jwk| jwk.common.key_id.as_deref() == Some(kid));
        let jwk = matching_keys
            .next()
            .ok_or(CanonicalVerifierError::UnknownKeyId)?;
        if matching_keys.next().is_some() {
            return Err(CanonicalVerifierError::InvalidKey);
        }
        validate_canonical_jwk(jwk, header.alg)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| CanonicalVerifierError::InvalidKey)?;

        let mut validation = Validation::new(header.alg);
        validation.leeway = self.policy.clock_skew_seconds;
        validation.set_issuer(&[self.policy.issuer.as_str()]);
        validation.set_audience(&[self.policy.audience.as_str()]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud"]);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        let decoded = decode::<CanonicalRawJwtClaims>(token, &key, &validation)
            .map_err(|_| CanonicalVerifierError::InvalidToken)?;

        let subject = canonical_claim_string(&decoded.claims.claims, &self.policy.subject_claim)?;
        let issued_at = canonical_claim_i64(&decoded.claims.claims, "iat")?;
        let not_before = canonical_optional_claim_i64(&decoded.claims.claims, "nbf")?
            .unwrap_or(issued_at)
            .max(issued_at);
        let expires_at = canonical_claim_i64(&decoded.claims.claims, "exp")?;
        let lifetime = expires_at
            .checked_sub(issued_at)
            .ok_or(CanonicalVerifierError::InvalidTimeBounds)?;
        if lifetime <= 0 || lifetime as u64 > self.policy.maximum_token_lifetime_seconds {
            return Err(CanonicalVerifierError::InvalidTimeBounds);
        }
        let not_before = DateTime::from_timestamp(not_before, 0)
            .ok_or(CanonicalVerifierError::InvalidTimeBounds)?;
        let expires_at = DateTime::from_timestamp(expires_at, 0)
            .ok_or(CanonicalVerifierError::InvalidTimeBounds)?;

        let attested_event_author_pubkey = self
            .policy
            .event_author_claim
            .as_deref()
            .map(|claim| canonical_claim_string(&decoded.claims.claims, claim))
            .transpose()?
            .map(|claim| canonical_parse_pubkey(&claim))
            .transpose()?;
        let principal =
            FederatedPrincipal::from_verified_parts(self.policy.issuer.clone(), subject)
                .ok_or(CanonicalVerifierError::InvalidClaim)?;
        let assertion_fingerprint: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let stamp = VerifierPolicyStamp::new(self.policy.id(), key_set.generation());

        VerifiedFederatedAssertion::from_verifier_with_stamp(
            authorization_domain,
            principal,
            transport,
            assertion_fingerprint,
            target_fingerprint,
            request_fingerprint,
            transport_context_fingerprint,
            not_before,
            expires_at,
            stamp,
            attested_event_author_pubkey,
        )
        .ok_or(CanonicalVerifierError::MalformedInput)
    }
}

impl fmt::Debug for CanonicalFederatedAssertionVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalFederatedAssertionVerifier([REDACTED])")
    }
}

/// Closed, stable canonical-verifier failures with no credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CanonicalVerifierError {
    /// Stable verifier policy was invalid.
    #[error("invalid canonical verifier policy")]
    InvalidPolicy,
    /// Token or request-bound coordinates were malformed.
    #[error("malformed canonical verifier input")]
    MalformedInput,
    /// JWT header or signed claims failed canonical verification.
    #[error("canonical verifier rejected token")]
    InvalidToken,
    /// JWT used an algorithm outside the asymmetric allow-list.
    #[error("canonical verifier rejected algorithm")]
    UnsupportedAlgorithm,
    /// JWT header omitted its bounded key identifier.
    #[error("canonical verifier requires key id")]
    MissingKeyId,
    /// Key id was absent from the supplied generation snapshot.
    #[error("canonical verifier key id unavailable")]
    UnknownKeyId,
    /// JWK was not admissible for signature verification.
    #[error("canonical verifier rejected key")]
    InvalidKey,
    /// Required provider-free subject or event-author claim was invalid.
    #[error("canonical verifier rejected claim")]
    InvalidClaim,
    /// JWT lifetime was empty, malformed, or exceeded policy.
    #[error("canonical verifier rejected time bounds")]
    InvalidTimeBounds,
}

impl CanonicalVerifierError {
    /// Unique stable machine code safe for logs, metrics, and clients.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "nip_fi_verifier_invalid_policy",
            Self::MalformedInput => "nip_fi_verifier_malformed_input",
            Self::InvalidToken => "nip_fi_verifier_invalid_token",
            Self::UnsupportedAlgorithm => "nip_fi_verifier_unsupported_algorithm",
            Self::MissingKeyId => "nip_fi_verifier_missing_key_id",
            Self::UnknownKeyId => "nip_fi_verifier_unknown_key_id",
            Self::InvalidKey => "nip_fi_verifier_invalid_key",
            Self::InvalidClaim => "nip_fi_verifier_invalid_claim",
            Self::InvalidTimeBounds => "nip_fi_verifier_invalid_time_bounds",
        }
    }
}

fn canonical_algorithm_allowed(algorithm: Algorithm) -> bool {
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

fn validate_canonical_jwk(
    jwk: &Jwk,
    token_algorithm: Algorithm,
) -> Result<(), CanonicalVerifierError> {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|key_use| key_use != &PublicKeyUse::Signature)
        || jwk
            .common
            .key_operations
            .as_ref()
            .is_some_and(|operations| !operations.contains(&KeyOperations::Verify))
        || jwk
            .common
            .key_algorithm
            .is_some_and(|algorithm| !canonical_jwk_algorithm_matches(algorithm, token_algorithm))
    {
        return Err(CanonicalVerifierError::InvalidKey);
    }
    Ok(())
}

fn canonical_jwk_algorithm_matches(key: KeyAlgorithm, token: Algorithm) -> bool {
    matches!(
        (key, token),
        (KeyAlgorithm::RS256, Algorithm::RS256)
            | (KeyAlgorithm::RS384, Algorithm::RS384)
            | (KeyAlgorithm::RS512, Algorithm::RS512)
            | (KeyAlgorithm::PS256, Algorithm::PS256)
            | (KeyAlgorithm::PS384, Algorithm::PS384)
            | (KeyAlgorithm::PS512, Algorithm::PS512)
            | (KeyAlgorithm::ES256, Algorithm::ES256)
            | (KeyAlgorithm::ES384, Algorithm::ES384)
            | (KeyAlgorithm::EdDSA, Algorithm::EdDSA)
    )
}

fn canonical_claim_string(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<String, CanonicalVerifierError> {
    claims
        .get(claim)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 2_048)
        .map(str::to_owned)
        .ok_or(CanonicalVerifierError::InvalidClaim)
}

fn canonical_claim_i64(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<i64, CanonicalVerifierError> {
    claims
        .get(claim)
        .and_then(Value::as_i64)
        .ok_or(CanonicalVerifierError::InvalidTimeBounds)
}

fn canonical_optional_claim_i64(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<Option<i64>, CanonicalVerifierError> {
    claims
        .get(claim)
        .map(|value| {
            value
                .as_i64()
                .ok_or(CanonicalVerifierError::InvalidTimeBounds)
        })
        .transpose()
}

fn canonical_parse_pubkey(value: &str) -> Result<PublicKey, CanonicalVerifierError> {
    PublicKey::from_hex(value)
        .or_else(|_| PublicKey::from_bech32(value))
        .map_err(|_| CanonicalVerifierError::InvalidClaim)
}

/// Closed route capability used by local policy and delegation checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RouteCapability {
    /// Read messages.
    MessagesRead,
    /// Write messages.
    MessagesWrite,
    /// Read channel metadata.
    ChannelsRead,
    /// Mutate channels.
    ChannelsWrite,
    /// Perform channel administration.
    AdminChannels,
    /// Read user metadata.
    UsersRead,
    /// Mutate user metadata.
    UsersWrite,
    /// Perform user administration.
    AdminUsers,
    /// Read jobs.
    JobsRead,
    /// Mutate jobs.
    JobsWrite,
    /// Read subscriptions.
    SubscriptionsRead,
    /// Mutate subscriptions.
    SubscriptionsWrite,
    /// Read files.
    FilesRead,
    /// Write files.
    FilesWrite,
    /// Read repositories.
    ReposRead,
    /// Write repositories.
    ReposWrite,
    /// Read Git objects and refs.
    GitRead,
    /// Mutate Git objects and refs.
    GitWrite,
    /// Keep a bounded Git streaming operation alive.
    GitStream,
    /// Read media using GET or HEAD.
    MediaRead,
    /// Upload or mutate media.
    MediaWrite,
    /// Perform moderation operations.
    Moderation,
    /// Join an audio session.
    AudioJoin,
    /// Send or receive bounded audio media.
    AudioMedia,
    /// Read protected discovery data.
    Discovery,
    /// Read current local binding status.
    BindingStatus,
    /// Enroll a local binding.
    Enrollment,
    /// Mint an invitation without consuming one.
    InviteMint,
    /// Claim an invitation without gaining mint authority.
    InviteClaim,
}

/// Closed authorization outcome reason retained by immutable contexts.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthorizationReason {
    /// An existing active direct binding authorized the request.
    ExistingBinding,
    /// A signed assertion attested the enrolled Nostr author.
    EnrolledAttestedKey,
    /// Local provisioning authorized enrollment.
    EnrolledProvisioned,
    /// Explicit risk-labelled trust-on-first-use authorized enrollment.
    EnrolledRiskLabelledTofu,
    /// A current owner binding and exact delegation authorized the request.
    DelegatedOwnerBinding,
}

impl AuthorizationReason {
    /// Unique stable machine code safe for durable receipts and metrics.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ExistingBinding => "nip_fi_existing_binding",
            Self::EnrolledAttestedKey => "nip_fi_enrolled_attested_key",
            Self::EnrolledProvisioned => "nip_fi_enrolled_provisioned",
            Self::EnrolledRiskLabelledTofu => "nip_fi_enrolled_risk_labelled_tofu",
            Self::DelegatedOwnerBinding => "nip_fi_delegated_owner_binding",
        }
    }
}

impl fmt::Debug for AuthorizationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationReason([REDACTED])")
    }
}

/// Closed capability advertised by the concrete local binding resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBindingResolverCapability {
    /// Direct active-binding resolution only.
    Direct,
    /// Direct resolution plus explicit owner-bound delegated resolution.
    DirectAndDelegatedOwnerBound,
}

/// Non-mutating request for current local binding status evidence.
#[derive(Clone)]
pub struct CurrentBindingStatusEvidenceRequest {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
}

impl CurrentBindingStatusEvidenceRequest {
    /// Construct a status request from a server-resolved domain and exact key.
    pub fn new(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
    ) -> Result<Self, AuthorizationError> {
        if authorization_domain.as_uuid().is_nil() {
            return Err(AuthorizationError::InvalidInput);
        }
        Ok(Self {
            authorization_domain,
            event_author_pubkey,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact Nostr author whose current binding is requested.
    pub const fn event_author_pubkey(&self) -> PublicKey {
        self.event_author_pubkey
    }
}

impl fmt::Debug for CurrentBindingStatusEvidenceRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CurrentBindingStatusEvidenceRequest([REDACTED])")
    }
}

/// Cross-crate contract implemented by the concrete PostgreSQL local resolver.
///
/// This is a storage boundary, not a provider SPI. Resolution is limited to
/// exact local binding facts. Both status methods are read-only: implementations
/// must not enroll, consume selectors, append history, or update `last_seen`.
/// Public evidence construction validates data shape only. Composition trusts
/// the configured resolver implementation and must use its atomic recheck plus
/// `CanonicalCurrentBindingEvidence::accepts_exact_recheck` before delivery;
/// an arbitrary constructed tuple is never authority by itself.
pub trait LocalBindingResolver: Send + Sync {
    /// Storage/read failure returned fail closed to composition.
    type Error: Send;

    /// Whether this resolver supports direct-only or explicit owner-bound
    /// delegated resolution.
    fn capability(&self) -> LocalBindingResolverCapability;

    /// Resolve one origin-sealed direct, delegated, or enrollment request.
    fn resolve<'a>(
        &'a self,
        request: &'a BindingResolutionRequest,
    ) -> impl Future<Output = Result<LocalBindingResolution, Self::Error>> + Send + 'a;

    /// Read privacy-safe current status without mutating identity state.
    fn current_status_evidence<'a>(
        &'a self,
        request: &'a CurrentBindingStatusEvidenceRequest,
    ) -> impl Future<Output = Result<CanonicalCurrentBindingEvidence, Self::Error>> + Send + 'a;

    /// Recheck the complete evidence tuple atomically against PostgreSQL
    /// immediately before presentation. Error, staleness, or any tuple mismatch
    /// withholds presentation without changing authorization.
    fn recheck_current_status_evidence<'a>(
        &'a self,
        evidence: &'a CanonicalCurrentBindingEvidence,
    ) -> impl Future<Output = Result<CanonicalCurrentBindingEvidence, Self::Error>> + Send + 'a;
}

/// Whether a route participates in protected composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteProtection {
    /// Explicitly unprotected route; the only route accepting NotRequired.
    Unprotected,
    /// Protected route requiring the exact capability.
    Protected(RouteCapability),
}

/// Exact opaque federated identity admitted by a verifier.
#[derive(Clone, PartialEq, Eq)]
pub struct FederatedPrincipal {
    issuer: String,
    subject: String,
}

impl FederatedPrincipal {
    #[allow(dead_code)]
    pub(crate) fn from_verified_parts(issuer: String, subject: String) -> Option<Self> {
        if issuer.is_empty() || subject.is_empty() || issuer.len() > 2048 || subject.len() > 2048 {
            return None;
        }
        Some(Self { issuer, subject })
    }

    /// Borrow the exact opaque storage key without enabling construction from
    /// unverified claims.
    pub fn storage_key(&self) -> FederatedPrincipalStorageKey<'_> {
        FederatedPrincipalStorageKey {
            issuer: &self.issuer,
            subject: &self.subject,
        }
    }
}

/// Borrowed exact storage key for an origin-sealed principal.
#[derive(Clone, Copy)]
pub struct FederatedPrincipalStorageKey<'a> {
    issuer: &'a str,
    subject: &'a str,
}

impl FederatedPrincipalStorageKey<'_> {
    /// Exact validated issuer bytes for a parameterized database bind.
    pub fn issuer(&self) -> &str {
        self.issuer
    }

    /// Exact opaque subject bytes for a parameterized database bind.
    pub fn subject(&self) -> &str {
        self.subject
    }
}

impl fmt::Debug for FederatedPrincipalStorageKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FederatedPrincipalStorageKey([REDACTED])")
    }
}

impl fmt::Debug for FederatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FederatedPrincipal([REDACTED])")
    }
}

/// Origin-sealed result of federated assertion validation.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedFederatedAssertion {
    authorization_domain: CommunityId,
    principal: FederatedPrincipal,
    transport: ProofTransport,
    assertion_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    request_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    not_before: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    verifier_stamp: VerifierPolicyStamp,
    attested_event_author_pubkey: Option<PublicKey>,
}

impl VerifiedFederatedAssertion {
    #[allow(clippy::too_many_arguments, dead_code)]
    pub(crate) fn from_verifier(
        authorization_domain: CommunityId,
        principal: FederatedPrincipal,
        transport: ProofTransport,
        assertion_fingerprint: [u8; 32],
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
        not_before: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Option<Self> {
        Self::from_verifier_with_stamp(
            authorization_domain,
            principal,
            transport,
            assertion_fingerprint,
            target_fingerprint,
            request_fingerprint,
            transport_context_fingerprint,
            not_before,
            expires_at,
            VerifierPolicyStamp::new(CanonicalVerifierPolicyId([1; 32]), VerifierKeyGeneration(1)),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_verifier_with_stamp(
        authorization_domain: CommunityId,
        principal: FederatedPrincipal,
        transport: ProofTransport,
        assertion_fingerprint: [u8; 32],
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
        not_before: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        verifier_stamp: VerifierPolicyStamp,
        attested_event_author_pubkey: Option<PublicKey>,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || assertion_fingerprint == [0; 32]
            || target_fingerprint == [0; 32]
            || request_fingerprint == [0; 32]
            || transport_context_fingerprint == [0; 32]
            || not_before >= expires_at
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            principal,
            transport,
            assertion_fingerprint,
            target_fingerprint,
            request_fingerprint,
            transport_context_fingerprint,
            not_before,
            expires_at,
            verifier_stamp,
            attested_event_author_pubkey,
        })
    }

    /// Server-resolved domain sealed by the verifier.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Clone the origin-sealed principal for an authoritative storage result.
    pub fn principal_for_storage(&self) -> FederatedPrincipal {
        self.principal.clone()
    }

    /// Exact principal lookup key for parameterized database reads.
    pub fn principal_storage_key(&self) -> FederatedPrincipalStorageKey<'_> {
        self.principal.storage_key()
    }

    /// Stable policy and exact JWKS generation used to verify this assertion.
    pub const fn verifier_stamp(&self) -> VerifierPolicyStamp {
        self.verifier_stamp
    }

    /// Event-author key explicitly attested by the signed assertion, if any.
    pub const fn attested_event_author_pubkey(&self) -> Option<PublicKey> {
        self.attested_event_author_pubkey
    }
}

impl fmt::Debug for VerifiedFederatedAssertion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedFederatedAssertion([REDACTED])")
    }
}

/// Origin-sealed Nostr proof bound to one server-resolved domain and request.
#[derive(Clone)]
pub struct VerifiedNostrProof {
    authorization_domain: CommunityId,
    actor_pubkey: PublicKey,
    transport: ProofTransport,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    bound_assertion_fingerprint: Option<[u8; 32]>,
    delegation_conditions_fingerprint: Option<[u8; 32]>,
    expires_at: DateTime<Utc>,
}

impl VerifiedNostrProof {
    #[allow(clippy::too_many_arguments, dead_code)]
    pub(crate) fn from_verifier(
        authorization_domain: CommunityId,
        actor_pubkey: PublicKey,
        transport: ProofTransport,
        request_fingerprint: [u8; 32],
        target_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
        bound_assertion_fingerprint: Option<[u8; 32]>,
        delegation_conditions_fingerprint: Option<[u8; 32]>,
        expires_at: DateTime<Utc>,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || request_fingerprint == [0; 32]
            || target_fingerprint == [0; 32]
            || transport_context_fingerprint == [0; 32]
            || bound_assertion_fingerprint == Some([0; 32])
            || delegation_conditions_fingerprint == Some([0; 32])
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            actor_pubkey,
            transport,
            request_fingerprint,
            target_fingerprint,
            transport_context_fingerprint,
            bound_assertion_fingerprint,
            delegation_conditions_fingerprint,
            expires_at,
        })
    }

    /// Server-resolved domain sealed by the proof verifier.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact verified Nostr actor.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Transport whose verifier origin-sealed this proof.
    pub const fn transport(&self) -> ProofTransport {
        self.transport
    }

    /// Exact request fingerprint sealed by the verifier.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    /// Exact protected target fingerprint sealed by the verifier.
    pub const fn target_fingerprint(&self) -> &[u8; 32] {
        &self.target_fingerprint
    }

    /// Exact transport-context fingerprint sealed by the verifier.
    pub const fn transport_context_fingerprint(&self) -> &[u8; 32] {
        &self.transport_context_fingerprint
    }

    /// Optional assertion fingerprint co-located with this proof.
    pub const fn bound_assertion_fingerprint(&self) -> Option<&[u8; 32]> {
        self.bound_assertion_fingerprint.as_ref()
    }

    /// Optional delegated-conditions fingerprint co-located with this proof.
    pub const fn delegation_conditions_fingerprint(&self) -> Option<&[u8; 32]> {
        self.delegation_conditions_fingerprint.as_ref()
    }

    /// Exclusive verifier-owned proof expiry.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl fmt::Debug for VerifiedNostrProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedNostrProof([REDACTED])")
    }
}

/// Closed enrollment authority selected by local server policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectEnrollmentMode {
    /// The signed federated assertion attests the exact Nostr event author.
    Attested,
    /// A locally provisioned subject-to-key assignment authorizes enrollment.
    Provisioned,
    /// Explicitly risk-labelled trust-on-first-use with no silent promotion.
    RiskLabelledTofu,
}

impl DirectEnrollmentMode {
    /// Stable database code shared with `binding_provenance` and
    /// `identity_enrollment_policies.enrollment_mode`.
    pub const fn database_code(self) -> i16 {
        match self {
            Self::Attested => 1,
            Self::Provisioned => 2,
            Self::RiskLabelledTofu => 3,
        }
    }

    /// Decode the closed database representation.
    pub const fn from_database_code(code: i16) -> Option<Self> {
        match code {
            1 => Some(Self::Attested),
            2 => Some(Self::Provisioned),
            3 => Some(Self::RiskLabelledTofu),
            _ => None,
        }
    }
}

/// Exact local enrollment authority read from the configured database.
///
/// This value is deliberately distinct from verifier evidence. Provisioned
/// and risk-labelled TOFU enrollment cannot be selected merely by passing an
/// enum to the proposal constructor; the database policy, key assignment and
/// evidence digest must all be present and later match the inserted binding.
#[derive(Clone)]
pub struct LocalEnrollmentAuthority {
    authorization_domain: CommunityId,
    principal: FederatedPrincipal,
    event_author_pubkey: PublicKey,
    mode: DirectEnrollmentMode,
    policy_revision: u64,
    evidence_digest: [u8; 32],
}

impl LocalEnrollmentAuthority {
    /// Seal an exact enrollment-policy result returned by authoritative local
    /// storage. Raw credentials and provider configuration are never retained.
    pub fn from_database_parts(
        authorization_domain: CommunityId,
        issuer: String,
        subject: String,
        event_author_pubkey: PublicKey,
        mode_code: i16,
        policy_revision: u64,
        evidence_digest: [u8; 32],
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || policy_revision == 0
            || evidence_digest == [0; 32]
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            principal: FederatedPrincipal::from_verified_parts(issuer, subject)?,
            event_author_pubkey,
            mode: DirectEnrollmentMode::from_database_code(mode_code)?,
            policy_revision,
            evidence_digest,
        })
    }
}

impl fmt::Debug for LocalEnrollmentAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalEnrollmentAuthority([REDACTED])")
    }
}

/// Origin-sealed direct-enrollment proposal made from co-bound verified evidence.
#[derive(Clone)]
pub struct DirectEnrollmentProposal {
    mode: DirectEnrollmentMode,
    policy_revision: u64,
    evidence_digest: [u8; 32],
    assertion: VerifiedFederatedAssertion,
    proof: VerifiedNostrProof,
}

impl DirectEnrollmentProposal {
    /// Validate and seal an enrollment proposal under an exact database-read
    /// local authority. The proposal cannot invent its own enrollment mode.
    pub fn from_verified_evidence(
        authority: LocalEnrollmentAuthority,
        assertion: VerifiedFederatedAssertion,
        proof: VerifiedNostrProof,
        authoritative_now: DateTime<Utc>,
    ) -> Result<Self, AuthorizationError> {
        if assertion.authorization_domain != proof.authorization_domain
            || assertion.transport != proof.transport
            || assertion.assertion_fingerprint
                != proof
                    .bound_assertion_fingerprint
                    .ok_or(AuthorizationError::BindingMismatch)?
            || assertion.target_fingerprint != proof.target_fingerprint
            || assertion.request_fingerprint != proof.request_fingerprint
            || assertion.transport_context_fingerprint != proof.transport_context_fingerprint
            || proof.delegation_conditions_fingerprint.is_some()
        {
            return Err(AuthorizationError::BindingMismatch);
        }
        if authoritative_now < assertion.not_before
            || authoritative_now >= assertion.expires_at
            || authoritative_now >= proof.expires_at
        {
            return Err(AuthorizationError::Expired);
        }
        if authority.authorization_domain != assertion.authorization_domain
            || authority.principal != assertion.principal
            || authority.event_author_pubkey != proof.actor_pubkey
        {
            return Err(AuthorizationError::EnrollmentAuthorityMismatch);
        }
        if authority.mode == DirectEnrollmentMode::Attested
            && assertion.attested_event_author_pubkey != Some(proof.actor_pubkey)
        {
            return Err(AuthorizationError::EnrollmentAuthorityMismatch);
        }
        Ok(Self {
            mode: authority.mode,
            policy_revision: authority.policy_revision,
            evidence_digest: authority.evidence_digest,
            assertion,
            proof,
        })
    }

    /// Closed enrollment authority retained through storage resolution.
    pub const fn mode(&self) -> DirectEnrollmentMode {
        self.mode
    }

    /// Server-resolved authorization domain sealed by both proofs.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.assertion.authorization_domain
    }

    /// Exact local policy revision and privacy-safe evidence digest that the
    /// resulting immutable binding must retain.
    pub const fn local_authority(&self) -> (u64, &[u8; 32]) {
        (self.policy_revision, &self.evidence_digest)
    }

    fn assertion(&self) -> &VerifiedFederatedAssertion {
        &self.assertion
    }

    fn proof(&self) -> &VerifiedNostrProof {
        &self.proof
    }
}

impl fmt::Debug for DirectEnrollmentProposal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DirectEnrollmentProposal([REDACTED])")
    }
}

/// Origin-sealed, assertion-free delegated authority.
#[derive(Clone)]
pub struct VerifiedDelegation {
    authorization_domain: CommunityId,
    owner_pubkey: PublicKey,
    delegate_pubkey: PublicKey,
    transport: ProofTransport,
    relationship_id: Uuid,
    relationship_revision: u64,
    capabilities: Vec<RouteCapability>,
    conditions_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    request_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
    expires_at: DateTime<Utc>,
}

impl VerifiedDelegation {
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub(crate) fn from_verifier(
        authorization_domain: CommunityId,
        owner_pubkey: PublicKey,
        delegate_pubkey: PublicKey,
        transport: ProofTransport,
        relationship_id: Uuid,
        relationship_revision: u64,
        mut capabilities: Vec<RouteCapability>,
        conditions_fingerprint: [u8; 32],
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
        expires_at: DateTime<Utc>,
    ) -> Option<Self> {
        capabilities.sort_unstable();
        capabilities.dedup();
        if authorization_domain.as_uuid().is_nil()
            || relationship_id.is_nil()
            || relationship_revision == 0
            || capabilities.is_empty()
            || conditions_fingerprint == [0; 32]
            || target_fingerprint == [0; 32]
            || request_fingerprint == [0; 32]
            || transport_context_fingerprint == [0; 32]
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            owner_pubkey,
            delegate_pubkey,
            transport,
            relationship_id,
            relationship_revision,
            capabilities,
            conditions_fingerprint,
            target_fingerprint,
            request_fingerprint,
            transport_context_fingerprint,
            expires_at,
        })
    }

    /// Server-resolved domain sealed by the delegation verifier.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact owner key whose active binding must be resolved.
    pub const fn owner_pubkey(&self) -> PublicKey {
        self.owner_pubkey
    }

    /// Exact delegate key that must match the Nostr proof actor.
    pub const fn delegate_pubkey(&self) -> PublicKey {
        self.delegate_pubkey
    }
}

impl fmt::Debug for VerifiedDelegation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedDelegation([REDACTED])")
    }
}

/// Explicit request variants accepted by the one local binding resolver.
#[derive(Clone)]
pub enum BindingResolutionRequest {
    /// Resolve an existing binding for a verified assertion and Nostr proof.
    Direct {
        /// Origin-sealed federated assertion.
        assertion: VerifiedFederatedAssertion,
        /// Origin-sealed Nostr proof.
        proof: VerifiedNostrProof,
        /// Exact protected capability.
        capability: RouteCapability,
    },
    /// Resolve the exact active owner binding for an assertion-free delegate.
    Delegated {
        /// Origin-sealed delegation.
        delegation: VerifiedDelegation,
        /// Origin-sealed proof by the delegate.
        proof: VerifiedNostrProof,
        /// Exact owner-bound delegated capability.
        capability: RouteCapability,
    },
    /// Resolve or create a binding under the local enrollment policy.
    Enrollment {
        /// Origin-sealed proposal retaining its explicit enrollment authority.
        proposal: DirectEnrollmentProposal,
        /// Exact protected capability.
        capability: RouteCapability,
    },
}

/// One active, versioned local binding returned from authoritative storage.
#[derive(Clone)]
pub struct ActiveLocalBinding {
    authorization_domain: CommunityId,
    principal: FederatedPrincipal,
    binding_id: Uuid,
    binding_version: u64,
    event_author_pubkey: PublicKey,
    expires_at: Option<DateTime<Utc>>,
    enrollment_authority: Option<(DirectEnrollmentMode, u64, [u8; 32])>,
}

impl ActiveLocalBinding {
    /// Construct one exact active-binding row from raw authoritative storage
    /// columns.
    ///
    /// This adapter validates and seals the opaque issuer/subject pair without
    /// exposing a raw-claims principal constructor. It is intended only for the
    /// configured [`LocalBindingResolver`]; the returned storage fact is not
    /// authority until the finalizer matches it to independently verified
    /// inputs and policy.
    #[allow(clippy::too_many_arguments)]
    pub fn from_storage_parts(
        authorization_domain: CommunityId,
        issuer: String,
        subject: String,
        binding_id: Uuid,
        binding_version: u64,
        event_author_pubkey: PublicKey,
        expires_at: Option<DateTime<Utc>>,
    ) -> Option<Self> {
        let principal = FederatedPrincipal::from_verified_parts(issuer, subject)?;
        Self::from_storage(
            authorization_domain,
            principal,
            binding_id,
            binding_version,
            event_author_pubkey,
            expires_at,
        )
    }

    /// Construct one exact active-binding row returned by authoritative storage.
    pub fn from_storage(
        authorization_domain: CommunityId,
        principal: FederatedPrincipal,
        binding_id: Uuid,
        binding_version: u64,
        event_author_pubkey: PublicKey,
        expires_at: Option<DateTime<Utc>>,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil() || binding_id.is_nil() || binding_version == 0 {
            return None;
        }
        Some(Self {
            authorization_domain,
            principal,
            binding_id,
            binding_version,
            event_author_pubkey,
            expires_at,
            enrollment_authority: None,
        })
    }

    /// Construct an immutable binding generation created by an enrollment
    /// operation. Its provenance, policy revision and evidence digest are
    /// rechecked against the proposal before authorization can be prepared.
    #[allow(clippy::too_many_arguments)]
    pub fn from_enrollment_storage_parts(
        authorization_domain: CommunityId,
        issuer: String,
        subject: String,
        binding_id: Uuid,
        binding_version: u64,
        event_author_pubkey: PublicKey,
        expires_at: Option<DateTime<Utc>>,
        provenance_code: i16,
        policy_revision: u64,
        enrollment_evidence_digest: [u8; 32],
    ) -> Option<Self> {
        let mut binding = Self::from_storage_parts(
            authorization_domain,
            issuer,
            subject,
            binding_id,
            binding_version,
            event_author_pubkey,
            expires_at,
        )?;
        let mode = DirectEnrollmentMode::from_database_code(provenance_code)?;
        if policy_revision == 0 || enrollment_evidence_digest == [0; 32] {
            return None;
        }
        binding.enrollment_authority = Some((mode, policy_revision, enrollment_evidence_digest));
        Some(binding)
    }
}

impl fmt::Debug for ActiveLocalBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ActiveLocalBinding([REDACTED])")
    }
}

/// Authoritative local resolver result with origin-sealed authority inputs.
#[derive(Clone)]
pub struct LocalBindingResolution(LocalBindingResolutionKind);

#[derive(Clone)]
enum LocalBindingResolutionKind {
    Direct {
        assertion: VerifiedFederatedAssertion,
        binding: ActiveLocalBinding,
    },
    Enrollment {
        proposal: Box<DirectEnrollmentProposal>,
        binding: ActiveLocalBinding,
    },
    Delegated {
        delegation: VerifiedDelegation,
        owner_binding: ActiveLocalBinding,
    },
}

impl LocalBindingResolution {
    /// Bind an origin-sealed direct assertion to an authoritative storage row.
    pub fn direct(assertion: VerifiedFederatedAssertion, binding: ActiveLocalBinding) -> Self {
        Self(LocalBindingResolutionKind::Direct { assertion, binding })
    }

    /// Bind enrollment output to its origin-sealed, explicitly labelled proposal.
    pub fn enrollment(proposal: DirectEnrollmentProposal, binding: ActiveLocalBinding) -> Self {
        Self(LocalBindingResolutionKind::Enrollment {
            proposal: Box::new(proposal),
            binding,
        })
    }

    /// Bind an assertion-free delegation to the exact authoritative owner row.
    pub fn delegated(delegation: VerifiedDelegation, owner_binding: ActiveLocalBinding) -> Self {
        Self(LocalBindingResolutionKind::Delegated {
            delegation,
            owner_binding,
        })
    }
}

impl fmt::Debug for LocalBindingResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match &self.0 {
            LocalBindingResolutionKind::Direct { .. } => "Direct([REDACTED])",
            LocalBindingResolutionKind::Enrollment { .. } => "Enrollment([REDACTED])",
            LocalBindingResolutionKind::Delegated { .. } => "Delegated([REDACTED])",
        };
        formatter.write_str(name)
    }
}

/// Database-derived final policy state for one exact capability.
#[derive(Clone)]
pub struct LocalAuthorizationPolicy {
    authorization_domain: CommunityId,
    lease_id: Uuid,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
    capability: RouteCapability,
    expires_at: DateTime<Utc>,
    delegated_max_expires_at: Option<DateTime<Utc>>,
    stronger_owner_expires_at: Option<DateTime<Utc>>,
}

impl LocalAuthorizationPolicy {
    /// Construct a policy decision from an authoritative PostgreSQL result.
    #[allow(clippy::too_many_arguments)]
    pub fn from_database(
        authorization_domain: CommunityId,
        lease_id: Uuid,
        policy_revision: u64,
        invalidation_generation: u64,
        authority_epoch: u64,
        fence: AuthorizationLeaseFence,
        capability: RouteCapability,
        expires_at: DateTime<Utc>,
        delegated_max_expires_at: Option<DateTime<Utc>>,
        stronger_owner_expires_at: Option<DateTime<Utc>>,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || lease_id.is_nil()
            || policy_revision == 0
            || authority_epoch == 0
        {
            return None;
        }
        Some(Self {
            authorization_domain,
            lease_id,
            policy_revision,
            invalidation_generation,
            authority_epoch,
            fence,
            capability,
            expires_at,
            delegated_max_expires_at,
            stronger_owner_expires_at,
        })
    }
}

impl fmt::Debug for LocalAuthorizationPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalAuthorizationPolicy([REDACTED])")
    }
}

/// Inputs already verified before local binding resolution.
#[derive(Clone)]
pub struct AuthorizationInput {
    authorization_domain: CommunityId,
    correlation_id: Uuid,
    proof: VerifiedNostrProof,
    capability: RouteCapability,
}

impl AuthorizationInput {
    /// Bind one verified proof to server-resolved route metadata.
    pub fn new(
        authorization_domain: CommunityId,
        correlation_id: Uuid,
        proof: VerifiedNostrProof,
        capability: RouteCapability,
    ) -> Result<Self, AuthorizationError> {
        if authorization_domain.as_uuid().is_nil() || correlation_id.is_nil() {
            return Err(AuthorizationError::InvalidInput);
        }
        if proof.authorization_domain != authorization_domain {
            return Err(AuthorizationError::DomainMismatch);
        }
        Ok(Self {
            authorization_domain,
            correlation_id,
            proof,
            capability,
        })
    }
}

/// Bounded authorization lease shared by request and audio use sites.
#[derive(Clone)]
pub struct BoundedAuthorizationLease {
    lease_id: Uuid,
    authorization_domain: CommunityId,
    capability: RouteCapability,
    actor_pubkey: PublicKey,
    owner_pubkey: Option<PublicKey>,
    binding_id: Uuid,
    binding_version: u64,
    relationship_id: Option<Uuid>,
    relationship_revision: Option<u64>,
    delegation_conditions_fingerprint: Option<[u8; 32]>,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport: ProofTransport,
    transport_context_fingerprint: [u8; 32],
    issued_at: DateTime<Utc>,
    fence: AuthorizationLeaseFence,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    expires_at: DateTime<Utc>,
}

impl BoundedAuthorizationLease {
    /// Exclusive validity check using an authoritative PostgreSQL-derived time.
    pub fn is_valid_at(&self, authoritative_now: DateTime<Utc>) -> bool {
        authoritative_now >= self.issued_at && authoritative_now < self.expires_at
    }

    /// Stable lease identifier allocated by the authoritative transaction.
    pub const fn lease_id(&self) -> Uuid {
        self.lease_id
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact closed capability guarded by this lease.
    pub const fn capability(&self) -> RouteCapability {
        self.capability
    }

    /// Exact Nostr actor.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Exact delegated owner, when delegated authorization was used.
    pub const fn owner_pubkey(&self) -> Option<PublicKey> {
        self.owner_pubkey
    }

    /// Exact active owner/direct binding identifier and version.
    pub const fn binding(&self) -> (Uuid, u64) {
        (self.binding_id, self.binding_version)
    }

    /// Delegated relationship identity and revision, when present.
    pub const fn delegated_relationship(&self) -> Option<(Uuid, u64)> {
        match (self.relationship_id, self.relationship_revision) {
            (Some(id), Some(revision)) => Some((id, revision)),
            _ => None,
        }
    }

    /// Exact request, target, and transport-context binding.
    pub const fn request_binding(&self) -> (&[u8; 32], &[u8; 32], &[u8; 32]) {
        (
            &self.request_fingerprint,
            &self.target_fingerprint,
            &self.transport_context_fingerprint,
        )
    }

    /// Proof transport used for this lease.
    pub const fn transport(&self) -> ProofTransport {
        self.transport
    }

    /// Policy, invalidation, and authority versions captured by the lease.
    pub const fn dependency_versions(&self) -> (u64, u64, u64) {
        (
            self.policy_revision,
            self.invalidation_generation,
            self.authority_epoch,
        )
    }

    /// Authoritative PostgreSQL issue time.
    pub const fn issued_at(&self) -> DateTime<Utc> {
        self.issued_at
    }

    /// Observable dependency fence.
    pub const fn fence(&self) -> AuthorizationLeaseFence {
        self.fence
    }

    /// Exclusive lease expiry.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Clone the complete sealed dependency tuple for a database re-fence.
    pub fn dependency_snapshot(&self) -> AuthorizationLeaseDependencySnapshot {
        AuthorizationLeaseDependencySnapshot {
            lease_id: self.lease_id,
            authorization_domain: self.authorization_domain,
            capability: self.capability,
            actor_pubkey: self.actor_pubkey,
            owner_pubkey: self.owner_pubkey,
            binding_id: self.binding_id,
            binding_version: self.binding_version,
            relationship_id: self.relationship_id,
            relationship_revision: self.relationship_revision,
            delegation_conditions_fingerprint: self.delegation_conditions_fingerprint,
            request_fingerprint: self.request_fingerprint,
            target_fingerprint: self.target_fingerprint,
            transport: self.transport,
            transport_context_fingerprint: self.transport_context_fingerprint,
            issued_at: self.issued_at,
            policy_revision: self.policy_revision,
            invalidation_generation: self.invalidation_generation,
            authority_epoch: self.authority_epoch,
            fence: self.fence,
            expires_at: self.expires_at,
        }
    }
}

impl fmt::Debug for BoundedAuthorizationLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundedAuthorizationLease([REDACTED])")
    }
}

/// Complete sealed input for re-fencing a cloned authorization lease.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationLeaseDependencySnapshot {
    lease_id: Uuid,
    authorization_domain: CommunityId,
    capability: RouteCapability,
    actor_pubkey: PublicKey,
    owner_pubkey: Option<PublicKey>,
    binding_id: Uuid,
    binding_version: u64,
    relationship_id: Option<Uuid>,
    relationship_revision: Option<u64>,
    delegation_conditions_fingerprint: Option<[u8; 32]>,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport: ProofTransport,
    transport_context_fingerprint: [u8; 32],
    issued_at: DateTime<Utc>,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
    expires_at: DateTime<Utc>,
}

impl AuthorizationLeaseDependencySnapshot {
    /// Stable lease identifier and server-resolved domain.
    pub const fn identity(&self) -> (Uuid, CommunityId) {
        (self.lease_id, self.authorization_domain)
    }

    /// Exact capability, actor, and optional delegated owner.
    pub const fn authority(&self) -> (RouteCapability, PublicKey, Option<PublicKey>) {
        (self.capability, self.actor_pubkey, self.owner_pubkey)
    }

    /// Exact active direct/owner binding identifier and version.
    pub const fn binding(&self) -> (Uuid, u64) {
        (self.binding_id, self.binding_version)
    }

    /// Exact relationship identity/revision and conditions, when delegated.
    pub const fn delegated_relationship(&self) -> Option<(Uuid, u64, &[u8; 32])> {
        match (
            self.relationship_id,
            self.relationship_revision,
            &self.delegation_conditions_fingerprint,
        ) {
            (Some(id), Some(revision), Some(conditions)) => Some((id, revision, conditions)),
            _ => None,
        }
    }

    /// Exact request, target, transport, and transport-context coordinates.
    pub const fn request_binding(&self) -> (&[u8; 32], &[u8; 32], ProofTransport, &[u8; 32]) {
        (
            &self.request_fingerprint,
            &self.target_fingerprint,
            self.transport,
            &self.transport_context_fingerprint,
        )
    }

    /// Exact policy, invalidation, and authority versions.
    pub const fn dependency_versions(&self) -> (u64, u64, u64) {
        (
            self.policy_revision,
            self.invalidation_generation,
            self.authority_epoch,
        )
    }

    /// Opaque database-issued fence.
    pub const fn fence(&self) -> AuthorizationLeaseFence {
        self.fence
    }

    /// Authoritative issue time and exclusive expiry.
    pub const fn time_bounds(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        (self.issued_at, self.expires_at)
    }
}

impl fmt::Debug for AuthorizationLeaseDependencySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationLeaseDependencySnapshot([REDACTED])")
    }
}

/// Immutable post-policy authorization result.
#[derive(Clone)]
pub struct AuthContext {
    authorization_domain: CommunityId,
    correlation_id: Uuid,
    actor_pubkey: PublicKey,
    owner_pubkey: Option<PublicKey>,
    binding_id: Uuid,
    binding_version: u64,
    capability: RouteCapability,
    reason: AuthorizationReason,
    transport: ProofTransport,
    request_fingerprint: [u8; 32],
    lease: BoundedAuthorizationLease,
}

impl AuthContext {
    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact Nostr actor.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Server-generated correlation identifier.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Delegated owner, when the assertion-free delegated path was used.
    pub const fn owner_pubkey(&self) -> Option<PublicKey> {
        self.owner_pubkey
    }

    /// Exact protected capability.
    pub const fn capability(&self) -> RouteCapability {
        self.capability
    }

    /// Closed stable reason for the final authorization outcome.
    pub const fn reason(&self) -> AuthorizationReason {
        self.reason
    }

    /// Exact active direct/owner binding identifier and version.
    pub const fn binding(&self) -> (Uuid, u64) {
        (self.binding_id, self.binding_version)
    }

    /// Verified proof transport.
    pub const fn transport(&self) -> ProofTransport {
        self.transport
    }

    /// Exact request fingerprint bound by the finalized proof.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    /// Bounded shared lease.
    pub const fn lease(&self) -> &BoundedAuthorizationLease {
        &self.lease
    }
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthContext([REDACTED])")
    }
}

/// Immutable pre-delivery authorization awaiting an exact final recheck.
#[derive(Clone)]
pub struct PreparedAuthorization {
    context: AuthContext,
    recheck: PreparedAuthorizationRecheck,
}

impl PreparedAuthorization {
    /// Clone the exact dependency tuple that authoritative storage and the
    /// verifier cache must recheck after all intervening I/O.
    pub fn recheck_request(&self) -> PreparedAuthorizationRecheck {
        self.recheck.clone()
    }

    /// Earliest exclusive expiry retained while the value is prepared.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.context.lease.expires_at
    }
}

impl fmt::Debug for PreparedAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedAuthorization([REDACTED])")
    }
}

/// Exact read-only dependency tuple required for final authorization.
#[derive(Clone, PartialEq, Eq)]
pub struct PreparedAuthorizationRecheck {
    lease_dependencies: AuthorizationLeaseDependencySnapshot,
    verifier_stamp: Option<VerifierPolicyStamp>,
}

impl PreparedAuthorizationRecheck {
    /// Clone the database lease dependencies for an atomic current-state read.
    pub fn lease_dependencies(&self) -> AuthorizationLeaseDependencySnapshot {
        self.lease_dependencies.clone()
    }

    /// Stable policy and key generation requiring final verifier revalidation.
    pub const fn verifier_stamp(&self) -> Option<VerifierPolicyStamp> {
        self.verifier_stamp
    }
}

impl fmt::Debug for PreparedAuthorizationRecheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedAuthorizationRecheck([REDACTED])")
    }
}

/// Atomic database/verifier observation produced only while executing the
/// configured final rechecker.
#[derive(Clone)]
pub struct AuthoritativeAuthorizationRecheck {
    recheck: PreparedAuthorizationRecheck,
    authoritative_now: DateTime<Utc>,
}

impl AuthoritativeAuthorizationRecheck {
    /// Adapt one atomic local database observation and current verifier-cache
    /// generation. This value is data, not a finalization witness: callers
    /// cannot pass it directly to [`AuthorizationFinalizer::finalize`].
    pub const fn from_authoritative_parts(
        lease_dependencies: AuthorizationLeaseDependencySnapshot,
        verifier_stamp: Option<VerifierPolicyStamp>,
        authoritative_now: DateTime<Utc>,
    ) -> Self {
        Self {
            recheck: PreparedAuthorizationRecheck {
                lease_dependencies,
                verifier_stamp,
            },
            authoritative_now,
        }
    }
}

impl fmt::Debug for AuthoritativeAuthorizationRecheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthoritativeAuthorizationRecheck([REDACTED])")
    }
}

/// Configured read-only boundary used by the finalizer after intervening I/O.
///
/// Implementations must atomically read the complete lease dependency tuple
/// and authoritative database time, then include the current canonical
/// verifier generation. They must not enroll, mutate lifecycle state or reuse
/// the prepared tuple as their result.
pub trait AuthorizationFinalizationRechecker: Send + Sync {
    /// Perform the final atomic read. Storage/cache failures map to a closed,
    /// redacted authorization error and therefore fail closed.
    fn recheck<'a>(
        &'a self,
        request: &'a PreparedAuthorizationRecheck,
    ) -> impl Future<Output = Result<AuthoritativeAuthorizationRecheck, AuthorizationError>> + Send + 'a;
}

/// One-use witness that exact dependencies were rechecked after intervening I/O.
pub struct AuthorizationFinalizationWitness {
    recheck: PreparedAuthorizationRecheck,
    authoritative_now: DateTime<Utc>,
}

impl AuthorizationFinalizationWitness {
    /// Compare authoritative post-I/O results with the prepared tuple and seal
    /// a one-use witness. Any policy, binding, fence, proof, key-generation, or
    /// time drift fails closed.
    fn from_authoritative_recheck(
        prepared: &PreparedAuthorization,
        observation: AuthoritativeAuthorizationRecheck,
    ) -> Result<Self, AuthorizationError> {
        if prepared.recheck != observation.recheck {
            return Err(AuthorizationError::StaleRecheck);
        }
        if !prepared
            .context
            .lease
            .is_valid_at(observation.authoritative_now)
        {
            return Err(AuthorizationError::Expired);
        }
        Ok(Self {
            recheck: observation.recheck,
            authoritative_now: observation.authoritative_now,
        })
    }
}

impl fmt::Debug for AuthorizationFinalizationWitness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationFinalizationWitness([REDACTED])")
    }
}

/// The one provider-free authorization finalizer.
#[derive(Debug, Default, Clone, Copy)]
pub struct AuthorizationFinalizer;

impl AuthorizationFinalizer {
    /// Execute the configured post-I/O rechecker and return a one-use witness.
    /// The witness has no public constructor, so an ordinary caller cannot
    /// finalize by echoing [`PreparedAuthorization::recheck_request`].
    pub async fn recheck<R: AuthorizationFinalizationRechecker>(
        prepared: &PreparedAuthorization,
        rechecker: &R,
    ) -> Result<AuthorizationFinalizationWitness, AuthorizationError> {
        let observation = rechecker.recheck(&prepared.recheck).await?;
        AuthorizationFinalizationWitness::from_authoritative_recheck(prepared, observation)
    }

    /// Prepare one protected route and compute the earliest exclusive expiry.
    ///
    /// Explicitly unprotected routes bypass this API. They produce no
    /// [`AuthContext`], policy lookup, fence, or lease.
    pub fn prepare(
        input: AuthorizationInput,
        resolution: LocalBindingResolution,
        policy: LocalAuthorizationPolicy,
        authoritative_now: DateTime<Utc>,
    ) -> Result<PreparedAuthorization, AuthorizationError> {
        if policy.authorization_domain != input.authorization_domain {
            return Err(AuthorizationError::DomainMismatch);
        }
        if policy.capability != input.capability {
            return Err(AuthorizationError::BindingMismatch);
        }
        if policy.expires_at <= authoritative_now || input.proof.expires_at <= authoritative_now {
            return Err(AuthorizationError::Expired);
        }

        let (
            owner_pubkey,
            binding_id,
            binding_version,
            relationship_id,
            relationship_revision,
            expires_at,
            reason,
            verifier_stamp,
        ) = match resolution.0 {
            LocalBindingResolutionKind::Direct { assertion, binding } => {
                if assertion.authorization_domain != input.authorization_domain
                    || binding.authorization_domain != input.authorization_domain
                    || assertion.principal != binding.principal
                    || assertion.transport != input.proof.transport
                    || binding.event_author_pubkey != input.proof.actor_pubkey
                    || assertion.assertion_fingerprint
                        != input
                            .proof
                            .bound_assertion_fingerprint
                            .ok_or(AuthorizationError::BindingMismatch)?
                    || assertion.target_fingerprint != input.proof.target_fingerprint
                    || assertion.request_fingerprint != input.proof.request_fingerprint
                    || assertion.transport_context_fingerprint
                        != input.proof.transport_context_fingerprint
                    || input.proof.delegation_conditions_fingerprint.is_some()
                {
                    return Err(AuthorizationError::BindingMismatch);
                }
                if authoritative_now < assertion.not_before {
                    return Err(AuthorizationError::Expired);
                }
                let expires_at = Self::effective_lease_upper_bound(
                    authoritative_now,
                    &[
                        Some(input.proof.expires_at),
                        Some(assertion.expires_at),
                        binding.expires_at,
                        Some(policy.expires_at),
                    ],
                )?;
                (
                    None,
                    binding.binding_id,
                    binding.binding_version,
                    None,
                    None,
                    expires_at,
                    AuthorizationReason::ExistingBinding,
                    Some(assertion.verifier_stamp),
                )
            }
            LocalBindingResolutionKind::Enrollment { proposal, binding } => {
                let assertion = proposal.assertion();
                let proposed_proof = proposal.proof();
                let (proposal_policy_revision, proposal_evidence_digest) =
                    proposal.local_authority();
                if proposal.authorization_domain() != input.authorization_domain
                    || binding.authorization_domain != input.authorization_domain
                    || assertion.principal != binding.principal
                    || binding.event_author_pubkey != input.proof.actor_pubkey
                    || proposed_proof.authorization_domain != input.proof.authorization_domain
                    || proposed_proof.actor_pubkey != input.proof.actor_pubkey
                    || proposed_proof.transport != input.proof.transport
                    || proposed_proof.request_fingerprint != input.proof.request_fingerprint
                    || proposed_proof.target_fingerprint != input.proof.target_fingerprint
                    || proposed_proof.transport_context_fingerprint
                        != input.proof.transport_context_fingerprint
                    || proposed_proof.bound_assertion_fingerprint
                        != input.proof.bound_assertion_fingerprint
                    || proposed_proof.delegation_conditions_fingerprint
                        != input.proof.delegation_conditions_fingerprint
                    || proposed_proof.expires_at != input.proof.expires_at
                    || binding.enrollment_authority
                        != Some((
                            proposal.mode(),
                            proposal_policy_revision,
                            *proposal_evidence_digest,
                        ))
                {
                    return Err(AuthorizationError::BindingMismatch);
                }
                let expires_at = Self::effective_lease_upper_bound(
                    authoritative_now,
                    &[
                        Some(input.proof.expires_at),
                        Some(assertion.expires_at),
                        binding.expires_at,
                        Some(policy.expires_at),
                    ],
                )?;
                let reason = match proposal.mode() {
                    DirectEnrollmentMode::Attested => AuthorizationReason::EnrolledAttestedKey,
                    DirectEnrollmentMode::Provisioned => AuthorizationReason::EnrolledProvisioned,
                    DirectEnrollmentMode::RiskLabelledTofu => {
                        AuthorizationReason::EnrolledRiskLabelledTofu
                    }
                };
                (
                    None,
                    binding.binding_id,
                    binding.binding_version,
                    None,
                    None,
                    expires_at,
                    reason,
                    Some(assertion.verifier_stamp),
                )
            }
            LocalBindingResolutionKind::Delegated {
                delegation,
                owner_binding,
            } => {
                if delegation.authorization_domain != input.authorization_domain
                    || owner_binding.authorization_domain != input.authorization_domain
                    || delegation.transport != input.proof.transport
                    || delegation.delegate_pubkey != input.proof.actor_pubkey
                    || delegation.owner_pubkey != owner_binding.event_author_pubkey
                    || delegation.request_fingerprint != input.proof.request_fingerprint
                    || delegation.target_fingerprint != input.proof.target_fingerprint
                    || delegation.transport_context_fingerprint
                        != input.proof.transport_context_fingerprint
                    || input.proof.bound_assertion_fingerprint.is_some()
                    || input.proof.delegation_conditions_fingerprint
                        != Some(delegation.conditions_fingerprint)
                    || !delegation.capabilities.contains(&input.capability)
                {
                    return Err(AuthorizationError::DelegationMismatch);
                }
                let delegated_max = policy
                    .delegated_max_expires_at
                    .ok_or(AuthorizationError::DelegationMismatch)?;
                let expires_at = Self::effective_lease_upper_bound(
                    authoritative_now,
                    &[
                        Some(input.proof.expires_at),
                        Some(delegation.expires_at),
                        owner_binding.expires_at,
                        Some(policy.expires_at),
                        Some(delegated_max),
                        policy.stronger_owner_expires_at,
                    ],
                )?;
                (
                    Some(delegation.owner_pubkey),
                    owner_binding.binding_id,
                    owner_binding.binding_version,
                    Some(delegation.relationship_id),
                    Some(delegation.relationship_revision),
                    expires_at,
                    AuthorizationReason::DelegatedOwnerBinding,
                    None,
                )
            }
        };

        let context = AuthContext {
            authorization_domain: input.authorization_domain,
            correlation_id: input.correlation_id,
            actor_pubkey: input.proof.actor_pubkey,
            owner_pubkey,
            binding_id,
            binding_version,
            capability: input.capability,
            reason,
            transport: input.proof.transport,
            request_fingerprint: input.proof.request_fingerprint,
            lease: BoundedAuthorizationLease {
                lease_id: policy.lease_id,
                authorization_domain: input.authorization_domain,
                capability: input.capability,
                actor_pubkey: input.proof.actor_pubkey,
                owner_pubkey,
                binding_id,
                binding_version,
                relationship_id,
                relationship_revision,
                delegation_conditions_fingerprint: input.proof.delegation_conditions_fingerprint,
                request_fingerprint: input.proof.request_fingerprint,
                target_fingerprint: input.proof.target_fingerprint,
                transport: input.proof.transport,
                transport_context_fingerprint: input.proof.transport_context_fingerprint,
                issued_at: authoritative_now,
                fence: policy.fence,
                policy_revision: policy.policy_revision,
                invalidation_generation: policy.invalidation_generation,
                authority_epoch: policy.authority_epoch,
                expires_at,
            },
        };
        let recheck = PreparedAuthorizationRecheck {
            lease_dependencies: context.lease.dependency_snapshot(),
            verifier_stamp,
        };
        Ok(PreparedAuthorization { context, recheck })
    }

    /// Consume a prepared value and its exact post-I/O witness.
    pub fn finalize(
        prepared: PreparedAuthorization,
        witness: AuthorizationFinalizationWitness,
    ) -> Result<AuthContext, AuthorizationError> {
        if prepared.recheck != witness.recheck {
            return Err(AuthorizationError::StaleRecheck);
        }
        if !prepared
            .context
            .lease
            .is_valid_at(witness.authoritative_now)
        {
            return Err(AuthorizationError::Expired);
        }
        Ok(prepared.context)
    }

    fn effective_lease_upper_bound(
        authoritative_now: DateTime<Utc>,
        bounds: &[Option<DateTime<Utc>>],
    ) -> Result<DateTime<Utc>, AuthorizationError> {
        let expires_at = bounds
            .iter()
            .flatten()
            .copied()
            .min()
            .ok_or(AuthorizationError::Expired)?;
        if authoritative_now >= expires_at {
            return Err(AuthorizationError::Expired);
        }
        Ok(expires_at)
    }
}

/// Stable fail-closed authorization errors without credential material.
#[derive(Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorizationError {
    /// Input identifiers were not valid server-generated identifiers.
    #[error("invalid authorization input")]
    InvalidInput,
    /// Two independently verified values named different domains.
    #[error("authorization domain mismatch")]
    DomainMismatch,
    /// Assertion, proof, or active binding did not match exactly.
    #[error("binding mismatch")]
    BindingMismatch,
    /// Delegation, actor, owner, request, or capability did not match exactly.
    #[error("delegation mismatch")]
    DelegationMismatch,
    /// Enrollment mode did not have its required local or signed authority.
    #[error("enrollment authority mismatch")]
    EnrollmentAuthorityMismatch,
    /// A post-I/O dependency or verifier generation changed.
    #[error("authorization recheck changed")]
    StaleRecheck,
    /// A half-open time bound was no longer valid.
    #[error("authorization expired")]
    Expired,
}

impl AuthorizationError {
    /// Unique stable machine code without credential or policy detail.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "nip_fi_auth_invalid_input",
            Self::DomainMismatch => "nip_fi_auth_domain_mismatch",
            Self::BindingMismatch => "nip_fi_auth_binding_mismatch",
            Self::DelegationMismatch => "nip_fi_auth_delegation_mismatch",
            Self::EnrollmentAuthorityMismatch => "nip_fi_auth_enrollment_authority_mismatch",
            Self::StaleRecheck => "nip_fi_auth_stale_recheck",
            Self::Expired => "nip_fi_auth_expired",
        }
    }
}

impl fmt::Debug for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationError([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::AuthorizationLeaseFence;
    use chrono::{Duration, TimeZone};
    use nostr::Keys;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap()
    }

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(1))
    }

    fn finalized(
        input: AuthorizationInput,
        resolution: LocalBindingResolution,
        policy: LocalAuthorizationPolicy,
        authoritative_now: DateTime<Utc>,
    ) -> Result<AuthContext, AuthorizationError> {
        let prepared =
            AuthorizationFinalizer::prepare(input, resolution, policy, authoritative_now)?;
        let rechecked = prepared.recheck_request();
        let witness = AuthorizationFinalizationWitness::from_authoritative_recheck(
            &prepared,
            AuthoritativeAuthorizationRecheck {
                recheck: rechecked,
                authoritative_now,
            },
        )?;
        AuthorizationFinalizer::finalize(prepared, witness)
    }

    fn proof(
        actor_pubkey: PublicKey,
        bound_assertion_fingerprint: Option<[u8; 32]>,
        delegation_conditions_fingerprint: Option<[u8; 32]>,
        expires_at: DateTime<Utc>,
    ) -> VerifiedNostrProof {
        proof_for_transport(
            actor_pubkey,
            ProofTransport::Nip42,
            bound_assertion_fingerprint,
            delegation_conditions_fingerprint,
            expires_at,
        )
    }

    fn proof_for_transport(
        actor_pubkey: PublicKey,
        transport: ProofTransport,
        bound_assertion_fingerprint: Option<[u8; 32]>,
        delegation_conditions_fingerprint: Option<[u8; 32]>,
        expires_at: DateTime<Utc>,
    ) -> VerifiedNostrProof {
        VerifiedNostrProof::from_verifier(
            domain(),
            actor_pubkey,
            transport,
            [3; 32],
            [8; 32],
            [9; 32],
            bound_assertion_fingerprint,
            delegation_conditions_fingerprint,
            expires_at,
        )
        .unwrap()
    }

    fn policy(expires_at: DateTime<Utc>) -> LocalAuthorizationPolicy {
        policy_for(RouteCapability::MessagesRead, expires_at)
    }

    fn policy_for(
        capability: RouteCapability,
        expires_at: DateTime<Utc>,
    ) -> LocalAuthorizationPolicy {
        LocalAuthorizationPolicy::from_database(
            domain(),
            Uuid::from_u128(10),
            1,
            0,
            1,
            AuthorizationLeaseFence::from_bytes([4; 32]).unwrap(),
            capability,
            expires_at,
            Some(expires_at),
            None,
        )
        .unwrap()
    }

    #[test]
    fn enforce_requires_explicit_validated_audit_capacity() {
        let fixture = AuthorizationEventCapacityPolicy::new(10_000, 16 << 20, 16 << 10).unwrap();
        assert_eq!(
            AuthorizationAuditConfig::new(NipFiMode::Enforce, None),
            Err(AuthorizationAuditConfigError::MissingEventCapacity)
        );
        assert!(AuthorizationAuditConfig::new(NipFiMode::Enforce, Some(fixture)).is_ok());
        assert_eq!(
            AuthorizationAuditConfig::new(NipFiMode::Off, Some(fixture)),
            Err(AuthorizationAuditConfigError::UnexpectedEventCapacity)
        );
    }

    #[test]
    fn audit_capacity_accepts_exact_hard_maxima_and_rejects_each_successor() {
        let exact_maxima = AuthorizationEventCapacityPolicy::new(
            HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN,
            HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN,
            HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES,
        )
        .unwrap();
        assert_eq!(
            exact_maxima.max_events_per_domain(),
            HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN
        );
        assert_eq!(
            exact_maxima.max_bytes_per_domain(),
            HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN
        );
        assert_eq!(
            exact_maxima.max_envelope_bytes(),
            HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES
        );

        assert_eq!(
            AuthorizationEventCapacityPolicy::new(
                HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN + 1,
                HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN,
                HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES,
            ),
            Err(AuthorizationEventCapacityPolicyError::EventCount)
        );
        assert_eq!(
            AuthorizationEventCapacityPolicy::new(
                HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN,
                HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN + 1,
                HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES,
            ),
            Err(AuthorizationEventCapacityPolicyError::DomainBytes)
        );
        assert_eq!(
            AuthorizationEventCapacityPolicy::new(
                HARD_MAX_AUTHORIZATION_EVENTS_PER_DOMAIN,
                HARD_MAX_AUTHORIZATION_EVENT_BYTES_PER_DOMAIN,
                HARD_MAX_AUTHORIZATION_EVENT_ENVELOPE_BYTES + 1,
            ),
            Err(AuthorizationEventCapacityPolicyError::EnvelopeBytes)
        );
    }

    #[test]
    fn unprotected_routes_need_no_policy_or_lease_configuration() {
        assert!(AuthorizationAuditConfig::new(NipFiMode::Off, None).is_ok());
        assert_eq!(RouteProtection::Unprotected, RouteProtection::Unprotected);
    }

    #[test]
    fn direct_and_delegated_policies_reject_unallocated_authority_epoch() {
        for delegated_max_expires_at in [None, Some(now() + Duration::minutes(1))] {
            assert!(LocalAuthorizationPolicy::from_database(
                domain(),
                Uuid::from_u128(10),
                1,
                0,
                0,
                AuthorizationLeaseFence::from_bytes([4; 32]).unwrap(),
                RouteCapability::MessagesRead,
                now() + Duration::minutes(1),
                delegated_max_expires_at,
                None,
            )
            .is_none());
        }
    }

    #[test]
    fn direct_lease_uses_earliest_exclusive_expiry() {
        let actor = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "opaque-subject".into(),
        )
        .unwrap();
        let assertion = VerifiedFederatedAssertion::from_verifier(
            domain(),
            principal.clone(),
            ProofTransport::Nip42,
            [2; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() - Duration::seconds(1),
            now() + Duration::seconds(90),
        )
        .unwrap();
        let binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            actor,
            Some(now() + Duration::seconds(60)),
        )
        .unwrap();
        let input = AuthorizationInput::new(
            domain(),
            Uuid::from_u128(5),
            proof(actor, Some([2; 32]), None, now() + Duration::seconds(120)),
            RouteCapability::MessagesRead,
        )
        .unwrap();
        let context = finalized(
            input,
            LocalBindingResolution::direct(assertion, binding),
            policy(now() + Duration::seconds(180)),
            now(),
        )
        .unwrap();

        assert_eq!(context.lease().expires_at(), now() + Duration::seconds(60));
        assert!(context.lease().is_valid_at(now() + Duration::seconds(59)));
        assert!(!context.lease().is_valid_at(now() + Duration::seconds(60)));
    }

    #[test]
    fn root_http_transports_are_exact_and_non_substitutable() {
        let actor = Keys::generate().public_key();
        for (transport, substitutes) in [
            (
                ProofTransport::GitSmartHttpSession,
                [ProofTransport::Nip98, ProofTransport::Blossom],
            ),
            (
                ProofTransport::Blossom,
                [ProofTransport::Nip98, ProofTransport::GitSmartHttpSession],
            ),
        ] {
            let principal = FederatedPrincipal::from_verified_parts(
                "https://issuer.example".into(),
                "opaque-subject".into(),
            )
            .unwrap();
            let assertion = VerifiedFederatedAssertion::from_verifier(
                domain(),
                principal.clone(),
                transport,
                [2; 32],
                [8; 32],
                [3; 32],
                [9; 32],
                now() - Duration::seconds(1),
                now() + Duration::seconds(90),
            )
            .unwrap();
            let binding = ActiveLocalBinding::from_storage(
                domain(),
                principal,
                Uuid::from_u128(6),
                1,
                actor,
                None,
            )
            .unwrap();
            let finalize = |proof: VerifiedNostrProof| {
                finalized(
                    AuthorizationInput::new(
                        domain(),
                        Uuid::from_u128(5),
                        proof,
                        RouteCapability::MessagesRead,
                    )
                    .unwrap(),
                    LocalBindingResolution::direct(assertion.clone(), binding.clone()),
                    policy(now() + Duration::seconds(180)),
                    now(),
                )
            };
            let exact = proof_for_transport(
                actor,
                transport,
                Some([2; 32]),
                None,
                now() + Duration::seconds(120),
            );
            let context = finalize(exact.clone()).unwrap();
            assert_eq!(context.transport(), transport);
            assert_eq!(
                context.lease().request_binding(),
                (&[3; 32], &[8; 32], &[9; 32])
            );

            for substitute in substitutes {
                let mut changed = exact.clone();
                changed.transport = substitute;
                assert!(matches!(
                    finalize(changed),
                    Err(AuthorizationError::BindingMismatch)
                ));
            }
            for changed in [
                {
                    let mut changed = exact.clone();
                    changed.request_fingerprint = [12; 32];
                    changed
                },
                {
                    let mut changed = exact.clone();
                    changed.target_fingerprint = [12; 32];
                    changed
                },
                {
                    let mut changed = exact.clone();
                    changed.transport_context_fingerprint = [12; 32];
                    changed
                },
            ] {
                assert!(matches!(
                    finalize(changed),
                    Err(AuthorizationError::BindingMismatch)
                ));
            }
        }
    }

    #[test]
    fn delegated_path_is_assertion_free_and_actor_bound() {
        let owner = Keys::generate().public_key();
        let delegate = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "owner-subject".into(),
        )
        .unwrap();
        let owner_binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            owner,
            Some(now() + Duration::seconds(100)),
        )
        .unwrap();
        let delegation = VerifiedDelegation::from_verifier(
            domain(),
            owner,
            delegate,
            ProofTransport::Nip42,
            Uuid::from_u128(7),
            1,
            vec![RouteCapability::MessagesRead],
            [11; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() + Duration::seconds(80),
        )
        .unwrap();
        let input = AuthorizationInput::new(
            domain(),
            Uuid::from_u128(5),
            proof(
                delegate,
                None,
                Some([11; 32]),
                now() + Duration::seconds(120),
            ),
            RouteCapability::MessagesRead,
        )
        .unwrap();
        let context = finalized(
            input,
            LocalBindingResolution::delegated(delegation, owner_binding),
            policy(now() + Duration::seconds(180)),
            now(),
        )
        .unwrap();

        assert_eq!(context.owner_pubkey(), Some(owner));
        assert_eq!(context.lease().expires_at(), now() + Duration::seconds(80));
        let snapshot = context.lease().dependency_snapshot();
        assert_eq!(snapshot.binding(), (Uuid::from_u128(6), 1));
        assert_eq!(
            snapshot.delegated_relationship(),
            Some((Uuid::from_u128(7), 1, &[11; 32]))
        );
    }

    #[test]
    fn direct_fingerprints_and_optional_expiry_fail_closed() {
        let actor = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "opaque-subject".into(),
        )
        .unwrap();
        let base_assertion = VerifiedFederatedAssertion::from_verifier(
            domain(),
            principal.clone(),
            ProofTransport::Nip42,
            [2; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() - Duration::seconds(1),
            now() + Duration::seconds(90),
        )
        .unwrap();
        let base_binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            actor,
            None,
        )
        .unwrap();
        let base_proof = proof(actor, Some([2; 32]), None, now() + Duration::seconds(120));
        let finalize = |assertion: VerifiedFederatedAssertion,
                        binding: ActiveLocalBinding,
                        proof: VerifiedNostrProof| {
            finalized(
                AuthorizationInput::new(
                    domain(),
                    Uuid::from_u128(5),
                    proof,
                    RouteCapability::MessagesRead,
                )
                .unwrap(),
                LocalBindingResolution::direct(assertion, binding),
                policy(now() + Duration::seconds(180)),
                now(),
            )
        };

        let mut assertion = base_assertion.clone();
        assertion.target_fingerprint = [12; 32];
        assert!(matches!(
            finalize(assertion, base_binding.clone(), base_proof.clone()),
            Err(AuthorizationError::BindingMismatch)
        ));
        let mut assertion = base_assertion.clone();
        assertion.request_fingerprint = [12; 32];
        assert!(matches!(
            finalize(assertion, base_binding.clone(), base_proof.clone()),
            Err(AuthorizationError::BindingMismatch)
        ));
        let mut assertion = base_assertion.clone();
        assertion.transport_context_fingerprint = [12; 32];
        assert!(matches!(
            finalize(assertion, base_binding.clone(), base_proof.clone()),
            Err(AuthorizationError::BindingMismatch)
        ));
        let mut assertion = base_assertion.clone();
        assertion.assertion_fingerprint = [12; 32];
        assert!(matches!(
            finalize(assertion, base_binding.clone(), base_proof),
            Err(AuthorizationError::BindingMismatch)
        ));

        let mut expiring_binding = base_binding;
        expiring_binding.expires_at = Some(now());
        assert!(matches!(
            finalize(
                base_assertion,
                expiring_binding,
                proof(actor, Some([2; 32]), None, now() + Duration::seconds(120),)
            ),
            Err(AuthorizationError::Expired)
        ));
    }

    #[test]
    fn delegation_coordinates_each_deny_on_mismatch() {
        let owner = Keys::generate().public_key();
        let delegate = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "owner-subject".into(),
        )
        .unwrap();
        let owner_binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            owner,
            None,
        )
        .unwrap();
        let delegation = VerifiedDelegation::from_verifier(
            domain(),
            owner,
            delegate,
            ProofTransport::Nip42,
            Uuid::from_u128(7),
            1,
            vec![RouteCapability::MessagesRead],
            [11; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() + Duration::seconds(80),
        )
        .unwrap();
        let proof = proof(
            delegate,
            None,
            Some([11; 32]),
            now() + Duration::seconds(120),
        );
        let finalize = |delegation: VerifiedDelegation, proof: VerifiedNostrProof| {
            finalized(
                AuthorizationInput::new(
                    domain(),
                    Uuid::from_u128(5),
                    proof,
                    RouteCapability::MessagesRead,
                )
                .unwrap(),
                LocalBindingResolution::delegated(delegation, owner_binding.clone()),
                policy(now() + Duration::seconds(180)),
                now(),
            )
        };

        let mut changed = delegation.clone();
        changed.owner_pubkey = Keys::generate().public_key();
        assert!(matches!(
            finalize(changed, proof.clone()),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.actor_pubkey = Keys::generate().public_key();
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.transport = ProofTransport::Nip98;
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.target_fingerprint = [12; 32];
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.request_fingerprint = [12; 32];
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.transport_context_fingerprint = [12; 32];
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed_proof = proof.clone();
        changed_proof.delegation_conditions_fingerprint = Some([12; 32]);
        assert!(matches!(
            finalize(delegation.clone(), changed_proof),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed = delegation.clone();
        changed.capabilities = vec![RouteCapability::GitRead];
        assert!(matches!(
            finalize(changed, proof.clone()),
            Err(AuthorizationError::DelegationMismatch)
        ));
        let mut changed = delegation;
        changed.authorization_domain = CommunityId::from_uuid(Uuid::from_u128(99));
        assert!(matches!(
            finalize(changed, proof),
            Err(AuthorizationError::DelegationMismatch)
        ));

        assert!(VerifiedDelegation::from_verifier(
            domain(),
            owner,
            delegate,
            ProofTransport::Nip42,
            Uuid::from_u128(7),
            0,
            vec![RouteCapability::MessagesRead],
            [11; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() + Duration::seconds(80),
        )
        .is_none());
    }

    #[test]
    fn invite_mint_and_claim_are_non_substitutable() {
        let owner = Keys::generate().public_key();
        let delegate = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "owner-subject".into(),
        )
        .unwrap();
        let owner_binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            owner,
            None,
        )
        .unwrap();
        let delegation = VerifiedDelegation::from_verifier(
            domain(),
            owner,
            delegate,
            ProofTransport::Nip42,
            Uuid::from_u128(7),
            1,
            vec![RouteCapability::InviteMint],
            [11; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() + Duration::minutes(1),
        )
        .unwrap();
        let result = finalized(
            AuthorizationInput::new(
                domain(),
                Uuid::from_u128(5),
                proof(delegate, None, Some([11; 32]), now() + Duration::minutes(1)),
                RouteCapability::InviteClaim,
            )
            .unwrap(),
            LocalBindingResolution::delegated(delegation, owner_binding),
            policy_for(RouteCapability::InviteClaim, now() + Duration::minutes(1)),
            now(),
        );
        assert!(matches!(
            result,
            Err(AuthorizationError::DelegationMismatch)
        ));
    }

    #[test]
    fn reasons_and_errors_have_unique_stable_codes_and_redacted_debug() {
        use std::collections::HashSet;

        let reasons = [
            AuthorizationReason::ExistingBinding,
            AuthorizationReason::EnrolledAttestedKey,
            AuthorizationReason::EnrolledProvisioned,
            AuthorizationReason::EnrolledRiskLabelledTofu,
            AuthorizationReason::DelegatedOwnerBinding,
        ];
        assert_eq!(
            reasons
                .iter()
                .map(|reason| reason.code())
                .collect::<HashSet<_>>()
                .len(),
            reasons.len()
        );
        for reason in reasons {
            assert_eq!(format!("{reason:?}"), "AuthorizationReason([REDACTED])");
        }

        let errors = [
            AuthorizationError::InvalidInput,
            AuthorizationError::DomainMismatch,
            AuthorizationError::BindingMismatch,
            AuthorizationError::DelegationMismatch,
            AuthorizationError::EnrollmentAuthorityMismatch,
            AuthorizationError::StaleRecheck,
            AuthorizationError::Expired,
        ];
        assert_eq!(
            errors
                .iter()
                .map(|error| error.code())
                .collect::<HashSet<_>>()
                .len(),
            errors.len()
        );
        for error in errors {
            assert_eq!(format!("{error:?}"), "AuthorizationError([REDACTED])");
        }

        let verifier_errors = [
            CanonicalVerifierError::InvalidPolicy,
            CanonicalVerifierError::MalformedInput,
            CanonicalVerifierError::InvalidToken,
            CanonicalVerifierError::UnsupportedAlgorithm,
            CanonicalVerifierError::MissingKeyId,
            CanonicalVerifierError::UnknownKeyId,
            CanonicalVerifierError::InvalidKey,
            CanonicalVerifierError::InvalidClaim,
            CanonicalVerifierError::InvalidTimeBounds,
        ];
        assert_eq!(
            verifier_errors
                .iter()
                .map(|error| error.code())
                .collect::<HashSet<_>>()
                .len(),
            verifier_errors.len()
        );
    }

    #[test]
    fn prepared_authorization_requires_exact_post_io_witness() {
        let actor = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "opaque-subject".into(),
        )
        .unwrap();
        let assertion = VerifiedFederatedAssertion::from_verifier(
            domain(),
            principal.clone(),
            ProofTransport::Nip42,
            [2; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() - Duration::seconds(1),
            now() + Duration::seconds(30),
        )
        .unwrap();
        let binding = ActiveLocalBinding::from_storage(
            domain(),
            principal,
            Uuid::from_u128(6),
            1,
            actor,
            None,
        )
        .unwrap();
        let prepared = AuthorizationFinalizer::prepare(
            AuthorizationInput::new(
                domain(),
                Uuid::from_u128(5),
                proof(actor, Some([2; 32]), None, now() + Duration::seconds(30)),
                RouteCapability::MessagesRead,
            )
            .unwrap(),
            LocalBindingResolution::direct(assertion, binding),
            policy(now() + Duration::seconds(30)),
            now(),
        )
        .unwrap();

        let mut changed = prepared.recheck_request();
        changed.lease_dependencies.policy_revision += 1;
        assert!(matches!(
            AuthorizationFinalizationWitness::from_authoritative_recheck(
                &prepared,
                AuthoritativeAuthorizationRecheck {
                    recheck: changed,
                    authoritative_now: now() + Duration::seconds(1),
                },
            ),
            Err(AuthorizationError::StaleRecheck)
        ));

        let mut rotated = prepared.recheck_request();
        rotated.verifier_stamp = Some(VerifierPolicyStamp::new(
            CanonicalVerifierPolicyId([1; 32]),
            VerifierKeyGeneration(2),
        ));
        assert!(matches!(
            AuthorizationFinalizationWitness::from_authoritative_recheck(
                &prepared,
                AuthoritativeAuthorizationRecheck {
                    recheck: rotated,
                    authoritative_now: now() + Duration::seconds(1),
                },
            ),
            Err(AuthorizationError::StaleRecheck)
        ));

        assert!(matches!(
            AuthorizationFinalizationWitness::from_authoritative_recheck(
                &prepared,
                AuthoritativeAuthorizationRecheck {
                    recheck: prepared.recheck_request(),
                    authoritative_now: prepared.expires_at(),
                },
            ),
            Err(AuthorizationError::Expired)
        ));
    }

    #[test]
    fn enrollment_proposals_are_sealed_and_explicitly_label_risk() {
        let actor = Keys::generate().public_key();
        let principal = FederatedPrincipal::from_verified_parts(
            "https://issuer.example".into(),
            "opaque-subject".into(),
        )
        .unwrap();
        let mut assertion = VerifiedFederatedAssertion::from_verifier(
            domain(),
            principal.clone(),
            ProofTransport::Nip42,
            [2; 32],
            [8; 32],
            [3; 32],
            [9; 32],
            now() - Duration::seconds(1),
            now() + Duration::minutes(1),
        )
        .unwrap();
        assertion.attested_event_author_pubkey = Some(actor);
        let proof = proof(actor, Some([2; 32]), None, now() + Duration::minutes(1));

        for mode in [
            DirectEnrollmentMode::Attested,
            DirectEnrollmentMode::Provisioned,
            DirectEnrollmentMode::RiskLabelledTofu,
        ] {
            let authority = LocalEnrollmentAuthority::from_database_parts(
                domain(),
                principal.issuer.clone(),
                principal.subject.clone(),
                actor,
                mode.database_code(),
                7,
                [7; 32],
            )
            .unwrap();
            let proposal = DirectEnrollmentProposal::from_verified_evidence(
                authority,
                assertion.clone(),
                proof.clone(),
                now(),
            )
            .unwrap();
            assert_eq!(proposal.mode(), mode);
            assert_eq!(
                format!("{proposal:?}"),
                "DirectEnrollmentProposal([REDACTED])"
            );
        }

        assertion.attested_event_author_pubkey = Some(Keys::generate().public_key());
        let authority = LocalEnrollmentAuthority::from_database_parts(
            domain(),
            principal.issuer,
            principal.subject,
            actor,
            DirectEnrollmentMode::Attested.database_code(),
            7,
            [7; 32],
        )
        .unwrap();
        assert!(matches!(
            DirectEnrollmentProposal::from_verified_evidence(authority, assertion, proof, now(),),
            Err(AuthorizationError::EnrollmentAuthorityMismatch)
        ));
    }

    #[test]
    fn verifier_policy_identifier_excludes_key_generation() {
        let policy = CanonicalVerifierPolicy::new(
            "https://issuer.example".to_owned(),
            "buzz".to_owned(),
            "sub".to_owned(),
            Some("npub".to_owned()),
            60,
            3_600,
        )
        .unwrap();
        let policy_again = CanonicalVerifierPolicy::new(
            "https://issuer.example".to_owned(),
            "buzz".to_owned(),
            "sub".to_owned(),
            Some("npub".to_owned()),
            60,
            3_600,
        )
        .unwrap();
        assert_eq!(policy.id(), policy_again.id());
        assert_ne!(
            VerifierPolicyStamp::new(policy.id(), VerifierKeyGeneration(1)),
            VerifierPolicyStamp::new(policy.id(), VerifierKeyGeneration(2))
        );
        assert_eq!(format!("{policy:?}"), "CanonicalVerifierPolicy([REDACTED])");
        assert!(VerifierKeyGeneration::new(0).is_none());
        assert!(CanonicalVerifierPolicy::new(
            String::new(),
            "buzz".to_owned(),
            "sub".to_owned(),
            None,
            60,
            3_600,
        )
        .is_err());

        let verifier = CanonicalFederatedAssertionVerifier::new(policy);
        let key_set = CanonicalVerifierKeySet::new(
            VerifierKeyGeneration::new(1).unwrap(),
            JwkSet { keys: Vec::new() },
        );
        assert!(matches!(
            verifier.verify(
                "malformed",
                &key_set,
                domain(),
                ProofTransport::Nip42,
                [1; 32],
                [2; 32],
                [3; 32],
            ),
            Err(CanonicalVerifierError::InvalidToken)
        ));
    }
}
