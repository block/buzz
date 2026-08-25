use crate::{config::Config, model::Principal};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    decode, decode_header,
    errors::{Error as JwtError, ErrorKind},
    jwk::JwkSet,
    Algorithm, DecodingKey, Validation,
};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub exp: i64,
    pub aud: Option<Audience>,
    pub azp: Option<String>,
    pub client_id: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    pub sid: Option<String>,
    pub events: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Audience {
    One(String),
    Many(Vec<String>),
}
impl Audience {
    fn has(&self, value: &str) -> bool {
        match self {
            Self::One(v) => v == value,
            Self::Many(v) => v.iter().any(|item| item == value),
        }
    }
}

struct Cache {
    set: JwkSet,
    fetched: Instant,
}

fn verification_error_code(error: &JwtError) -> &'static str {
    match error.kind() {
        ErrorKind::InvalidSignature => "jwt_signature_invalid",
        ErrorKind::ExpiredSignature => "jwt_expired",
        ErrorKind::InvalidIssuer => "jwt_issuer_mismatch",
        ErrorKind::InvalidAudience => "jwt_audience_mismatch",
        ErrorKind::ImmatureSignature => "jwt_not_yet_valid",
        ErrorKind::MissingRequiredClaim(_) => "jwt_required_claim_missing",
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => "jwt_algorithm_rejected",
        ErrorKind::Json(_) => "jwt_claims_invalid",
        _ => "jwt_verification_failed",
    }
}

fn unverified_issuer(token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct IssuerOnly {
        iss: String,
    }
    let encoded = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    let issuer = serde_json::from_slice::<IssuerOnly>(&decoded).ok()?.iss;
    (issuer.len() <= 2048).then_some(issuer)
}

#[derive(Clone)]
pub struct JwtVerifier {
    client: reqwest::Client,
    issuer: String,
    workbench_client_id: String,
    business_client_id: String,
    cache: Arc<RwLock<Option<Cache>>>,
}

impl JwtVerifier {
    pub fn new(config: &Config) -> Self {
        Self {
            client: reqwest::Client::new(),
            issuer: config.authentik_issuer.clone(),
            workbench_client_id: config.workbench_client_id.clone(),
            business_client_id: config.business_client_id.clone(),
            cache: Arc::new(RwLock::new(None)),
        }
    }
    async fn refresh(&self) -> Result<(), String> {
        #[derive(Deserialize)]
        struct Discovery {
            jwks_uri: String,
        }
        let discovery: Discovery = self
            .client
            .get(format!(
                "{}/.well-known/openid-configuration",
                self.issuer.trim_end_matches('/')
            ))
            .send()
            .await
            .map_err(|_| "oidc discovery failed")?
            .error_for_status()
            .map_err(|_| "oidc discovery rejected")?
            .json()
            .await
            .map_err(|_| "invalid oidc discovery")?;
        let set: JwkSet = self
            .client
            .get(discovery.jwks_uri)
            .send()
            .await
            .map_err(|_| "jwks fetch failed")?
            .error_for_status()
            .map_err(|_| "jwks fetch rejected")?
            .json()
            .await
            .map_err(|_| "invalid jwks")?;
        *self.cache.write().await = Some(Cache {
            set,
            fetched: Instant::now(),
        });
        Ok(())
    }
    async fn claims(&self, token: &str, expected: &str) -> Result<Claims, String> {
        let header = decode_header(token).map_err(|_| "invalid jwt header")?;
        if header.alg != Algorithm::RS256 {
            return Err("unsupported jwt algorithm".into());
        }
        let kid = header.kid.ok_or("jwt kid missing")?;
        let stale = self
            .cache
            .read()
            .await
            .as_ref()
            .map(|v| v.fetched.elapsed() > Duration::from_secs(300))
            .unwrap_or(true);
        if stale {
            self.refresh().await?;
        }
        let find = |cache: &Option<Cache>| cache.as_ref().and_then(|v| v.set.find(&kid)).cloned();
        let mut jwk = {
            let guard = self.cache.read().await;
            find(&guard)
        };
        if jwk.is_none() {
            self.refresh().await?;
            jwk = {
                let guard = self.cache.read().await;
                find(&guard)
            };
        }
        let key = DecodingKey::from_jwk(&jwk.ok_or("jwt signing key not found")?)
            .map_err(|_| "invalid jwt signing key")?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        // Authentik deployments may use a resource-server audience and carry
        // the OAuth client in `azp`/`client_id`. Verify that exact OR policy
        // below instead of making jsonwebtoken require `aud == client_id`.
        validation.validate_aud = false;
        validation.validate_exp = true;
        validation.leeway = 30;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|error| {
                if matches!(error.kind(), ErrorKind::InvalidIssuer) {
                    tracing::warn!(
                        presented_issuer = unverified_issuer(token).as_deref().unwrap_or("invalid"),
                        expected_issuer = %self.issuer,
                        "OIDC issuer mismatch"
                    );
                }
                verification_error_code(&error).to_string()
            })?
            .claims;
        let presented_client = claims.azp.as_deref().or(claims.client_id.as_deref());
        let audience_matches = claims
            .aud
            .as_ref()
            .is_some_and(|audience| audience.has(expected));
        let client_matches = presented_client == Some(expected);
        if presented_client.is_some_and(|client| client != expected)
            || (!audience_matches && !client_matches)
        {
            return Err("jwt client mismatch".into());
        }
        Ok(claims)
    }
    pub async fn workbench(&self, token: &str) -> Result<Claims, String> {
        self.claims(token, &self.workbench_client_id).await
    }
    pub async fn logout(&self, token: &str) -> Result<Claims, String> {
        let claims = self.claims(token, &self.business_client_id).await?;
        let valid_event = claims
            .events
            .as_ref()
            .and_then(|v| v.get("http://schemas.openid.net/event/backchannel-logout"))
            .is_some();
        if !valid_event {
            return Err("invalid logout token claims".into());
        }
        Ok(claims)
    }
}

pub fn bearer(headers: &axum::http::HeaderMap) -> Result<&str, String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "bearer token required".into())
}

impl Claims {
    pub fn provisional_principal(&self) -> Result<Principal, String> {
        let token_expires_at =
            DateTime::from_timestamp(self.exp, 0).ok_or("invalid token expiry")?;
        if token_expires_at <= Utc::now() {
            return Err("token expired".into());
        }
        Ok(Principal {
            user_id: uuid::Uuid::nil(),
            issuer: self.iss.clone(),
            subject: self.sub.clone(),
            email: self.email.clone(),
            display_name: self
                .name
                .clone()
                .or_else(|| self.preferred_username.clone())
                .unwrap_or_else(|| self.sub.clone()),
            sid: self.sid.clone(),
            workbench_session_id: uuid::Uuid::nil(),
            token_expires_at,
        })
    }
}
