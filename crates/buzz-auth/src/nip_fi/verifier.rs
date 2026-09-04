//! The single provider-neutral assertion verifier (`FI-INV-16`).
//!
//! Every accepted compact JWS feeds this one contract and produces a sealed
//! [`VerifiedAssertion`]. Multi-issuer selection happens here: the exact `iss`
//! carried by the token selects one [`IssuerPolicy`] and its key source; there
//! is no single-global-issuer assumption. Almost every failure collapses to the
//! public [`DenialClass::EvidenceRejected`] class; the exceptions are the
//! unreadable required current dependencies
//! [`VerifierError::KeySourceUnavailable`] and
//! [`VerifierError::StatusWitnessUnavailable`], which map to
//! [`DenialClass::AuthorizationUnavailable`] so a missing authoritative
//! dependency never masquerades as rejected evidence. The granular
//! [`VerifierError`] variants are for access-controlled logs and metrics only.
//!
//! Corrections applied to the mined #1476 verifier, per the settled spec:
//!
//! - **Token class + `typ` enforcement**: a policy selects exactly one class
//!   before parsing claims; `at+jwt` and `nip-fi+jwt` `typ` values are enforced
//!   exactly, and the long-form `application/at+jwt` is rejected.
//! - **ID-token denial**: OIDC ID tokens deny even when `iss`, `aud`, `sub`
//!   match, via exact `typ` mismatch against every accepted class.
//! - **Fixed `nostr_pubkey`**: accepted only as lowercase hex of exactly one
//!   32-byte key; bech32 and other aliases deny.
//! - **Spec-exact time arithmetic**: `now < exp`, `iat <= now + skew`,
//!   `now < iat + maximum_assertion_age`, `nbf <= now + skew`, equality at an
//!   expiry is expired.

use super::assertion::{CanonicalCapabilities, RevalidationDependencies, VerifiedAssertion};
use super::config::{
    is_asymmetric_algorithm, ClientSubjectPosture, FreshnessClass, IssuerPolicy, IssuerRegistry,
    SubjectClass, TokenClass, TransportContractId, MAX_CLIENT_ID_BYTES, MAX_JWKS_KEYS,
    MAX_KID_BYTES, MAX_SUBJECT_BYTES, MAX_TOKEN_BYTES, NOSTR_PUBKEY_CLAIM, OAUTH_CLIENT_ID_CLAIM,
    SUBJECT_CLAIM,
};
use super::denial::DenialClass;
use chrono::{DateTime, TimeZone, Utc};
use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse,
};
use jsonwebtoken::{decode, jwk::Jwk, Algorithm, DecodingKey, Validation};
use nostr::PublicKey;
use serde::de::{Deserializer, Error as _, MapAccess, Visitor};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fmt;

/// Sealing for [`IssuerKeySource`]: only types defined in this crate can name
/// this private supertrait, so no external `buzz_auth` consumer can implement
/// the key-source trait. Combined with the crate-private [`AssertionKeySet`]
/// constructor, this makes the accepted issuer→JWKS authority impossible to
/// synthesize outside the crate's trusted configuration path.
pub(crate) mod sealed {
    /// Private marker preventing external implementations of the key source.
    pub trait Sealed {}

    // Blanket seal for `Arc<S>` so `Arc<ProductionJwksSource>` satisfies
    // the sealed supertrait without requiring callers to implement it.
    impl<S: Sealed> Sealed for std::sync::Arc<S> {}
}

/// One issuer's key source: a JWKS snapshot bound to the exact `iss` it
/// authenticates, with a positive generation and a required hard deadline
/// beyond which the snapshot can no longer authorize.
///
/// The issuer binding is the anti-cross-issuer control (`FI-INV`): a snapshot
/// authenticates only tokens whose signed `iss` equals [`Self::issuer`]. The
/// binding is not caller-forgeable, at the request seam or the authority-
/// construction seam: [`verify`] takes no snapshot argument, and this type has
/// no public constructor, so an external consumer cannot build a snapshot that
/// labels issuer B's JWKS as issuer A. Building a snapshot (and the source that
/// serves it) is the trusted configuration act the `jwks` runtime performs at
/// startup, not a per-request or external input.
///
/// The crate-private constructor is a live regression: an external crate that
/// tries to build a snapshot — the pass-2 exploit's relabelling step — cannot
/// even name the constructor, so this fails to compile.
///
/// ```compile_fail
/// use buzz_auth::AssertionKeySet;
/// let _forge = AssertionKeySet::new;
/// ```
///
/// [`verify`]: FederatedAssertionVerifier::verify
#[derive(Clone)]
pub struct AssertionKeySet {
    issuer: String,
    generation: u64,
    jwks: JwkSet,
    hard_deadline: DateTime<Utc>,
}

impl AssertionKeySet {
    /// Seal a parsed JWKS for exactly one issuer, with a positive cache
    /// generation and a required key-snapshot hard deadline. Rejects a zero
    /// generation, an empty issuer, an empty or oversized key set
    /// ([`MAX_JWKS_KEYS`]), or a non-positive deadline. Crate-private: only the
    /// trusted in-crate configuration path (the `jwks` runtime) may bind key
    /// material to an issuer.
    ///
    /// Bounding the key count here is the pre-lookup control (NIP-FI.md:166-171):
    /// [`verify`] scans this snapshot by an attacker-controlled `kid` on every
    /// token naming the issuer, so an unbounded snapshot would let an oversized
    /// JWKS turn each lookup into an attacker-driven O(keys) scan. The deadline
    /// is required rather than optional so every sealed assertion carries a
    /// finite key-snapshot bound into `revalidation_dependencies`
    /// (NIP-FI.md:240-249).
    ///
    pub(crate) fn new(
        issuer: String,
        generation: u64,
        jwks: JwkSet,
        hard_deadline: DateTime<Utc>,
    ) -> Option<Self> {
        if generation == 0
            || issuer.is_empty()
            || jwks.keys.is_empty()
            || jwks.keys.len() > MAX_JWKS_KEYS
            || hard_deadline.timestamp() <= 0
        {
            return None;
        }
        Some(Self {
            issuer,
            generation,
            jwks,
            hard_deadline,
        })
    }

    /// The exact `iss` this snapshot authenticates.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The positive snapshot generation carried into `revalidation_dependencies`.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The snapshot hard deadline. Test-only accessor for deadline-crossing
    /// oracles; not compiled into production builds.
    #[cfg(test)]
    pub(crate) fn hard_deadline(&self) -> chrono::DateTime<chrono::Utc> {
        self.hard_deadline
    }
}

impl fmt::Debug for AssertionKeySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertionKeySet([REDACTED])")
    }
}

/// The trusted, verifier-owned mapping from an authenticated issuer to its key
/// snapshot. This is the sole path by which key material enters verification:
/// [`FederatedAssertionVerifier::verify`] takes no snapshot from its caller and
/// instead asks this source for the snapshot bound to the token's
/// signature-authenticated `iss`. A request-path caller therefore cannot
/// relabel one issuer's JWKS as another's — the cross-issuer bypass at the old
/// `verify(token, key_set)` seam. Configuring the source (the `jwks` runtime)
/// is a trusted startup act, not per-request input.
///
/// This trait is sealed via a private supertrait, so it cannot be implemented
/// outside `buzz_auth`. That closes the authority-construction seam: an
/// external consumer cannot supply its own source that returns issuer B's JWKS
/// labelled as issuer A, because it can neither implement this trait nor build
/// an [`AssertionKeySet`]. The accepted issuer→JWKS authority is entirely
/// crate-owned.
///
/// The seal is a live regression: an external crate that tries to implement
/// this trait fails to compile because the private supertrait cannot be named.
///
/// ```compile_fail
/// use buzz_auth::{AssertionKeySet, IssuerKeySource};
/// struct Forge;
/// impl IssuerKeySource for Forge {
///     fn key_set(&self, _issuer: &str) -> Option<AssertionKeySet> { None }
/// }
/// ```
pub trait IssuerKeySource: sealed::Sealed {
    /// The current key snapshot bound to this exact issuer, or `None` when the
    /// issuer has no available snapshot. Implementations MUST return only a
    /// snapshot whose [`AssertionKeySet::issuer`] equals `issuer`.
    fn key_set(&self, issuer: &str) -> Option<AssertionKeySet>;
}

/// Forwarding implementation so a single `Arc<S>` can be cheaply cloned and
/// shared across multiple [`FederatedAssertionVerifier`] instances while all
/// of them observe every refresh committed to the shared source.
///
/// This is the canonical sharing path for `ProductionJwksSource`, which is
/// not itself `Clone` (its internal `RwLock`-protected state is not cheaply
/// copyable). Wrap it in `Arc` at startup, then pass `Arc::clone(&source)` to
/// each verifier — all verifiers read from the same underlying cache and see
/// key rotations as soon as `get_snapshot` commits them.
///
/// The blanket seal (`impl<S: Sealed> Sealed for Arc<S>`) in the `sealed`
/// module ensures this forwarding impl remains crate-owned: an external crate
/// still cannot implement `IssuerKeySource` for its own type.
impl<S: IssuerKeySource> IssuerKeySource for std::sync::Arc<S> {
    fn key_set(&self, issuer: &str) -> Option<AssertionKeySet> {
        (**self).key_set(issuer)
    }
}

/// A fixed issuer→snapshot key source for the in-crate verifier tests,
/// standing in for the `jwks` runtime. It is `cfg(test)`-only — not behind a
/// downstream-selectable Cargo feature — so no dependent crate can enable it to
/// reconstruct the authority. An honest source returns only the snapshot bound
/// to the exact issuer requested, the invariant the real runtime source
/// guarantees.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct StaticIssuerKeySource {
    snapshots: std::collections::HashMap<String, AssertionKeySet>,
    /// When set, returned for every requested issuer regardless of its binding,
    /// to exercise the verifier's defensive issuer re-check.
    misbound: Option<AssertionKeySet>,
}

#[cfg(test)]
impl StaticIssuerKeySource {
    /// Build an honest source from a set of snapshots, keyed by each snapshot's
    /// issuer.
    pub(crate) fn new(snapshots: impl IntoIterator<Item = AssertionKeySet>) -> Self {
        Self {
            snapshots: snapshots
                .into_iter()
                .map(|s| (s.issuer().to_owned(), s))
                .collect(),
            misbound: None,
        }
    }

    /// A hostile/buggy source that returns the given snapshot — bound to a
    /// different issuer than requested — for every lookup, to exercise the
    /// verifier's defensive issuer re-check.
    pub(crate) fn misbinding(snapshot: AssertionKeySet) -> Self {
        Self {
            snapshots: std::collections::HashMap::new(),
            misbound: Some(snapshot),
        }
    }
}

#[cfg(test)]
impl sealed::Sealed for StaticIssuerKeySource {}

#[cfg(test)]
impl IssuerKeySource for StaticIssuerKeySource {
    fn key_set(&self, issuer: &str) -> Option<AssertionKeySet> {
        self.misbound
            .clone()
            .or_else(|| self.snapshots.get(issuer).cloned())
    }
}

/// The provider-neutral assertion verifier over a closed multi-issuer registry
/// and a trusted [`IssuerKeySource`].
#[derive(Debug, Clone)]
pub struct FederatedAssertionVerifier<S: IssuerKeySource> {
    registry: IssuerRegistry,
    key_source: S,
    transport_contract_id: TransportContractId,
}

impl<S: IssuerKeySource> FederatedAssertionVerifier<S> {
    /// Construct a verifier over a registry of issuer policies and the trusted
    /// key source that serves each issuer's snapshot.
    pub fn new(registry: IssuerRegistry, key_source: S) -> Self {
        Self {
            registry,
            key_source,
            transport_contract_id: TransportContractId::core_client_attached(),
        }
    }

    /// The registry this verifier selects policies from.
    pub const fn registry(&self) -> &IssuerRegistry {
        &self.registry
    }

    /// Verify one compact JWS and mint a sealed [`VerifiedAssertion`].
    ///
    /// The caller supplies only the token. The key snapshot is resolved
    /// internally from the trusted [`IssuerKeySource`] by the token's
    /// signature-authenticated `iss`, so no caller can inject or relabel key
    /// material for another issuer.
    pub fn verify(&self, token: &str) -> Result<VerifiedAssertion, VerifierError> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(VerifierError::MalformedToken);
        }

        // Parse the JOSE header without trusting it. Reject duplicate members,
        // `alg=none`, symmetric algorithms, any critical header, and a
        // missing/oversized `kid` before touching claims.
        //
        // Every check up to the key-source lookup below is bounded and
        // dependency-independent, so rejected evidence is classified (403)
        // before an unreadable snapshot could produce a 503: exact compact
        // structure, the protected header, the signature segment's shape, the
        // selected policy, and the policy's algorithm and token-class contract
        // all precede key resolution (NIP-FI.md:151-171, :458-475). This is
        // round-3's offline-before-deferral guarantee at the pipeline's front
        // end.
        enforce_compact_structure(token)?;
        let header = parse_header(token)?;
        enforce_signature_shape(token)?;
        let signed_issuer = self.unverified_issuer(token)?;
        let policy = self
            .registry
            .policy_for_issuer(&signed_issuer)
            .ok_or(VerifierError::UnknownIssuer)?;

        if !policy.algorithms().contains(&header.algorithm) {
            return Err(VerifierError::UnsupportedAlgorithm);
        }
        enforce_token_type(policy.token_class(), header.typ.as_deref())?;

        // Resolve the key snapshot internally from the trusted source, keyed by
        // the policy's exact `iss`. The snapshot is never a caller argument, so
        // issuer B's keys cannot be relabelled as issuer A at the request seam.
        let key_set = self
            .key_source
            .key_set(policy.issuer())
            .ok_or(VerifierError::KeySourceUnavailable)?;
        // Defensive invariant: a correct source binds the snapshot to the exact
        // issuer requested. A source that violates this contract cannot cross
        // issuers.
        if key_set.issuer() != policy.issuer() {
            return Err(VerifierError::IssuerKeyMismatch);
        }

        // A `current-status` policy requires a runtime status witness this
        // verifier does not gather (delivered by a later PR); its deferral is
        // resolved only after every offline check below passes, so that
        // malformed or invalidly-signed input is rejected (403) rather than
        // masquerading as an availability failure (503) — see the deferral just
        // before sealing.

        // Select exactly one matching key by `kid`.
        let jwk = select_unique_jwk(&key_set.jwks, &header.kid)?;
        validate_jwk(jwk, header.algorithm)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| VerifierError::InvalidKey)?;

        // Verify signature, `iss`, and `aud`. jsonwebtoken deserializes claims
        // with last-wins duplicate handling, so its map is used only for the
        // signature/iss/aud gate; every value the result depends on is read
        // from `claims` below, our duplicate-rejecting parse of the same
        // signature-authenticated payload bytes. A duplicate member fails that
        // parse, so the two parses can never disagree on an accepted token.
        let mut validation = Validation::new(header.algorithm);
        validation.set_issuer(&[policy.issuer()]);
        validation.set_audience(policy.audiences());
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud"]);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        decode::<Map<String, Value>>(token, &key, &validation)
            .map_err(|_| VerifierError::InvalidSignatureOrClaims)?;
        let claims = parse_unique_claims(token)?;

        enforce_claim_semantics(policy, &claims)?;

        let subject = claim_string(&claims, SUBJECT_CLAIM, MAX_SUBJECT_BYTES)?;
        let asserted_key = parse_nostr_pubkey_claim(&claims)?;

        let now = Utc::now();
        let deadlines = self.check_time_and_deadlines(policy, &key_set, &claims, now)?;
        let capabilities = capture_capabilities(policy, &claims);

        // Offline validation (token-class, key, signature, audience, claims,
        // time) has now fully passed. Only an otherwise-valid `current-status`
        // assertion is deferred to the status-bearing runtime this verifier does
        // not yet gather (delivered by a later PR): an invalid token deny above
        // is `evidence_rejected` (403), and this defers a valid one as
        // `authorization_unavailable` (503) so a missing witness never
        // masquerades as rejected evidence, nor invalid input as unavailable
        // (NIP-FI.md:459-476).
        if policy.freshness() == FreshnessClass::CurrentStatus {
            return Err(VerifierError::StatusWitnessUnavailable);
        }

        Ok(VerifiedAssertion::seal(
            policy.issuer().to_owned(),
            subject,
            asserted_key,
            capabilities,
            deadlines,
            policy.id(),
            self.transport_contract_id,
            RevalidationDependencies::new(
                header.kid,
                key_set.generation(),
                key_set.hard_deadline,
                token.to_owned(),
            ),
        ))
    }

    fn unverified_issuer(&self, token: &str) -> Result<String, VerifierError> {
        let claims = parse_unique_claims(token)?;
        claim_string(&claims, "iss", MAX_SUBJECT_BYTES).map_err(|_| VerifierError::MalformedToken)
    }

    fn check_time_and_deadlines(
        &self,
        policy: &IssuerPolicy,
        key_set: &AssertionKeySet,
        claims: &Map<String, Value>,
        now: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>, VerifierError> {
        let iat = numeric_date(claims, "iat")?;
        let exp = numeric_date(claims, "exp")?;
        let skew = seconds(policy.skew_seconds());
        let max_age = seconds(policy.maximum_assertion_age_seconds());

        // now < exp (equality is expired).
        if now >= exp {
            return Err(VerifierError::Expired);
        }
        // iat <= now + skew.
        if iat > checked_add(now, skew)? {
            return Err(VerifierError::NotYetValid);
        }
        // now < iat + maximum_assertion_age.
        if now >= checked_add(iat, max_age)? {
            return Err(VerifierError::Expired);
        }
        // Optional nbf <= now + skew.
        if let Some(nbf) = optional_numeric_date(claims, "nbf")? {
            if nbf > checked_add(now, skew)? {
                return Err(VerifierError::NotYetValid);
            }
        }

        // offline authority deadline = min(exp, iat + max_age, key hard deadline).
        let mut deadlines = vec![exp, checked_add(iat, max_age)?];
        if now >= key_set.hard_deadline {
            return Err(VerifierError::Expired);
        }
        deadlines.push(key_set.hard_deadline);
        // `current-status` adds a runtime status deadline in a later PR; the
        // offline deadlines computed here always bound it.
        debug_assert!(matches!(
            policy.freshness(),
            FreshnessClass::OfflineJwt | FreshnessClass::CurrentStatus
        ));
        Ok(deadlines)
    }
}

/// A closed, stable verifier failure carrying no credential material. Almost
/// every variant maps to the public [`DenialClass::EvidenceRejected`] class;
/// [`Self::KeySourceUnavailable`] and [`Self::StatusWitnessUnavailable`] map to
/// [`DenialClass::AuthorizationUnavailable`] instead (see [`Self::denial_class`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VerifierError {
    /// The compact JWS was empty, oversized, or structurally malformed.
    #[error("malformed token")]
    MalformedToken,
    /// A protected-header or claim member appeared more than once. Ambiguous
    /// duplicate members are rejected before any value is trusted.
    #[error("duplicate member")]
    DuplicateMember,
    /// No policy is registered for the token's issuer.
    #[error("unknown issuer")]
    UnknownIssuer,
    /// The supplied key snapshot authenticates a different issuer than the
    /// token's signed `iss`. Defensive: the trusted [`IssuerKeySource`] is
    /// contracted to return only issuer-bound snapshots, so a correct source
    /// never triggers this.
    #[error("issuer/key mismatch")]
    IssuerKeyMismatch,
    /// The token's issuer is registered, but the trusted key source has no
    /// available snapshot for it (for example, a JWKS refresh has not yet
    /// succeeded). An unreadable authoritative dependency, not rejected
    /// evidence: the token may be perfectly valid.
    #[error("key source unavailable")]
    KeySourceUnavailable,
    /// The policy declares `current-status` freshness, whose runtime status
    /// witness this verifier does not yet gather. Verification defers to the
    /// status-bearing runtime rather than sealing without the witness.
    #[error("status witness unavailable")]
    StatusWitnessUnavailable,
    /// The header algorithm is `none`, symmetric, or outside the policy set.
    #[error("unsupported algorithm")]
    UnsupportedAlgorithm,
    /// The header carried a critical extension this verifier does not support.
    #[error("unsupported critical header")]
    UnsupportedCriticalHeader,
    /// The header omitted its bounded `kid`.
    #[error("missing key id")]
    MissingKeyId,
    /// No key, or more than one key, matched the header `kid`.
    #[error("ambiguous or unknown key id")]
    AmbiguousKeyId,
    /// The selected JWK was not admissible for signature verification.
    #[error("invalid key")]
    InvalidKey,
    /// The `typ` header did not match the policy's token class.
    #[error("token type rejected")]
    TokenTypeRejected,
    /// A required or forbidden claim rule for the token class failed, including
    /// resource-owner/client-subject ambiguity.
    #[error("claim contract rejected")]
    ClaimContractRejected,
    /// A required provider-free claim was missing or malformed, including a
    /// `nostr_pubkey` that was not lowercase-hex of one 32-byte key.
    #[error("claim rejected")]
    ClaimRejected,
    /// The signature, issuer, or audience did not validate.
    #[error("signature or claims rejected")]
    InvalidSignatureOrClaims,
    /// The assertion was expired or beyond its maximum age or key deadline.
    #[error("expired")]
    Expired,
    /// The assertion was not yet valid under `iat`/`nbf` and skew.
    #[error("not yet valid")]
    NotYetValid,
    /// A time claim was missing, non-integer, or arithmetically out of range.
    #[error("invalid time bounds")]
    InvalidTimeBounds,
}

impl VerifierError {
    /// The public denial class. Almost every verifier failure is evidence
    /// rejection (malformed, invalid, or expired evidence). The exceptions are
    /// the two unreadable required current dependencies —
    /// [`Self::KeySourceUnavailable`] (no verification-key snapshot) and
    /// [`Self::StatusWitnessUnavailable`] (no current-status witness) — which
    /// map to [`DenialClass::AuthorizationUnavailable`] (503) so that a missing
    /// authoritative dependency never masquerades as rejected evidence
    /// (NIP-FI.md, rejection table).
    pub const fn denial_class(self) -> DenialClass {
        match self {
            Self::KeySourceUnavailable | Self::StatusWitnessUnavailable => {
                DenialClass::AuthorizationUnavailable
            }
            _ => DenialClass::EvidenceRejected,
        }
    }

    /// A unique stable machine code, safe for access-controlled logs.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MalformedToken => "nip_fi_malformed_token",
            Self::DuplicateMember => "nip_fi_duplicate_member",
            Self::UnknownIssuer => "nip_fi_unknown_issuer",
            Self::IssuerKeyMismatch => "nip_fi_issuer_key_mismatch",
            Self::KeySourceUnavailable => "nip_fi_key_source_unavailable",
            Self::StatusWitnessUnavailable => "nip_fi_status_witness_unavailable",
            Self::UnsupportedAlgorithm => "nip_fi_unsupported_algorithm",
            Self::UnsupportedCriticalHeader => "nip_fi_unsupported_critical_header",
            Self::MissingKeyId => "nip_fi_missing_key_id",
            Self::AmbiguousKeyId => "nip_fi_ambiguous_key_id",
            Self::InvalidKey => "nip_fi_invalid_key",
            Self::TokenTypeRejected => "nip_fi_token_type_rejected",
            Self::ClaimContractRejected => "nip_fi_claim_contract_rejected",
            Self::ClaimRejected => "nip_fi_claim_rejected",
            Self::InvalidSignatureOrClaims => "nip_fi_invalid_signature_or_claims",
            Self::Expired => "nip_fi_expired",
            Self::NotYetValid => "nip_fi_not_yet_valid",
            Self::InvalidTimeBounds => "nip_fi_invalid_time_bounds",
        }
    }
}

/// A minimally parsed JOSE header.
struct ParsedHeader {
    algorithm: Algorithm,
    kid: String,
    typ: Option<String>,
}

/// Reject any token that is not exactly three compact-JWS segments.
///
/// This is a bounded, dependency-independent shape check run before key-source
/// lookup: two- or four-segment garbage (which the header/claims parsers, each
/// reading a single fixed segment, would otherwise carry past the outage seam)
/// is classified as malformed evidence (403), never as an unreadable snapshot
/// (503). The signature segment's well-formedness — non-empty and valid
/// base64url — is validated separately by [`enforce_signature_shape`] after
/// header parsing, so that no structurally malformed token can defer to the
/// key-source lookup and masquerade as a 503 outage (NIP-FI.md:151-171).
fn enforce_compact_structure(token: &str) -> Result<(), VerifierError> {
    if token.split('.').count() == 3 {
        Ok(())
    } else {
        Err(VerifierError::MalformedToken)
    }
}

/// Reject a missing or malformed signature segment before key-source lookup.
///
/// A dependency-independent shape check: the third compact segment must be
/// non-empty and valid base64url. Only cryptographic *validity* of the
/// signature needs the resolved key, so an empty or non-base64url signature is
/// malformed evidence (403) and must not defer to the outage seam (503). Run
/// after [`parse_header`], so `alg=none`'s empty-signature token is already
/// rejected at header parsing (unsupported algorithm) before this distinction
/// matters (NIP-FI.md:151-171).
fn enforce_signature_shape(token: &str) -> Result<(), VerifierError> {
    let signature = token
        .split('.')
        .nth(2)
        .filter(|s| !s.is_empty())
        .ok_or(VerifierError::MalformedToken)?;
    base64url_decode(signature).map(|_| ())
}

fn parse_header(token: &str) -> Result<ParsedHeader, VerifierError> {
    let segment = token
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or(VerifierError::MalformedToken)?;
    let bytes = base64url_decode(segment)?;
    let header = parse_unique_object(&bytes)?;

    // Any critical extension is unknown to this verifier and denies.
    if header.contains_key("crit") {
        return Err(VerifierError::UnsupportedCriticalHeader);
    }

    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or(VerifierError::MalformedToken)?;
    let algorithm = parse_algorithm(alg)?;
    if !is_asymmetric_algorithm(algorithm) {
        return Err(VerifierError::UnsupportedAlgorithm);
    }

    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty() && k.len() <= MAX_KID_BYTES)
        .ok_or(VerifierError::MissingKeyId)?
        .to_owned();

    let typ = match header.get("typ") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        // A present but non-string `typ` is malformed.
        Some(_) => return Err(VerifierError::MalformedToken),
    };

    Ok(ParsedHeader {
        algorithm,
        kid,
        typ,
    })
}

fn parse_algorithm(alg: &str) -> Result<Algorithm, VerifierError> {
    match alg {
        "RS256" => Ok(Algorithm::RS256),
        "RS384" => Ok(Algorithm::RS384),
        "RS512" => Ok(Algorithm::RS512),
        "PS256" => Ok(Algorithm::PS256),
        "PS384" => Ok(Algorithm::PS384),
        "PS512" => Ok(Algorithm::PS512),
        "ES256" => Ok(Algorithm::ES256),
        "ES384" => Ok(Algorithm::ES384),
        "EdDSA" => Ok(Algorithm::EdDSA),
        // `none` and symmetric HMAC algorithms are rejected as unsupported.
        "none" | "HS256" | "HS384" | "HS512" => Err(VerifierError::UnsupportedAlgorithm),
        _ => Err(VerifierError::UnsupportedAlgorithm),
    }
}

/// Enforce the policy's single token class against the header `typ`.
fn enforce_token_type(class: &TokenClass, typ: Option<&str>) -> Result<(), VerifierError> {
    match class {
        TokenClass::AccessTokenAtJwt { .. } => match typ {
            Some("at+jwt") => Ok(()),
            _ => Err(VerifierError::TokenTypeRejected),
        },
        TokenClass::DedicatedNipFi => match typ {
            Some("nip-fi+jwt") => Ok(()),
            _ => Err(VerifierError::TokenTypeRejected),
        },
    }
}

/// Enforce class-specific claim rules: `at+jwt` `client_id` presence and
/// resource-owner/client-subject classification via the issuer's
/// [`SubjectClassContract`].
fn enforce_claim_semantics(
    policy: &IssuerPolicy,
    claims: &Map<String, Value>,
) -> Result<(), VerifierError> {
    match policy.token_class() {
        TokenClass::AccessTokenAtJwt { subject_class } => {
            // One non-empty bounded `client_id` is mandatory (exact bytes, no
            // canonicalization).
            claims
                .get(OAUTH_CLIENT_ID_CLAIM)
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty() && c.len() <= MAX_CLIENT_ID_BYTES)
                .ok_or(VerifierError::ClaimContractRejected)?;
            // Classify the subject from the authenticated marker claim. A value
            // matching neither set (or the claim absent) is ambiguous and
            // denies; a client-subject token denies unless the issuer recorded
            // the non-collision guarantee.
            let marker = claims
                .get(subject_class.marker_claim())
                .and_then(Value::as_str);
            match subject_class.classify(marker) {
                Some(SubjectClass::ResourceOwner) => Ok(()),
                Some(SubjectClass::ClientSubject) => match subject_class.posture() {
                    ClientSubjectPosture::AcceptNonColliding => Ok(()),
                    ClientSubjectPosture::Reject => Err(VerifierError::ClaimContractRejected),
                },
                None => Err(VerifierError::ClaimContractRejected),
            }
        }
        TokenClass::DedicatedNipFi => Ok(()),
    }
}

/// Parse the fixed `nostr_pubkey` claim: lowercase hex of exactly one 32-byte
/// key. Bech32 and other aliases deny. Absence denies; the merged NIP-FI
/// spec v2 (PR #7214) requires the `nostr_pubkey` claim unconditionally.
fn parse_nostr_pubkey_claim(
    claims: &Map<String, Value>,
) -> Result<Option<PublicKey>, VerifierError> {
    match claims.get(NOSTR_PUBKEY_CLAIM) {
        None => Err(VerifierError::ClaimRejected),
        Some(value) => {
            let raw = value.as_str().ok_or(VerifierError::ClaimRejected)?;
            if raw.len() != 64
                || !raw
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(VerifierError::ClaimRejected);
            }
            let key = PublicKey::from_hex(raw).map_err(|_| VerifierError::ClaimRejected)?;
            Ok(Some(key))
        }
    }
}

/// Capture only the claim names the policy reads into a canonical set. The
/// closed set is the `scope` claim, split on ASCII space; unchecked claims
/// never enter the result.
fn capture_capabilities(
    _policy: &IssuerPolicy,
    claims: &Map<String, Value>,
) -> CanonicalCapabilities {
    let mut entries = Vec::new();
    if let Some(scope) = claims.get("scope").and_then(Value::as_str) {
        for token in scope.split(' ').filter(|s| !s.is_empty()) {
            entries.push(("scope".to_owned(), token.to_owned()));
        }
    }
    CanonicalCapabilities::from_pairs(entries)
}

fn select_unique_jwk<'a>(jwks: &'a JwkSet, kid: &str) -> Result<&'a Jwk, VerifierError> {
    let mut matching = jwks
        .keys
        .iter()
        .filter(|jwk| jwk.common.key_id.as_deref() == Some(kid));
    let jwk = matching.next().ok_or(VerifierError::AmbiguousKeyId)?;
    if matching.next().is_some() {
        return Err(VerifierError::AmbiguousKeyId);
    }
    Ok(jwk)
}

fn validate_jwk(jwk: &Jwk, token_algorithm: Algorithm) -> Result<(), VerifierError> {
    let usage_ok = jwk
        .common
        .public_key_use
        .as_ref()
        .is_none_or(|use_| use_ == &PublicKeyUse::Signature);
    // NIP-FI.md:166-169 rejects incompatible JWK usage. When `key_ops` is
    // present it MUST authorize `verify`; a key restricted to other operations
    // (for example `encrypt`) cannot validate an assertion signature.
    let key_ops_ok = jwk
        .common
        .key_operations
        .as_ref()
        .is_none_or(|ops| ops.contains(&KeyOperations::Verify));
    let algorithm_ok = jwk
        .common
        .key_algorithm
        .is_none_or(|alg| jwk_algorithm_matches(alg, token_algorithm));
    // NIP-FI.md:166-169 rejects algorithm/key mismatch. The optional `alg`
    // header is advisory; the key's actual material (`kty`/`crv`) is what
    // signs. Bind the selected JOSE algorithm to the required key family and
    // curve so a JWK declaring, say, `alg=ES256` over P-384 material (or any
    // cross-family/cross-curve substitution) cannot verify an ES256 token.
    if usage_ok
        && key_ops_ok
        && algorithm_ok
        && key_material_matches(&jwk.algorithm, token_algorithm)
    {
        Ok(())
    } else {
        Err(VerifierError::InvalidKey)
    }
}

/// Bind a JOSE signature algorithm to the JWK key family and curve it requires.
/// Every algorithm the policy can accept (`is_asymmetric_algorithm`) has an
/// exact key-material shape; anything else denies.
fn key_material_matches(params: &AlgorithmParameters, token: Algorithm) -> bool {
    match token {
        Algorithm::ES256 => is_ec_curve(params, EllipticCurve::P256),
        Algorithm::ES384 => is_ec_curve(params, EllipticCurve::P384),
        Algorithm::EdDSA => is_okp_curve(params, EllipticCurve::Ed25519),
        Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512 => matches!(params, AlgorithmParameters::RSA(_)),
        // Symmetric and `none` never reach key selection (rejected at header
        // parse); deny defensively rather than accept unknown material.
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => false,
    }
}

fn is_ec_curve(params: &AlgorithmParameters, curve: EllipticCurve) -> bool {
    matches!(params, AlgorithmParameters::EllipticCurve(ec) if ec.curve == curve)
}

fn is_okp_curve(params: &AlgorithmParameters, curve: EllipticCurve) -> bool {
    matches!(params, AlgorithmParameters::OctetKeyPair(okp) if okp.curve == curve)
}

fn jwk_algorithm_matches(key: KeyAlgorithm, token: Algorithm) -> bool {
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

fn claim_string(
    claims: &Map<String, Value>,
    claim: &str,
    max_len: usize,
) -> Result<String, VerifierError> {
    // Exact bytes: no trimming or canonicalization. `iss`/`sub` are identity
    // components; distinct byte strings must stay distinct.
    claims
        .get(claim)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= max_len)
        .map(str::to_owned)
        .ok_or(VerifierError::ClaimRejected)
}

fn numeric_date(claims: &Map<String, Value>, claim: &str) -> Result<DateTime<Utc>, VerifierError> {
    let value = claims.get(claim).ok_or(VerifierError::InvalidTimeBounds)?;
    parse_numeric_date(value)
}

fn optional_numeric_date(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<Option<DateTime<Utc>>, VerifierError> {
    match claims.get(claim) {
        None => Ok(None),
        Some(value) => parse_numeric_date(value).map(Some),
    }
}

/// Parse an RFC 7519 `NumericDate`: seconds since the epoch, integer *or*
/// fractional. Integers are exact; a finite fractional value (real IdPs emit
/// them) is converted with subsecond nanosecond precision. NaN, infinity, a
/// non-number, and any magnitude outside the representable `i64`-seconds range
/// deny as invalid time bounds.
fn parse_numeric_date(value: &Value) -> Result<DateTime<Utc>, VerifierError> {
    // Integer NumericDate: exact, no float round-trip.
    if let Some(secs) = value.as_i64() {
        return Utc
            .timestamp_opt(secs, 0)
            .single()
            .ok_or(VerifierError::InvalidTimeBounds);
    }
    // Fractional NumericDate. `as_f64` yields `None` for a non-number, so a
    // string or object `exp`/`iat`/`nbf` denies here.
    let seconds = value.as_f64().ok_or(VerifierError::InvalidTimeBounds)?;
    if !seconds.is_finite() {
        return Err(VerifierError::InvalidTimeBounds);
    }
    let whole = seconds.floor();
    // Guard the `i64` cast: reject magnitudes at or beyond the representable
    // range before casting (an out-of-range `as` cast would saturate silently).
    if whole < i64::MIN as f64 || whole >= i64::MAX as f64 {
        return Err(VerifierError::InvalidTimeBounds);
    }
    let mut secs = whole as i64;
    // `seconds - whole` is in `[0, 1)`; rounding can reach 1e9, so carry it.
    let mut nanos = ((seconds - whole) * 1_000_000_000.0).round() as u32;
    if nanos >= 1_000_000_000 {
        secs = secs
            .checked_add(1)
            .ok_or(VerifierError::InvalidTimeBounds)?;
        nanos -= 1_000_000_000;
    }
    Utc.timestamp_opt(secs, nanos)
        .single()
        .ok_or(VerifierError::InvalidTimeBounds)
}

fn seconds(value: u64) -> chrono::Duration {
    chrono::Duration::seconds(value as i64)
}

fn checked_add(at: DateTime<Utc>, delta: chrono::Duration) -> Result<DateTime<Utc>, VerifierError> {
    at.checked_add_signed(delta)
        .ok_or(VerifierError::InvalidTimeBounds)
}

/// Parse the claims segment as a JSON object, rejecting any duplicate member.
fn parse_unique_claims(token: &str) -> Result<Map<String, Value>, VerifierError> {
    let segment = token
        .split('.')
        .nth(1)
        .filter(|s| !s.is_empty())
        .ok_or(VerifierError::MalformedToken)?;
    let bytes = base64url_decode(segment)?;
    parse_unique_object(&bytes)
}

/// Deserialize a JSON object, denying a repeated key. `serde_json`'s default
/// `Map` deserialization is last-wins, which would let a duplicate `alg`,
/// `typ`, `iss`, `sub`, or time member be interpreted differently than a
/// verifier that reads the first occurrence — a parser-differential ambiguity
/// (NIP-FI.md, "rejects ambiguous protected-header or claim members"). This
/// visitor rejects the second occurrence outright.
fn parse_unique_object(bytes: &[u8]) -> Result<Map<String, Value>, VerifierError> {
    struct UniqueObject;

    impl<'de> Visitor<'de> for UniqueObject {
        type Value = Map<String, Value>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a JSON object with unique member names")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut map = Map::new();
            let mut seen = BTreeSet::new();
            while let Some(key) = access.next_key::<String>()? {
                if !seen.insert(key.clone()) {
                    return Err(A::Error::custom("duplicate member"));
                }
                let value = access.next_value::<Value>()?;
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    let mut de = serde_json::Deserializer::from_slice(bytes);
    let map = de
        .deserialize_map(UniqueObject)
        .map_err(|e| classify_json_error(&e))?;
    // Reject trailing bytes after the object (a second concatenated document).
    de.end().map_err(|_| VerifierError::MalformedToken)?;
    Ok(map)
}

/// A duplicate-member custom error maps to [`VerifierError::DuplicateMember`];
/// every other parse failure is a malformed token.
fn classify_json_error(error: &serde_json::Error) -> VerifierError {
    if error.to_string().contains("duplicate member") {
        VerifierError::DuplicateMember
    } else {
        VerifierError::MalformedToken
    }
}

fn base64url_decode(segment: &str) -> Result<Vec<u8>, VerifierError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| VerifierError::MalformedToken)
}

#[cfg(test)]
mod tests;
