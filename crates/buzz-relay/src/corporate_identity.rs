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
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use nostr::{Event, EventBuilder, FromBech32, Kind, PublicKey, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use buzz_core::{kind::KIND_USER_TRUSTED_ASSERTION, CommunityId};
use buzz_db::event::EventQuery;
use buzz_db::identity_binding::{BindIdentityResult, SOURCE_DB_BINDING, SOURCE_JWT_NPUB};

use crate::config::CorporateIdentityConfig;
use crate::state::AppState;

const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
struct CachedJwks {
    set: JwkSet,
    expires_at: Instant,
}

/// Validated corporate identity claims used by Buzz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorporateJwtClaims {
    /// Stable corporate uid claim.
    pub uid: String,
    /// Human-readable verified identity claim.
    pub display_name: String,
    /// Optional pubkey carried by the IdP.
    pub pubkey: Option<PublicKey>,
}

#[derive(Debug, Deserialize)]
struct RawJwtClaims {
    #[serde(flatten)]
    claims: Map<String, Value>,
}

/// Service that verifies corporate identity JWTs against configured JWKS.
#[derive(Debug)]
pub struct CorporateIdentityService {
    config: CorporateIdentityConfig,
    http: reqwest::Client,
    jwks: RwLock<Option<CachedJwks>>,
}

impl CorporateIdentityService {
    /// Build a corporate identity verifier from relay config.
    pub fn new(config: CorporateIdentityConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            jwks: RwLock::new(None),
        }
    }

    /// Validate a JWT and extract the configured corporate identity claims.
    pub async fn validate_jwt(
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
        let decoding_key = DecodingKey::from_jwk(&jwk).map_err(|e| {
            CorporateIdentityError::InvalidJwt(format!("invalid JWK for kid {kid}: {e}"))
        })?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.set_audience(&[self.config.audience.as_str()]);

        let decoded = decode::<RawJwtClaims>(token, &decoding_key, &validation)
            .map_err(|e| CorporateIdentityError::InvalidJwt(e.to_string()))?;

        let uid = claim_string(&decoded.claims.claims, &self.config.uid_claim)?;
        let display_name = claim_string(&decoded.claims.claims, &self.config.display_claim)?;
        let pubkey =
            optional_pubkey_claim(&decoded.claims.claims, self.config.npub_claim.as_deref())?;

        Ok(CorporateJwtClaims {
            uid,
            display_name,
            pubkey,
        })
    }

    async fn jwk_for_kid(&self, kid: &str) -> Result<Jwk, CorporateIdentityError> {
        let now = Instant::now();
        {
            let cache = self.jwks.read().await;
            if let Some(cached) = cache.as_ref() {
                if cached.expires_at > now {
                    if let Some(jwk) = cached.set.find(kid) {
                        return Ok(jwk.clone());
                    }
                    return Err(CorporateIdentityError::Jwks(format!(
                        "kid not found in fresh JWKS cache: {kid}"
                    )));
                }
            }
        }

        let set = self.fetch_jwks().await?;
        let jwk = set.find(kid).cloned();
        *self.jwks.write().await = Some(CachedJwks {
            set,
            expires_at: Instant::now() + JWKS_CACHE_TTL,
        });
        jwk.ok_or_else(|| CorporateIdentityError::Jwks(format!("kid not found: {kid}")))
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, CorporateIdentityError> {
        let response = self
            .http
            .get(&self.config.jwks_uri)
            .send()
            .await
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))?
            .error_for_status()
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))?;
        response
            .json::<JwkSet>()
            .await
            .map_err(|e| CorporateIdentityError::Jwks(e.to_string()))
    }
}

/// Outcome of corporate identity enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorporateIdentityDecision {
    /// Corporate identity is disabled for this relay.
    NotRequired,
    /// The signer authenticated directly with a corporate identity JWT.
    Direct {
        /// Stable corporate uid claim.
        uid: String,
        /// Verified display claim.
        display_name: String,
        /// Binding operation outcome.
        binding: BindIdentityResult,
    },
    /// The signer is an agent admitted through a bound owner pubkey.
    Delegated {
        /// NIP-OA owner pubkey that already has an active corporate binding.
        owner_pubkey: PublicKey,
    },
}

/// Errors produced by corporate identity verification.
#[derive(Debug, Error)]
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
    /// NIP-OA delegation was present but did not satisfy corporate identity.
    #[error("corporate identity delegation denied")]
    DelegationDenied,
    /// Database operation failed.
    #[error("corporate identity database error: {0}")]
    Db(#[from] buzz_db::DbError),
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
            | Self::DelegationDenied => StatusCode::FORBIDDEN,
            Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Sanitized message safe to return to clients.
    pub fn public_message(&self) -> &'static str {
        match self {
            Self::MissingJwt => "corporate identity required",
            Self::MissingKid | Self::InvalidJwt(_) | Self::Jwks(_) => {
                "corporate identity verification failed"
            }
            Self::InvalidClaim { .. } => "corporate identity claim invalid",
            Self::NpubMismatch => "corporate identity pubkey mismatch",
            Self::BindingConflict => "corporate identity binding conflict",
            Self::DelegationDenied => "corporate identity delegation denied",
            Self::Db(_) => "corporate identity unavailable",
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

/// Enforce corporate identity for an already NIP-authenticated signer.
pub async fn enforce_corporate_identity(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    identity_jwt: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    let result =
        enforce_corporate_identity_inner(state, community_id, signer, identity_jwt, auth_tag_json)
            .await;
    if let Err(error) = &result {
        record_corporate_identity_denial(error);
    }
    result
}

async fn enforce_corporate_identity_inner(
    state: &AppState,
    community_id: CommunityId,
    signer: PublicKey,
    identity_jwt: Option<&str>,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    let Some(service) = state.corporate_identity.as_ref() else {
        return Ok(CorporateIdentityDecision::NotRequired);
    };

    if let Some(token) = identity_jwt {
        let claims = service.validate_jwt(token).await?;
        let source = binding_source_for_signer(claims.pubkey, signer)?;

        let binding = state
            .db
            .bind_or_validate_identity(
                community_id,
                &claims.uid,
                signer.as_bytes(),
                Some(&claims.display_name),
                source,
            )
            .await?;
        let binding = match binding {
            BindIdentityResult::Conflict(conflict) => {
                metrics::counter!("buzz_corporate_identity_bindings_total", "result" => "conflict")
                    .increment(1);
                record_identity_binding_audit(
                    state,
                    community_id,
                    buzz_audit::AuditAction::CorporateIdentityBindingConflict,
                    signer,
                    &claims.uid,
                    serde_json::json!({
                        "source": source,
                        "existing_uid": conflict.uid,
                        "existing_pubkey": hex::encode(conflict.pubkey),
                        "existing_source": conflict.source,
                    }),
                )
                .await;
                warn!(
                    uid = %claims.uid,
                    signer = %signer.to_hex(),
                    "corporate identity binding conflict"
                );
                return Err(CorporateIdentityError::BindingConflict);
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
                &claims.uid,
                serde_json::json!({ "source": source }),
            )
            .await;
        }
        if let Err(error) = ensure_identity_assertion(
            state,
            community_id,
            signer,
            &claims.display_name,
            &service.config.issuer,
        )
        .await
        {
            // The binding remains the authorization authority. A projection
            // failure removes the verified affordance but must not lock an
            // otherwise authorized user out of the relay.
            warn!(
                signer = %signer.to_hex(),
                error = %error,
                "failed to publish corporate identity assertion"
            );
            metrics::counter!("buzz_corporate_identity_assertions_total", "result" => "error")
                .increment(1);
        }

        debug!(
            uid = %claims.uid,
            signer = %signer.to_hex(),
            source,
            "corporate identity verified"
        );
        return Ok(CorporateIdentityDecision::Direct {
            uid: claims.uid,
            display_name: claims.display_name,
            binding,
        });
    }

    enforce_delegated_corporate_identity(
        &state.db,
        &service.config,
        community_id,
        signer,
        auth_tag_json,
    )
    .await
}

fn build_identity_assertion(
    relay_keypair: &nostr::Keys,
    subject: PublicKey,
    display_name: &str,
    issuer: &str,
    created_at: Timestamp,
) -> Result<Event, String> {
    let subject = subject.to_hex();
    let tags = [
        Tag::parse(["d", subject.as_str()]),
        Tag::parse(["p", subject.as_str()]),
        Tag::parse(["verified", "corporate"]),
        Tag::parse(["display_name", display_name]),
        Tag::parse(["issuer", issuer]),
    ]
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
    display_name: &str,
    issuer: &str,
) -> bool {
    let has_tag = |name: &str, value: &str| {
        event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == name && parts[1] == value
        })
    };
    has_tag("d", subject)
        && has_tag("p", subject)
        && has_tag("verified", "corporate")
        && has_tag("display_name", display_name)
        && has_tag("issuer", issuer)
}

async fn ensure_identity_assertion(
    state: &AppState,
    community_id: CommunityId,
    subject: PublicKey,
    display_name: &str,
    issuer: &str,
) -> Result<(), String> {
    let subject_hex = subject.to_hex();
    let existing = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![KIND_USER_TRUSTED_ASSERTION as i32]),
            pubkey: Some(state.relay_keypair.public_key().to_bytes().to_vec()),
            d_tag: Some(subject_hex.clone()),
            global_only: true,
            limit: Some(1),
            ..EventQuery::for_community(community_id)
        })
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .next();

    if existing.as_ref().is_some_and(|stored| {
        identity_assertion_matches(&stored.event, &subject_hex, display_name, issuer)
    }) {
        return Ok(());
    }

    let now = Timestamp::now().as_secs();
    let created_at = existing
        .as_ref()
        .map(|stored| stored.event.created_at.as_secs().saturating_add(1))
        .unwrap_or(now)
        .max(now);
    let event = build_identity_assertion(
        &state.relay_keypair,
        subject,
        display_name,
        issuer,
        Timestamp::from(created_at),
    )?;

    state
        .db
        .replace_parameterized_event(community_id, &event, &subject_hex, None)
        .await
        .map_err(|error| error.to_string())?;
    metrics::counter!("buzz_corporate_identity_assertions_total", "result" => "published")
        .increment(1);
    Ok(())
}

async fn enforce_delegated_corporate_identity(
    db: &buzz_db::Db,
    config: &CorporateIdentityConfig,
    community_id: CommunityId,
    signer: PublicKey,
    auth_tag_json: Option<&str>,
) -> Result<CorporateIdentityDecision, CorporateIdentityError> {
    if config.allow_delegation {
        if let Some(owner_pubkey) =
            crate::api::relay_members::extract_nip_oa_owner(signer.as_bytes(), auth_tag_json)
        {
            let owner_binding = db
                .get_active_identity_binding_by_pubkey(community_id, owner_pubkey.as_bytes())
                .await?;
            if owner_binding.is_some() {
                debug!(
                    agent = %signer.to_hex(),
                    owner = %owner_pubkey.to_hex(),
                    "corporate identity granted via NIP-OA owner binding"
                );
                return Ok(CorporateIdentityDecision::Delegated { owner_pubkey });
            }
        }
    }
    if auth_tag_json.is_some() {
        Err(CorporateIdentityError::DelegationDenied)
    } else {
        Err(CorporateIdentityError::MissingJwt)
    }
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

fn optional_claim_string(
    claims: &Map<String, Value>,
    claim: &str,
) -> Result<Option<String>, CorporateIdentityError> {
    let Some(value) = claims.get(claim) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CorporateIdentityError::InvalidClaim {
            claim: claim.to_string(),
            reason: "must be a non-empty string".to_string(),
        })?;
    Ok(Some(value.to_string()))
}

fn optional_pubkey_claim(
    claims: &Map<String, Value>,
    claim: Option<&str>,
) -> Result<Option<PublicKey>, CorporateIdentityError> {
    match claim {
        Some(claim) => optional_claim_string(claims, claim)?
            .as_deref()
            .map(|raw| parse_pubkey_claim(claim, raw))
            .transpose(),
        None => Ok(None),
    }
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

fn record_identity_binding_metric(binding: &BindIdentityResult) {
    let result = match binding {
        BindIdentityResult::Created => "created",
        BindIdentityResult::Matched => "matched",
        BindIdentityResult::Conflict(_) => "conflict",
    };
    metrics::counter!("buzz_corporate_identity_bindings_total", "result" => result).increment(1);
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
        CorporateIdentityError::DelegationDenied => "delegation_denied",
        CorporateIdentityError::Db(_) => "db",
    };
    metrics::counter!("buzz_auth_failures_total", "reason" => "corporate_identity_denied")
        .increment(1);
    metrics::counter!("buzz_corporate_identity_denials_total", "reason" => reason).increment(1);
}

async fn record_identity_binding_audit(
    state: &AppState,
    community_id: CommunityId,
    action: buzz_audit::AuditAction,
    actor: PublicKey,
    uid: &str,
    detail: serde_json::Value,
) {
    let Some(audit_tx) = &state.audit_tx else {
        return;
    };
    if let Err(e) = audit_tx
        .send(buzz_audit::NewAuditEntry {
            community_id,
            action,
            actor_pubkey: Some(actor.to_bytes().to_vec()),
            object_id: Some(uid.to_string()),
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
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use jsonwebtoken::jwk::JwkSet;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use nostr::Keys;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    fn test_config() -> CorporateIdentityConfig {
        CorporateIdentityConfig {
            require: true,
            jwt_header: "x-buzz-identity-token".to_string(),
            allow_delegation: true,
            jwks_uri: "http://127.0.0.1:9/jwks".to_string(),
            issuer: "https://idp.example".to_string(),
            audience: "buzz-relay".to_string(),
            uid_claim: "sub".to_string(),
            display_claim: "email".to_string(),
            npub_claim: Some("buzz_npub".to_string()),
        }
    }

    #[test]
    fn corporate_identity_projects_as_relay_signed_nip85_assertion() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let event = build_identity_assertion(
            &relay,
            subject,
            "Franco Sola",
            "cf-doorman-production",
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
            "Franco Sola",
            "cf-doorman-production"
        ));
        assert!(
            !event
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().is_some_and(|name| name == "uid")),
            "the public assertion must not expose the stable corporate uid"
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
    fn configured_npub_claim_is_optional_but_malformed_value_is_invalid() {
        let mut claims = Map::new();
        assert_eq!(
            optional_pubkey_claim(&claims, Some("buzz_npub")).expect("missing is optional"),
            None
        );

        claims.insert(
            "buzz_npub".to_string(),
            Value::String("not-an-npub".to_string()),
        );
        let err = optional_pubkey_claim(&claims, Some("buzz_npub"))
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
        *service.jwks.write().await = Some(CachedJwks {
            set: JwkSet { keys: Vec::new() },
            expires_at: Instant::now() + Duration::from_secs(60),
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
        let host = format!("corporate-identity-test-{}.example", id.simple());
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

        let err = enforce_delegated_corporate_identity(
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
            "owner-uid",
            owner_keys.public_key().as_bytes(),
            Some("owner@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create owner binding");

        let decision = enforce_delegated_corporate_identity(
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
            CorporateIdentityDecision::Delegated {
                owner_pubkey: owner_keys.public_key()
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

        let err = enforce_delegated_corporate_identity(&db, &config, community, signer, None)
            .await
            .expect_err("no JWT and no delegation tag");
        assert!(matches!(err, CorporateIdentityError::MissingJwt));
    }
}
