//! Firebase App Check verification for the Android transport profile.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

const APP_CHECK_JWKS_URL: &str = "https://firebaseappcheck.googleapis.com/v1/jwks";
const MAX_APP_CHECK_TOKEN_BYTES: usize = 4096;
const JWKS_CACHE_LIFETIME: Duration = Duration::from_secs(3600);

#[derive(Debug, Error)]
pub enum AppCheckError {
    /// The token is malformed, expired, incorrectly signed, or for another app.
    #[error("invalid Firebase App Check token")]
    Invalid,
    /// Verification could not reach or decode Firebase's signing-key service.
    #[error("Firebase App Check verification is temporarily unavailable")]
    Unavailable,
}

/// Application-identity verifier used by the Android enrollment seam.
#[async_trait]
pub trait AppCheckTokenVerifier: Send + Sync {
    /// Verify one Firebase App Check token for the configured Android app.
    async fn verify(&self, token: &str) -> Result<(), AppCheckError>;
}

#[derive(Clone)]
struct CachedJwks {
    keys: JwkSet,
    fetched_at: std::time::Instant,
}

/// Verifies Firebase App Check JWTs against Google's bounded, cached JWKS.
pub struct FirebaseAppCheckVerifier {
    client: reqwest::Client,
    project_number: String,
    app_id: String,
    cache: Arc<RwLock<Option<CachedJwks>>>,
    jwks_url: String,
}

impl FirebaseAppCheckVerifier {
    /// Build a verifier pinned to one Firebase project number and app ID.
    pub fn new(project_number: String, app_id: String) -> Result<Self, AppCheckError> {
        if project_number.is_empty() || !project_number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AppCheckError::Invalid);
        }
        let app_id_prefix = format!("1:{project_number}:android:");
        if app_id.len() > 256
            || !app_id.strip_prefix(&app_id_prefix).is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(AppCheckError::Invalid);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| AppCheckError::Unavailable)?;
        Ok(Self {
            client,
            project_number,
            app_id,
            cache: Arc::new(RwLock::new(None)),
            jwks_url: APP_CHECK_JWKS_URL.to_owned(),
        })
    }

    async fn jwks(&self) -> Result<JwkSet, AppCheckError> {
        if let Some(cached) = self.cache.read().await.as_ref() {
            if cached.fetched_at.elapsed() < JWKS_CACHE_LIFETIME {
                return Ok(cached.keys.clone());
            }
        }
        let response = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| AppCheckError::Unavailable)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > 64 * 1024)
        {
            return Err(AppCheckError::Unavailable);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| AppCheckError::Unavailable)?;
        if bytes.len() > 64 * 1024 {
            return Err(AppCheckError::Unavailable);
        }
        let keys: JwkSet =
            serde_json::from_slice(&bytes).map_err(|_| AppCheckError::Unavailable)?;
        if keys.keys.is_empty() || keys.keys.len() > 16 {
            return Err(AppCheckError::Unavailable);
        }
        *self.cache.write().await = Some(CachedJwks {
            keys: keys.clone(),
            fetched_at: std::time::Instant::now(),
        });
        Ok(keys)
    }
}

#[derive(Deserialize)]
struct AppCheckClaims {
    sub: String,
}

#[async_trait]
impl AppCheckTokenVerifier for FirebaseAppCheckVerifier {
    async fn verify(&self, token: &str) -> Result<(), AppCheckError> {
        if token.is_empty() || token.len() > MAX_APP_CHECK_TOKEN_BYTES {
            return Err(AppCheckError::Invalid);
        }
        let header = decode_header(token).map_err(|_| AppCheckError::Invalid)?;
        if header.alg != Algorithm::RS256 || header.typ.as_deref() != Some("JWT") {
            return Err(AppCheckError::Invalid);
        }
        let kid = header.kid.ok_or(AppCheckError::Invalid)?;
        let keys = self.jwks().await?;
        let jwk = keys.find(&kid).ok_or(AppCheckError::Invalid)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AppCheckError::Invalid)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[format!(
            "https://firebaseappcheck.googleapis.com/{}",
            self.project_number
        )]);
        validation.set_audience(&[format!("projects/{}", self.project_number)]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
        let claims = decode::<AppCheckClaims>(token, &key, &validation)
            .map_err(|_| AppCheckError::Invalid)?
            .claims;
        if claims.sub != self.app_id {
            return Err(AppCheckError::Invalid);
        }
        Ok(())
    }
}
