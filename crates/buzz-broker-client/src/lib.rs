//! Shared HTTP transport for the agent broker.
//!
//! Implements [`buzz_sdk::broker::BrokerClient`], the transport primitive the
//! contract crate deliberately omits: frozen request bytes out, one envelope
//! back. Callers use [`buzz_sdk::broker::BrokerClientExt::execute`], which adds
//! correlation checks; this type never interprets a verdict.

use buzz_sdk::broker::actions::MAX_ENCODED_MESSAGE_BYTES;
use buzz_sdk::broker::{
    BrokerClient, BrokerFuture, BrokerResponse, BrokerTransportError, Dispatch, PreparedRequest,
    BROKER_ACTION_PATH, BROKER_CREDENTIAL_HEADER,
};

const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

fn max_response_bytes(request: &PreparedRequest) -> usize {
    request
        .channel_read_limit()
        .map_or(MAX_RESPONSE_BYTES, |limit| {
            MAX_RESPONSE_BYTES
                .saturating_add((limit as usize).saturating_mul(MAX_ENCODED_MESSAGE_BYTES))
        })
}

/// Invalid broker provisioning rejected before a bearer credential can leave
/// the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerClientConfigError(String);

impl std::fmt::Display for BrokerClientConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BrokerClientConfigError {}

/// A broker host endpoint plus the agent's bearer credential.
///
/// Holds no key and knows nothing of the relay: its whole authority is the
/// opaque credential it replays on every request.
#[derive(Clone)]
pub struct HttpBrokerClient {
    base_url: String,
    credential: String,
    http: reqwest::Client,
    request_timeout: std::time::Duration,
}

impl HttpBrokerClient {
    /// A client posting to `base_url` with `credential` as its bearer token.
    pub fn new(
        base_url: impl Into<String>,
        credential: impl Into<String>,
    ) -> Result<Self, BrokerClientConfigError> {
        Self::with_builder_timeout(
            base_url,
            credential,
            reqwest::Client::builder(),
            DEFAULT_REQUEST_TIMEOUT,
        )
    }

    fn with_builder_timeout(
        base_url: impl Into<String>,
        credential: impl Into<String>,
        mut http: reqwest::ClientBuilder,
        request_timeout: std::time::Duration,
    ) -> Result<Self, BrokerClientConfigError> {
        let base_url = base_url.into();
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|error| BrokerClientConfigError(format!("invalid broker URL: {error}")))?;
        let loopback = match parsed.host() {
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        };
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback) {
            return Err(BrokerClientConfigError(
                "broker URL must use HTTPS (plain HTTP is allowed only for loopback)".into(),
            ));
        }
        if parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(BrokerClientConfigError(
                "broker URL must be an origin/path without credentials, query, or fragment".into(),
            ));
        }

        let credential = credential.into();
        if credential.trim().is_empty() {
            return Err(BrokerClientConfigError(
                "broker credential must not be empty".into(),
            ));
        }
        reqwest::header::HeaderValue::from_str(&format!("Bearer {credential}"))
            .map_err(|_| BrokerClientConfigError("broker credential is not header-safe".into()))?;

        http = http.redirect(reqwest::redirect::Policy::none());
        if parsed.scheme() == "http" && loopback {
            // Plaintext loopback is safe only when it is structurally direct.
            // System or explicitly configured proxies must never receive the
            // bearer credential on this path.
            http = http.no_proxy();
        }
        let http = http.build().map_err(|error| {
            BrokerClientConfigError(format!("failed to build broker HTTP client: {error}"))
        })?;

        Ok(Self {
            base_url,
            credential,
            http,
            request_timeout,
        })
    }
}

impl BrokerClient for HttpBrokerClient {
    fn send<'a>(&'a self, request: &'a PreparedRequest, _dispatch: Dispatch) -> BrokerFuture<'a> {
        Box::pin(async move {
            let max_response_bytes = max_response_bytes(request);
            let url = format!(
                "{}{BROKER_ACTION_PATH}",
                self.base_url.trim_end_matches('/')
            );
            let (status, body) = tokio::time::timeout(self.request_timeout, async {
                let mut response = self
                    .http
                    .post(url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .header(
                        BROKER_CREDENTIAL_HEADER,
                        format!("Bearer {}", self.credential),
                    )
                    .body(request.body().to_vec())
                    .send()
                    .await
                    .map_err(|e| BrokerTransportError::Unreachable(e.to_string()))?;

                let status = response.status().as_u16();
                if response
                    .content_length()
                    .is_some_and(|length| length > max_response_bytes as u64)
                {
                    return Err(BrokerTransportError::NoEnvelope {
                        status,
                        detail: format!("response exceeds {max_response_bytes} bytes"),
                    });
                }
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|e| BrokerTransportError::Unreachable(e.to_string()))?
                {
                    if body.len().saturating_add(chunk.len()) > max_response_bytes {
                        return Err(BrokerTransportError::NoEnvelope {
                            status,
                            detail: format!("response exceeds {max_response_bytes} bytes"),
                        });
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok::<_, BrokerTransportError>((status, body))
            })
            .await
            .map_err(|_| {
                BrokerTransportError::Unreachable(format!(
                    "broker request timed out after {} seconds",
                    self.request_timeout.as_secs_f64()
                ))
            })??;

            // Parse an envelope whatever the status. Only its absence makes the
            // status meaningful, and then only as operator detail.
            serde_json::from_slice::<BrokerResponse>(&body).map_err(|e| {
                BrokerTransportError::NoEnvelope {
                    status,
                    detail: e.to_string(),
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::body::{Body, Bytes};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::Response;
    use axum::routing::{any, post};
    use axum::Router;
    use buzz_sdk::broker::actions::{MAX_CONTENT_BYTES, MAX_ENCODED_MESSAGE_BYTES};
    use buzz_sdk::broker::{
        ActionArgs, ActionOutcome, BrokerClientExt, BrokerMessage, BrokerRequest, BrokerResult,
        ChannelReadArgs, MessagePage, MessagePostArgs,
    };
    use nostr::{EventBuilder, Keys, Kind};
    use tokio::net::TcpListener;

    const CHANNEL: &str = "5df7dfa8-e919-43df-8efd-f1dcb8af7071";
    const CRED: &str = "test-cred";

    /// Spawn a broker that answers `/v1/action` with a fixed `(status, body)`
    /// and records the `Authorization` header it saw.
    async fn spawn(status: StatusCode, body: String) -> (String, Arc<Mutex<Option<String>>>) {
        let seen_auth = Arc::new(Mutex::new(None));
        type S = (Arc<(StatusCode, String)>, Arc<Mutex<Option<String>>>);
        let state: S = (Arc::new((status, body)), seen_auth.clone());

        let app = Router::new()
            .route(
                BROKER_ACTION_PATH,
                post(
                    |State((canned, seen)): State<S>, headers: HeaderMap, _body: Bytes| async move {
                        *seen.lock().unwrap() = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        Response::builder()
                            .status(canned.0)
                            .header("content-type", "application/json")
                            .body(Body::from(canned.1.clone()))
                            .unwrap()
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen_auth)
    }

    async fn spawn_capture_proxy() -> (String, Arc<Mutex<Option<String>>>) {
        let seen_auth = Arc::new(Mutex::new(None));
        let state = seen_auth.clone();
        let app = Router::new()
            .fallback(any(
                |State(seen): State<Arc<Mutex<Option<String>>>>, headers: HeaderMap| async move {
                    *seen.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    StatusCode::BAD_GATEWAY
                },
            ))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen_auth)
    }

    fn post_request() -> PreparedRequest {
        let args = ActionArgs::MessagePost(MessagePostArgs {
            channel_id: CHANNEL.to_string(),
            content: "hi".to_string(),
            mentions: Vec::new(),
        });
        BrokerRequest::new("req-post-1", args)
            .unwrap()
            .prepare()
            .unwrap()
    }

    fn channel_read_request() -> PreparedRequest {
        let args = ActionArgs::ChannelRead(ChannelReadArgs {
            channel_id: CHANNEL.to_string(),
            root_event_id: None,
            mentions_only: false,
            cursor: None,
            limit: Some(100),
        });
        BrokerRequest::new("req-read-1", args)
            .unwrap()
            .prepare()
            .unwrap()
    }

    #[tokio::test]
    async fn success_round_trips_and_sends_bearer_credential() {
        let req = post_request();
        let body = format!(
            r#"{{"type":"broker_result","protocolVersion":1,"requestId":"{}","status":"succeeded","action":"message.post","outcome":{{"eventId":"{}","kind":9,"createdAt":1700000000}}}}"#,
            req.request_id(),
            "a".repeat(64),
        );
        let (base, seen_auth) = spawn(StatusCode::OK, body).await;

        let client = HttpBrokerClient::new(base, CRED).unwrap();
        let validated = client.execute(&req).await.expect("a verdict");

        assert!(matches!(validated.result(), BrokerResult::Succeeded { .. }));
        assert_eq!(
            seen_auth.lock().unwrap().as_deref(),
            Some("Bearer test-cred")
        );
    }

    #[tokio::test]
    async fn rejected_credential_is_a_verdict_not_a_transport_error() {
        let req = post_request();
        let body = format!(
            r#"{{"type":"broker_result","protocolVersion":1,"requestId":"{}","status":"failed","error":{{"code":"unauthenticated","message":"nope"}}}}"#,
            req.request_id(),
        );
        let (base, _) = spawn(StatusCode::OK, body).await;

        let client = HttpBrokerClient::new(base, CRED).unwrap();
        let validated = client.execute(&req).await.expect("a verdict");

        assert!(matches!(validated.result(), BrokerResult::Failed { .. }));
    }

    #[tokio::test]
    async fn non_envelope_response_is_a_transport_error() {
        let req = post_request();
        let (base, _) = spawn(StatusCode::BAD_GATEWAY, "upstream boom".to_string()).await;

        let client = HttpBrokerClient::new(base, CRED).unwrap();
        let err = client.execute(&req).await.expect_err("no envelope");

        assert!(matches!(
            err,
            BrokerTransportError::NoEnvelope { status: 502, .. }
        ));
    }

    #[tokio::test]
    async fn unreachable_host_is_a_transport_error() {
        let req = post_request();
        let client = HttpBrokerClient::new("http://127.0.0.1:1", CRED).unwrap();
        let err = client.execute(&req).await.expect_err("unreachable");

        assert!(matches!(err, BrokerTransportError::Unreachable(_)));
    }

    #[tokio::test]
    async fn hung_host_is_bounded_by_the_transport_timeout() {
        let app = Router::new().route(
            BROKER_ACTION_PATH,
            post(|| async { std::future::pending::<Response>().await }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = HttpBrokerClient::with_builder_timeout(
            format!("http://{addr}"),
            CRED,
            reqwest::Client::builder(),
            std::time::Duration::from_millis(20),
        )
        .unwrap();
        let err = client.execute(&post_request()).await.expect_err("timeout");

        assert!(
            matches!(err, BrokerTransportError::Unreachable(message) if message.contains("timed out"))
        );
    }

    #[test]
    fn remote_plaintext_and_empty_credentials_are_rejected() {
        assert!(HttpBrokerClient::new("http://broker.example", CRED).is_err());
        assert!(HttpBrokerClient::new("https://broker.example", " ").is_err());
        assert!(HttpBrokerClient::new("http://localhost:8787", CRED).is_ok());
        assert!(HttpBrokerClient::new("http://[::1]:8787", CRED).is_ok());
    }

    #[tokio::test]
    async fn plaintext_loopback_never_sends_the_bearer_through_a_proxy() {
        let request = post_request();
        let body = format!(
            r#"{{"type":"broker_result","protocolVersion":1,"requestId":"{}","status":"succeeded","action":"message.post","outcome":{{"eventId":"{}","kind":9,"createdAt":1700000000}}}}"#,
            request.request_id(),
            "a".repeat(64),
        );
        let (broker, broker_auth) = spawn(StatusCode::OK, body).await;
        let (proxy, proxy_auth) = spawn_capture_proxy().await;
        let client = HttpBrokerClient::with_builder_timeout(
            broker,
            CRED,
            reqwest::Client::builder().proxy(reqwest::Proxy::all(proxy).unwrap()),
            DEFAULT_REQUEST_TIMEOUT,
        )
        .unwrap();

        client
            .execute(&request)
            .await
            .expect("direct broker response");

        assert_eq!(
            broker_auth.lock().unwrap().as_deref(),
            Some("Bearer test-cred")
        );
        assert_eq!(proxy_auth.lock().unwrap().as_deref(), None);
    }

    #[tokio::test]
    async fn oversized_response_is_rejected_before_parsing() {
        let (base, _) = spawn(StatusCode::OK, "x".repeat(MAX_RESPONSE_BYTES + 1)).await;
        let error = HttpBrokerClient::new(base, CRED)
            .unwrap()
            .execute(&post_request())
            .await
            .expect_err("oversized response");
        assert!(matches!(
            error,
            BrokerTransportError::NoEnvelope { detail, .. } if detail.contains("exceeds")
        ));
    }

    #[tokio::test]
    async fn contract_valid_channel_page_may_exceed_default_response_bound() {
        let request = channel_read_request();
        let message = BrokerMessage(
            EventBuilder::new(Kind::Custom(9), "x".repeat(MAX_CONTENT_BYTES))
                .sign_with_keys(&Keys::generate())
                .expect("fixture event signs"),
        );
        let outcome = ActionOutcome::ChannelRead(MessagePage {
            messages: vec![message; 40],
            next_cursor: None,
        });
        let body = serde_json::to_string(&BrokerResponse::new(
            request.request_id(),
            BrokerResult::succeeded(outcome),
        ))
        .unwrap();
        assert!(body.len() > MAX_RESPONSE_BYTES);
        assert!(body.len() < max_response_bytes(&request));
        let (base, _) = spawn(StatusCode::OK, body).await;

        let response = HttpBrokerClient::new(base, CRED)
            .unwrap()
            .execute(&request)
            .await
            .expect("valid channel page");

        assert!(matches!(response.result(), BrokerResult::Succeeded { .. }));
    }

    #[tokio::test]
    async fn channel_read_limit_one_hundred_has_a_request_sized_response_cap() {
        let request = channel_read_request();
        let response_cap = max_response_bytes(&request);
        assert_eq!(request.channel_read_limit(), Some(100));
        assert_eq!(
            response_cap,
            MAX_RESPONSE_BYTES + 100 * MAX_ENCODED_MESSAGE_BYTES
        );

        let (base, _) = spawn(StatusCode::OK, "x".repeat(response_cap + 1)).await;
        let error = HttpBrokerClient::new(base, CRED)
            .unwrap()
            .execute(&request)
            .await
            .expect_err("response over the requested page budget");
        assert!(matches!(
            error,
            BrokerTransportError::NoEnvelope { detail, .. }
                if detail.contains(&response_cap.to_string())
        ));
    }

    #[tokio::test]
    async fn valid_maximally_escaped_channel_page_fits_the_request_cap() {
        let request = channel_read_request();
        let message = BrokerMessage(
            EventBuilder::new(Kind::Custom(9), "\u{001b}".repeat(MAX_CONTENT_BYTES))
                .tags([nostr::Tag::parse(["h", CHANNEL]).unwrap()])
                .sign_with_keys(&Keys::generate())
                .expect("fixture event signs"),
        );
        assert!(serde_json::to_vec(&message).unwrap().len() <= MAX_ENCODED_MESSAGE_BYTES);
        let outcome = ActionOutcome::ChannelRead(MessagePage {
            messages: vec![message; 100],
            next_cursor: None,
        });
        let body = serde_json::to_string(&BrokerResponse::new(
            request.request_id(),
            BrokerResult::succeeded(outcome),
        ))
        .unwrap();
        assert!(body.len() > 32 * 1024 * 1024);
        assert!(body.len() <= max_response_bytes(&request));
        let (base, _) = spawn(StatusCode::OK, body).await;

        HttpBrokerClient::new(base, CRED)
            .unwrap()
            .execute(&request)
            .await
            .expect("contract-valid escaped page");
    }
}
