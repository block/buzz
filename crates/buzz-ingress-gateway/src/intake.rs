//! Public intake: `POST /webhooks/{integration_id}`.
//!
//! ```text
//! provider ──POST──▶ host == integration.community_host? ──▶ enabled? ──▶ adapter.verify
//!                                                                        (opened secrets)
//!                                                                              │
//!                     202 queued / 200 duplicate ◀── insert_delivery ◀── rate limit
//! ```
//!
//! The intake path never calls the relay. A delivery is acknowledged once its
//! row is committed; verification happens before any write. The rate limit
//! sits after verification so an unauthenticated caller cannot spend an
//! integration's budget; it bounds how fast verified events reach the queue.
use crate::http::{error, AppState};
use crate::metrics::{record_intake, IntakeOutcome};
use crate::providers::{adapter, VerifyError};
use crate::store::{InsertOutcome, NewDelivery};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header::HOST, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use chrono::{DateTime, Duration, SubsecRound, Utc};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// Intake routes, mounted on the public router only.
pub fn routes() -> Router<AppState> {
    Router::new().route("/webhooks/{integration_id}", post(receive))
}

/// Fixed one-minute window per integration. Keys are ids of integrations
/// that passed verification, so the map is bounded by the integration count;
/// stale windows are pruned once it grows large.
pub struct RateLimiter {
    per_minute: u32,
    windows: Mutex<HashMap<Uuid, (i64, u32)>>,
}

const RATE_LIMITER_PRUNE_AT: usize = 10_000;

impl RateLimiter {
    /// Limiter admitting `per_minute` verified deliveries per integration.
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Whether one more verified delivery for `integration_id` fits in the
    /// current minute. Lock poisoning fails open: the limiter protects the
    /// queue from floods, it is not a security boundary.
    pub fn admit(&self, integration_id: Uuid, now: DateTime<Utc>) -> bool {
        let minute = now.timestamp().div_euclid(60);
        let Ok(mut windows) = self.windows.lock() else {
            return true;
        };
        if windows.len() >= RATE_LIMITER_PRUNE_AT {
            windows.retain(|_, (window, _)| *window == minute);
        }
        let entry = windows.entry(integration_id).or_insert((minute, 0));
        if entry.0 != minute {
            *entry = (minute, 0);
        }
        if entry.1 >= self.per_minute {
            return false;
        }
        entry.1 += 1;
        true
    }
}

fn request_host(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(HOST)?.to_str().ok()?.trim();
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn reply(outcome: IntakeOutcome, status: StatusCode, code: &'static str) -> Response {
    record_intake(outcome);
    error(status, code)
}

async fn receive(
    State(state): State<AppState>,
    Path(integration_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = (state.now)().trunc_subsecs(6);
    let integration = match state.store.integration(integration_id).await {
        Ok(Some(integration)) => integration,
        Ok(None) => return reply(IntakeOutcome::NotFound, StatusCode::NOT_FOUND, "not_found"),
        Err(_) => {
            return reply(
                IntakeOutcome::Unavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
            )
        }
    };
    if request_host(&headers).as_deref() != Some(integration.community_host.as_str()) {
        return reply(IntakeOutcome::NotFound, StatusCode::NOT_FOUND, "not_found");
    }
    if !integration.enabled {
        return reply(
            IntakeOutcome::Disabled,
            StatusCode::GONE,
            "integration_disabled",
        );
    }
    let Some(adapter) = adapter(integration.provider) else {
        return reply(
            IntakeOutcome::Invalid,
            StatusCode::BAD_REQUEST,
            "unsupported_provider",
        );
    };
    let sealed = match state.store.verifying_secrets(integration_id, now).await {
        Ok(sealed) => sealed,
        Err(_) => {
            return reply(
                IntakeOutcome::Unavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
            )
        }
    };
    if sealed.is_empty() {
        return reply(
            IntakeOutcome::PendingSecret,
            StatusCode::CONFLICT,
            "pending_secret",
        );
    }
    let secrets: Vec<Vec<u8>> = sealed
        .iter()
        .filter_map(|secret| state.secrets.open(&secret.ciphertext).ok())
        .collect();
    if secrets.is_empty() {
        tracing::error!(%integration_id, "no stored secret opens with the configured keyring");
        return reply(
            IntakeOutcome::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        );
    }
    let verified = match adapter.verify(
        &headers,
        &body,
        &secrets,
        now,
        Duration::seconds(state.intake.timestamp_tolerance_seconds),
    ) {
        Ok(verified) => verified,
        Err(
            VerifyError::MissingSignature | VerifyError::BadSignature | VerifyError::StaleTimestamp,
        ) => {
            return reply(
                IntakeOutcome::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            )
        }
        Err(VerifyError::MalformedHeaders | VerifyError::UnsupportedEvent) => {
            return reply(
                IntakeOutcome::Invalid,
                StatusCode::BAD_REQUEST,
                "invalid_delivery",
            )
        }
    };
    if !state.rate_limiter.admit(integration_id, now) {
        return reply(
            IntakeOutcome::RateLimited,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
        );
    }
    let retained: serde_json::Map<String, serde_json::Value> = adapter
        .retained_headers()
        .iter()
        .filter_map(|name| {
            let value = headers.get(*name)?.to_str().ok()?;
            Some((
                (*name).to_owned(),
                serde_json::Value::String(value.to_owned()),
            ))
        })
        .collect();
    let delivery_id = verified.delivery_id.clone();
    let outcome = state
        .store
        .insert_delivery(NewDelivery {
            integration_id,
            delivery_id: delivery_id.clone(),
            event_type: verified.event_type,
            provider_timestamp: verified.provider_timestamp,
            headers: serde_json::Value::Object(retained),
            body: body.to_vec(),
            received_at: now,
        })
        .await;
    match outcome {
        Ok(InsertOutcome::Inserted) => {
            record_intake(IntakeOutcome::Queued);
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({ "delivery_id": delivery_id, "state": "queued" })),
            )
                .into_response()
        }
        Ok(InsertOutcome::Duplicate) => {
            record_intake(IntakeOutcome::Duplicate);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "delivery_id": delivery_id, "state": "duplicate" })),
            )
                .into_response()
        }
        Err(_) => reply(
            IntakeOutcome::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::IntakeLimits;
    use crate::secrets::{SecretKey, SecretKeyring};
    use crate::store::{DeliveryState, MemoryStore, Mutation, Operation, Provider, Store};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use std::sync::{atomic::AtomicBool, Arc};
    use tower::ServiceExt;

    const HOST_NAME: &str = "acme.communities.example.org";

    struct Fixture {
        public: Router,
        store: Arc<MemoryStore>,
        id: Uuid,
    }

    async fn fixture(per_minute: u32) -> Fixture {
        let store = Arc::new(MemoryStore::default());
        let secrets =
            Arc::new(SecretKeyring::new(vec![SecretKey::new("test", &[9; 32]).unwrap()]).unwrap());
        let id = Uuid::new_v4();
        let now = Utc::now();
        store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id: id,
                    revision: 1,
                    mutation: Mutation::PutIntegration {
                        community_host: HOST_NAME.into(),
                        provider: Provider::Github,
                    },
                },
                now,
            )
            .await
            .unwrap();
        let state = AppState {
            store: store.clone(),
            secrets,
            relay_pubkeys: Arc::new(Vec::new()),
            intake: IntakeLimits {
                max_body_bytes: 1024,
                timestamp_tolerance_seconds: 300,
            },
            rate_limiter: Arc::new(RateLimiter::new(per_minute)),
            now: Utc::now,
            accepting: Arc::new(AtomicBool::new(true)),
            public_scheme: crate::config::RelayScheme::Https,
        };
        let (public, _) = crate::http::router(state);
        Fixture { public, store, id }
    }

    async fn set_secret(fx: &Fixture, revision: i64, secret: &[u8], grace_secs: i64) {
        let keyring = SecretKeyring::new(vec![SecretKey::new("test", &[9; 32]).unwrap()]).unwrap();
        fx.store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id: fx.id,
                    revision,
                    mutation: Mutation::PutSecret {
                        key_id: "test".into(),
                        ciphertext: keyring.seal(secret).unwrap(),
                        grace: Duration::seconds(grace_secs),
                    },
                },
                Utc::now(),
            )
            .await
            .unwrap();
    }

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn deliver(
        fx: &Fixture,
        host: &str,
        secret: &[u8],
        delivery_id: &str,
        body: &[u8],
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::post(format!("/webhooks/{}", fx.id))
            .header("host", host)
            .header("content-type", "application/json")
            .header("x-github-delivery", delivery_id)
            .header("x-github-event", "pull_request")
            .header("x-github-hook-id", "12345")
            .header("x-hub-signature-256", sign(secret, body))
            .body(Body::from(body.to_vec()))
            .unwrap();
        let response = fx.public.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn verified_deliveries_are_queued_once_and_stored_byte_for_byte() {
        let fx = fixture(100).await;
        let body = br#"{"action":"opened","pull_request":{"number":7}}"#;

        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{json}");
        assert_eq!(json["error"], "pending_secret");

        set_secret(&fx, 2, b"whsec", 0).await;
        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{json}");
        assert_eq!(
            json,
            serde_json::json!({ "delivery_id": "d1", "state": "queued" })
        );

        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["state"], "duplicate");

        let stored = fx.store.delivery(fx.id, "d1").unwrap();
        assert_eq!(stored.state, DeliveryState::Queued);
        assert_eq!(stored.delivery.body, body.to_vec());
        assert_eq!(stored.delivery.event_type, "pull_request.opened");
        assert_eq!(stored.delivery.headers["x-github-event"], "pull_request");
        assert_eq!(stored.delivery.headers["x-github-hook-id"], "12345");
        assert!(stored.delivery.headers.get("x-hub-signature-256").is_none());
        assert_eq!(stored.next_attempt_at, Some(stored.delivery.received_at));
    }

    #[tokio::test]
    async fn host_binding_disablement_and_signatures_gate_intake() {
        let fx = fixture(100).await;
        set_secret(&fx, 2, b"whsec", 0).await;
        let body = br#"{"action":"opened"}"#;

        let (status, _) = deliver(&fx, "other.communities.example.org", b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = deliver(&fx, &format!("{HOST_NAME}:443"), b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let (status, _) = deliver(&fx, HOST_NAME, b"wrong", "d2", body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(fx.store.delivery(fx.id, "d2").is_none());

        let (status, _) = deliver(&fx, HOST_NAME, b"whsec", "", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let unknown = Request::post(format!("/webhooks/{}", Uuid::new_v4()))
            .header("host", HOST_NAME)
            .body(Body::from("{}"))
            .unwrap();
        let response = fx.public.clone().oneshot(unknown).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        fx.store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id: fx.id,
                    revision: 3,
                    mutation: Mutation::Disable,
                },
                Utc::now(),
            )
            .await
            .unwrap();
        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d3", body).await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(json["error"], "integration_disabled");
    }

    #[tokio::test]
    async fn rotated_secrets_verify_through_their_grace_window() {
        let fx = fixture(100).await;
        set_secret(&fx, 2, b"old", 0).await;
        set_secret(&fx, 3, b"new", 600).await;
        let body = br#"{"action":"closed"}"#;
        let (status, _) = deliver(&fx, HOST_NAME, b"old", "d1", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, _) = deliver(&fx, HOST_NAME, b"new", "d2", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);

        set_secret(&fx, 4, b"newest", 0).await;
        let (status, _) = deliver(&fx, HOST_NAME, b"new", "d3", body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = deliver(&fx, HOST_NAME, b"newest", "d3", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn floods_and_oversized_bodies_are_refused_before_storage() {
        let fx = fixture(2).await;
        set_secret(&fx, 2, b"whsec", 0).await;
        let body = br#"{"action":"opened"}"#;
        for _ in 0..3 {
            let (status, _) = deliver(&fx, HOST_NAME, b"wrong", "d0", body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = deliver(&fx, HOST_NAME, b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, _) = deliver(&fx, HOST_NAME, b"whsec", "d2", body).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d3", body).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json["error"], "rate_limited");
        assert!(fx.store.delivery(fx.id, "d3").is_none());

        let fx = fixture(100).await;
        set_secret(&fx, 2, b"whsec", 0).await;
        let big = vec![b' '; 1025];
        let (status, _) = deliver(&fx, HOST_NAME, b"whsec", "d4", &big).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn secrets_sealed_under_an_unknown_key_fail_closed() {
        let fx = fixture(100).await;
        let other = SecretKeyring::new(vec![SecretKey::new("other", &[1; 32]).unwrap()]).unwrap();
        fx.store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id: fx.id,
                    revision: 2,
                    mutation: Mutation::PutSecret {
                        key_id: "other".into(),
                        ciphertext: other.seal(b"whsec").unwrap(),
                        grace: Duration::zero(),
                    },
                },
                Utc::now(),
            )
            .await
            .unwrap();
        let body = br#"{"action":"opened"}"#;
        let (status, json) = deliver(&fx, HOST_NAME, b"whsec", "d1", body).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json["error"], "temporarily_unavailable");
        assert!(fx.store.delivery(fx.id, "d1").is_none());
    }

    #[test]
    fn rate_limiter_windows_roll_over_and_prune() {
        let limiter = RateLimiter::new(1);
        let id = Uuid::new_v4();
        let t0 = DateTime::from_timestamp(1_700_000_040, 0).unwrap();
        assert!(limiter.admit(id, t0));
        assert!(!limiter.admit(id, t0 + Duration::seconds(59)));
        assert!(limiter.admit(id, t0 + Duration::seconds(60)));
        for _ in 0..RATE_LIMITER_PRUNE_AT {
            limiter.admit(Uuid::new_v4(), t0);
        }
        assert!(limiter.admit(Uuid::new_v4(), t0 + Duration::seconds(120)));
        assert!(limiter.windows.lock().unwrap().len() < RATE_LIMITER_PRUNE_AT);
    }

    #[test]
    fn request_host_strips_ports_and_normalizes_case() {
        let mut h = HeaderMap::new();
        h.insert("host", "Acme.Example.ORG:8443".parse().unwrap());
        assert_eq!(request_host(&h).as_deref(), Some("acme.example.org"));
        h.insert("host", "[::1]:8080".parse().unwrap());
        assert_eq!(request_host(&h).as_deref(), Some("[::1]"));
        h.insert("host", ":8080".parse().unwrap());
        assert_eq!(request_host(&h), None);
        h.remove("host");
        assert_eq!(request_host(&h), None);
    }
}
