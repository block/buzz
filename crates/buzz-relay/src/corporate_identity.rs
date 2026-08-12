//! Corporate identity verification and uid/pubkey binding.
//!
//! This module is intentionally relay-local. `buzz-auth` remains the generic
//! Nostr proof layer; corporate identity is deployment policy layered after a
//! request proves control of a Nostr key.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
};
use base64::Engine as _;
#[cfg(test)]
use jsonwebtoken::jwk::{KeyAlgorithm, KeyOperations, PublicKeyUse};
use jsonwebtoken::{
    decode_header,
    jwk::{Jwk, JwkSet},
    Algorithm,
};
#[cfg(test)]
use nostr::{Event, EventBuilder, Kind, Tag};
use nostr::{FromBech32, PublicKey, Timestamp};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use buzz_auth::{
    CanonicalFederatedAssertionVerifier, CanonicalVerifierError, CanonicalVerifierKeySet,
    CanonicalVerifierPolicy, ProofTransport, VerifierKeyGeneration, VerifierPolicyStamp,
};
#[cfg(test)]
use buzz_core::kind::KIND_USER_TRUSTED_ASSERTION;
use buzz_core::CommunityId;
use buzz_db::identity_binding::{BindIdentityResult, SOURCE_DB_BINDING, SOURCE_JWT_NPUB};

use crate::config::{CorporateIdentityAuthPrecedence, CorporateIdentityConfig};
use crate::state::AppState;

const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);
const JWKS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const JWKS_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const JWKS_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
// Permit a bounded issuer/relay clock difference while keeping expiry enforcement explicit.
const JWT_CLOCK_SKEW_LEEWAY_SECS: u64 = 60;
const IDENTITY_ASSERTION_MAX_TTL_SECS: u64 = 60 * 60;
const IDENTITY_SESSION_REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);

fn unique_jwk_for_kid<'a>(
    set: &'a JwkSet,
    kid: &str,
) -> Result<Option<&'a Jwk>, CorporateIdentityError> {
    let mut matches = set
        .keys
        .iter()
        .filter(|jwk| jwk.common.key_id.as_deref() == Some(kid));
    let first = matches.next();
    if matches.next().is_some() {
        return Err(CorporateIdentityError::Jwks(
            "ambiguous duplicate key identifier".to_owned(),
        ));
    }
    Ok(first)
}

fn coordinate_digest(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

fn corporate_identity_coordinates(
    authorization_domain: CommunityId,
    signer: PublicKey,
    token: &str,
    auth_tag_json: &str,
) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let community = authorization_domain.as_uuid();
    let community_bytes = community.as_bytes();
    let signer_bytes = signer.as_bytes();
    let target = coordinate_digest(
        b"buzz.nip-fi.corporate-identity.target.v1\0",
        &[community_bytes, signer_bytes],
    );
    let request = coordinate_digest(
        b"buzz.nip-fi.corporate-identity.request.v1\0",
        &[
            community_bytes,
            signer_bytes,
            token.as_bytes(),
            auth_tag_json.as_bytes(),
        ],
    );
    let transport = coordinate_digest(
        b"buzz.nip-fi.corporate-identity.transport.v1\0",
        &[community_bytes, signer_bytes, b"relay-auth"],
    );
    (request, target, transport)
}

#[derive(Debug, Clone)]
struct CachedJwks {
    set: JwkSet,
    generation: VerifierKeyGeneration,
    expires_at: Instant,
}

/// Validated corporate identity claims used by Buzz.
#[derive(Clone, PartialEq, Eq)]
pub struct CorporateJwtClaims {
    /// Validated identity-provider issuer.
    pub issuer: String,
    /// Stable corporate uid claim.
    pub uid: String,
    /// Human-readable verified identity claim.
    pub display_name: String,
    /// Optional pubkey carried by the IdP.
    pub pubkey: Option<PublicKey>,
    /// JWT expiration as a Unix timestamp.
    pub expires_at: u64,
    verifier_stamp: VerifierPolicyStamp,
    verified_assertion: buzz_auth::VerifiedFederatedAssertion,
}

impl std::fmt::Debug for CorporateJwtClaims {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CorporateJwtClaims([REDACTED])")
    }
}

/// Service that verifies corporate identity JWTs against configured JWKS.
#[derive(Debug)]
pub struct CorporateIdentityService {
    config: CorporateIdentityConfig,
    verifier: Result<CanonicalFederatedAssertionVerifier, CanonicalVerifierError>,
    http: Result<reqwest::Client, String>,
    jwks: RwLock<Option<CachedJwks>>,
    refresh: Mutex<()>,
}

impl CorporateIdentityService {
    /// Build a corporate identity verifier from relay config.
    pub fn new(config: CorporateIdentityConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(JWKS_CONNECT_TIMEOUT)
            .timeout(JWKS_REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string());
        let verifier = CanonicalVerifierPolicy::new(
            config.issuer.clone(),
            config.audience.clone(),
            config.uid_claim.clone(),
            config.npub_claim.clone(),
            JWT_CLOCK_SKEW_LEEWAY_SECS,
            IDENTITY_ASSERTION_MAX_TTL_SECS,
        )
        .map(CanonicalFederatedAssertionVerifier::new);
        Self {
            config,
            verifier,
            http,
            jwks: RwLock::new(None),
            refresh: Mutex::new(()),
        }
    }

    /// Verify one assertion against exact provider-free route coordinates.
    pub(crate) async fn verify_route_assertion(
        &self,
        token: &str,
        authorization_domain: CommunityId,
        transport: ProofTransport,
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
    ) -> Result<buzz_auth::VerifiedFederatedAssertion, CorporateIdentityError> {
        let header = decode_header(token)
            .map_err(|e| CorporateIdentityError::InvalidJwt(format!("invalid JWT header: {e}")))?;
        if !is_allowed_jwt_algorithm(header.alg) {
            return Err(CorporateIdentityError::InvalidJwt(format!(
                "unsupported JWT algorithm: {:?}",
                header.alg
            )));
        }
        let kid = header
            .kid
            .as_deref()
            .ok_or(CorporateIdentityError::MissingKid)?;
        let (jwk, generation) = self.jwk_snapshot_for_kid(kid).await?;
        let key_set = CanonicalVerifierKeySet::new(generation, JwkSet { keys: vec![jwk] });
        self.verifier
            .as_ref()
            .map_err(|error| CorporateIdentityError::InvalidJwt(error.code().to_owned()))?
            .verify(
                token,
                &key_set,
                authorization_domain,
                transport,
                target_fingerprint,
                request_fingerprint,
                transport_context_fingerprint,
            )
            .map_err(|error| CorporateIdentityError::InvalidJwt(error.code().to_owned()))
    }

    /// Validate a JWT and extract the configured corporate identity claims.
    pub async fn validate_jwt(
        &self,
        token: &str,
        authorization_domain: CommunityId,
        signer: PublicKey,
        auth_tag_json: Option<&str>,
    ) -> Result<CorporateJwtClaims, CorporateIdentityError> {
        let header = decode_header(token)
            .map_err(|e| CorporateIdentityError::InvalidJwt(format!("invalid JWT header: {e}")))?;
        if !is_allowed_jwt_algorithm(header.alg) {
            return Err(CorporateIdentityError::InvalidJwt(format!(
                "unsupported JWT algorithm: {:?}",
                header.alg
            )));
        }
        let kid = header
            .kid
            .as_deref()
            .ok_or(CorporateIdentityError::MissingKid)?;
        let (jwk, generation) = self.jwk_snapshot_for_kid(kid).await?;
        let key_set = CanonicalVerifierKeySet::new(generation, JwkSet { keys: vec![jwk] });
        let (request_fingerprint, target_fingerprint, transport_fingerprint) =
            corporate_identity_coordinates(
                authorization_domain,
                signer,
                token,
                auth_tag_json.unwrap_or_default(),
            );
        let verifier = self
            .verifier
            .as_ref()
            .map_err(|error| CorporateIdentityError::InvalidJwt(error.code().to_owned()))?;
        let assertion = verifier
            .verify(
                token,
                &key_set,
                authorization_domain,
                ProofTransport::Nip42,
                target_fingerprint,
                request_fingerprint,
                transport_fingerprint,
            )
            .map_err(|error| CorporateIdentityError::InvalidJwt(error.code().to_owned()))?;

        // Projection is deliberately non-authoritative: the canonical verifier
        // above is the only signature/claim authority. This parser can only be
        // reached with its origin-sealed evidence and must agree with the
        // verifier-sealed issuer/subject storage key.
        let claims = projected_verified_claims(token)?;
        let issuer = claim_string(&claims, "iss")?;
        let uid = claim_string(&claims, &self.config.uid_claim)?;
        let storage_key = assertion.principal_storage_key();
        if storage_key.issuer() != issuer || storage_key.subject() != uid {
            return Err(CorporateIdentityError::InvalidJwt(
                "canonical subject projection mismatch".into(),
            ));
        }
        let display_name = claim_string(&claims, &self.config.display_claim)?;
        let pubkey = configured_pubkey_claim(&claims, self.config.npub_claim.as_deref())?;
        let expires_at = claim_u64(&claims, "exp")?;

        Ok(CorporateJwtClaims {
            issuer,
            uid,
            display_name,
            pubkey,
            expires_at,
            verifier_stamp: assertion.verifier_stamp(),
            verified_assertion: assertion,
        })
    }

    async fn accepts_final_verifier_stamp(&self, stamp: VerifierPolicyStamp) -> bool {
        let Ok(verifier) = self.verifier.as_ref() else {
            return false;
        };
        if verifier.policy_id() != stamp.policy_id() {
            return false;
        }
        self.jwks.read().await.as_ref().is_some_and(|cached| {
            cached.generation == stamp.key_generation() && cached.expires_at > Instant::now()
        })
    }

    async fn jwk_snapshot_for_kid(
        &self,
        kid: &str,
    ) -> Result<(Jwk, VerifierKeyGeneration), CorporateIdentityError> {
        let now = Instant::now();
        let observed_generation;
        {
            let cache = self.jwks.read().await;
            observed_generation = cache.as_ref().map(|cached| cached.generation);
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > now {
                    if let Some(jwk) = unique_jwk_for_kid(&cached.set, kid)? {
                        return Ok((jwk.clone(), cached.generation));
                    }
                    // An unknown key in an otherwise fresh cache forces one
                    // single-flight refresh to bound key-rotation denial.
                }
            }
        }

        // Only one request may refresh at a time. Re-check after acquiring the
        // mutex because another waiter may already have populated the cache.
        let _refresh = self.refresh.lock().await;
        let now = Instant::now();
        {
            let cache = self.jwks.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > now {
                    if let Some(jwk) = unique_jwk_for_kid(&cached.set, kid)? {
                        return Ok((jwk.clone(), cached.generation));
                    }
                    if observed_generation.is_some()
                        && observed_generation != Some(cached.generation)
                    {
                        return Err(CorporateIdentityError::Jwks(
                            "kid not found after JWKS refresh".to_owned(),
                        ));
                    }
                }
            }
        }

        let set = self.fetch_jwks().await?;
        let jwk = unique_jwk_for_kid(&set, kid)?.cloned();
        let generation = self
            .jwks
            .read()
            .await
            .as_ref()
            .map(|cached| cached.generation.get().saturating_add(1))
            .unwrap_or(1);
        let generation = VerifierKeyGeneration::new(generation)
            .ok_or_else(|| CorporateIdentityError::Jwks("key generation exhausted".to_owned()))?;
        *self.jwks.write().await = Some(CachedJwks {
            set,
            generation,
            expires_at: Instant::now() + JWKS_CACHE_TTL,
        });
        jwk.map(|jwk| (jwk, generation))
            .ok_or_else(|| CorporateIdentityError::Jwks("kid not found after JWKS refresh".into()))
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, CorporateIdentityError> {
        let client = self
            .http
            .as_ref()
            .map_err(|error| CorporateIdentityError::Jwks(error.clone()))?;
        let mut response = client
            .get(&self.config.jwks_uri)
            .send()
            .await
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))?
            .error_for_status()
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))?;
        if response
            .content_length()
            .is_some_and(|length| length > JWKS_MAX_RESPONSE_BYTES as u64)
        {
            return Err(CorporateIdentityError::Jwks(
                "JWKS response exceeds size limit".to_string(),
            ));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))?
        {
            if body.len().saturating_add(chunk.len()) > JWKS_MAX_RESPONSE_BYTES {
                return Err(CorporateIdentityError::Jwks(
                    "JWKS response exceeds size limit".to_string(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<JwkSet>(&body)
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))
    }
}

impl buzz_db::authorization_admission::AdmissionVerifierRechecker for CorporateIdentityService {
    fn recheck<'a>(
        &'a self,
        expected: VerifierPolicyStamp,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<(), buzz_db::authorization_admission::AdmissionCommitError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if self.accepts_final_verifier_stamp(expected).await {
                Ok(())
            } else {
                Err(buzz_db::authorization_admission::AdmissionCommitError::AuthorizationDenied)
            }
        })
    }
}

impl crate::state::InviteAssertionVerifier for CorporateIdentityService {
    fn verify<'a>(
        &'a self,
        token: &'a str,
        authorization_domain: CommunityId,
        transport: ProofTransport,
        target_fingerprint: [u8; 32],
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        buzz_auth::VerifiedFederatedAssertion,
                        crate::state::InviteAssertionError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.verify_route_assertion(
                token,
                authorization_domain,
                transport,
                target_fingerprint,
                request_fingerprint,
                transport_context_fingerprint,
            )
            .await
            .map_err(|error| match error {
                CorporateIdentityError::Jwks(_)
                | CorporateIdentityError::Db(_)
                | CorporateIdentityError::FoundationIntegrationRequired => {
                    crate::state::InviteAssertionError::Unavailable
                }
                _ => crate::state::InviteAssertionError::Denied,
            })
        })
    }
}

fn projected_verified_claims(token: &str) -> Result<Map<String, Value>, CorporateIdentityError> {
    let mut segments = token.split('.');
    let _header = segments.next();
    let payload = segments
        .next()
        .filter(|payload| !payload.is_empty())
        .ok_or_else(|| CorporateIdentityError::InvalidJwt("malformed JWT payload".into()))?;
    if segments.next().is_none() || segments.next().is_some() {
        return Err(CorporateIdentityError::InvalidJwt(
            "malformed JWT segments".into(),
        ));
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|_| CorporateIdentityError::InvalidJwt("invalid JWT payload encoding".into()))?;
    serde_json::from_slice::<CanonicalRawProjection>(&bytes)
        .map(|projection| projection.claims)
        .map_err(|_| CorporateIdentityError::InvalidJwt("invalid JWT payload JSON".into()))
}

#[derive(Deserialize)]
struct CanonicalRawProjection {
    #[serde(flatten)]
    claims: Map<String, Value>,
}

/// Read-only result of cryptographically validating corporate identity.
///
/// Callers must complete admission/authorization before passing this proof to
/// [`finalize_corporate_identity`]. This ordering prevents rejected requests
/// from creating identity bindings or public assertions.
#[derive(Clone, PartialEq, Eq)]
pub enum CorporateIdentityProof {
    /// Corporate identity is disabled for this relay.
    NotRequired,
    /// A JWT was validated, but no binding mutation has occurred yet.
    Direct {
        /// Validated claims staged for post-authorization binding.
        claims: Box<CorporateJwtClaims>,
        /// Binding source selected from the configured npub policy.
        source: &'static str,
    },
    /// A NIP-OA owner with an active binding authorized this agent.
    Delegated {
        /// Bound owner pubkey.
        owner_pubkey: PublicKey,
        /// Expected issuer of the owner's active binding.
        owner_issuer: String,
        /// Expected uid of the owner's active binding.
        owner_uid: String,
    },
}

impl std::fmt::Debug for CorporateIdentityProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self {
            Self::NotRequired => "NotRequired",
            Self::Direct { .. } => "Direct([REDACTED])",
            Self::Delegated { .. } => "Delegated([REDACTED])",
        };
        formatter.write_str(variant)
    }
}

/// Borrow staged direct-identity data for an atomic admission transaction.
pub fn binding_input_for_proof<'a>(
    proof: &'a CorporateIdentityProof,
    signer: &'a PublicKey,
) -> Option<buzz_db::identity_binding::IdentityBindingInput<'a>> {
    match proof {
        CorporateIdentityProof::Direct { claims, source } => {
            Some(buzz_db::identity_binding::IdentityBindingInput {
                issuer: &claims.issuer,
                uid: &claims.uid,
                pubkey: signer.as_bytes(),
                display_name: Some(&claims.display_name),
                source,
            })
        }
        CorporateIdentityProof::NotRequired | CorporateIdentityProof::Delegated { .. } => None,
    }
}

/// Whether this proof relies on a delegated owner rather than a direct JWT.
pub fn proof_is_delegated(proof: &CorporateIdentityProof) -> bool {
    matches!(proof, CorporateIdentityProof::Delegated { .. })
}

/// Outcome of corporate identity enforcement.
#[derive(Clone, PartialEq, Eq)]
pub enum CorporateIdentityDecision {
    /// Corporate identity is disabled for this relay.
    NotRequired,
    /// The signer authenticated directly with a corporate identity JWT.
    Direct {
        /// Validated identity-provider issuer.
        issuer: String,
        /// Stable corporate uid claim.
        uid: String,
        /// Verified display claim.
        display_name: String,
        /// JWT expiration used to bound long-lived sessions.
        expires_at: u64,
        /// Binding operation outcome.
        binding: BindIdentityResult,
    },
    /// The signer is an agent admitted through a bound owner pubkey.
    Delegated {
        /// NIP-OA owner pubkey that already has an active corporate binding.
        owner_pubkey: PublicKey,
        /// Expected issuer of the owner's active binding.
        owner_issuer: String,
        /// Expected uid of the owner's active binding.
        owner_uid: String,
    },
}

impl std::fmt::Debug for CorporateIdentityDecision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self {
            Self::NotRequired => "NotRequired",
            Self::Direct { .. } => "Direct([REDACTED])",
            Self::Delegated { .. } => "Delegated([REDACTED])",
        };
        formatter.write_str(variant)
    }
}

struct SessionRevalidationPlan {
    binding_pubkey: PublicKey,
    expected_issuer: String,
    expected_uid: String,
    expires_at: Option<u64>,
}

fn session_revalidation_plan(
    signer: PublicKey,
    decision: CorporateIdentityDecision,
) -> Option<SessionRevalidationPlan> {
    match decision {
        CorporateIdentityDecision::NotRequired => None,
        CorporateIdentityDecision::Direct {
            issuer,
            uid,
            expires_at,
            ..
        } => Some(SessionRevalidationPlan {
            binding_pubkey: signer,
            expected_issuer: issuer,
            expected_uid: uid,
            expires_at: Some(expires_at),
        }),
        CorporateIdentityDecision::Delegated {
            owner_pubkey,
            owner_issuer,
            owner_uid,
        } => Some(SessionRevalidationPlan {
            binding_pubkey: owner_pubkey,
            expected_issuer: owner_issuer,
            expected_uid: owner_uid,
            expires_at: None,
        }),
    }
}

async fn cancel_session_at_expiry(
    expires_at: u64,
    now_secs: u64,
    cancel: tokio_util::sync::CancellationToken,
) {
    let delay = Duration::from_secs(expires_at.saturating_sub(now_secs));
    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = tokio::time::sleep(delay) => cancel.cancel(),
    }
}

async fn run_session_binding_revalidation<F, Fut, E>(
    interval: Duration,
    signer: PublicKey,
    binding_pubkey: PublicKey,
    expected_issuer: String,
    expected_uid: String,
    cancel: tokio_util::sync::CancellationToken,
    mut lookup: F,
) where
    F: FnMut() -> Fut,
    Fut:
        std::future::Future<Output = Result<Option<buzz_db::identity_binding::IdentityBinding>, E>>,
    E: std::fmt::Display,
{
    let mut interval = tokio::time::interval(interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = interval.tick() => {
                match lookup().await {
                    Ok(Some(binding))
                        if binding.issuer == expected_issuer && binding.uid == expected_uid => {}
                    Ok(Some(_)) | Ok(None) => {
                        warn!(
                            signer = %signer.to_hex(),
                            binding_pubkey = %binding_pubkey.to_hex(),
                            "corporate identity session evicted after binding revocation"
                        );
                        cancel.cancel();
                        return;
                    }
                    Err(error) => {
                        warn!(
                            signer = %signer.to_hex(),
                            error = %error,
                            "corporate identity session revalidation failed closed"
                        );
                        cancel.cancel();
                        return;
                    }
                }
            }
        }
    }
}

/// Revalidate a long-lived corporate identity session until it closes.
///
/// Direct sessions are cancelled at JWT expiry and when their binding stops
/// being active. Delegated sessions re-check the owner binding, so revoking an
/// owner also evicts every agent session within one bounded interval.
pub fn spawn_session_revalidation(
    state: Arc<AppState>,
    community_id: CommunityId,
    signer: PublicKey,
    decision: CorporateIdentityDecision,
    cancel: tokio_util::sync::CancellationToken,
) {
    let Some(plan) = session_revalidation_plan(signer, decision) else {
        return;
    };
    let SessionRevalidationPlan {
        binding_pubkey,
        expected_issuer,
        expected_uid,
        expires_at,
    } = plan;

    if let Some(expires_at) = expires_at {
        let expiry_cancel = cancel.clone();
        tokio::spawn(async move {
            cancel_session_at_expiry(expires_at, Timestamp::now().as_secs(), expiry_cancel).await;
        });
    }

    let lookup_state = Arc::clone(&state);
    let lookup_pubkey = binding_pubkey;
    tokio::spawn(run_session_binding_revalidation(
        IDENTITY_SESSION_REVALIDATION_INTERVAL,
        signer,
        binding_pubkey,
        expected_issuer,
        expected_uid,
        cancel,
        move || {
            let state = Arc::clone(&lookup_state);
            async move {
                state
                    .db
                    .get_active_identity_binding_by_pubkey(community_id, lookup_pubkey.as_bytes())
                    .await
            }
        },
    ));
}

/// Errors produced by corporate identity verification.
#[derive(Error)]
pub enum CorporateIdentityError {
    /// No JWT was available and delegation did not apply.
    #[error("corporate identity JWT missing")]
    MissingJwt,
    /// JWT header did not include a `kid`.
    #[error("corporate identity JWT missing kid")]
    MissingKid,
    /// JWT signature or claims failed validation.
    #[error("invalid corporate identity JWT: {0}")]
    InvalidJwt(String),
    /// JWKS fetch or lookup failed.
    #[error("corporate identity JWKS unavailable: {0}")]
    Jwks(String),
    /// A configured claim is missing or not a string.
    #[error("invalid corporate identity claim {claim}: {reason}")]
    InvalidClaim {
        /// Claim name.
        claim: String,
        /// Validation reason.
        reason: String,
    },
    /// The IdP-provided pubkey does not match the authenticated signer.
    #[error("corporate identity npub claim does not match authenticated signer")]
    NpubMismatch,
    /// The requested uid/pubkey binding conflicts with an active binding.
    #[error("corporate identity binding conflict")]
    BindingConflict,
    /// The requested uid/pubkey binding was previously revoked.
    #[error("corporate identity binding revoked")]
    BindingRevoked,
    /// NIP-OA delegation was present but did not satisfy corporate identity.
    #[error("corporate identity delegation denied")]
    DelegationDenied,
    /// Database operation failed.
    #[error("corporate identity database error: {0}")]
    Db(#[from] buzz_db::DbError),
    /// Root callers have not yet adopted the direct-final resolver/finalizer.
    #[error("corporate identity direct-final integration required")]
    FoundationIntegrationRequired,
}

impl std::fmt::Debug for CorporateIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CorporateIdentityError([REDACTED])")
    }
}

impl CorporateIdentityError {
    /// HTTP status appropriate for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::MissingJwt | Self::MissingKid | Self::InvalidJwt(_) | Self::Jwks(_) => {
                StatusCode::UNAUTHORIZED
            }
            Self::InvalidClaim { .. }
            | Self::NpubMismatch
            | Self::BindingConflict
            | Self::BindingRevoked
            | Self::DelegationDenied => StatusCode::FORBIDDEN,
            Self::Db(_) | Self::FoundationIntegrationRequired => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Sanitized message safe to return to clients.
    pub fn public_message(&self) -> &'static str {
        match self {
            Self::MissingJwt => "relay-verified identity required",
            Self::MissingKid | Self::InvalidJwt(_) | Self::Jwks(_) => {
                "relay identity verification failed"
            }
            Self::InvalidClaim { .. } => "relay identity claim invalid",
            Self::NpubMismatch => "relay identity pubkey mismatch",
            Self::BindingConflict => "relay identity binding conflict",
            Self::BindingRevoked => "relay identity binding revoked",
            Self::DelegationDenied => "relay identity delegation denied",
            Self::Db(_) | Self::FoundationIntegrationRequired => "relay identity unavailable",
        }
    }

    /// Convert to the standard API error shape.
    pub fn into_api_error(self) -> (StatusCode, Json<Value>) {
        let status = self.status_code();
        let message = self.public_message();
        if status.is_server_error() {
            warn!(error = %self, "corporate identity enforcement failed");
        }
        (status, Json(serde_json::json!({ "error": message })))
    }
}

/// Extract a corporate identity JWT from the configured request header.
pub fn identity_jwt_from_headers(
    headers: &HeaderMap,
    config: &CorporateIdentityConfig,
) -> Option<String> {
    headers
        .get(config.jwt_header.as_str())
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .and_then(|raw| {
            raw.strip_prefix("Bearer ")
                .unwrap_or(raw)
                .trim()
                .split(',')
                .next()
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Validate corporate identity without creating bindings or assertions.
pub async fn verify_corporate_identity(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    identity_jwt: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityProof, CorporateIdentityError> {
    let result =
        verify_corporate_identity_inner(state, community_id, signer, identity_jwt, auth_tag_json)
            .await;
    if let Err(error) = &result {
        record_corporate_identity_denial(error);
    }
    result
}

async fn verify_corporate_identity_inner(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    identity_jwt: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityProof, CorporateIdentityError> {
    let Some(service) = state.corporate_identity.as_ref() else {
        return Ok(CorporateIdentityProof::NotRequired);
    };

    // Requests can carry both a direct identity JWT and a cryptographically
    // verified NIP-OA owner declaration. The deployment selects which identity
    // source wins; the provider-neutral default treats the JWT as the signer's
    // identity. Delegated precedence supports identity-aware gateways that
    // attach an owner's token to requests made by that owner's agents.
    if select_identity_auth_path(&service.config, identity_jwt, auth_tag_json)
        == IdentityAuthPath::Delegated
    {
        return verify_delegated_corporate_identity(
            &state.db,
            &service.config,
            community_id,
            signer,
            auth_tag_json,
        )
        .await;
    }

    if let Some(token) = identity_jwt {
        let claims = service
            .validate_jwt(token, community_id, signer, auth_tag_json)
            .await?;
        let source = binding_source_for_signer(claims.pubkey, signer)?;
        return Ok(CorporateIdentityProof::Direct {
            claims: Box::new(claims),
            source,
        });
    }

    verify_delegated_corporate_identity(
        &state.db,
        &service.config,
        community_id,
        signer,
        auth_tag_json,
    )
    .await
}

/// Commit a previously validated proof after request authorization succeeds.
pub async fn finalize_corporate_identity(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    proof: CorporateIdentityProof,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    let result = finalize_corporate_identity_inner(state, community_id, signer, proof).await;
    if let Err(error) = &result {
        record_corporate_identity_denial(error);
    }
    result
}

/// Complete metrics/assertion/audit work for an identity result produced by an
/// atomic admission transaction. Rejected results were rolled back, but still
/// need the same denial audit as the ordinary finalization path.
pub async fn finalize_atomic_corporate_identity_result(
    state: &AppState,
    community_id: CommunityId,
    _signer: PublicKey,
    proof: CorporateIdentityProof,
    committed_binding: Option<BindIdentityResult>,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    let result = match proof {
        CorporateIdentityProof::NotRequired => Ok(CorporateIdentityDecision::NotRequired),
        CorporateIdentityProof::Delegated {
            owner_pubkey,
            owner_issuer,
            owner_uid,
        } => Ok(CorporateIdentityDecision::Delegated {
            owner_pubkey,
            owner_issuer,
            owner_uid,
        }),
        CorporateIdentityProof::Direct { claims, source } => {
            let _ = (source, committed_binding);
            require_final_verifier_stamp(state, community_id, &claims).await?;
            Err(CorporateIdentityError::FoundationIntegrationRequired)
        }
    };
    if let Err(error) = &result {
        record_corporate_identity_denial(error);
    }
    result
}

async fn finalize_corporate_identity_inner(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    proof: CorporateIdentityProof,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    match proof {
        CorporateIdentityProof::NotRequired => Ok(CorporateIdentityDecision::NotRequired),
        CorporateIdentityProof::Delegated {
            owner_pubkey,
            owner_issuer,
            owner_uid,
        } => Ok(CorporateIdentityDecision::Delegated {
            owner_pubkey,
            owner_issuer,
            owner_uid,
        }),
        CorporateIdentityProof::Direct { claims, source } => {
            let _ = (signer, source);
            require_final_verifier_stamp(state, community_id, &claims).await?;
            Err(CorporateIdentityError::FoundationIntegrationRequired)
        }
    }
}

async fn require_final_verifier_stamp(
    state: &AppState,
    community_id: CommunityId,
    claims: &CorporateJwtClaims,
) -> Result<(), CorporateIdentityError> {
    if claims.verified_assertion.authorization_domain() != community_id
        || claims.verified_assertion.verifier_stamp() != claims.verifier_stamp
        || Timestamp::now().as_secs() >= claims.expires_at
    {
        return Err(CorporateIdentityError::InvalidJwt(
            "canonical evidence expired or changed before finalization".to_owned(),
        ));
    }
    let current = match state.corporate_identity.as_ref() {
        Some(service) => {
            service
                .accepts_final_verifier_stamp(claims.verifier_stamp)
                .await
        }
        None => false,
    };
    if current {
        Ok(())
    } else {
        Err(CorporateIdentityError::InvalidJwt(
            "canonical verifier generation changed before finalization".to_owned(),
        ))
    }
}

#[cfg(test)]
fn build_identity_assertion(
    relay_keypair: &nostr::Keys,
    subject: PublicKey,
    display_name: Option<&str>,
    expires_at: u64,
    created_at: Timestamp,
) -> Result<Event, String> {
    let subject = subject.to_hex();
    let active = if display_name.is_some() {
        "true"
    } else {
        "false"
    };
    let expires_at = expires_at.to_string();
    let mut tags = vec![
        Tag::parse(["d", subject.as_str()]),
        Tag::parse(["p", subject.as_str()]),
        Tag::parse(["verified", "relay"]),
        Tag::parse(["active", active]),
        Tag::parse(["expiration", expires_at.as_str()]),
    ];
    if let Some(display_name) = display_name {
        tags.push(Tag::parse(["display_name", display_name]));
    }
    let tags = tags
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid corporate identity assertion tag: {error}"))?;

    EventBuilder::new(Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
        .tags(tags)
        .custom_created_at(created_at)
        .sign_with_keys(relay_keypair)
        .map_err(|error| format!("failed to sign corporate identity assertion: {error}"))
}

#[cfg(test)]
fn identity_assertion_matches(
    event: &Event,
    subject: &str,
    display_name: Option<&str>,
    expires_at: u64,
) -> bool {
    let has_tag = |name: &str, value: &str| {
        event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == name && parts[1] == value
        })
    };
    has_tag("d", subject)
        && has_tag("p", subject)
        && has_tag("verified", "relay")
        && has_tag(
            "active",
            if display_name.is_some() {
                "true"
            } else {
                "false"
            },
        )
        && has_tag("expiration", &expires_at.to_string())
        && display_name.is_none_or(|name| has_tag("display_name", name))
}

#[cfg(test)]
fn identity_assertion_expiration(display_name: Option<&str>, jwt_expires_at: u64, now: u64) -> u64 {
    if display_name.is_some() {
        jwt_expires_at.min(now.saturating_add(IDENTITY_ASSERTION_MAX_TTL_SECS))
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityAuthPath {
    Direct,
    Delegated,
}

fn select_identity_auth_path(
    config: &CorporateIdentityConfig,
    identity_jwt: Option<&str>,
    auth_tag_json: Option<&str>,
) -> IdentityAuthPath {
    match (identity_jwt.is_some(), auth_tag_json.is_some()) {
        (true, true) => match config.auth_precedence {
            CorporateIdentityAuthPrecedence::Direct => IdentityAuthPath::Direct,
            CorporateIdentityAuthPrecedence::Delegated => IdentityAuthPath::Delegated,
        },
        (true, false) => IdentityAuthPath::Direct,
        (false, _) => IdentityAuthPath::Delegated,
    }
}

async fn verify_delegated_corporate_identity(
    db: &buzz_db::Db,
    config: &CorporateIdentityConfig,
    community_id: CommunityId,
    signer: PublicKey,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityProof, CorporateIdentityError> {
    if config.allow_delegation {
        if let Some(owner_pubkey) = extract_unconditional_nip_oa_owner(signer, auth_tag_json) {
            let owner_binding = db
                .get_active_identity_binding_by_pubkey(community_id, owner_pubkey.as_bytes())
                .await?;
            if let Some(owner_binding) = owner_binding {
                debug!(
                    agent = %signer.to_hex(),
                    owner = %owner_pubkey.to_hex(),
                    "corporate identity granted via NIP-OA owner binding"
                );
                return Ok(CorporateIdentityProof::Delegated {
                    owner_pubkey,
                    owner_issuer: owner_binding.issuer,
                    owner_uid: owner_binding.uid,
                });
            }
        }
    }
    if auth_tag_json.is_some() {
        Err(CorporateIdentityError::DelegationDenied)
    } else {
        Err(CorporateIdentityError::MissingJwt)
    }
}

fn extract_unconditional_nip_oa_owner(
    signer: PublicKey,
    auth_tag_json: Option<&str>,
) -> Option<PublicKey> {
    let tag_json = auth_tag_json?;
    let tag: Vec<Value> = serde_json::from_str(tag_json).ok()?;
    if tag.len() != 4 || tag.get(2).and_then(Value::as_str) != Some("") {
        return None;
    }
    buzz_sdk::nip_oa::verify_auth_tag(tag_json, &signer).ok()
}

fn is_allowed_jwt_algorithm(algorithm: Algorithm) -> bool {
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

#[cfg(test)]
fn validate_jwk_signature_metadata(
    jwk: &Jwk,
    token_algorithm: Algorithm,
) -> Result<(), CorporateIdentityError> {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|key_use| key_use != &PublicKeyUse::Signature)
    {
        return Err(CorporateIdentityError::InvalidJwt(
            "JWK use must be sig for JWT verification".to_string(),
        ));
    }
    if jwk
        .common
        .key_operations
        .as_ref()
        .is_some_and(|operations| !operations.contains(&KeyOperations::Verify))
    {
        return Err(CorporateIdentityError::InvalidJwt(
            "JWK key_ops must include verify for JWT verification".to_string(),
        ));
    }
    if jwk
        .common
        .key_algorithm
        .is_some_and(|algorithm| !jwk_algorithm_matches(algorithm, token_algorithm))
    {
        return Err(CorporateIdentityError::InvalidJwt(format!(
            "JWT algorithm {token_algorithm:?} does not match JWK algorithm"
        )));
    }
    Ok(())
}

#[cfg(test)]
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

fn binding_source_for_signer(
    claim_pubkey: Option<PublicKey>,
    signer: PublicKey,
) -> Result<&'static str, CorporateIdentityError> {
    match claim_pubkey {
        Some(claim_pubkey) => {
            if claim_pubkey != signer {
                warn!(
                    signer = %signer.to_hex(),
                    claim_pubkey = %claim_pubkey.to_hex(),
                    "corporate identity JWT npub claim does not match signer"
                );
                return Err(CorporateIdentityError::NpubMismatch);
            }
            Ok(SOURCE_JWT_NPUB)
        }
        None => Ok(SOURCE_DB_BINDING),
    }
}

fn claim_string(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<String, CorporateIdentityError> {
    let value = claims
        .get(claim)
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: "missing".to_string(),
        })?;
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: "must be a non-empty string".to_string(),
        })?;
    Ok(value.to_string())
}

fn configured_pubkey_claim(
    claims: &Map<String, Value>,
    claim: Option<&str>,
) -> Result<Option<PublicKey>, CorporateIdentityError> {
    match claim {
        Some(claim) => claim_string(claims, claim)
            .and_then(|raw| parse_pubkey_claim(claim, &raw))
            .map(Some),
        None => Ok(None),
    }
}

fn claim_u64(claims: &Map<String, Value>, claim: &str) -> Result<u64, CorporateIdentityError> {
    claims
        .get(claim)
        .and_then(Value::as_u64)
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: "must be an unsigned integer".to_string(),
        })
}

fn parse_pubkey_claim(claim: &str, value: &str) -> Result<PublicKey, CorporateIdentityError> {
    if value.starts_with("npub1") {
        PublicKey::from_bech32(value).map_err(|e| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: format!("invalid npub: {e}"),
        })
    } else {
        PublicKey::from_hex(value).map_err(|e| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: format!("invalid pubkey hex: {e}"),
        })
    }
}

/// Create an optional service from config.
pub fn service_from_config(
    config: &CorporateIdentityConfig,
) -> Option<Arc<CorporateIdentityService>> {
    config
        .require
        .then(|| Arc::new(CorporateIdentityService::new(config.clone())))
}

fn record_corporate_identity_denial(error: &CorporateIdentityError) {
    let reason = match error {
        CorporateIdentityError::MissingJwt => "missing_jwt",
        CorporateIdentityError::MissingKid => "missing_kid",
        CorporateIdentityError::InvalidJwt(_) => "invalid_jwt",
        CorporateIdentityError::Jwks(_) => "jwks",
        CorporateIdentityError::InvalidClaim { .. } => "invalid_claim",
        CorporateIdentityError::NpubMismatch => "npub_mismatch",
        CorporateIdentityError::BindingConflict => "binding_conflict",
        CorporateIdentityError::BindingRevoked => "binding_revoked",
        CorporateIdentityError::DelegationDenied => "delegation_denied",
        CorporateIdentityError::Db(_) => "db",
        CorporateIdentityError::FoundationIntegrationRequired => "foundation_integration_required",
    };
    metrics::counter!("buzz_auth_failures_total", "reason" => "corporate_identity_denied")
        .increment(1);
    metrics::counter!("buzz_corporate_identity_denials_total", "reason" => reason).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use aws_lc_rs::{
        rand::SystemRandom,
        rsa::KeySize,
        signature::{KeyPair, RsaKeyPair, RsaPublicKeyComponents, RSA_PKCS1_SHA256},
    };
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use jsonwebtoken::jwk::JwkSet;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use nostr::Keys;
    use sqlx::PgPool;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    fn loopback_test_database_url() -> String {
        format!(
            "{}://{}:{}@{}:{}/{}",
            "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
        )
    }

    fn test_config() -> CorporateIdentityConfig {
        CorporateIdentityConfig {
            require: true,
            jwt_header: "x-buzz-identity-token".to_string(),
            allow_delegation: true,
            auth_precedence: CorporateIdentityAuthPrecedence::Direct,
            jwks_uri: "http://127.0.0.1:9/jwks".to_string(),
            issuer: "https://idp.example".to_string(),
            audience: "buzz-relay".to_string(),
            uid_claim: "sub".to_string(),
            display_claim: "email".to_string(),
            public_display_claim: None,
            npub_claim: Some("buzz_npub".to_string()),
        }
    }

    fn test_identity_binding(
        issuer: &str,
        uid: &str,
        pubkey: PublicKey,
    ) -> buzz_db::identity_binding::IdentityBinding {
        let now = chrono::Utc::now();
        buzz_db::identity_binding::IdentityBinding {
            issuer: issuer.to_string(),
            uid: uid.to_string(),
            pubkey: pubkey.to_bytes().to_vec(),
            display_name: None,
            source: SOURCE_DB_BINDING.to_string(),
            created_at: now,
            updated_at: now,
            last_seen_at: now,
        }
    }

    fn spawn_test_revalidation(
        signer: PublicKey,
        plan: SessionRevalidationPlan,
        cancel: tokio_util::sync::CancellationToken,
        result: Result<Option<buzz_db::identity_binding::IdentityBinding>, &'static str>,
    ) -> (tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
        let lookups = Arc::new(AtomicUsize::new(0));
        let task_lookups = Arc::clone(&lookups);
        let task = tokio::spawn(run_session_binding_revalidation(
            IDENTITY_SESSION_REVALIDATION_INTERVAL,
            signer,
            plan.binding_pubkey,
            plan.expected_issuer,
            plan.expected_uid,
            cancel,
            move || {
                task_lookups.fetch_add(1, Ordering::SeqCst);
                let result = result.clone();
                async move { result }
            },
        ));
        (task, lookups)
    }

    #[tokio::test(start_paused = true)]
    async fn direct_session_stays_live_before_expiry_and_cancels_at_expiry() {
        let cancel = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(cancel_session_at_expiry(110, 100, cancel.clone()));
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert!(!cancel.is_cancelled());

        tokio::time::advance(Duration::from_secs(1)).await;
        cancel.cancelled().await;
        task.await.expect("expiry task");
    }

    #[tokio::test(start_paused = true)]
    async fn matching_session_binding_stays_live() {
        let signer = Keys::generate().public_key();
        let plan = SessionRevalidationPlan {
            binding_pubkey: signer,
            expected_issuer: "https://idp.example".to_string(),
            expected_uid: "user-1".to_string(),
            expires_at: None,
        };
        let binding = test_identity_binding("https://idp.example", "user-1", signer);
        let cancel = tokio_util::sync::CancellationToken::new();
        let (task, lookups) =
            spawn_test_revalidation(signer, plan, cancel.clone(), Ok(Some(binding)));

        while lookups.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert!(!cancel.is_cancelled());
        cancel.cancel();
        task.await.expect("revalidation task");
    }

    #[tokio::test(start_paused = true)]
    async fn missing_or_mismatched_session_binding_cancels() {
        for binding in [
            None,
            Some(test_identity_binding(
                "https://idp.example",
                "different-user",
                Keys::generate().public_key(),
            )),
        ] {
            let signer = Keys::generate().public_key();
            let plan = SessionRevalidationPlan {
                binding_pubkey: signer,
                expected_issuer: "https://idp.example".to_string(),
                expected_uid: "user-1".to_string(),
                expires_at: None,
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            let (task, _) = spawn_test_revalidation(signer, plan, cancel.clone(), Ok(binding));

            cancel.cancelled().await;
            task.await.expect("revalidation task");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn delegated_session_cancels_when_owner_binding_is_revoked() {
        let signer = Keys::generate().public_key();
        let owner = Keys::generate().public_key();
        let plan = session_revalidation_plan(
            signer,
            CorporateIdentityDecision::Delegated {
                owner_pubkey: owner,
                owner_issuer: "https://idp.example".to_string(),
                owner_uid: "owner-1".to_string(),
            },
        )
        .expect("delegated session plan");
        assert_eq!(plan.binding_pubkey, owner);
        let cancel = tokio_util::sync::CancellationToken::new();
        let (task, _) = spawn_test_revalidation(signer, plan, cancel.clone(), Ok(None));

        cancel.cancelled().await;
        task.await.expect("revalidation task");
    }

    #[tokio::test(start_paused = true)]
    async fn session_revalidation_database_error_cancels_fail_closed() {
        let signer = Keys::generate().public_key();
        let plan = SessionRevalidationPlan {
            binding_pubkey: signer,
            expected_issuer: "https://idp.example".to_string(),
            expected_uid: "user-1".to_string(),
            expires_at: None,
        };
        let cancel = tokio_util::sync::CancellationToken::new();
        let (task, _) =
            spawn_test_revalidation(signer, plan, cancel.clone(), Err("database unavailable"));

        cancel.cancelled().await;
        task.await.expect("revalidation task");
    }

    #[test]
    fn identity_projects_as_relay_signed_nip85_assertion_without_provider_details() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let event = build_identity_assertion(
            &relay,
            subject,
            Some("Example User"),
            456,
            Timestamp::from(123),
        )
        .unwrap();

        assert_eq!(event.kind.as_u16() as u32, KIND_USER_TRUSTED_ASSERTION);
        assert_eq!(event.pubkey, relay.public_key());
        assert!(event.verify_id());
        assert!(event.verify_signature());
        assert!(identity_assertion_matches(
            &event,
            &subject.to_hex(),
            Some("Example User"),
            456,
        ));
        assert!(
            !event
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().is_some_and(|name| name == "uid")),
            "the public assertion must not expose the stable corporate uid"
        );
        assert!(
            !event
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().is_some_and(|name| name == "issuer")),
            "the public assertion must not expose the upstream identity provider"
        );
    }

    #[test]
    fn identity_assertions_are_bounded_and_can_be_retired() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let now = 1_000;

        assert_eq!(
            identity_assertion_expiration(
                Some("Example User"),
                now + IDENTITY_ASSERTION_MAX_TTL_SECS + 1,
                now,
            ),
            now + IDENTITY_ASSERTION_MAX_TTL_SECS,
        );
        assert_eq!(
            identity_assertion_expiration(Some("Example User"), now + 60, now),
            now + 60,
        );
        assert_eq!(identity_assertion_expiration(None, u64::MAX, now), 0);

        let retired = build_identity_assertion(&relay, subject, None, 0, Timestamp::from(now))
            .expect("build inactive assertion");
        assert!(identity_assertion_matches(
            &retired,
            &subject.to_hex(),
            None,
            0,
        ));
        assert!(retired.tags.iter().any(|tag| {
            tag.as_slice().first().is_some_and(|part| part == "active")
                && tag.as_slice().get(1).is_some_and(|part| part == "false")
        }));
        assert!(!retired.tags.iter().any(|tag| {
            tag.as_slice()
                .first()
                .is_some_and(|part| part == "display_name")
        }));
    }

    #[test]
    fn direct_jwt_precedes_delegation_by_default() {
        let config = test_config();
        assert_eq!(
            select_identity_auth_path(&config, Some("jwt"), Some("auth-tag")),
            IdentityAuthPath::Direct
        );
    }

    #[test]
    fn deployment_can_select_delegated_owner_precedence() {
        let mut config = test_config();
        config.auth_precedence = CorporateIdentityAuthPrecedence::Delegated;
        assert_eq!(
            select_identity_auth_path(&config, Some("jwt"), Some("auth-tag")),
            IdentityAuthPath::Delegated
        );
        assert_eq!(
            select_identity_auth_path(&config, Some("jwt"), None),
            IdentityAuthPath::Direct
        );
    }

    #[test]
    fn rejects_hmac_jwt_algorithms_in_allowlist() {
        assert!(!is_allowed_jwt_algorithm(Algorithm::HS256));
        assert!(!is_allowed_jwt_algorithm(Algorithm::HS384));
        assert!(!is_allowed_jwt_algorithm(Algorithm::HS512));
        assert!(is_allowed_jwt_algorithm(Algorithm::RS256));
    }

    #[tokio::test]
    async fn validate_jwt_rejects_hs256_before_jwks_lookup() {
        let service = CorporateIdentityService::new(test_config());
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("hs256-kid".to_string());
        let token = encode(
            &header,
            &serde_json::json!({
                "iss": "https://idp.example",
                "aud": "buzz-relay",
                "sub": "user-1",
                "email": "user@example.com",
            }),
            &EncodingKey::from_secret(b"test-secret"),
        )
        .expect("encode test jwt");

        let err = service
            .validate_jwt(
                &token,
                CommunityId::from_uuid(Uuid::from_u128(1)),
                Keys::generate().public_key(),
                None,
            )
            .await
            .expect_err("HS256 must be rejected");
        assert!(matches!(err, CorporateIdentityError::InvalidJwt(_)));
    }

    #[tokio::test]
    async fn validate_jwt_accepts_matching_rs256_jwk() {
        let key = rsa_private_key(0);
        let token = rsa_test_jwt(key, "rsa-key");
        let claims = validate_rsa_jwt(&token, rsa_test_jwk(key, "rsa-key"))
            .await
            .expect("matching RSA JWT must validate");

        assert_eq!(claims.uid, "user-1");
        assert_eq!(claims.display_name, "user@example.com");
    }

    #[tokio::test]
    async fn validate_jwt_rejects_rs256_token_signed_by_wrong_key() {
        let signing_key = rsa_private_key(0);
        let advertised_key = rsa_private_key(1);
        let token = rsa_test_jwt(signing_key, "rsa-key");

        let error = validate_rsa_jwt(&token, rsa_test_jwk(advertised_key, "rsa-key"))
            .await
            .expect_err("JWT signed by another RSA key must fail");
        assert!(matches!(error, CorporateIdentityError::InvalidJwt(_)));
    }

    #[tokio::test]
    async fn validate_jwt_rejects_jwk_advertised_algorithm_mismatch() {
        let key = rsa_private_key(0);
        let token = rsa_test_jwt(key, "rsa-key");
        let mut jwk = rsa_test_jwk(key, "rsa-key");
        jwk.common.key_algorithm = Some(KeyAlgorithm::RS512);

        let error = validate_rsa_jwt(&token, jwk)
            .await
            .expect_err("JWK alg must agree with JWT alg");
        assert!(matches!(
            error,
            CorporateIdentityError::InvalidJwt(ref message)
                if message == "nip_fi_verifier_invalid_key"
        ));
    }

    #[tokio::test]
    async fn validate_jwt_accepts_jwk_with_omitted_algorithm() {
        let key = rsa_private_key(0);
        let token = rsa_test_jwt(key, "rsa-key");
        let mut jwk = rsa_test_jwk(key, "rsa-key");
        jwk.common.key_algorithm = None;

        validate_rsa_jwt(&token, jwk)
            .await
            .expect("an omitted optional JWK alg must not prevent RSA verification");
    }

    #[test]
    fn validate_jwk_requires_signature_use_and_verify_operation_when_present() {
        let key = rsa_private_key(0);
        let mut jwk = rsa_test_jwk(key, "rsa-key");
        jwk.common.public_key_use = Some(PublicKeyUse::Encryption);
        assert!(matches!(
            validate_jwk_signature_metadata(&jwk, Algorithm::RS256),
            Err(CorporateIdentityError::InvalidJwt(ref message))
                if message.contains("use must be sig")
        ));

        jwk.common.public_key_use = Some(PublicKeyUse::Signature);
        jwk.common.key_operations = Some(vec![KeyOperations::Sign]);
        assert!(matches!(
            validate_jwk_signature_metadata(&jwk, Algorithm::RS256),
            Err(CorporateIdentityError::InvalidJwt(ref message))
                if message.contains("key_ops must include verify")
        ));

        jwk.common.key_operations = Some(vec![KeyOperations::Sign, KeyOperations::Verify]);
        validate_jwk_signature_metadata(&jwk, Algorithm::RS256)
            .expect("JWK key_ops containing verify must be accepted");
    }

    #[test]
    fn jwt_validation_rejects_missing_and_malformed_audience_claims() {
        let now = Timestamp::now().as_secs();
        let missing = serde_json::json!({
            "iss": "https://idp.example",
            "sub": "user-1",
            "email": "user@example.com",
            "exp": now + 3_600,
        });
        let malformed = serde_json::json!({
            "iss": "https://idp.example",
            "aud": 42,
            "sub": "user-1",
            "email": "user@example.com",
            "exp": now + 3_600,
        });

        for claims in [missing, malformed] {
            decode_test_jwt(claims, Algorithm::RS256, b"test-secret", b"test-secret")
                .expect_err("invalid audience must not enroll an identity binding");
        }
    }

    #[test]
    fn jwt_validation_requires_expiration_issuer_and_audience() {
        let now = Timestamp::now().as_secs();
        for claim in ["exp", "iss", "aud"] {
            let mut claims = valid_test_claims(now)
                .as_object()
                .expect("claims object")
                .clone();
            claims.remove(claim);
            assert!(
                decode_test_jwt(
                    Value::Object(claims),
                    Algorithm::RS256,
                    b"test-secret",
                    b"test-secret",
                )
                .is_err(),
                "missing {claim} must fail closed",
            );
        }
    }

    #[test]
    fn jwt_validation_rejects_malformed_registered_claim_types() {
        let now = Timestamp::now().as_secs();
        for (claim, value) in [
            ("iss", Value::from(42)),
            ("aud", Value::from(42)),
            ("exp", Value::String("tomorrow".to_string())),
            ("nbf", Value::String("tomorrow".to_string())),
        ] {
            let mut claims = valid_test_claims(now)
                .as_object()
                .expect("claims object")
                .clone();
            claims.insert(claim.to_string(), value);
            assert!(
                decode_test_jwt(
                    Value::Object(claims),
                    Algorithm::RS256,
                    b"test-secret",
                    b"test-secret",
                )
                .is_err(),
                "malformed {claim} must fail closed",
            );
        }
    }

    #[test]
    fn jwt_validation_rejects_future_and_malformed_not_before_claims() {
        let now = Timestamp::now().as_secs();
        let mut future = valid_test_claims(now)
            .as_object()
            .expect("claims object")
            .clone();
        future.insert("nbf".to_string(), Value::from(now + 3_600));

        let mut malformed = valid_test_claims(now)
            .as_object()
            .expect("claims object")
            .clone();
        malformed.insert("nbf".to_string(), Value::String("tomorrow".to_string()));

        for claims in [Value::Object(future), Value::Object(malformed)] {
            decode_test_jwt(claims, Algorithm::RS256, b"test-secret", b"test-secret")
                .expect_err("invalid nbf must fail closed");
        }
    }

    #[test]
    fn jwt_validation_rejects_wrong_issuer_audience_and_expiry() {
        let now = Timestamp::now().as_secs();
        for (claim, value) in [
            ("iss", Value::String("https://attacker.example".to_string())),
            ("aud", Value::String("some-other-service".to_string())),
            ("exp", Value::from(now.saturating_sub(3_600))),
        ] {
            let mut claims = valid_test_claims(now)
                .as_object()
                .expect("claims object")
                .clone();
            claims.insert(claim.to_string(), value);
            assert!(
                decode_test_jwt(
                    Value::Object(claims),
                    Algorithm::RS256,
                    b"test-secret",
                    b"test-secret",
                )
                .is_err(),
                "invalid {claim} must fail closed",
            );
        }
    }

    #[test]
    fn jwt_validation_rejects_algorithm_and_key_mismatch() {
        let claims = valid_test_claims(Timestamp::now().as_secs());

        decode_test_jwt(
            claims.clone(),
            Algorithm::HS384,
            b"test-secret",
            b"test-secret",
        )
        .expect_err("the token algorithm must match verifier policy");
        decode_test_jwt(
            claims,
            Algorithm::HS256,
            b"signing-secret",
            b"different-verification-secret",
        )
        .expect_err("a token signed by a different key must fail");
    }

    #[test]
    fn extracts_bearer_token_from_comma_list_header() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-buzz-identity-token"),
            HeaderValue::from_static("Bearer token-a, Bearer token-b"),
        );

        assert_eq!(
            identity_jwt_from_headers(&headers, &config).as_deref(),
            Some("token-a")
        );
    }

    #[test]
    fn missing_required_claim_is_invalid() {
        let claims = Map::new();
        let err = claim_string(&claims, "sub").expect_err("missing claim");
        assert!(matches!(
            err,
            CorporateIdentityError::InvalidClaim { ref claim, .. } if claim == "sub"
        ));
    }

    #[test]
    fn configured_npub_claim_is_required_and_malformed_value_is_invalid() {
        let mut claims = Map::new();
        let missing = configured_pubkey_claim(&claims, Some("buzz_npub"))
            .expect_err("configured claim must be present");
        assert!(matches!(
            missing,
            CorporateIdentityError::InvalidClaim { ref claim, .. } if claim == "buzz_npub"
        ));

        claims.insert(
            "buzz_npub".to_string(),
            Value::String("not-an-npub".to_string()),
        );
        let err = configured_pubkey_claim(&claims, Some("buzz_npub"))
            .expect_err("present malformed claim must fail");
        assert!(matches!(
            err,
            CorporateIdentityError::InvalidClaim { ref claim, .. } if claim == "buzz_npub"
        ));
    }

    #[test]
    fn npub_claim_must_match_authenticated_signer() {
        let signer = Keys::generate().public_key();
        let other = Keys::generate().public_key();

        assert!(matches!(
            binding_source_for_signer(Some(other), signer),
            Err(CorporateIdentityError::NpubMismatch)
        ));
        assert_eq!(
            binding_source_for_signer(Some(signer), signer).expect("match"),
            SOURCE_JWT_NPUB
        );
        assert_eq!(
            binding_source_for_signer(None, signer).expect("db fallback"),
            SOURCE_DB_BINDING
        );
    }

    #[tokio::test]
    async fn fresh_jwks_cache_miss_forces_one_bounded_refresh() {
        let response = http_response(
            "200 OK",
            &["Content-Type: application/json"],
            r#"{"keys":[]}"#,
        );
        let (uri, requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        let service = CorporateIdentityService::new(config);
        *service.jwks.write().await = Some(CachedJwks {
            set: JwkSet { keys: Vec::new() },
            generation: VerifierKeyGeneration::new(1).expect("positive generation"),
            expires_at: Instant::now() + Duration::from_secs(60),
        });

        let err = service
            .jwk_snapshot_for_kid("attacker-controlled-kid")
            .await
            .expect_err("fresh cache miss should fail after one refresh");
        assert!(matches!(
            err,
            CorporateIdentityError::Jwks(ref msg) if msg.contains("after JWKS refresh")
        ));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn final_recheck_rejects_an_expired_key_generation() {
        let service = CorporateIdentityService::new(test_config());
        let generation = VerifierKeyGeneration::new(1).expect("positive generation");
        let policy_id = service
            .verifier
            .as_ref()
            .expect("test verifier policy")
            .policy_id();
        *service.jwks.write().await = Some(CachedJwks {
            set: JwkSet { keys: Vec::new() },
            generation,
            expires_at: Instant::now() - Duration::from_secs(1),
        });

        assert!(
            !service
                .accepts_final_verifier_stamp(VerifierPolicyStamp::new(policy_id, generation))
                .await
        );
    }

    #[tokio::test]
    async fn jwks_refresh_is_single_flight() {
        let body = r#"{"keys":[{"kty":"RSA","n":"AQAB","e":"AQAB","kid":"test-kid","alg":"RS256","use":"sig"}]}"#;
        let response = http_response("200 OK", &["Content-Type: application/json"], body);
        let (uri, requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        let service = CorporateIdentityService::new(config);

        let (first, second, third, fourth) = tokio::join!(
            service.jwk_snapshot_for_kid("test-kid"),
            service.jwk_snapshot_for_kid("test-kid"),
            service.jwk_snapshot_for_kid("test-kid"),
            service.jwk_snapshot_for_kid("test-kid"),
        );
        for result in [first, second, third, fourth] {
            result.expect("all waiters should reuse the refreshed JWKS");
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn jwks_response_content_length_is_capped_before_buffering() {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            JWKS_MAX_RESPONSE_BYTES + 1,
        );
        let (uri, _requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        let service = CorporateIdentityService::new(config);

        let error = service
            .fetch_jwks()
            .await
            .expect_err("oversized JWKS must fail before buffering the body");
        assert!(matches!(
            error,
            CorporateIdentityError::Jwks(ref message) if message.contains("size limit")
        ));
        server.abort();
    }

    #[tokio::test]
    async fn jwks_streaming_response_is_capped_without_content_length() {
        let response = format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
            " ".repeat(JWKS_MAX_RESPONSE_BYTES + 1),
        );
        let (uri, _requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        let service = CorporateIdentityService::new(config);

        let error = service
            .fetch_jwks()
            .await
            .expect_err("streamed oversized JWKS must stop at the cap");
        assert!(matches!(
            error,
            CorporateIdentityError::Jwks(ref message) if message.contains("size limit")
        ));
        server.abort();
    }

    #[test]
    fn transport_wide_delegation_rejects_conditional_nip_oa_tags() {
        let owner = Keys::generate();
        let agent = Keys::generate().public_key();
        let unconditional =
            buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent, "").expect("unconditional auth tag");
        let conditional = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent, "kind=1")
            .expect("conditional auth tag");

        assert_eq!(
            extract_unconditional_nip_oa_owner(agent, Some(&unconditional)),
            Some(owner.public_key()),
        );
        assert_eq!(
            extract_unconditional_nip_oa_owner(agent, Some(&conditional)),
            None,
        );
    }

    fn valid_test_claims(now: u64) -> Value {
        serde_json::json!({
            "iss": "https://idp.example",
            "aud": "buzz-relay",
            "sub": "user-1",
            "email": "user@example.com",
            "iat": now,
            "exp": now + 3_600,
        })
    }

    fn decode_test_jwt(
        claims: Value,
        signing_algorithm: Algorithm,
        signing_key: &[u8],
        verification_key: &[u8],
    ) -> Result<(), CanonicalVerifierError> {
        let private_key = rsa_private_key(0);
        let token = if signing_algorithm == Algorithm::RS256 {
            rsa_test_jwt_with_claims(private_key, "canonical-test-key", &claims)
        } else {
            let mut header = Header::new(signing_algorithm);
            header.kid = Some("canonical-test-key".to_owned());
            encode(&header, &claims, &EncodingKey::from_secret(signing_key))
                .expect("encode rejected symmetric test JWT")
        };
        let verification_material = if signing_key == verification_key {
            private_key
        } else {
            rsa_private_key(1)
        };
        let key_set = CanonicalVerifierKeySet::new(
            VerifierKeyGeneration::new(1).expect("generation"),
            JwkSet {
                keys: vec![rsa_test_jwk(verification_material, "canonical-test-key")],
            },
        );
        let policy = CanonicalVerifierPolicy::new(
            "https://idp.example".to_owned(),
            "buzz-relay".to_owned(),
            "sub".to_owned(),
            None,
            JWT_CLOCK_SKEW_LEEWAY_SECS,
            IDENTITY_ASSERTION_MAX_TTL_SECS,
        )
        .expect("canonical policy");
        CanonicalFederatedAssertionVerifier::new(policy)
            .verify(
                &token,
                &key_set,
                CommunityId::from_uuid(Uuid::from_u128(1)),
                ProofTransport::Nip42,
                [1; 32],
                [2; 32],
                [3; 32],
            )
            .map(|_| ())
    }

    fn rsa_private_key(index: usize) -> &'static RsaKeyPair {
        static KEYS: std::sync::OnceLock<[RsaKeyPair; 2]> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| {
            std::array::from_fn(|_| {
                RsaKeyPair::generate(KeySize::Rsa2048).expect("generate RSA test key")
            })
        })
        .get(index)
        .expect("RSA test key index")
    }

    fn rsa_test_jwk(private_key: &RsaKeyPair, kid: &str) -> Jwk {
        let components = RsaPublicKeyComponents::<Vec<u8>>::from(private_key.public_key());
        serde_json::from_value(serde_json::json!({
            "kty": "RSA",
            "n": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(components.n),
            "e": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(components.e),
            "kid": kid,
            "alg": "RS256",
            "use": "sig",
            "key_ops": ["verify"],
        }))
        .expect("derive RSA JWK from generated test key")
    }

    fn rsa_test_jwt(private_key: &RsaKeyPair, kid: &str) -> String {
        rsa_test_jwt_with_claims(
            private_key,
            kid,
            &valid_test_claims(Timestamp::now().as_secs()),
        )
    }

    fn rsa_test_jwt_with_claims(private_key: &RsaKeyPair, kid: &str, claims: &Value) -> String {
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT",
            "kid": kid,
        });
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).expect("serialize RSA JWT test header"));
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).expect("serialize RSA JWT test claims"));
        let signing_input = format!("{header}.{claims}");
        let mut signature = vec![0_u8; private_key.public_modulus_len()];
        private_key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .expect("sign generated RSA test JWT");
        format!(
            "{signing_input}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
        )
    }

    async fn validate_rsa_jwt(
        token: &str,
        jwk: Jwk,
    ) -> Result<CorporateJwtClaims, CorporateIdentityError> {
        let body =
            serde_json::to_string(&JwkSet { keys: vec![jwk] }).expect("serialize RSA test JWKS");
        let response = http_response("200 OK", &["Content-Type: application/json"], &body);
        let (uri, _requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        config.npub_claim = None;
        let result = CorporateIdentityService::new(config)
            .validate_jwt(
                token,
                CommunityId::from_uuid(Uuid::from_u128(1)),
                Keys::generate().public_key(),
                None,
            )
            .await;
        server.abort();
        result
    }

    fn http_response(status: &str, headers: &[&str], body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            headers
                .iter()
                .map(|header| format!("{header}\r\n"))
                .collect::<String>(),
            body.len(),
        )
    }

    async fn spawn_http_server(
        response: String,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let address = listener.local_addr().expect("test server address");
        let requests = Arc::new(AtomicUsize::new(0));
        let request_count = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                request_count.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 2_048];
                let Ok(bytes_read) = stream.read(&mut request).await else {
                    return;
                };
                if bytes_read == 0 {
                    return;
                }
                if stream.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        (format!("http://{address}/jwks"), requests, server)
    }

    async fn setup_db() -> (buzz_db::Db, PgPool) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| loopback_test_database_url());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        let db = buzz_db::Db::from_pool(pool.clone());
        db.migrate().await.expect("run migrations");
        (db, pool)
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("relay-identity-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn delegation_requires_owner_identity_binding() {
        let (db, pool) = setup_db().await;
        let community = make_community(&pool).await;
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key();
        let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_pubkey, "").unwrap();
        let config = test_config();

        let err = verify_delegated_corporate_identity(
            &db,
            &config,
            community,
            agent_pubkey,
            Some(&auth_tag),
        )
        .await
        .expect_err("owner without binding should be denied");
        assert!(matches!(err, CorporateIdentityError::DelegationDenied));

        db.bind_or_validate_identity(
            community,
            &config.issuer,
            "owner-uid",
            owner_keys.public_key().as_bytes(),
            Some("owner@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create owner binding");

        let decision = verify_delegated_corporate_identity(
            &db,
            &config,
            community,
            agent_pubkey,
            Some(&auth_tag),
        )
        .await
        .expect("owner binding admits agent");
        assert_eq!(
            decision,
            CorporateIdentityProof::Delegated {
                owner_pubkey: owner_keys.public_key(),
                owner_issuer: config.issuer.clone(),
                owner_uid: "owner-uid".to_string(),
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn missing_jwt_without_auth_tag_is_missing_jwt() {
        let (db, pool) = setup_db().await;
        let community = make_community(&pool).await;
        let signer = Keys::generate().public_key();
        let config = test_config();

        let err = verify_delegated_corporate_identity(&db, &config, community, signer, None)
            .await
            .expect_err("no JWT and no delegation tag");
        assert!(matches!(err, CorporateIdentityError::MissingJwt));
    }
}
