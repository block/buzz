//! APNs envelope construction, endpoint encryption, and response classification.

use std::{sync::Mutex, time::Duration};

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use p256::{
    ecdsa::{signature::Signer, Signature, SigningKey},
    pkcs8::DecodePrivateKey,
};
use reqwest::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    StatusCode,
};
use serde::Deserialize;
use thiserror::Error;

use crate::model::{AppProfile, APNS_RECONNECT_PAYLOAD};

/// Sanitized delivery outcome. Raw provider bodies never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// APNs accepted the request (not proof of device delivery).
    Accepted,
    /// This endpoint generation is permanently invalid. APNs may provide the time it became invalid.
    InvalidEndpoint {
        /// APNs' timestamp for when the endpoint became invalid, if supplied.
        unregistered_at: Option<i64>,
    },
    /// A bounded retry is safe. A sanitized server hint may raise the delay.
    Retry {
        /// Retry-After delay in seconds, clamped by the transport.
        retry_after_seconds: Option<i64>,
    },
    /// Refresh the cached provider JWT, then retry once within normal attempt bounds.
    RefreshCredential,
    /// Provider credential/profile configuration is unhealthy; do not invalidate endpoints.
    ConfigurationFault,
    /// The locally-generated request is permanently invalid.
    PermanentRequestFault,
}

/// Classify APNs status/reason without conflating provider faults with endpoints.
pub fn classify(code: u16, reason: Option<&str>, timestamp: Option<i64>) -> DeliveryOutcome {
    match (code, reason) {
        (200, _) => DeliveryOutcome::Accepted,
        (410, Some("Unregistered")) => DeliveryOutcome::InvalidEndpoint {
            unregistered_at: timestamp,
        },
        (400, Some("BadDeviceToken" | "DeviceTokenNotForTopic")) => {
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            }
        }
        (403, Some("ExpiredProviderToken")) => DeliveryOutcome::RefreshCredential,
        (403, _) | (429, Some("TooManyProviderTokenUpdates")) => {
            DeliveryOutcome::ConfigurationFault
        }
        (429 | 500 | 503, _)
        | (
            _,
            Some(
                "IdleTimeout"
                | "InternalServerError"
                | "ServiceUnavailable"
                | "Shutdown"
                | "TooManyRequests",
            ),
        ) => DeliveryOutcome::Retry {
            retry_after_seconds: None,
        },
        _ => DeliveryOutcome::PermanentRequestFault,
    }
}

/// Closed APNs transport controls. No field can be serialized into application
/// content; the concrete transport always uses `APNS_RECONNECT_PAYLOAD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryAttempt {
    pub request_id: uuid::Uuid,
    pub expires_at: i64,
}

/// APNs sender abstraction for live-validation tests.
#[async_trait]
pub trait PushTransport: Send + Sync {
    /// Send one durable job.
    async fn send(
        &self,
        attempt: DeliveryAttempt,
        profile: AppProfile,
        endpoint: &str,
    ) -> DeliveryOutcome;
    /// Discard a cached credential after APNs reports expiry.
    fn refresh_credential(&self) {}
}

struct CachedJwt {
    token: String,
    issued_at: i64,
}

/// Direct HTTP/2 APNs transport using a cached ES256 provider token.
pub struct ApnsTransport {
    client: reqwest::Client,
    signing_key: SigningKey,
    key_id: String,
    team_id: String,
    topic: String,
    production_base_url: String,
    sandbox_base_url: String,
    cached_jwt: Mutex<Option<CachedJwt>>,
}

impl ApnsTransport {
    /// Build a reusable APNs client from an Apple `.p8` private key.
    pub fn token(p8: &[u8], key_id: &str, team_id: &str, topic: String) -> Result<Self, ApnsError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| ApnsError::Client)?;
        Self::token_with_client(
            p8,
            key_id,
            team_id,
            topic,
            client,
            "https://api.push.apple.com".to_owned(),
            "https://api.sandbox.push.apple.com".to_owned(),
        )
    }

    fn token_with_client(
        p8: &[u8],
        key_id: &str,
        team_id: &str,
        topic: String,
        client: reqwest::Client,
        production_base_url: String,
        sandbox_base_url: String,
    ) -> Result<Self, ApnsError> {
        let pem = std::str::from_utf8(p8).map_err(|_| ApnsError::Credential)?;
        let signing_key = SigningKey::from_pkcs8_pem(pem).map_err(|_| ApnsError::Credential)?;
        Ok(Self {
            client,
            signing_key,
            key_id: key_id.to_owned(),
            team_id: team_id.to_owned(),
            topic,
            production_base_url,
            sandbox_base_url,
            cached_jwt: Mutex::new(None),
        })
    }

    fn jwt(&self, now: i64) -> Result<String, ApnsError> {
        let mut cached = self.cached_jwt.lock().map_err(|_| ApnsError::Credential)?;
        if let Some(jwt) = cached.as_ref().filter(|jwt| now - jwt.issued_at < 50 * 60) {
            return Ok(jwt.token.clone());
        }
        let header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"alg":"ES256","kid":self.key_id}))
                .map_err(|_| ApnsError::Credential)?,
        );
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"iss":self.team_id,"iat":now}))
                .map_err(|_| ApnsError::Credential)?,
        );
        let signing_input = format!("{header}.{claims}");
        let signature: Signature = self.signing_key.sign(signing_input.as_bytes());
        let token = format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        );
        *cached = Some(CachedJwt {
            token: token.clone(),
            issued_at: now,
        });
        Ok(token)
    }
}

/// APNs transport setup failure. It intentionally carries no credential material.
#[derive(Debug, Error)]
pub enum ApnsError {
    /// Invalid provider key material.
    #[error("invalid APNs credential")]
    Credential,
    /// HTTP client setup failed.
    #[error("failed to construct APNs client")]
    Client,
}

#[derive(Deserialize)]
struct ApnsErrorBody {
    reason: Option<String>,
    timestamp: Option<i64>,
}

#[async_trait]
impl PushTransport for ApnsTransport {
    async fn send(
        &self,
        attempt: DeliveryAttempt,
        profile: AppProfile,
        endpoint: &str,
    ) -> DeliveryOutcome {
        // This is the only APNs application body in the program. It is a
        // byte constant, not a serialization of the relay request, grant,
        // endpoint, headers, route, provider response, or any generic JSON map.
        let body = APNS_RECONNECT_PAYLOAD;
        let now = chrono::Utc::now().timestamp();
        let token = match self.jwt(now) {
            Ok(token) => token,
            Err(_) => return DeliveryOutcome::ConfigurationFault,
        };
        let base_url = match profile {
            AppProfile::BuzzIosProduction => &self.production_base_url,
            AppProfile::BuzzIosSandbox => &self.sandbox_base_url,
        };
        let response = self
            .client
            .post(format!("{base_url}/3/device/{endpoint}"))
            .header(AUTHORIZATION, format!("bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .header("apns-id", attempt.request_id.to_string())
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-expiration", attempt.expires_at.to_string())
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                return DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                }
            }
        };
        if response.status() == StatusCode::OK {
            return DeliveryOutcome::Accepted;
        }
        let code = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .map(|seconds| seconds.clamp(1, 3600));
        let detail = response.json::<ApnsErrorBody>().await.ok();
        let timestamp = detail.as_ref().and_then(|d| d.timestamp);
        match classify(
            code,
            detail.as_ref().and_then(|d| d.reason.as_deref()),
            timestamp,
        ) {
            DeliveryOutcome::Retry { .. } => DeliveryOutcome::Retry {
                retry_after_seconds: retry_after,
            },
            outcome => outcome,
        }
    }

    fn refresh_credential(&self) {
        if let Ok(mut cached) = self.cached_jwt.lock() {
            *cached = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, http::StatusCode, routing::post, Router};
    use p256::pkcs8::{EncodePrivateKey, LineEnding};
    use std::sync::Arc;

    async fn capture_body(
        State(bodies): State<Arc<Mutex<Vec<Vec<u8>>>>>,
        body: Bytes,
    ) -> StatusCode {
        bodies.lock().unwrap().push(body.to_vec());
        StatusCode::OK
    }
    #[tokio::test]
    async fn real_outbound_http_body_is_the_exact_constant_for_every_attempt() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/3/device/{endpoint}", post(capture_body))
            .with_state(bodies.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let signing_key = SigningKey::from_slice(&[7; 32]).unwrap();
        let pem = signing_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let transport = ApnsTransport::token_with_client(
            pem.as_bytes(),
            "kid",
            "team",
            "app.topic".to_owned(),
            reqwest::Client::new(),
            base_url.clone(),
            base_url,
        )
        .unwrap();
        for (request_id, expires_at, profile, endpoint) in [
            (
                uuid::Uuid::nil(),
                1,
                AppProfile::BuzzIosProduction,
                "00".repeat(32),
            ),
            (
                uuid::Uuid::max(),
                i64::MAX,
                AppProfile::BuzzIosSandbox,
                "ff".repeat(32),
            ),
        ] {
            assert_eq!(
                transport
                    .send(
                        DeliveryAttempt {
                            request_id,
                            expires_at,
                        },
                        profile,
                        &endpoint,
                    )
                    .await,
                DeliveryOutcome::Accepted
            );
        }
        let captured = bodies.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(captured
            .iter()
            .all(|body| body.as_slice() == APNS_RECONNECT_PAYLOAD));
    }

    #[test]
    fn response_classes_do_not_massacre_endpoints_on_provider_faults() {
        assert_eq!(
            classify(410, Some("Unregistered"), Some(7)),
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: Some(7)
            }
        );
        assert_eq!(
            classify(403, Some("InvalidProviderToken"), None),
            DeliveryOutcome::ConfigurationFault
        );
        assert_eq!(
            classify(429, Some("TooManyRequests"), None),
            DeliveryOutcome::Retry {
                retry_after_seconds: None
            }
        );
        assert_eq!(
            classify(400, Some("BadTopic"), None),
            DeliveryOutcome::PermanentRequestFault
        );
    }
}
