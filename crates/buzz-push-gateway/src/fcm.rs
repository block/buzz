//! Firebase Cloud Messaging HTTP v1 transport.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    apns::{DeliveryAttempt, DeliveryOutcome, PushTransport},
    model::{FCM_RECONNECT_BODY, FCM_RECONNECT_TITLE},
};

const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const OAUTH_AUDIENCE: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Debug, Serialize)]
struct OAuthClaims<'a> {
    iss: &'a str,
    scope: &'static str,
    aud: &'static str,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct OAuthResponse {
    access_token: String,
    expires_in: i64,
}

#[derive(Clone)]
struct CachedAccessToken {
    value: String,
    expires_at: i64,
}

/// Reusable FCM sender. OAuth credentials are read once at startup and never logged.
pub struct FcmTransport {
    client: reqwest::Client,
    service_account: ServiceAccount,
    encoding_key: EncodingKey,
    access_token: Arc<Mutex<Option<CachedAccessToken>>>,
    send_base_url: String,
}

impl FcmTransport {
    /// Construct the sender from a service-account JSON document and require
    /// it to belong to the configured Firebase project.
    pub fn service_account(json: &[u8], expected_project_id: &str) -> Result<Self, &'static str> {
        let account: ServiceAccount =
            serde_json::from_slice(json).map_err(|_| "invalid FCM service account")?;
        if account.project_id != expected_project_id
            || account.client_email.is_empty()
            || account.token_uri != OAUTH_AUDIENCE
        {
            return Err("invalid FCM service account");
        }
        let encoding_key = EncodingKey::from_rsa_pem(account.private_key.as_bytes())
            .map_err(|_| "invalid FCM service account")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "invalid FCM HTTP configuration")?;
        Ok(Self {
            client,
            send_base_url: "https://fcm.googleapis.com".to_owned(),
            service_account: account,
            encoding_key,
            access_token: Arc::new(Mutex::new(None)),
        })
    }

    async fn access_token(&self, now: i64) -> Result<String, ()> {
        let mut cached = self.access_token.lock().await;
        if let Some(token) = cached.as_ref().filter(|token| token.expires_at > now + 60) {
            return Ok(token.value.clone());
        }
        let assertion = encode(
            &Header::new(Algorithm::RS256),
            &OAuthClaims {
                iss: &self.service_account.client_email,
                scope: FCM_SCOPE,
                aud: OAUTH_AUDIENCE,
                iat: now,
                exp: now.saturating_add(3600),
            },
            &self.encoding_key,
        )
        .map_err(|_| ())?;
        let response = self
            .client
            .post(&self.service_account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > 16 * 1024)
        {
            return Err(());
        }
        let body = response.bytes().await.map_err(|_| ())?;
        if body.len() > 16 * 1024 {
            return Err(());
        }
        let response: OAuthResponse = serde_json::from_slice(&body).map_err(|_| ())?;
        if response.access_token.is_empty() || !(60..=7200).contains(&response.expires_in) {
            return Err(());
        }
        let value = response.access_token;
        *cached = Some(CachedAccessToken {
            value: value.clone(),
            expires_at: now.saturating_add(response.expires_in),
        });
        Ok(value)
    }

    async fn invalidate_access_token(&self, rejected: &str) {
        let mut cached = self.access_token.lock().await;
        if cached.as_ref().is_some_and(|token| token.value == rejected) {
            *cached = None;
        }
    }

    fn payload(endpoint: &str, expires_at: i64, now: i64) -> serde_json::Value {
        let ttl = expires_at.saturating_sub(now).clamp(0, 28 * 24 * 60 * 60);
        serde_json::json!({
            "message": {
                "fid": endpoint,
                "data": { "wake": "1" },
                "android": {
                    "priority": "high",
                    "ttl": format!("{ttl}s"),
                    "collapse_key": "buzz-reconnect",
                    "notification": {
                        "title": FCM_RECONNECT_TITLE,
                        "body": FCM_RECONNECT_BODY,
                        "icon": "ic_notification",
                        "channel_id": "buzz_messages",
                        "default_sound": true
                    }
                }
            }
        })
    }
}

#[derive(Deserialize)]
struct FcmErrorEnvelope {
    error: Option<FcmError>,
}

#[derive(Deserialize)]
struct FcmError {
    details: Option<Vec<FcmErrorDetail>>,
}

#[derive(Deserialize)]
struct FcmErrorDetail {
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
}

fn classify(status: StatusCode, body: Option<&FcmErrorEnvelope>) -> DeliveryOutcome {
    if status.is_success() {
        return DeliveryOutcome::Accepted;
    }
    let provider_status = body.and_then(|body| body.error.as_ref());
    let error_code = provider_status
        .and_then(|error| error.details.as_ref())
        .and_then(|details| {
            details
                .iter()
                .find_map(|detail| detail.error_code.as_deref())
        });
    // The request payload is gateway-authored and fixed, so FCM's
    // INVALID_ARGUMENT can only identify a malformed registration endpoint here.
    // SENDER_ID_MISMATCH likewise makes this endpoint permanently unusable by the
    // configured project rather than indicating a provider-wide outage.
    if matches!(
        error_code,
        Some("UNREGISTERED" | "INVALID_ARGUMENT" | "SENDER_ID_MISMATCH")
    ) {
        return DeliveryOutcome::InvalidEndpoint {
            unregistered_at: None,
        };
    }
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            DeliveryOutcome::ConfigurationFault
        }
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::INTERNAL_SERVER_ERROR
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => DeliveryOutcome::Retry {
            retry_after_seconds: None,
        },
        _ => DeliveryOutcome::PermanentRequestFault,
    }
}

#[async_trait]
impl PushTransport for FcmTransport {
    async fn send(&self, attempt: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        crate::metrics::record_fcm_send_attempt();
        let now = chrono::Utc::now().timestamp();
        let token = match self.access_token(now).await {
            Ok(token) => token,
            Err(()) => return DeliveryOutcome::ConfigurationFault,
        };
        let response = self
            .client
            .post(format!(
                "{}/v1/projects/{}/messages:send",
                self.send_base_url, self.service_account.project_id
            ))
            .bearer_auth(&token)
            .json(&Self::payload(endpoint, attempt.expires_at, now))
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
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            // Fence invalidation by the credential actually rejected: a late
            // response must not clear a newer token minted by another send.
            self.invalidate_access_token(&token).await;
        }
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .map(|seconds| seconds.clamp(1, 3600));
        let detail = if response
            .content_length()
            .is_some_and(|length| length > 16 * 1024)
        {
            None
        } else {
            response.bytes().await.ok().and_then(|body| {
                (body.len() <= 16 * 1024)
                    .then(|| serde_json::from_slice::<FcmErrorEnvelope>(&body).ok())
                    .flatten()
            })
        };
        match classify(status, detail.as_ref()) {
            DeliveryOutcome::Retry { .. } => DeliveryOutcome::Retry {
                retry_after_seconds: retry_after,
            },
            outcome => outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_payload_contains_no_relay_or_event_content() {
        let payload = FcmTransport::payload("firebase-installation-id", 130, 100);
        assert_eq!(payload["message"]["fid"], "firebase-installation-id");
        assert_eq!(payload["message"]["data"], serde_json::json!({"wake": "1"}));
        assert_eq!(payload["message"]["android"]["ttl"], "30s");
        let encoded = serde_json::to_string(&payload).unwrap();
        for forbidden in ["event_id", "relay_url", "message_content"] {
            assert!(!encoded.contains(forbidden));
        }
    }

    #[test]
    fn provider_errors_do_not_invalidate_tokens_on_configuration_faults() {
        assert_eq!(
            classify(StatusCode::UNAUTHORIZED, None),
            DeliveryOutcome::ConfigurationFault
        );
        assert_eq!(
            classify(StatusCode::TOO_MANY_REQUESTS, None),
            DeliveryOutcome::Retry {
                retry_after_seconds: None
            }
        );
        let missing_project = FcmErrorEnvelope {
            error: Some(FcmError { details: None }),
        };
        assert_eq!(
            classify(StatusCode::NOT_FOUND, Some(&missing_project)),
            DeliveryOutcome::ConfigurationFault
        );
        let unregistered = FcmErrorEnvelope {
            error: Some(FcmError {
                details: Some(vec![FcmErrorDetail {
                    error_code: Some("UNREGISTERED".into()),
                }]),
            }),
        };
        assert_eq!(
            classify(StatusCode::NOT_FOUND, Some(&unregistered)),
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None
            }
        );
        for (status, code) in [
            (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
            (StatusCode::FORBIDDEN, "SENDER_ID_MISMATCH"),
        ] {
            let invalid_endpoint = FcmErrorEnvelope {
                error: Some(FcmError {
                    details: Some(vec![FcmErrorDetail {
                        error_code: Some(code.into()),
                    }]),
                }),
            };
            assert_eq!(
                classify(status, Some(&invalid_endpoint)),
                DeliveryOutcome::InvalidEndpoint {
                    unregistered_at: None
                }
            );
        }
    }
}
