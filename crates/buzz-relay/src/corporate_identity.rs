//! Corporate identity verification and uid/pubkey binding.
//!
//! This module is intentionally relay-local. `buzz-auth` remains the generic
//! Nostr proof layer; corporate identity is deployment policy layered after a
//! request proves control of a Nostr key.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    http::{HeaderMap, StatusCode},
    response::Json,
};
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
    Algorithm, DecodingKey, Validation,
};
use nostr::{Event, EventBuilder, FromBech32, Kind, PublicKey, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use buzz_auth::{
    AuthorizationClock, AuthorizationClockError, SharedAuthorizationClock, SystemAuthorizationClock,
};
use buzz_core::{kind::KIND_USER_TRUSTED_ASSERTION, CommunityId};
use buzz_db::identity_binding::{BindIdentityResult, SOURCE_DB_BINDING, SOURCE_JWT_NPUB};
use buzz_pubsub::EventTopic;

use crate::config::{CorporateIdentityAuthPrecedence, CorporateIdentityConfig};
use crate::state::AppState;

const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);
const JWKS_CACHE_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const JWKS_REFRESH_FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const JWKS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const JWKS_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const JWKS_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
// Permit a bounded issuer/relay clock difference while keeping expiry enforcement explicit.
const JWT_CLOCK_SKEW_LEEWAY_SECS: u64 = 60;
const IDENTITY_ASSERTION_MAX_TTL_SECS: u64 = 60 * 60;
const IDENTITY_SESSION_REVALIDATION_INTERVAL: Duration = Duration::from_secs(30);
const PUBLIC_PROJECTION_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(1);
const PUBLIC_PROJECTION_STARTUP_LIMIT: usize = 4096;

#[derive(Debug, Clone)]
struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
    fresh_until: Instant,
    hard_expires_at: Instant,
    refresh_after: Instant,
}

fn record_jwks_cache_age(cached: &CachedJwks, now: Instant) {
    metrics::gauge!("buzz_jwks_cache_age_seconds").set(
        now.saturating_duration_since(cached.fetched_at)
            .as_secs_f64(),
    );
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
    /// Optional operator-approved label that may be published in NIP-85.
    pub public_display_name: Option<String>,
    /// Optional pubkey carried by the IdP.
    pub pubkey: Option<PublicKey>,
    /// JWT expiration as a Unix timestamp.
    pub expires_at: u64,
}

impl fmt::Debug for CorporateJwtClaims {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CorporateJwtClaims")
            .field("issuer", &"[redacted]")
            .field("uid", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("public_display_name", &"[redacted]")
            .field("pubkey", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .finish()
    }
}

#[derive(Deserialize)]
struct RawJwtClaims {
    #[serde(flatten)]
    claims: Map<String, Value>,
}

impl fmt::Debug for RawJwtClaims {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawJwtClaims")
            .field("claims", &"[redacted]")
            .finish()
    }
}

/// Service that verifies corporate identity JWTs against configured JWKS.
pub struct CorporateIdentityService {
    config: CorporateIdentityConfig,
    http: Result<reqwest::Client, String>,
    jwks: RwLock<Option<CachedJwks>>,
    refresh: Mutex<()>,
    authorization_clock: SharedAuthorizationClock,
}

impl fmt::Debug for CorporateIdentityService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CorporateIdentityService")
            .field("config", &"[redacted]")
            .field("http", &"[initialized]")
            .field("jwks", &"[cache]")
            .field("refresh", &"[lock]")
            .field("authorization_clock", &"[injected]")
            .finish()
    }
}

impl CorporateIdentityService {
    /// Build a corporate identity verifier from relay config.
    pub fn new(config: CorporateIdentityConfig) -> Self {
        Self::with_authorization_clock(config, Arc::new(SystemAuthorizationClock))
    }

    /// Build a verifier with the shared authorization clock.
    pub fn with_authorization_clock(
        config: CorporateIdentityConfig,
        authorization_clock: SharedAuthorizationClock,
    ) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(JWKS_CONNECT_TIMEOUT)
            .timeout(JWKS_REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string());
        Self {
            config,
            http,
            jwks: RwLock::new(None),
            refresh: Mutex::new(()),
            authorization_clock,
        }
    }

    /// Read the shared verifier clock for portable evidence finalization.
    pub fn authorization_now(&self) -> Result<u64, CorporateIdentityError> {
        Ok(self.authorization_clock.now()?.unix_seconds())
    }

    /// Validate a JWT and extract the configured corporate identity claims.
    pub async fn validate_jwt(
        &self,
        token: &str,
    ) -> Result<CorporateJwtClaims, CorporateIdentityError> {
        let result = self.validate_jwt_inner(token).await;
        if result.is_err() {
            metrics::counter!("buzz_jwt_verification_errors_total").increment(1);
        }
        result
    }

    async fn validate_jwt_inner(
        &self,
        token: &str,
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
        let jwk = self.jwk_for_kid(kid).await?;
        validate_jwk_signature_metadata(&jwk, header.alg)?;
        let decoding_key = DecodingKey::from_jwk(&jwk).map_err(|e| {
            CorporateIdentityError::InvalidJwt(format!("invalid JWK for kid {kid}: {e}"))
        })?;

        let validation = jwt_validation(header.alg, &self.config);

        let decoded = decode::<RawJwtClaims>(token, &decoding_key, &validation)
            .map_err(|e| CorporateIdentityError::InvalidJwt(e.to_string()))?;
        validate_optional_iat(
            &decoded.claims.claims,
            self.authorization_clock.now()?.unix_seconds(),
        )?;

        let issuer = claim_string_exact(&decoded.claims.claims, "iss")?;
        let uid = claim_string_exact(&decoded.claims.claims, &self.config.uid_claim)?;
        let display_name = claim_string(&decoded.claims.claims, &self.config.display_claim)?;
        let public_display_name = self
            .config
            .public_display_claim
            .as_deref()
            .map(|claim| claim_string(&decoded.claims.claims, claim))
            .transpose()?;
        let pubkey =
            configured_pubkey_claim(&decoded.claims.claims, self.config.npub_claim.as_deref())?;
        let expires_at = claim_u64(&decoded.claims.claims, "exp")?;

        Ok(CorporateJwtClaims {
            issuer,
            uid,
            display_name,
            public_display_name,
            pubkey,
            expires_at,
        })
    }

    async fn jwk_for_kid(&self, kid: &str) -> Result<Jwk, CorporateIdentityError> {
        let now = Instant::now();
        {
            let cache = self.jwks.read().await;
            if let Some(cached) = cache.as_ref() {
                record_jwks_cache_age(cached, now);
                if cached.fresh_until > now {
                    if let Some(jwk) = cached.set.find(kid) {
                        return Ok(jwk.clone());
                    }
                    metrics::counter!("buzz_jwks_unknown_kid_total").increment(1);
                    return Err(CorporateIdentityError::Jwks(format!(
                        "kid not found in fresh JWKS cache: {kid}"
                    )));
                }
                if cached.refresh_after > now {
                    if cached.hard_expires_at > now {
                        if let Some(jwk) = cached.set.find(kid) {
                            metrics::counter!("buzz_jwks_stale_key_uses_total").increment(1);
                            return Ok(jwk.clone());
                        }
                    }
                    metrics::counter!("buzz_jwks_unknown_kid_total").increment(1);
                    return Err(CorporateIdentityError::Jwks(
                        "JWKS refresh is temporarily unavailable".to_string(),
                    ));
                }
            }
        }

        // Only one request may refresh at a time. Re-check after acquiring the
        // mutex because another waiter may already have populated the cache.
        let _refresh = self.refresh.lock().await;
        let now = Instant::now();
        let stale_known_key = {
            let cache = self.jwks.read().await;
            if let Some(cached) = cache.as_ref() {
                record_jwks_cache_age(cached, now);
                if cached.fresh_until > now {
                    return cached.set.find(kid).cloned().ok_or_else(|| {
                        metrics::counter!("buzz_jwks_unknown_kid_total").increment(1);
                        CorporateIdentityError::Jwks(format!(
                            "kid not found in fresh JWKS cache: {kid}"
                        ))
                    });
                }
                if cached.refresh_after > now {
                    if cached.hard_expires_at > now {
                        if let Some(jwk) = cached.set.find(kid) {
                            metrics::counter!("buzz_jwks_stale_key_uses_total").increment(1);
                            return Ok(jwk.clone());
                        }
                    }
                    metrics::counter!("buzz_jwks_unknown_kid_total").increment(1);
                    return Err(CorporateIdentityError::Jwks(
                        "JWKS refresh is temporarily unavailable".to_string(),
                    ));
                }
                if cached.hard_expires_at > now {
                    cached.set.find(kid).cloned()
                } else {
                    None
                }
            } else {
                None
            }
        };

        metrics::counter!("buzz_jwks_refresh_total", "result" => "attempt").increment(1);
        let set = match self.fetch_jwks().await {
            Ok(set) => {
                metrics::counter!("buzz_jwks_refresh_total", "result" => "success").increment(1);
                set
            }
            Err(error) => {
                metrics::counter!("buzz_jwks_refresh_total", "result" => "failure").increment(1);
                let now = Instant::now();
                if let Some(cached) = self.jwks.write().await.as_mut() {
                    cached.refresh_after = now + JWKS_REFRESH_FAILURE_BACKOFF;
                }
                if let Some(jwk) = stale_known_key {
                    metrics::counter!("buzz_jwks_stale_key_uses_total").increment(1);
                    return Ok(jwk);
                }
                return Err(error);
            }
        };
        let jwk = set.find(kid).cloned();
        let fetched_at = Instant::now();
        *self.jwks.write().await = Some(CachedJwks {
            set,
            fetched_at,
            fresh_until: fetched_at + JWKS_CACHE_TTL,
            hard_expires_at: fetched_at + JWKS_CACHE_MAX_AGE,
            refresh_after: fetched_at + JWKS_CACHE_TTL,
        });
        jwk.ok_or_else(|| {
            metrics::counter!("buzz_jwks_unknown_kid_total").increment(1);
            CorporateIdentityError::Jwks(format!("kid not found: {kid}"))
        })
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

fn jwt_validation(algorithm: Algorithm, config: &CorporateIdentityConfig) -> Validation {
    let mut validation = Validation::new(algorithm);
    validation.leeway = JWT_CLOCK_SKEW_LEEWAY_SECS;
    validation.set_issuer(&[config.issuer.as_str()]);
    validation.set_audience(&[config.audience.as_str()]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation
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
        claims: CorporateJwtClaims,
        /// Binding source selected from the configured npub policy.
        source: &'static str,
        /// Transport provenance proved by the deployment adapter. `None` is
        /// retained only for the disabled legacy lane and can never be sealed
        /// into protected runtime evidence.
        assertion_transport: Option<buzz_auth::AssertionTransport>,
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

impl fmt::Debug for CorporateIdentityProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotRequired => "CorporateIdentityProof::NotRequired",
            Self::Direct { .. } => "CorporateIdentityProof::Direct([redacted])",
            Self::Delegated { .. } => "CorporateIdentityProof::Delegated([redacted])",
        })
    }
}

/// Borrow staged direct-identity data for an atomic admission transaction.
pub fn binding_input_for_proof<'a>(
    proof: &'a CorporateIdentityProof,
    signer: &'a PublicKey,
) -> Option<buzz_db::identity_binding::IdentityBindingInput<'a>> {
    match proof {
        CorporateIdentityProof::Direct { claims, source, .. } => {
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

/// Translate a read-only direct verifier result into sealed provider evidence.
pub fn verified_assertion_for_proof(
    proof: &CorporateIdentityProof,
    authorization_domain: CommunityId,
    transport: buzz_auth::AuthTransport,
    now_unix_seconds: u64,
) -> Result<Option<buzz_auth::VerifiedFederatedAssertion>, CorporateIdentityError> {
    let CorporateIdentityProof::Direct {
        claims,
        assertion_transport,
        ..
    } = proof
    else {
        return Ok(None);
    };
    let assertion_transport = assertion_transport.ok_or_else(|| {
        CorporateIdentityError::Evidence("verified ingress provenance is unavailable".to_owned())
    })?;
    buzz_auth::VerifiedEvidenceAdapter::new()
        .federated_assertion_from_validated_claims(
            authorization_domain,
            transport,
            &claims.issuer,
            &claims.uid,
            claims.pubkey,
            assertion_transport,
            None,
            claims.expires_at,
            now_unix_seconds,
        )
        .map(Some)
        .map_err(|error| CorporateIdentityError::Evidence(error.to_string()))
}

/// Seal current direct evidence with the same clock used by the configured
/// verifier. Non-direct proofs require neither a verifier instance nor time.
pub fn current_verified_assertion_for_proof(
    state: &crate::state::AppState,
    proof: &CorporateIdentityProof,
    authorization_domain: CommunityId,
    transport: buzz_auth::AuthTransport,
) -> Result<Option<buzz_auth::VerifiedFederatedAssertion>, CorporateIdentityError> {
    if !matches!(proof, CorporateIdentityProof::Direct { .. }) {
        return Ok(None);
    }
    if matches!(
        proof,
        CorporateIdentityProof::Direct {
            assertion_transport: None,
            ..
        }
    ) && crate::authorization_runtime::transport::legacy_identity_lane(
        state,
        authorization_domain,
    ) == crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
    {
        // Off/absent keeps the inherited verifier/finalizer behavior, but an
        // unproved header can never be promoted into protected evidence.
        return Ok(None);
    }
    let verifier = state
        .corporate_identity
        .as_ref()
        .ok_or(CorporateIdentityError::VerifierUnavailable)?;
    verified_assertion_for_proof(
        proof,
        authorization_domain,
        transport,
        verifier.authorization_now()?,
    )
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

impl fmt::Debug for CorporateIdentityDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotRequired => "CorporateIdentityDecision::NotRequired",
            Self::Direct { .. } => "CorporateIdentityDecision::Direct([redacted])",
            Self::Delegated { .. } => "CorporateIdentityDecision::Delegated([redacted])",
        })
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
    _signer: PublicKey,
    _binding_pubkey: PublicKey,
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
                        warn!("corporate identity session evicted after binding revocation");
                        cancel.cancel();
                        return;
                    }
                    Err(_error) => {
                        warn!("corporate identity session revalidation failed closed");
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
    /// Direct identity evidence existed without its configured verifier.
    #[error("relay identity verifier unavailable")]
    VerifierUnavailable,
    /// The configured identity header was duplicated, combined, or malformed.
    #[error("relay identity header is ambiguous")]
    AmbiguousIdentityHeader,
    /// Protected identity evidence lacked deployment-verified provenance.
    #[error("relay identity transport provenance is unavailable")]
    UntrustedIdentityTransport,
    /// The shared authorization clock could not provide verifier time.
    #[error("corporate identity authorization clock unavailable")]
    AuthorizationClock(#[from] AuthorizationClockError),
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
    /// No active binding exists and this compatibility path cannot enroll one.
    #[error("corporate identity binding requires authorized enrollment")]
    BindingRequired,
    /// NIP-OA delegation was present but did not satisfy corporate identity.
    #[error("corporate identity delegation denied")]
    DelegationDenied,
    /// Verified claims could not be sealed as portable assertion evidence.
    #[error("corporate identity evidence is inconsistent: {0}")]
    Evidence(String),
    /// Database operation failed.
    #[error("corporate identity database error: {0}")]
    Db(#[from] buzz_db::DbError),
}

impl fmt::Debug for CorporateIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CorporateIdentityError")
            .field("reason", &self.reason_code())
            .finish()
    }
}

impl CorporateIdentityError {
    /// HTTP status appropriate for this error.
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::AuthorizationClock(_)
            | Self::AmbiguousIdentityHeader
            | Self::UntrustedIdentityTransport
            | Self::MissingJwt
            | Self::MissingKid
            | Self::InvalidJwt(_)
            | Self::Jwks(_) => StatusCode::UNAUTHORIZED,
            Self::InvalidClaim { .. }
            | Self::NpubMismatch
            | Self::BindingConflict
            | Self::BindingRevoked
            | Self::BindingRequired
            | Self::DelegationDenied
            | Self::Evidence(_) => StatusCode::FORBIDDEN,
            Self::VerifierUnavailable | Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Sanitized message safe to return to clients.
    pub fn public_message(&self) -> &'static str {
        match self {
            Self::VerifierUnavailable | Self::AuthorizationClock(_) => {
                "relay identity verification unavailable"
            }
            Self::MissingJwt => "relay-verified identity required",
            Self::AmbiguousIdentityHeader | Self::UntrustedIdentityTransport => {
                "relay identity verification failed"
            }
            Self::MissingKid | Self::InvalidJwt(_) | Self::Jwks(_) => {
                "relay identity verification failed"
            }
            Self::InvalidClaim { .. } => "relay identity claim invalid",
            Self::NpubMismatch => "relay identity pubkey mismatch",
            Self::BindingConflict => "relay identity binding conflict",
            Self::BindingRevoked => "relay identity binding revoked",
            Self::BindingRequired => "relay identity binding required",
            Self::DelegationDenied => "relay identity delegation denied",
            Self::Evidence(_) => "relay identity verification failed",
            Self::Db(_) => "relay identity unavailable",
        }
    }

    /// Convert to the standard API error shape.
    pub fn into_api_error(self) -> (StatusCode, Json<Value>) {
        let status = self.status_code();
        let message = self.public_message();
        if status.is_server_error() {
            warn!(
                reason = self.reason_code(),
                "corporate identity enforcement failed"
            );
        }
        (status, Json(serde_json::json!({ "error": message })))
    }

    /// Stable, non-sensitive diagnostic class for metrics and runtime logs.
    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::VerifierUnavailable => "verifier_unavailable",
            Self::AmbiguousIdentityHeader => "ambiguous_identity_header",
            Self::UntrustedIdentityTransport => "untrusted_identity_transport",
            Self::AuthorizationClock(_) => "authorization_clock",
            Self::MissingJwt => "missing_jwt",
            Self::MissingKid => "missing_kid",
            Self::InvalidJwt(_) => "invalid_jwt",
            Self::Jwks(_) => "jwks",
            Self::InvalidClaim { .. } => "invalid_claim",
            Self::NpubMismatch => "npub_mismatch",
            Self::BindingConflict => "binding_conflict",
            Self::BindingRevoked => "binding_revoked",
            Self::BindingRequired => "binding_required",
            Self::DelegationDenied => "delegation_denied",
            Self::Evidence(_) => "evidence",
            Self::Db(_) => "db",
        }
    }
}

/// Deployment-owned proof that a direct assertion arrived through a reviewed
/// ingress boundary.
///
/// The OSS relay deliberately does not infer provenance from the presence of
/// an identity header. A composition root must install an implementation that
/// validates immediate-caller transport evidence and confirms that inbound
/// copies were stripped before exactly one value was set.
pub trait IdentityAssertionProvenanceVerifier: Send + Sync {
    /// Return the provider-neutral assertion transport proved for this request.
    fn verify(
        &self,
        headers: &HeaderMap,
    ) -> Result<buzz_auth::AssertionTransport, CorporateIdentityError>;
}

/// One exact direct assertion captured at the request boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityAssertionInput {
    jwt: String,
    assertion_transport: Option<buzz_auth::AssertionTransport>,
}

impl IdentityAssertionInput {
    /// Validated header payload passed only to the JWT verifier.
    pub fn jwt(&self) -> &str {
        &self.jwt
    }
}

impl fmt::Debug for IdentityAssertionInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityAssertionInput")
            .field("jwt", &"[redacted]")
            .field("assertion_transport", &self.assertion_transport)
            .finish()
    }
}

/// Extract exactly one corporate identity JWT from the configured header.
///
/// RFC 9110 permits intermediaries to combine repeated field lines. Identity
/// evidence is not list-valued, so both multiple field lines and any comma are
/// rejected rather than applying a first/last-wins rule.
pub fn identity_jwt_from_headers(
    headers: &HeaderMap,
    config: &CorporateIdentityConfig,
) -> Result<Option<String>, CorporateIdentityError> {
    let mut values = headers.get_all(config.jwt_header.as_str()).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(CorporateIdentityError::AmbiguousIdentityHeader);
    }
    let raw = value
        .to_str()
        .map_err(|_| CorporateIdentityError::AmbiguousIdentityHeader)?
        .trim();
    if raw.contains(',') {
        return Err(CorporateIdentityError::AmbiguousIdentityHeader);
    }
    let jwt = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
    if jwt.is_empty() {
        return Err(CorporateIdentityError::AmbiguousIdentityHeader);
    }
    Ok(Some(jwt.to_owned()))
}

/// Extract direct identity evidence and attach only deployment-proved
/// transport provenance. Disabled/Off legacy behavior can retain the header
/// payload, but cannot turn it into protected runtime evidence.
pub fn identity_assertion_from_headers(
    state: &AppState,
    authorization_domain: CommunityId,
    headers: &HeaderMap,
) -> Result<Option<IdentityAssertionInput>, CorporateIdentityError> {
    let Some(jwt) = identity_jwt_from_headers(headers, &state.config.corporate_identity)? else {
        return Ok(None);
    };
    let mode = state
        .protected_transport()
        .and_then(|runtime| runtime.mode_for_domain(authorization_domain));
    let assertion_transport = match mode {
        None | Some(crate::authorization_runtime::finalization::AuthorizationMode::Off) => None,
        Some(_) => {
            let verifier = state
                .identity_assertion_provenance()
                .ok_or(CorporateIdentityError::UntrustedIdentityTransport)?;
            let transport = verifier.verify(headers)?;
            if transport != buzz_auth::AssertionTransport::TrustedProxy {
                return Err(CorporateIdentityError::UntrustedIdentityTransport);
            }
            Some(transport)
        }
    };
    Ok(Some(IdentityAssertionInput {
        jwt,
        assertion_transport,
    }))
}

/// Validate corporate identity without creating bindings or assertions.
pub async fn verify_corporate_identity(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    identity_assertion: Option<&IdentityAssertionInput>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityProof, CorporateIdentityError> {
    let result = verify_corporate_identity_inner(
        state,
        community_id,
        signer,
        identity_assertion,
        auth_tag_json,
    )
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
    identity_assertion: Option<&IdentityAssertionInput>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityProof, CorporateIdentityError> {
    let Some(service) = state.corporate_identity.as_ref() else {
        return Ok(CorporateIdentityProof::NotRequired);
    };
    if !service.config.require
        && crate::authorization_runtime::transport::legacy_identity_lane(state, community_id)
            == crate::authorization_runtime::transport::LegacyIdentityLane::Legacy
    {
        return Ok(CorporateIdentityProof::NotRequired);
    }

    // Requests can carry both a direct identity JWT and a cryptographically
    // verified NIP-OA owner declaration. The deployment selects which identity
    // source wins; the provider-neutral default treats the JWT as the signer's
    // identity. Delegated precedence supports identity-aware gateways that
    // attach an owner's token to requests made by that owner's agents.
    if select_identity_auth_path(
        &service.config,
        identity_assertion.map(IdentityAssertionInput::jwt),
        auth_tag_json,
    ) == IdentityAuthPath::Delegated
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

    if let Some(assertion) = identity_assertion {
        let claims = service.validate_jwt(assertion.jwt()).await?;
        let source = binding_source_for_signer(claims.pubkey, signer)?;
        return Ok(CorporateIdentityProof::Direct {
            claims,
            source,
            assertion_transport: assertion.assertion_transport,
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
    signer: PublicKey,
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
        CorporateIdentityProof::Direct { claims, source, .. } => {
            let binding = committed_binding.ok_or_else(|| {
                buzz_db::DbError::InvalidData(
                    "atomic identity admission did not return a binding result".to_string(),
                )
            })?;
            complete_direct_corporate_identity(state, community_id, signer, claims, source, binding)
                .await
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
        CorporateIdentityProof::Direct { claims, source, .. } => {
            let binding = state
                .db
                .bind_or_validate_identity(
                    community_id,
                    &claims.issuer,
                    &claims.uid,
                    signer.as_bytes(),
                    Some(&claims.display_name),
                    source,
                )
                .await?;
            complete_direct_corporate_identity(state, community_id, signer, claims, source, binding)
                .await
        }
    }
}

async fn complete_direct_corporate_identity(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    claims: CorporateJwtClaims,
    source: &'static str,
    binding: BindIdentityResult,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    let binding = match binding {
        BindIdentityResult::Conflict(_) => {
            metrics::counter!("buzz_corporate_identity_bindings_total", "result" => "conflict")
                .increment(1);
            record_identity_binding_audit(
                state,
                community_id,
                buzz_audit::AuditAction::CorporateIdentityBindingConflict,
                signer,
                &claims.issuer,
                &claims.uid,
                serde_json::json!({
                    "source": source,
                }),
            )
            .await;
            warn!("corporate identity binding conflict");
            return Err(CorporateIdentityError::BindingConflict);
        }
        BindIdentityResult::Revoked => {
            metrics::counter!("buzz_corporate_identity_bindings_total", "result" => "revoked")
                .increment(1);
            record_identity_binding_audit(
                state,
                community_id,
                buzz_audit::AuditAction::CorporateIdentityBindingRevokedAttempt,
                signer,
                &claims.issuer,
                &claims.uid,
                serde_json::json!({ "source": source, "issuer": claims.issuer }),
            )
            .await;
            warn!("corporate identity binding was previously revoked");
            return Err(CorporateIdentityError::BindingRevoked);
        }
        BindIdentityResult::BindingRequired => {
            metrics::counter!(
                "buzz_corporate_identity_bindings_total",
                "result" => "binding_required"
            )
            .increment(1);
            warn!("corporate identity binding requires sealed enrollment evidence");
            return Err(CorporateIdentityError::BindingRequired);
        }
        binding => binding,
    };
    record_identity_binding_metric(&binding);
    if matches!(binding, BindIdentityResult::Created) {
        record_identity_binding_audit(
            state,
            community_id,
            buzz_audit::AuditAction::CorporateIdentityBindingCreated,
            signer,
            &claims.issuer,
            &claims.uid,
            serde_json::json!({ "source": source, "issuer": claims.issuer }),
        )
        .await;
    }
    let projection_mode = state
        .protected_transport()
        .and_then(|runtime| runtime.mode_for_domain(community_id));
    if public_projection_mutation_enabled(projection_mode) {
        if let Err(_error) = ensure_identity_assertion(
            state,
            community_id,
            signer,
            &claims.issuer,
            &claims.uid,
            claims.public_display_name.as_deref(),
            claims.expires_at,
        )
        .await
        {
            // The binding remains the authorization authority. A projection
            // failure removes the verified affordance but must not lock an
            // otherwise authorized user out of the relay.
            warn!("failed to publish corporate identity assertion");
            metrics::counter!("buzz_corporate_identity_assertions_total", "result" => "error")
                .increment(1);
        }
    }

    debug!(source, "corporate identity verified");
    Ok(CorporateIdentityDecision::Direct {
        issuer: claims.issuer,
        uid: claims.uid,
        display_name: claims.display_name,
        expires_at: claims.expires_at,
        binding,
    })
}

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

fn identity_assertion_matches(
    event: &Event,
    subject: &str,
    display_name: Option<&str>,
    expires_at: u64,
) -> bool {
    let exact_tag_count = |name: &str, value: &str| {
        event
            .tags
            .iter()
            .filter(|tag| {
                let parts = tag.as_slice();
                parts.len() == 2 && parts[0] == name && parts[1] == value
            })
            .count()
    };
    event.content.is_empty()
        && event.tags.len() == if display_name.is_some() { 6 } else { 5 }
        && exact_tag_count("d", subject) == 1
        && exact_tag_count("p", subject) == 1
        && exact_tag_count("verified", "relay") == 1
        && exact_tag_count(
            "active",
            if display_name.is_some() {
                "true"
            } else {
                "false"
            },
        ) == 1
        && exact_tag_count("expiration", &expires_at.to_string()) == 1
        && match display_name {
            Some(name) => exact_tag_count("display_name", name) == 1,
            None => !event.tags.iter().any(|tag| {
                tag.as_slice()
                    .first()
                    .is_some_and(|part| part == "display_name")
            }),
        }
}

fn identity_assertion_has_base_shape(
    event: &Event,
    relay_author: PublicKey,
    subject: &str,
) -> bool {
    let has_tag = |name: &str, value: &str| {
        event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == name && parts[1] == value
        })
    };
    event.content.is_empty()
        && event.kind.as_u16() as u32 == KIND_USER_TRUSTED_ASSERTION
        && event.pubkey == relay_author
        && event.verify_id()
        && event.verify_signature()
        && has_tag("d", subject)
        && has_tag("p", subject)
        && has_tag("verified", "relay")
}

async fn ensure_identity_assertion(
    state: &AppState,
    community_id: CommunityId,
    subject: PublicKey,
    issuer: &str,
    uid: &str,
    display_name: Option<&str>,
    jwt_expires_at: u64,
) -> Result<(), String> {
    let subject_hex = subject.to_hex();
    let permit = buzz_db::public_projection::begin_active_public_projection(
        &state.db,
        community_id,
        state.relay_keypair.public_key().as_bytes(),
        issuer,
        uid,
        subject.as_bytes(),
    )
    .await
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "identity binding changed before public projection".to_owned())?;
    let existing = permit.current_projection().cloned();

    // Privacy default: do not publish any assertion unless the operator opted
    // into a public label. An inactive replacement is emitted only to retire a
    // previously published assertion after that opt-in is removed.
    if display_name.is_none() && existing.is_none() {
        return Ok(());
    }

    let now = Timestamp::now().as_secs();
    let expires_at = identity_assertion_expiration(display_name, jwt_expires_at, now);
    if let Some(existing_match) = existing.as_ref().filter(|stored| {
        identity_assertion_matches(&stored.event, &subject_hex, display_name, expires_at)
    }) {
        permit
            .commit(
                &existing_match.event,
                if display_name.is_some() {
                    buzz_db::public_projection::ProjectionDisposition::Active
                } else {
                    buzz_db::public_projection::ProjectionDisposition::Inactive
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let created_at = existing
        .as_ref()
        .map(|stored| stored.event.created_at.as_secs().saturating_add(1))
        .unwrap_or(now)
        .max(now);
    let event = build_identity_assertion(
        &state.relay_keypair,
        subject,
        display_name,
        expires_at,
        Timestamp::from(created_at),
    )?;

    permit
        .commit(
            &event,
            if display_name.is_some() {
                buzz_db::public_projection::ProjectionDisposition::Active
            } else {
                buzz_db::public_projection::ProjectionDisposition::Inactive
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    metrics::counter!("buzz_corporate_identity_assertions_total", "result" => "published")
        .increment(1);
    Ok(())
}

fn identity_assertion_expiration(display_name: Option<&str>, jwt_expires_at: u64, now: u64) -> u64 {
    if display_name.is_some() {
        jwt_expires_at.min(now.saturating_add(IDENTITY_ASSERTION_MAX_TTL_SECS))
    } else {
        0
    }
}

/// Internal O4 reconciliation failure. This carries no identity or token data.
#[derive(Debug, Error)]
pub(crate) enum PublicProjectionReconciliationError {
    /// Durable projection state was unavailable or inconsistent.
    #[error("public identity projection database state is unavailable")]
    Database(#[from] buzz_db::DbError),
    /// Shared authorization time was unavailable.
    #[error(transparent)]
    Clock(#[from] AuthorizationClockError),
    /// A stored event did not satisfy the exact public projection contract.
    #[error("stored public identity projection is invalid")]
    InvalidProjection,
    /// The relay could not construct the canonical inactive event.
    #[error("failed to construct inactive public identity projection")]
    Build,
    /// A configured domain lacked its durable tenant mapping.
    #[error("public identity projection domain is unavailable")]
    DomainUnavailable,
    /// Cross-replica withdrawal delivery failed and remains retryable.
    #[error("public identity projection delivery is unavailable")]
    DeliveryUnavailable,
    /// Startup did not reach a complete reconciliation fixed point.
    #[error("public identity projection reconciliation is incomplete")]
    Incomplete,
}

async fn reconcile_one_public_projection(
    state: &AppState,
    domains: &[CommunityId],
) -> Result<bool, PublicProjectionReconciliationError> {
    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    buzz_db::public_projection::materialize_public_projection_retirements(
        &state.db,
        domains,
        relay_pubkey.as_slice(),
    )
    .await?;

    if let Some(claim) = buzz_db::public_projection::claim_public_projection_retirement(
        &state.db,
        domains,
        relay_pubkey.as_slice(),
    )
    .await?
    {
        let permit =
            buzz_db::public_projection::begin_public_projection_retirement(&state.db, claim)
                .await?;
        let Some(current) = permit.current_projection().cloned() else {
            permit.finish_no_projection().await?;
            return Ok(true);
        };
        let old_key = match PublicKey::from_slice(permit.old_pubkey()) {
            Ok(key) => key,
            Err(_) => {
                permit.defer().await?;
                return Err(PublicProjectionReconciliationError::InvalidProjection);
            }
        };
        let subject = old_key.to_hex();
        if !identity_assertion_has_base_shape(
            &current.event,
            state.relay_keypair.public_key(),
            &subject,
        ) {
            permit.defer().await?;
            return Err(PublicProjectionReconciliationError::InvalidProjection);
        }
        let current_is_inactive = identity_assertion_matches(&current.event, &subject, None, 0);
        if permit.head().is_some_and(|head| {
            head.event_id() != *current.event.id.as_bytes()
                || head.disposition()
                    != if current_is_inactive {
                        buzz_db::public_projection::ProjectionDisposition::Inactive
                    } else {
                        buzz_db::public_projection::ProjectionDisposition::Active
                    }
        }) {
            permit.defer().await?;
            return Err(PublicProjectionReconciliationError::InvalidProjection);
        }
        if current_is_inactive {
            permit.finish_existing_inactive().await?;
            return Ok(true);
        }
        let head_origin = permit.head().and_then(|head| head.origin());
        if let Some(active_origin) = permit.active_origin() {
            if permit.source_origin() == Some(active_origin) {
                permit.defer().await?;
                return Err(PublicProjectionReconciliationError::InvalidProjection);
            }
            if head_origin == Some(active_origin) {
                permit.finish_superseded(&current.event).await?;
                return Ok(true);
            }
        }
        if let (Some(source), Some(origin)) = (permit.source_origin(), head_origin) {
            let belongs_to_later_generation = if origin.binding_id() == source.binding_id() {
                origin.binding_version() > source.binding_version()
            } else {
                true
            };
            if belongs_to_later_generation {
                permit.finish_newer_projection().await?;
                return Ok(true);
            }
        }
        let now = SystemAuthorizationClock.now()?.unix_seconds();
        let Some(next_created_at) = current.event.created_at.as_secs().checked_add(1) else {
            permit.defer().await?;
            return Err(PublicProjectionReconciliationError::Build);
        };
        let inactive = match build_identity_assertion(
            &state.relay_keypair,
            old_key,
            None,
            0,
            Timestamp::from(next_created_at.max(now)),
        ) {
            Ok(event) => event,
            Err(_) => {
                permit.defer().await?;
                return Err(PublicProjectionReconciliationError::Build);
            }
        };
        permit.finish_inactive(&inactive).await?;
        return Ok(true);
    }

    let Some(delivery) = buzz_db::public_projection::begin_public_projection_delivery(
        &state.db,
        domains,
        relay_pubkey.as_slice(),
    )
    .await?
    else {
        return Ok(false);
    };
    let domain = delivery.community_id();
    let tenant = state
        .db
        .usage_community_hosts()
        .await?
        .into_iter()
        .find(|entry| CommunityId::from_uuid(entry.id) == domain)
        .map(|entry| buzz_core::TenantContext::resolved(domain, entry.host));
    let Some(tenant) = tenant else {
        delivery.defer().await?;
        return Err(PublicProjectionReconciliationError::DomainUnavailable);
    };
    state.mark_local_event(domain, &delivery.stored().event.id);
    if state
        .pubsub
        .publish_event(&tenant, EventTopic::Global, &delivery.stored().event)
        .await
        .is_err()
    {
        state
            .local_event_ids
            .invalidate(&(domain, delivery.stored().event.id.to_bytes()));
        delivery.defer().await?;
        return Err(PublicProjectionReconciliationError::DeliveryUnavailable);
    }
    crate::handlers::event::fan_out_event_to_local_subscribers(state, domain, delivery.stored())
        .await;
    delivery.complete().await?;
    Ok(true)
}

/// Drain every materialized retirement before protected routes become reachable.
pub(crate) async fn reconcile_public_projection_retirements_startup(
    state: &AppState,
    domains: &[CommunityId],
) -> Result<(), PublicProjectionReconciliationError> {
    for _ in 0..PUBLIC_PROJECTION_STARTUP_LIMIT {
        if !reconcile_one_public_projection(state, domains).await? {
            break;
        }
    }
    let unfinished = buzz_db::public_projection::unfinished_public_projection_retirements(
        &state.db,
        domains,
        state.relay_keypair.public_key().as_bytes(),
    )
    .await?;
    if unfinished != 0 {
        return Err(PublicProjectionReconciliationError::Incomplete);
    }
    Ok(())
}

/// Continuously discover committed lifecycle rows and retry withdrawal/delivery.
pub(crate) async fn run_public_projection_retirement_reconciliation(
    state: Arc<AppState>,
    domains: Vec<CommunityId>,
) {
    loop {
        match reconcile_one_public_projection(state.as_ref(), &domains).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "public identity projection reconciliation deferred")
            }
        }
        tokio::time::sleep(PUBLIC_PROJECTION_RECONCILIATION_INTERVAL).await;
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
        if let Some(relationship) = verify_unconditional_nip_oa_relationship(signer, auth_tag_json)
        {
            let owner_pubkey = relationship.owner_pubkey();
            let owner_binding = db
                .get_active_identity_binding_by_pubkey(community_id, owner_pubkey.as_bytes())
                .await?;
            if let Some(owner_binding) = owner_binding {
                debug!("corporate identity granted via NIP-OA owner binding");
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

/// Exact verified identity and revision of one immutable NIP-OA relationship.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VerifiedNipOaRelationship {
    owner_pubkey: PublicKey,
    relationship_id: uuid::Uuid,
    relationship_revision: u64,
}

impl VerifiedNipOaRelationship {
    /// Verified owner that signed the relationship.
    pub const fn owner_pubkey(self) -> PublicKey {
        self.owner_pubkey
    }

    /// Domain-separated identity of the exact verified signed relationship.
    pub const fn relationship_id(self) -> uuid::Uuid {
        self.relationship_id
    }

    /// Monotonic revision of this immutable relationship issuance.
    pub const fn relationship_revision(self) -> u64 {
        self.relationship_revision
    }
}

impl fmt::Debug for VerifiedNipOaRelationship {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedNipOaRelationship")
            .field("owner_pubkey", &"[redacted]")
            .field("relationship_id", &"[redacted]")
            .field("relationship_revision", &"[redacted]")
            .finish()
    }
}

/// Verify and identify an unconditional NIP-OA relationship.
///
/// The exact relationship ID is derived only after signature verification
/// from the canonical signed tag and authenticated delegate key. Each signed
/// immutable issuance starts at revision one; a different issuance receives a
/// different relationship ID rather than reusing an owner-wide selector.
pub fn verify_unconditional_nip_oa_relationship(
    signer: PublicKey,
    auth_tag_json: Option<&str>,
) -> Option<VerifiedNipOaRelationship> {
    let tag_json = auth_tag_json?;
    let tag: Vec<Value> = serde_json::from_str(tag_json).ok()?;
    if tag.len() != 4 || tag.get(2).and_then(Value::as_str) != Some("") {
        return None;
    }
    let owner_pubkey = buzz_sdk::nip_oa::verify_auth_tag(tag_json, &signer).ok()?;
    let canonical_tag = serde_json::to_vec(&tag).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(b"buzz:nip-oa:delegated-relationship:v1");
    hasher.update(signer.to_bytes());
    hasher.update((canonical_tag.len() as u64).to_be_bytes());
    hasher.update(canonical_tag);
    let digest = hasher.finalize();
    let mut identity = [0_u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity[6] = (identity[6] & 0x0f) | 0x80;
    identity[8] = (identity[8] & 0x3f) | 0x80;
    Some(VerifiedNipOaRelationship {
        owner_pubkey,
        relationship_id: uuid::Uuid::from_bytes(identity),
        relationship_revision: 1,
    })
}

/// Verify an unconditional NIP-OA owner attestation for transport-wide use.
///
/// Conditional attestations are deliberately rejected because their narrower
/// event constraints cannot be promoted into connection or request authority.
pub fn verify_unconditional_nip_oa_owner(
    signer: PublicKey,
    auth_tag_json: Option<&str>,
) -> Option<PublicKey> {
    verify_unconditional_nip_oa_relationship(signer, auth_tag_json)
        .map(VerifiedNipOaRelationship::owner_pubkey)
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
                warn!("corporate identity JWT npub claim does not match signer");
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

fn claim_string_exact(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<String, CorporateIdentityError> {
    claims
        .get(claim)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: "must be a non-empty literal string".to_string(),
        })
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

fn validate_optional_iat(
    claims: &Map<String, Value>,
    verifier_time: u64,
) -> Result<(), CorporateIdentityError> {
    let Some(value) = claims.get("iat") else {
        return Ok(());
    };
    let issued_at = value
        .as_u64()
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: "iat".to_string(),
            reason: "must be an unsigned integer when present".to_string(),
        })?;
    if issued_at > verifier_time.saturating_add(JWT_CLOCK_SKEW_LEEWAY_SECS) {
        return Err(CorporateIdentityError::InvalidClaim {
            claim: "iat".to_string(),
            reason: "must not be later than verifier time plus bounded skew".to_string(),
        });
    }
    Ok(())
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
        .verifier_configured()
        .then(|| Arc::new(CorporateIdentityService::new(config.clone())))
}

fn record_identity_binding_metric(binding: &BindIdentityResult) {
    let result = match binding {
        BindIdentityResult::Created => "created",
        BindIdentityResult::Matched => "matched",
        BindIdentityResult::Conflict(_) => "conflict",
        BindIdentityResult::Revoked => "revoked",
        BindIdentityResult::BindingRequired => "binding_required",
    };
    metrics::counter!("buzz_corporate_identity_bindings_total", "result" => result).increment(1);
}

const fn public_projection_mutation_enabled(
    mode: Option<crate::authorization_runtime::finalization::AuthorizationMode>,
) -> bool {
    use crate::authorization_runtime::finalization::AuthorizationMode;

    matches!(
        mode,
        None | Some(AuthorizationMode::Off) | Some(AuthorizationMode::Enforce)
    )
}

fn record_corporate_identity_denial(error: &CorporateIdentityError) {
    metrics::counter!("buzz_auth_failures_total", "reason" => "corporate_identity_denied")
        .increment(1);
    metrics::counter!(
        "buzz_corporate_identity_denials_total",
        "reason" => error.reason_code()
    )
    .increment(1);
}

async fn record_identity_binding_audit(
    state: &AppState,
    community_id: CommunityId,
    action: buzz_audit::AuditAction,
    actor: PublicKey,
    issuer: &str,
    uid: &str,
    detail: serde_json::Value,
) {
    if crate::protected_surface::require_effect_permit(
        state
            .protected_transport()
            .and_then(|runtime| runtime.mode_for_domain(community_id)),
        crate::protected_surface::EffectSurfaceId::LegacyAuditDelivery,
    )
    .is_err()
    {
        return;
    }
    let Some(audit_tx) = &state.audit_tx else {
        return;
    };
    if let Err(e) = audit_tx
        .send(buzz_audit::NewAuditEntry {
            community_id,
            action,
            actor_pubkey: Some(actor.to_bytes().to_vec()),
            object_id: Some(format!("{issuer}|{uid}")),
            detail,
        })
        .await
    {
        warn!("Corporate identity audit channel closed — entry lost: {e}");
        metrics::counter!("buzz_audit_send_errors_total").increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use base64::Engine as _;
    use buzz_auth::{AuthorizationClock, AuthorizationTime};
    use jsonwebtoken::jwk::JwkSet;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use nostr::Keys;
    use sqlx::PgPool;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    struct FixedAuthorizationClock(u64);

    impl AuthorizationClock for FixedAuthorizationClock {
        fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
            Ok(AuthorizationTime::from_unix_seconds(self.0))
        }
    }

    #[test]
    fn o4_shared_contract_repairs_are_redacted_and_revision_exact() {
        let sentinel_key = "private_raw_claim_key";
        let sentinel_value = "private_raw_claim_value";
        let raw_claims = RawJwtClaims {
            claims: Map::from_iter([(
                sentinel_key.to_string(),
                Value::String(sentinel_value.to_string()),
            )]),
        };
        let debug = format!("{raw_claims:?}");
        let delegation_evidence = include_str!("../../buzz-auth/src/context/evidence.rs");
        let invalidation_contract = include_str!("../../buzz-db/src/authorization_invalidation.rs");

        let mut missing = Vec::new();
        if debug.contains(sentinel_key) || debug.contains(sentinel_value) {
            missing.push("raw-jwt-debug-redaction");
        }
        if !(delegation_evidence.contains("DelegatedRelationshipId")
            && delegation_evidence.contains("DelegatedRelationshipRevision")
            && delegation_evidence.contains("relationship_id")
            && delegation_evidence.contains("relationship_revision"))
        {
            missing.push("delegated-relationship-identity-and-revision");
        }
        if !(invalidation_contract.contains("AuthorizationSessionTarget")
            && invalidation_contract.contains("issuance_fence")
            && invalidation_contract.contains("session-issuance-v2"))
        {
            missing.push("session-issuance-nonreuse-fence");
        }

        assert!(
            missing.is_empty(),
            "missing O4 shared-contract repairs: {missing:?}"
        );

        let session_id = Uuid::from_u128(0x801);
        let first_session = buzz_db::authorization_invalidation::AuthorizationSessionTarget::new(
            session_id,
            Uuid::from_u128(0x802),
        )
        .expect("first session issuance is valid");
        let second_session = buzz_db::authorization_invalidation::AuthorizationSessionTarget::new(
            session_id,
            Uuid::from_u128(0x803),
        )
        .expect("second session issuance is valid");
        let first_session_fingerprint =
            buzz_db::authorization_invalidation::AuthorizationSelector::session(first_session)
                .fingerprint();
        let second_session_fingerprint =
            buzz_db::authorization_invalidation::AuthorizationSelector::session(second_session)
                .fingerprint();
        assert_ne!(first_session_fingerprint, second_session_fingerprint);

        let relationship_id = Uuid::from_u128(0x804);
        let first_relationship =
            buzz_db::authorization_invalidation::AuthorizationSelector::delegated_relationship(
                relationship_id,
                1,
            )
            .expect("first relationship revision is valid");
        let second_relationship =
            buzz_db::authorization_invalidation::AuthorizationSelector::delegated_relationship(
                relationship_id,
                2,
            )
            .expect("second relationship revision is valid");
        assert_ne!(
            first_relationship.fingerprint(),
            second_relationship.fingerprint()
        );
    }

    #[test]
    fn observational_modes_never_mutate_the_public_projection() {
        use crate::authorization_runtime::finalization::AuthorizationMode;

        assert!(public_projection_mutation_enabled(None));
        assert!(public_projection_mutation_enabled(Some(
            AuthorizationMode::Off
        )));
        assert!(public_projection_mutation_enabled(Some(
            AuthorizationMode::Enforce
        )));
        assert!(!public_projection_mutation_enabled(Some(
            AuthorizationMode::Shadow
        )));
        assert!(!public_projection_mutation_enabled(Some(
            AuthorizationMode::VerifyOnly
        )));
    }

    #[test]
    fn identity_debug_never_exposes_provider_or_principal_data() {
        let pubkey = Keys::generate().public_key();
        let claims = CorporateJwtClaims {
            issuer: "https://private-issuer.invalid".into(),
            uid: "private-subject".into(),
            display_name: "Private Person".into(),
            public_display_name: Some("Public Label".into()),
            pubkey: Some(pubkey),
            expires_at: 1_234_567,
        };
        let proof = CorporateIdentityProof::Direct {
            claims: claims.clone(),
            source: "private-source",
            assertion_transport: Some(buzz_auth::AssertionTransport::TrustedProxy),
        };
        let decision = CorporateIdentityDecision::Delegated {
            owner_pubkey: pubkey,
            owner_issuer: claims.issuer.clone(),
            owner_uid: claims.uid.clone(),
        };
        let error = CorporateIdentityError::InvalidClaim {
            claim: "private-claim-name".into(),
            reason: "private-claim-value".into(),
        };

        for debug in [
            format!("{claims:?}"),
            format!("{proof:?}"),
            format!("{decision:?}"),
            format!("{error:?}"),
        ] {
            for secret in [
                "private-issuer",
                "private-subject",
                "Private Person",
                "Public Label",
                "private-source",
                "private-claim-name",
                "private-claim-value",
                &pubkey.to_hex(),
                "1234567",
            ] {
                assert!(
                    !debug.contains(secret),
                    "debug output exposed {secret:?}: {debug}"
                );
            }
        }
    }

    #[test]
    fn direct_identity_requires_verified_transport_provenance_before_sealing() {
        let claims = CorporateJwtClaims {
            issuer: "https://issuer.example".into(),
            uid: "synthetic-subject".into(),
            display_name: "Synthetic User".into(),
            public_display_name: None,
            pubkey: Some(Keys::generate().public_key()),
            expires_at: 2_000,
        };
        let domain = CommunityId::from_uuid(Uuid::nil());
        let unproved = CorporateIdentityProof::Direct {
            claims: claims.clone(),
            source: SOURCE_JWT_NPUB,
            assertion_transport: None,
        };

        assert!(matches!(
            verified_assertion_for_proof(
                &unproved,
                domain,
                buzz_auth::AuthTransport::RelayWebSocket,
                1_000,
            ),
            Err(CorporateIdentityError::Evidence(_))
        ));

        let proved = CorporateIdentityProof::Direct {
            claims,
            source: SOURCE_JWT_NPUB,
            assertion_transport: Some(buzz_auth::AssertionTransport::TrustedProxy),
        };
        let assertion = verified_assertion_for_proof(
            &proved,
            domain,
            buzz_auth::AuthTransport::RelayWebSocket,
            1_000,
        )
        .expect("synthetic proved assertion should seal")
        .expect("direct identity should produce a sealed assertion");

        assert_eq!(
            assertion.transport(),
            buzz_auth::AssertionTransport::TrustedProxy
        );
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

    #[test]
    fn issuer_and_subject_claims_preserve_literal_identity() {
        let claims = serde_json::json!({"iss": " issuer ", "sub": " subject "});
        let claims = claims.as_object().expect("claims object");

        assert_eq!(claim_string_exact(claims, "iss").unwrap(), " issuer ");
        assert_eq!(claim_string_exact(claims, "sub").unwrap(), " subject ");
        assert!(claim_string_exact(claims, "missing").is_err());
    }

    fn test_identity_binding(
        issuer: &str,
        uid: &str,
        pubkey: PublicKey,
    ) -> buzz_db::identity_binding::IdentityBinding {
        let now = chrono::Utc::now();
        buzz_db::identity_binding::IdentityBinding {
            binding_id: uuid::Uuid::new_v4(),
            issuer: issuer.to_string(),
            uid: uid.to_string(),
            pubkey: pubkey.to_bytes().to_vec(),
            binding_version: 1,
            binding_state: buzz_db::identity_binding::BindingState::Active,
            binding_provenance: buzz_db::identity_binding::BindingProvenance::Tofu,
            creation_attribution:
                buzz_db::identity_binding::CreationAttributionKind::AuthenticatedKey,
            created_by: Some(pubkey.to_bytes().to_vec()),
            created_policy_version: Some("relay-test-policy-v1".to_owned()),
            expires_at: None,
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

        let subject_hex = subject.to_hex();
        let malformed = EventBuilder::new(Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
            .tags([
                Tag::parse(["d", subject_hex.as_str()]).expect("d tag"),
                Tag::parse(["p", subject_hex.as_str()]).expect("p tag"),
                Tag::parse(["verified", "relay"]).expect("verified tag"),
                Tag::parse(["active", "false"]).expect("inactive tag"),
                Tag::parse(["expiration", "0"]).expect("expiration tag"),
                Tag::parse(["display_name", "Forbidden stale label"]).expect("display tag"),
            ])
            .custom_created_at(Timestamp::from(now + 1))
            .sign_with_keys(&relay)
            .expect("sign malformed inactive assertion");
        assert!(!identity_assertion_matches(
            &malformed,
            &subject_hex,
            None,
            0,
        ));

        for (display_name, expires_at, at) in [
            (Some("Example User"), now + 60, now + 2),
            (None, 0, now + 3),
        ] {
            let canonical = build_identity_assertion(
                &relay,
                subject,
                display_name,
                expires_at,
                Timestamp::from(at),
            )
            .expect("build canonical assertion");
            let nonempty = EventBuilder::new(
                Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16),
                "private content must never be projected",
            )
            .tags(canonical.tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(&relay)
            .expect("sign non-canonical assertion");
            assert!(!identity_assertion_matches(
                &nonempty,
                &subject_hex,
                display_name,
                expires_at,
            ));
            assert!(!identity_assertion_has_base_shape(
                &nonempty,
                relay.public_key(),
                &subject_hex,
            ));
        }
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
            .validate_jwt(&token)
            .await
            .expect_err("HS256 must be rejected");
        assert!(matches!(err, CorporateIdentityError::InvalidJwt(_)));
    }

    #[tokio::test]
    async fn validate_jwt_accepts_matching_rs256_jwk() {
        let key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let token = rsa_test_jwt(&key, "rsa-key");
        let claims = validate_rsa_jwt(&token, rsa_test_jwk(&key, "rsa-key"))
            .await
            .expect("matching RSA JWT must validate");

        assert_eq!(claims.uid, "user-1");
        assert_eq!(claims.display_name, "user@example.com");
    }

    #[tokio::test]
    async fn validate_jwt_rejects_future_and_malformed_optional_iat() {
        let key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let now = 2_000_000_000;
        let clock: SharedAuthorizationClock = Arc::new(FixedAuthorizationClock(now));

        let mut within_skew = valid_test_claims(now);
        within_skew["iat"] = Value::from(now + JWT_CLOCK_SKEW_LEEWAY_SECS);
        let token = rsa_test_jwt_with_claims(&key, "rsa-key", &within_skew);
        validate_rsa_jwt_at(&token, rsa_test_jwk(&key, "rsa-key"), Arc::clone(&clock))
            .await
            .expect("optional iat within bounded skew validates");

        for iat in [
            Value::from(now + JWT_CLOCK_SKEW_LEEWAY_SECS + 600),
            Value::String("tomorrow".to_string()),
        ] {
            let mut claims = valid_test_claims(now);
            claims["iat"] = iat;
            let token = rsa_test_jwt_with_claims(&key, "rsa-key", &claims);
            let error =
                validate_rsa_jwt_at(&token, rsa_test_jwk(&key, "rsa-key"), Arc::clone(&clock))
                    .await
                    .expect_err("future or malformed optional iat must fail closed");
            match error {
                CorporateIdentityError::InvalidClaim { claim, .. } => assert_eq!(claim, "iat"),
                CorporateIdentityError::InvalidJwt(_) => {}
                other => panic!("unexpected optional iat error: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn validate_jwt_rejects_rs256_token_signed_by_wrong_key() {
        let signing_key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let advertised_key = rsa_private_key(include_str!("testdata/rsa_private_key_2.der.b64"));
        let token = rsa_test_jwt(&signing_key, "rsa-key");

        let error = validate_rsa_jwt(&token, rsa_test_jwk(&advertised_key, "rsa-key"))
            .await
            .expect_err("JWT signed by another RSA key must fail");
        assert!(matches!(error, CorporateIdentityError::InvalidJwt(_)));
    }

    #[tokio::test]
    async fn validate_jwt_rejects_jwk_advertised_algorithm_mismatch() {
        let key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let token = rsa_test_jwt(&key, "rsa-key");
        let mut jwk = rsa_test_jwk(&key, "rsa-key");
        jwk.common.key_algorithm = Some(KeyAlgorithm::RS512);

        let error = validate_rsa_jwt(&token, jwk)
            .await
            .expect_err("JWK alg must agree with JWT alg");
        assert!(matches!(
            error,
            CorporateIdentityError::InvalidJwt(ref message)
                if message.contains("does not match JWK algorithm")
        ));
    }

    #[tokio::test]
    async fn validate_jwt_accepts_jwk_with_omitted_algorithm() {
        let key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let token = rsa_test_jwt(&key, "rsa-key");
        let mut jwk = rsa_test_jwk(&key, "rsa-key");
        jwk.common.key_algorithm = None;

        validate_rsa_jwt(&token, jwk)
            .await
            .expect("an omitted optional JWK alg must not prevent RSA verification");
    }

    #[test]
    fn validate_jwk_requires_signature_use_and_verify_operation_when_present() {
        let key = rsa_private_key(include_str!("testdata/rsa_private_key_1.der.b64"));
        let mut jwk = rsa_test_jwk(&key, "rsa-key");
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
            decode_test_jwt(claims, Algorithm::HS256, b"test-secret", b"test-secret")
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
                    Algorithm::HS256,
                    b"test-secret",
                    b"test-secret",
                )
                .is_err(),
                "missing {claim} must fail closed",
            );
        }
    }

    #[test]
    fn jwt_validation_pins_clock_skew_leeway() {
        let validation = jwt_validation(Algorithm::RS256, &test_config());
        assert_eq!(validation.leeway, JWT_CLOCK_SKEW_LEEWAY_SECS);
    }

    #[test]
    fn optional_iat_is_not_required_and_is_bounded_by_verifier_time() {
        let now = 1_800_000_000;
        let mut claims = valid_test_claims(now)
            .as_object()
            .expect("claims object")
            .clone();
        assert!(validate_optional_iat(&claims, now).is_ok());

        claims.insert(
            "iat".to_string(),
            Value::from(now + JWT_CLOCK_SKEW_LEEWAY_SECS),
        );
        assert!(validate_optional_iat(&claims, now).is_ok());

        for invalid in [
            Value::from(now + JWT_CLOCK_SKEW_LEEWAY_SECS + 1),
            Value::String("tomorrow".to_string()),
            Value::from(-1),
            Value::from(1.5),
            Value::Null,
        ] {
            claims.insert("iat".to_string(), invalid);
            assert!(matches!(
                validate_optional_iat(&claims, now),
                Err(CorporateIdentityError::InvalidClaim { ref claim, .. }) if claim == "iat"
            ));
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
                    Algorithm::HS256,
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
            decode_test_jwt(claims, Algorithm::HS256, b"test-secret", b"test-secret")
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
                    Algorithm::HS256,
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
    fn rejects_ambiguous_identity_header_values() {
        let config = test_config();
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-buzz-identity-token"),
            HeaderValue::from_static("Bearer token-a, Bearer token-b"),
        );

        assert!(identity_jwt_from_headers(&headers, &config).is_err());

        headers.remove("x-buzz-identity-token");
        headers.append(
            HeaderName::from_static("x-buzz-identity-token"),
            HeaderValue::from_static("Bearer token-a"),
        );
        headers.append(
            HeaderName::from_static("x-buzz-identity-token"),
            HeaderValue::from_static("Bearer token-b"),
        );
        assert!(identity_jwt_from_headers(&headers, &config).is_err());
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
    async fn fresh_jwks_cache_miss_does_not_refetch() {
        let service = CorporateIdentityService::new(test_config());
        let fetched_at = Instant::now();
        *service.jwks.write().await = Some(CachedJwks {
            set: JwkSet { keys: Vec::new() },
            fetched_at,
            fresh_until: fetched_at + Duration::from_secs(60),
            hard_expires_at: fetched_at + JWKS_CACHE_MAX_AGE,
            refresh_after: fetched_at + Duration::from_secs(60),
        });

        let err = service
            .jwk_for_kid("attacker-controlled-kid")
            .await
            .expect_err("fresh cache miss should fail without network fetch");
        assert!(matches!(
            err,
            CorporateIdentityError::Jwks(ref msg) if msg.contains("fresh JWKS cache")
        ));
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
            service.jwk_for_kid("test-kid"),
            service.jwk_for_kid("test-kid"),
            service.jwk_for_kid("test-kid"),
            service.jwk_for_kid("test-kid"),
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
            verify_unconditional_nip_oa_owner(agent, Some(&unconditional)),
            Some(owner.public_key()),
        );
        assert_eq!(
            verify_unconditional_nip_oa_owner(agent, Some(&conditional)),
            None,
        );
    }

    fn valid_test_claims(now: u64) -> Value {
        serde_json::json!({
            "iss": "https://idp.example",
            "aud": "buzz-relay",
            "sub": "user-1",
            "email": "user@example.com",
            "exp": now + 3_600,
        })
    }

    fn decode_test_jwt(
        claims: Value,
        signing_algorithm: Algorithm,
        signing_key: &[u8],
        verification_key: &[u8],
    ) -> Result<(), jsonwebtoken::errors::Error> {
        let token = encode(
            &Header::new(signing_algorithm),
            &claims,
            &EncodingKey::from_secret(signing_key),
        )?;
        decode::<RawJwtClaims>(
            &token,
            &DecodingKey::from_secret(verification_key),
            &jwt_validation(Algorithm::HS256, &test_config()),
        )?;
        Ok(())
    }

    fn rsa_private_key(encoded: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .expect("decode RSA test key")
    }

    fn rsa_test_jwk(private_key: &[u8], kid: &str) -> Jwk {
        let encoding_key = EncodingKey::from_rsa_der(private_key);
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::RS256)
            .expect("derive RSA JWK from test key");
        jwk.common.key_id = Some(kid.to_string());
        jwk.common.public_key_use = Some(PublicKeyUse::Signature);
        jwk.common.key_operations = Some(vec![KeyOperations::Verify]);
        jwk
    }

    fn rsa_test_jwt(private_key: &[u8], kid: &str) -> String {
        rsa_test_jwt_with_claims(
            private_key,
            kid,
            &valid_test_claims(Timestamp::now().as_secs()),
        )
    }

    fn rsa_test_jwt_with_claims(private_key: &[u8], kid: &str, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        encode(&header, claims, &EncodingKey::from_rsa_der(private_key))
            .expect("encode RSA test JWT")
    }

    async fn validate_rsa_jwt(
        token: &str,
        jwk: Jwk,
    ) -> Result<CorporateJwtClaims, CorporateIdentityError> {
        validate_rsa_jwt_at(token, jwk, Arc::new(SystemAuthorizationClock)).await
    }

    async fn validate_rsa_jwt_at(
        token: &str,
        jwk: Jwk,
        clock: SharedAuthorizationClock,
    ) -> Result<CorporateJwtClaims, CorporateIdentityError> {
        let body =
            serde_json::to_string(&JwkSet { keys: vec![jwk] }).expect("serialize RSA test JWKS");
        let response = http_response("200 OK", &["Content-Type: application/json"], &body);
        let (uri, _requests, server) = spawn_http_server(response).await;
        let mut config = test_config();
        config.jwks_uri = uri;
        config.npub_claim = None;
        let result = CorporateIdentityService::with_authorization_clock(config, clock)
            .validate_jwt(token)
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
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
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

    async fn seed_projection_binding(
        pool: &PgPool,
        community: CommunityId,
        issuer: &str,
        subject: &str,
        pubkey: &[u8],
        display_name: &str,
    ) {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, display_name, source, binding_id, \
              binding_version, binding_state, binding_provenance, created_by, \
              created_policy_version, creation_attribution_kind) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,1,'active','attested_key',$4,$8,'authenticated_key')",
        )
        .bind(community.as_uuid())
        .bind(issuer)
        .bind(subject)
        .bind(pubkey)
        .bind(display_name)
        .bind(SOURCE_DB_BINDING)
        .bind(Uuid::new_v4())
        .bind("relay-projection-test-v1")
        .execute(pool)
        .await
        .expect("seed authoritative projection binding");
    }

    async fn projection_test_state(
        db: buzz_db::Db,
        pool: PgPool,
        redis_url: &str,
        relay_keys: Keys,
    ) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default test config");
        config.redis_url = redis_url.to_owned();
        let redis_pool = deadpool_redis::Config::from_url(redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("test Redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(redis_url, redis_pool.clone())
                .await
                .expect("test pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool);
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("test media store");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            relay_keys,
            media_storage,
        );
        Arc::new(state)
    }

    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn projection_worker_retries_after_restart_and_fans_out_canonical_withdrawal() {
        let (db, pool) = setup_db().await;
        let community = make_community(&pool).await;
        let relay_keys = Keys::generate();
        let subject_keys = Keys::generate();
        let subject = subject_keys.public_key();
        seed_projection_binding(
            &pool,
            community,
            "https://provider.example",
            "synthetic-subject",
            subject.as_bytes(),
            "Synthetic Label",
        )
        .await;
        let active = build_identity_assertion(
            &relay_keys,
            subject,
            Some("Synthetic Label"),
            Timestamp::now().as_secs().saturating_add(300),
            Timestamp::now(),
        )
        .expect("build active projection");
        let active_permit = match buzz_db::public_projection::begin_active_public_projection(
            &db,
            community,
            relay_keys.public_key().as_bytes(),
            "https://provider.example",
            "synthetic-subject",
            subject.as_bytes(),
        )
        .await
        {
            Ok(Some(permit)) => permit,
            Err(buzz_db::DbError::InvalidData(message))
                if message == "public projection storage is not installed" =>
            {
                // AB/CD deliberately expose only the fail-closed compatibility
                // contract. EF installs storage and exercises the full worker.
                return;
            }
            Ok(None) => panic!("active binding exists"),
            Err(error) => panic!("begin active projection: {error:?}"),
        };
        active_permit
            .commit(
                &active,
                buzz_db::public_projection::ProjectionDisposition::Active,
            )
            .await
            .expect("commit active projection");
        db.revoke_identity_key(
            community,
            buzz_db::identity_lifecycle::LifecycleOperationId::issue(),
            subject.as_bytes(),
            subject.as_bytes(),
            "synthetic revocation",
        )
        .await
        .expect("revoke synthetic key");

        let observational = make_community(&pool).await;
        let observational_keys = Keys::generate();
        let observational_subject = observational_keys.public_key();
        seed_projection_binding(
            &pool,
            observational,
            "https://observational-provider.example",
            "observational-subject",
            observational_subject.as_bytes(),
            "Observational Label",
        )
        .await;
        let observational_active = build_identity_assertion(
            &relay_keys,
            observational_subject,
            Some("Observational Label"),
            Timestamp::now().as_secs().saturating_add(300),
            Timestamp::now(),
        )
        .expect("build observational projection");
        buzz_db::public_projection::begin_active_public_projection(
            &db,
            observational,
            relay_keys.public_key().as_bytes(),
            "https://observational-provider.example",
            "observational-subject",
            observational_subject.as_bytes(),
        )
        .await
        .expect("begin observational projection")
        .expect("observational binding exists")
        .commit(
            &observational_active,
            buzz_db::public_projection::ProjectionDisposition::Active,
        )
        .await
        .expect("commit observational projection");
        db.revoke_identity_key(
            observational,
            buzz_db::identity_lifecycle::LifecycleOperationId::issue(),
            observational_subject.as_bytes(),
            observational_subject.as_bytes(),
            "observational revocation",
        )
        .await
        .expect("revoke observational key");

        let unavailable = projection_test_state(
            db.clone(),
            pool.clone(),
            "redis://127.0.0.1:1",
            relay_keys.clone(),
        )
        .await;
        assert!(matches!(
            reconcile_public_projection_retirements_startup(&unavailable, &[community]).await,
            Err(PublicProjectionReconciliationError::DeliveryUnavailable)
        ));
        assert_eq!(
            buzz_db::public_projection::unfinished_public_projection_retirements(
                &db,
                &[community],
                relay_keys.public_key().as_bytes(),
            )
            .await
            .expect("retryable retirement remains"),
            1
        );
        drop(unavailable);

        let live_redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let restarted = projection_test_state(
            db.clone(),
            pool.clone(),
            &live_redis_url,
            relay_keys.clone(),
        )
        .await;
        let tenant = restarted
            .db
            .usage_community_hosts()
            .await
            .expect("load synthetic tenant")
            .into_iter()
            .find(|entry| CommunityId::from_uuid(entry.id) == community)
            .map(|entry| buzz_core::TenantContext::resolved(community, entry.host))
            .expect("synthetic tenant exists");
        let mut received = restarted.pubsub.subscribe_local();
        restarted
            .pubsub
            .retain_topic(&tenant, EventTopic::Global)
            .await;
        let subscriber_state = Arc::clone(&restarted.pubsub);
        let subscriber = tokio::spawn(async move { subscriber_state.run_subscriber().await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let worker_state = Arc::clone(&restarted);
        let worker = tokio::spawn(async move {
            run_public_projection_retirement_reconciliation(worker_state, vec![community]).await
        });
        let withdrawal = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let event = received
                    .recv()
                    .await
                    .expect("projection fan-out remains open");
                if event.community_id == community
                    && event.topic == EventTopic::Global
                    && event.event.kind.as_u16() as u32 == KIND_USER_TRUSTED_ASSERTION
                {
                    break event.event;
                }
            }
        })
        .await
        .expect("restart retries and fans out the withdrawal");
        assert!(withdrawal.content.is_empty());
        assert!(identity_assertion_matches(
            &withdrawal,
            &subject.to_hex(),
            None,
            0
        ));
        let serialized_withdrawal =
            serde_json::to_string(&withdrawal).expect("serialize withdrawal");
        assert!(!serialized_withdrawal.contains("provider.example"));
        assert!(!serialized_withdrawal.contains("synthetic-subject"));
        let mut completed = false;
        for _ in 0..40 {
            if buzz_db::public_projection::unfinished_public_projection_retirements(
                &db,
                &[community],
                relay_keys.public_key().as_bytes(),
            )
            .await
            .expect("count completed retirement")
                == 0
            {
                completed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        worker.abort();
        subscriber.abort();
        assert!(
            completed,
            "withdrawal delivery did not complete exactly once"
        );
        let observational_head: String = sqlx::query_scalar(
            "SELECT disposition FROM identity_public_projection_heads \
             WHERE community_id=$1 AND relay_pubkey=$2 AND subject_pubkey=$3",
        )
        .bind(observational.as_uuid())
        .bind(relay_keys.public_key().as_bytes())
        .bind(observational_subject.as_bytes())
        .fetch_one(&pool)
        .await
        .expect("observational head remains");
        assert_eq!(observational_head, "active");
        assert_eq!(
            buzz_db::public_projection::unfinished_public_projection_retirements(
                &db,
                &[observational],
                relay_keys.public_key().as_bytes(),
            )
            .await
            .expect("observational mode has no retirement queue"),
            0
        );
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

        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, display_name, source, binding_id, \
              binding_version, binding_state, binding_provenance, created_by, \
              created_policy_version, creation_attribution_kind) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,1,'active','attested_key',$4,$8,'authenticated_key')",
        )
        .bind(community.as_uuid())
        .bind(&config.issuer)
        .bind("owner-uid")
        .bind(owner_keys.public_key().as_bytes())
        .bind("owner@example.com")
        .bind(SOURCE_DB_BINDING)
        .bind(Uuid::new_v4())
        .bind("relay-test-policy-v1")
        .execute(&pool)
        .await
        .expect("seed verified owner binding fixture");

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
