//! APNs envelope construction, endpoint encryption, and response classification.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{header::CONTENT_TYPE, StatusCode};
use serde::Deserialize;
use thiserror::Error;

use crate::{config::ApnsEnvironment, model::APNS_RECONNECT_PAYLOAD};

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
        // Both reasons are ambiguous with deployment profile mistakes: APNs
        // uses BadDeviceToken for environment mismatches and
        // DeviceTokenNotForTopic for topic mismatches. Only Unregistered
        // crosses the permanent endpoint-invalidation boundary.
        (400, Some("BadDeviceToken" | "DeviceTokenNotForTopic")) => {
            DeliveryOutcome::ConfigurationFault
        }
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
    async fn send(&self, attempt: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome;
}

/// Direct HTTP/2 APNs transport using a client certificate identity.
pub struct ApnsTransport {
    client: reqwest::Client,
    topic: String,
    base_url: String,
}

impl ApnsTransport {
    /// Build a reusable APNs client from a combined PEM private key and certificate.
    pub fn certificate(
        identity_pem: &[u8],
        topic: String,
        environment: ApnsEnvironment,
    ) -> Result<Self, ApnsError> {
        let base_url = match environment {
            ApnsEnvironment::Production => "https://api.push.apple.com",
            ApnsEnvironment::Sandbox => "https://api.sandbox.push.apple.com",
        };
        Self::certificate_with_base_url(identity_pem, topic, base_url.to_owned())
    }

    fn certificate_with_base_url(
        identity_pem: &[u8],
        topic: String,
        base_url: String,
    ) -> Result<Self, ApnsError> {
        let identity =
            reqwest::Identity::from_pem(identity_pem).map_err(|_| ApnsError::Credential)?;
        let client = reqwest::Client::builder()
            // APNs requires HTTP/2. This no-op method reference is intentionally
            // feature-gated so removing reqwest's `http2` feature fails the build.
            .http2_keep_alive_while_idle(false)
            .identity(identity)
            .timeout(Duration::from_secs(15))
            // Identity validation completes while the TLS client is built, so a
            // malformed or mismatched certificate/key pair is a credential error.
            .build()
            .map_err(|_| ApnsError::Credential)?;
        Ok(Self {
            client,
            topic,
            base_url,
        })
    }

    fn request(&self, attempt: DeliveryAttempt, endpoint: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}/3/device/{endpoint}", self.base_url))
            .header(CONTENT_TYPE, "application/json")
            .header("apns-id", attempt.request_id.to_string())
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-expiration", attempt.expires_at.to_string())
            // This is the only APNs application body in the program. It is a
            // byte constant, not a serialization of the relay request, grant,
            // endpoint, headers, route, provider response, or any generic JSON map.
            .body(APNS_RECONNECT_PAYLOAD)
    }

    async fn send_response(
        &self,
        attempt: DeliveryAttempt,
        endpoint: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        self.request(attempt, endpoint).send().await
    }
}

/// APNs transport setup failure. It intentionally carries no credential material.
#[derive(Debug, Error)]
pub enum ApnsError {
    /// Invalid client certificate identity material.
    #[error("invalid APNs credential")]
    Credential,
}

#[derive(Deserialize)]
struct ApnsErrorBody {
    reason: Option<String>,
    timestamp: Option<i64>,
}

#[async_trait]
impl PushTransport for ApnsTransport {
    async fn send(&self, attempt: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        crate::metrics::record_apns_send_attempt();
        let response = self.send_response(attempt, endpoint).await;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::{Arc, Mutex};

    // Self-signed test-only identity material. None of these are Apple credentials.
    const TEST_IDENTITY_PEM: &[u8] = include_bytes!("../tests/fixtures/apns-test-identity.pem");
    const TEST_CERT_ONLY_PEM: &[u8] = include_bytes!("../tests/fixtures/apns-test-cert-only.pem");
    const TEST_KEY_ONLY_PEM: &[u8] = include_bytes!("../tests/fixtures/apns-test-key-only.pem");
    const TEST_ENCRYPTED_IDENTITY_PEM: &[u8] =
        include_bytes!("../tests/fixtures/apns-test-encrypted-identity.pem");
    const TEST_MISMATCHED_IDENTITY_PEM: &[u8] =
        include_bytes!("../tests/fixtures/apns-test-mismatched-identity.pem");

    #[derive(Default)]
    struct CapturedRequest {
        headers: HeaderMap,
        body: Vec<u8>,
    }

    async fn capture_request(
        State(requests): State<Arc<Mutex<Vec<CapturedRequest>>>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        requests.lock().unwrap().push(CapturedRequest {
            headers,
            body: body.to_vec(),
        });
        StatusCode::OK
    }

    #[tokio::test]
    async fn certificate_transport_sends_no_bearer_and_exact_body_for_every_attempt() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/3/device/{endpoint}", post(capture_request))
            .with_state(requests.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let transport = ApnsTransport::certificate_with_base_url(
            TEST_IDENTITY_PEM,
            "app.topic".to_owned(),
            base_url,
        )
        .unwrap();
        for (request_id, expires_at, endpoint) in [
            (uuid::Uuid::nil(), 1, "00".repeat(32)),
            (uuid::Uuid::max(), i64::MAX, "ff".repeat(32)),
        ] {
            assert_eq!(
                transport
                    .send(
                        DeliveryAttempt {
                            request_id,
                            expires_at,
                        },
                        &endpoint,
                    )
                    .await,
                DeliveryOutcome::Accepted
            );
        }
        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(captured
            .iter()
            .all(|request| request.body.as_slice() == APNS_RECONNECT_PAYLOAD));
        assert!(captured
            .iter()
            .all(|request| !request.headers.contains_key(reqwest::header::AUTHORIZATION)));
        assert!(captured.iter().all(|request| request
            .headers
            .get("apns-topic")
            .is_some_and(|topic| topic == "app.topic")));
    }

    #[tokio::test]
    #[ignore = "requires the exported dogfood Apple Push Services PEM"]
    async fn live_sandbox_probe_reports_literal_status_and_body() {
        let cert_path = std::env::var("BUZZ_PUSH_LIVE_APNS_CERT_PATH")
            .expect("set BUZZ_PUSH_LIVE_APNS_CERT_PATH to the dogfood identity PEM");
        let topic = std::env::var("BUZZ_PUSH_LIVE_APNS_TOPIC")
            .expect("set BUZZ_PUSH_LIVE_APNS_TOPIC to the dogfood bundle id");
        let identity = std::fs::read(cert_path).unwrap();
        let transport =
            ApnsTransport::certificate(&identity, topic, ApnsEnvironment::Sandbox).unwrap();
        let response = transport
            .send_response(
                DeliveryAttempt {
                    request_id: uuid::Uuid::nil(),
                    expires_at: chrono::Utc::now().timestamp() + 60,
                },
                &"00".repeat(32),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        eprintln!("live APNs response: status={status}, body={body}");
        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(body, r#"{"reason":"BadDeviceToken"}"#);
    }

    #[test]
    fn empty_certificate_identity_fails_as_a_credential_error() {
        assert_credential_error(b"");
    }

    #[test]
    fn malformed_certificate_identity_fails_as_a_credential_error() {
        assert_credential_error(b"not a PEM identity");
    }

    #[test]
    fn certificate_without_private_key_fails_as_a_credential_error() {
        assert_credential_error(TEST_CERT_ONLY_PEM);
    }

    #[test]
    fn private_key_without_certificate_fails_as_a_credential_error() {
        assert_credential_error(TEST_KEY_ONLY_PEM);
    }

    #[test]
    fn encrypted_private_key_fails_as_a_credential_error() {
        assert_credential_error(TEST_ENCRYPTED_IDENTITY_PEM);
    }

    #[test]
    fn mismatched_private_key_fails_as_a_credential_error() {
        // reqwest parses both PEM blocks, then rejects the mismatched pair while
        // building the TLS client. This locks the ClientBuilder error mapping.
        assert_credential_error(TEST_MISMATCHED_IDENTITY_PEM);
    }

    fn assert_credential_error(identity_pem: &[u8]) {
        assert!(matches!(
            ApnsTransport::certificate(
                identity_pem,
                "app.topic".to_owned(),
                ApnsEnvironment::Production,
            ),
            Err(ApnsError::Credential)
        ));
    }

    #[test]
    fn response_classes_do_not_massacre_endpoints_on_provider_faults() {
        assert_eq!(
            classify(410, Some("Unregistered"), Some(7)),
            DeliveryOutcome::InvalidEndpoint {
                unregistered_at: Some(7)
            }
        );
        for reason in ["InvalidProviderToken", "ExpiredProviderToken"] {
            assert_eq!(
                classify(403, Some(reason), None),
                DeliveryOutcome::ConfigurationFault
            );
        }
        for reason in ["BadDeviceToken", "DeviceTokenNotForTopic"] {
            assert_eq!(
                classify(400, Some(reason), None),
                DeliveryOutcome::ConfigurationFault
            );
        }
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
