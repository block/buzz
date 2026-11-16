//! Firebase Cloud Messaging HTTP v1 last hop for Android installations.
//!
//! The wake is a high-priority, data-only message. Its payload is built only
//! from the compiled-in constants below, the FCM registration token and a time
//! to live derived from the delivery's expiry: no relay-supplied bytes reach it.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use aws_lc_rs::{rand::SystemRandom, rsa::KeyPair, signature::RSA_PKCS1_SHA256};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::StatusCode;
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::apns::{DeliveryAttempt, DeliveryOutcome, PushTransport};

/// The one collapse key every wake carries. FCM replaces a wake it is still
/// holding for an unreachable phone, so a phone coming back online gets one
/// wake instead of a burst. An online phone still gets one wake per message.
pub const FCM_COLLAPSE_KEY: &str = "buzz-wake";
/// The only application data a wake carries: a constant, like
/// `APNS_RECONNECT_PAYLOAD`.
pub const FCM_WAKE_DATA: [(&str, &str); 1] = [("buzz_wake", "1")];
/// FCM discards messages held longer than four weeks.
const MAX_TTL_SECONDS: i64 = 2_419_200;
const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const FCM_BASE_URL: &str = "https://fcm.googleapis.com";
const ACCESS_TOKEN_LIFETIME_SECONDS: i64 = 3600;
/// Refresh an access token this long before Google would reject it.
const ACCESS_TOKEN_REFRESH_MARGIN_SECONDS: i64 = 120;
/// After a failed token request, fail fast for this long instead of sending
/// every delivery to Google's token endpoint.
const TOKEN_FAILURE_BACKOFF: Duration = Duration::from_secs(5);

/// Service account setup failure. It carries no credential material.
#[derive(Debug, Error)]
pub enum FcmError {
    /// The service account key is missing a field, malformed or not RSA.
    #[error("invalid FCM service account credential")]
    Credential,
}

#[derive(Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

struct CachedToken {
    value: String,
    refresh_at: i64,
}

enum TokenState {
    Empty,
    Fresh(CachedToken),
    Failed(Instant),
}

/// The ways a delivery cannot obtain an access token.
enum TokenFault {
    /// The token endpoint answered but refused the credential.
    Rejected,
    /// The token endpoint could not be reached or is unhealthy.
    Unavailable,
}

/// Direct HTTP/1.1+2 FCM HTTP v1 transport authenticated by a service account.
pub struct FcmTransport {
    client: reqwest::Client,
    send_url: String,
    client_email: String,
    token_uri: String,
    key: KeyPair,
    token: Mutex<TokenState>,
    now: fn() -> i64,
}

impl FcmTransport {
    /// Build the production transport for `project_id` from a Google service
    /// account key JSON.
    pub fn service_account(project_id: &str, key_json: &[u8]) -> Result<Self, FcmError> {
        Self::with_endpoints(project_id, key_json, FCM_BASE_URL, || {
            chrono::Utc::now().timestamp()
        })
    }

    fn with_endpoints(
        project_id: &str,
        key_json: &[u8],
        base_url: &str,
        now: fn() -> i64,
    ) -> Result<Self, FcmError> {
        let account: ServiceAccountKey =
            serde_json::from_slice(key_json).map_err(|_| FcmError::Credential)?;
        if project_id.is_empty() || account.client_email.is_empty() || account.token_uri.is_empty()
        {
            return Err(FcmError::Credential);
        }
        let key = KeyPair::from_pkcs8(&pkcs8_der(&account.private_key)?)
            .map_err(|_| FcmError::Credential)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| FcmError::Credential)?;
        Ok(Self {
            client,
            send_url: format!("{base_url}/v1/projects/{project_id}/messages:send"),
            client_email: account.client_email,
            token_uri: account.token_uri,
            key,
            token: Mutex::new(TokenState::Empty),
            now,
        })
    }

    /// Short-lived OAuth access token, refreshed through one in-flight request.
    async fn access_token(&self) -> Result<String, TokenFault> {
        let mut state = self.token.lock().await;
        let now = (self.now)();
        match &*state {
            TokenState::Fresh(token) if token.refresh_at > now => return Ok(token.value.clone()),
            TokenState::Failed(at) if at.elapsed() < TOKEN_FAILURE_BACKOFF => {
                return Err(TokenFault::Unavailable)
            }
            _ => {}
        }
        match self.request_access_token(now).await {
            Ok(token) => {
                let value = token.value.clone();
                *state = TokenState::Fresh(token);
                Ok(value)
            }
            Err(fault) => {
                *state = TokenState::Failed(Instant::now());
                Err(fault)
            }
        }
    }

    async fn request_access_token(&self, now: i64) -> Result<CachedToken, TokenFault> {
        let assertion = self
            .signed_assertion(now)
            .map_err(|_| TokenFault::Rejected)?;
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
            .append_pair("assertion", &assertion)
            .finish();
        let response = self
            .client
            .post(&self.token_uri)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|_| TokenFault::Unavailable)?;
        let status = response.status();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(TokenFault::Unavailable);
        }
        if !status.is_success() {
            return Err(TokenFault::Rejected);
        }
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: i64,
        }
        let token = response
            .json::<TokenResponse>()
            .await
            .map_err(|_| TokenFault::Unavailable)?;
        if token.access_token.is_empty() {
            return Err(TokenFault::Rejected);
        }
        let lifetime = token.expires_in.clamp(0, ACCESS_TOKEN_LIFETIME_SECONDS);
        Ok(CachedToken {
            value: token.access_token,
            refresh_at: now + lifetime - ACCESS_TOKEN_REFRESH_MARGIN_SECONDS,
        })
    }

    fn signed_assertion(&self, now: i64) -> Result<String, FcmError> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "iss": self.client_email,
                "scope": FCM_SCOPE,
                "aud": self.token_uri,
                "iat": now,
                "exp": now + ACCESS_TOKEN_LIFETIME_SECONDS,
            })
            .to_string(),
        );
        let signing_input = format!("{header}.{claims}");
        let mut signature = vec![0; self.key.public_modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| FcmError::Credential)?;
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    /// The only FCM request body in the program.
    fn message(&self, attempt: DeliveryAttempt, token: &str) -> serde_json::Value {
        let ttl = attempt
            .expires_at
            .saturating_sub((self.now)())
            .clamp(0, MAX_TTL_SECONDS);
        serde_json::json!({
            "message": {
                "token": token,
                "data": FCM_WAKE_DATA.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
                "android": {
                    "priority": "HIGH",
                    "collapse_key": FCM_COLLAPSE_KEY,
                    "ttl": format!("{ttl}s"),
                },
            }
        })
    }
}

/// Decode the PKCS#8 DER inside a service account's PEM private key.
fn pkcs8_der(pem: &str) -> Result<Vec<u8>, FcmError> {
    use base64::engine::general_purpose::STANDARD;
    let body = pem
        .trim()
        .strip_prefix("-----BEGIN PRIVATE KEY-----")
        .and_then(|rest| rest.strip_suffix("-----END PRIVATE KEY-----"))
        .ok_or(FcmError::Credential)?;
    let compact: String = body.split_whitespace().collect();
    STANDARD.decode(compact).map_err(|_| FcmError::Credential)
}

#[derive(Deserialize)]
struct FcmErrorBody {
    error: Option<FcmErrorDetail>,
}
#[derive(Deserialize)]
struct FcmErrorDetail {
    #[serde(default)]
    details: Vec<FcmErrorCode>,
}
#[derive(Deserialize)]
struct FcmErrorCode {
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
}

/// Classify an FCM HTTP v1 response without conflating provider faults with
/// endpoints. Only `UNREGISTERED` permanently invalidates a registration
/// token. `INVALID_ARGUMENT` and `SENDER_ID_MISMATCH` are ambiguous with
/// deployment mistakes (wrong project, malformed request), like APNs'
/// `BadDeviceToken`, so they never invalidate endpoints.
pub fn classify(code: u16, error_code: Option<&str>) -> DeliveryOutcome {
    match (code, error_code) {
        (200, _) => DeliveryOutcome::Accepted,
        (_, Some("UNREGISTERED")) => DeliveryOutcome::InvalidEndpoint {
            unregistered_at: None,
        },
        (400, Some("INVALID_ARGUMENT")) | (_, Some("SENDER_ID_MISMATCH")) => {
            DeliveryOutcome::ConfigurationFault
        }
        (401 | 403, _) => DeliveryOutcome::ConfigurationFault,
        (429 | 500 | 503, _) | (_, Some("QUOTA_EXCEEDED" | "UNAVAILABLE" | "INTERNAL")) => {
            DeliveryOutcome::Retry {
                retry_after_seconds: None,
            }
        }
        _ => DeliveryOutcome::PermanentRequestFault,
    }
}

#[async_trait]
impl PushTransport for FcmTransport {
    async fn send(&self, attempt: DeliveryAttempt, endpoint: &str) -> DeliveryOutcome {
        crate::metrics::record_fcm_send_attempt();
        let access_token = match self.access_token().await {
            Ok(token) => token,
            Err(TokenFault::Rejected) => return DeliveryOutcome::ConfigurationFault,
            Err(TokenFault::Unavailable) => {
                return DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                }
            }
        };
        let response = match self
            .client
            .post(&self.send_url)
            .bearer_auth(access_token)
            .json(&self.message(attempt, endpoint))
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                }
            }
        };
        let code = response.status().as_u16();
        if code == 200 {
            return DeliveryOutcome::Accepted;
        }
        if code == 401 {
            // The cached token was refused: fetch a new one on the next send.
            *self.token.lock().await = TokenState::Empty;
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .map(|seconds| seconds.clamp(1, 3600));
        let error_code = response
            .json::<FcmErrorBody>()
            .await
            .ok()
            .and_then(|body| body.error)
            .and_then(|error| {
                error
                    .details
                    .into_iter()
                    .find_map(|detail| detail.error_code)
            });
        match classify(code, error_code.as_deref()) {
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
        http::{HeaderMap, StatusCode as Status},
        routing::post,
        Router,
    };
    use std::sync::{Arc, Mutex};

    const NOW: i64 = 1_750_000_000;
    const TOKEN: &str = "cExample-token:APA91bExample_token";

    #[derive(Default)]
    struct Fake {
        token_requests: Vec<String>,
        sends: Vec<(HeaderMap, serde_json::Value)>,
        send_response: (u16, String),
    }

    fn service_account(token_uri: &str) -> Vec<u8> {
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let pem = openssl::pkey::PKey::from_rsa(rsa)
            .unwrap()
            .private_key_to_pem_pkcs8()
            .unwrap();
        serde_json::to_vec(&serde_json::json!({
            "client_email": "gateway@example-project.iam.example.org",
            "private_key": String::from_utf8(pem).unwrap(),
            "token_uri": token_uri,
        }))
        .unwrap()
    }

    async fn transport() -> (FcmTransport, Arc<Mutex<Fake>>) {
        let fake = Arc::new(Mutex::new(Fake {
            send_response: (200, "{}".into()),
            ..Fake::default()
        }));
        let app = Router::new()
            .route(
                "/token",
                post(|State(f): State<Arc<Mutex<Fake>>>, body: Bytes| async move {
                    f.lock()
                        .unwrap()
                        .token_requests
                        .push(String::from_utf8(body.to_vec()).unwrap());
                    format!(
                        r#"{{"access_token":"access-{}","expires_in":3600}}"#,
                        f.lock().unwrap().token_requests.len()
                    )
                }),
            )
            .route(
                "/v1/projects/example-project/messages:send",
                post(
                    |State(f): State<Arc<Mutex<Fake>>>, headers: HeaderMap, body: Bytes| async move {
                        let mut f = f.lock().unwrap();
                        f.sends.push((headers, serde_json::from_slice(&body).unwrap()));
                        (Status::from_u16(f.send_response.0).unwrap(), f.send_response.1.clone())
                    },
                ),
            )
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let transport = FcmTransport::with_endpoints(
            "example-project",
            &service_account(&format!("{base}/token")),
            &base,
            || NOW,
        )
        .unwrap();
        (transport, fake)
    }

    fn attempt(expires_in: i64) -> DeliveryAttempt {
        DeliveryAttempt {
            request_id: uuid::Uuid::nil(),
            expires_at: NOW + expires_in,
        }
    }

    #[tokio::test]
    async fn wake_is_a_fixed_high_priority_data_message_with_ttl_from_expiry() {
        let (transport, fake) = transport().await;
        for expires_in in [60, 0, -5, i64::MAX / 2] {
            assert_eq!(
                transport.send(attempt(expires_in), TOKEN).await,
                DeliveryOutcome::Accepted
            );
        }
        let fake = fake.lock().unwrap();
        // One service account token serves all four sends.
        assert_eq!(fake.token_requests.len(), 1);
        assert!(fake.token_requests[0]
            .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"));
        for (headers, _) in &fake.sends {
            assert_eq!(headers["authorization"], "Bearer access-1");
        }
        let ttls: Vec<_> = fake
            .sends
            .iter()
            .map(|(_, b)| b["message"]["android"]["ttl"].clone())
            .collect();
        assert_eq!(ttls, ["60s", "0s", "0s", "2419200s"]);
        // Everything but the token and ttl is compiled in.
        for (_, body) in &fake.sends {
            let mut body = body.clone();
            body["message"]["android"]["ttl"] = "TTL".into();
            assert_eq!(
                body,
                serde_json::json!({"message": {
                    "token": TOKEN,
                    "data": {"buzz_wake": "1"},
                    "android": {"priority": "HIGH", "collapse_key": "buzz-wake", "ttl": "TTL"},
                }})
            );
        }
    }

    #[tokio::test]
    async fn responses_map_to_endpoint_and_provider_outcomes() {
        let unregistered = r#"{"error":{"code":404,"status":"NOT_FOUND","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#;
        let (transport, fake) = transport().await;
        for (code, body, expected) in [
            (
                404,
                unregistered,
                DeliveryOutcome::InvalidEndpoint {
                    unregistered_at: None,
                },
            ),
            (
                400,
                r#"{"error":{"details":[{"errorCode":"INVALID_ARGUMENT"}]}}"#,
                DeliveryOutcome::ConfigurationFault,
            ),
            (
                403,
                r#"{"error":{"details":[{"errorCode":"SENDER_ID_MISMATCH"}]}}"#,
                DeliveryOutcome::ConfigurationFault,
            ),
            (
                429,
                r#"{"error":{"details":[{"errorCode":"QUOTA_EXCEEDED"}]}}"#,
                DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                },
            ),
            (
                503,
                "not json",
                DeliveryOutcome::Retry {
                    retry_after_seconds: None,
                },
            ),
            (400, "not json", DeliveryOutcome::PermanentRequestFault),
        ] {
            fake.lock().unwrap().send_response = (code, body.to_owned());
            assert_eq!(
                transport.send(attempt(60), TOKEN).await,
                expected,
                "{code} {body}"
            );
        }
        // A refused access token is dropped, so the next send fetches a new one.
        fake.lock().unwrap().send_response = (401, "{}".into());
        assert_eq!(
            transport.send(attempt(60), TOKEN).await,
            DeliveryOutcome::ConfigurationFault
        );
        fake.lock().unwrap().send_response = (200, "{}".into());
        assert_eq!(
            transport.send(attempt(60), TOKEN).await,
            DeliveryOutcome::Accepted
        );
        let fake = fake.lock().unwrap();
        assert_eq!(fake.token_requests.len(), 2);
        assert_eq!(
            fake.sends.last().unwrap().0["authorization"],
            "Bearer access-2"
        );
    }

    #[test]
    fn malformed_service_account_fails_as_a_credential_error() {
        for key in [
            &b""[..],
            b"{}",
            br#"{"client_email":"a","private_key":"nope","token_uri":"b"}"#,
        ] {
            assert!(matches!(
                FcmTransport::with_endpoints("example-project", key, FCM_BASE_URL, || NOW),
                Err(FcmError::Credential)
            ));
        }
    }
}
