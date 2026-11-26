//! Outbound submission of signed events to a community relay.
//!
//! ```text
//! forwarder ──POST {scheme}://{community_host}/events──▶ relay
//!            Authorization: Nostr <NIP-98 event signed by the message author>
//!            body: the kind-9 channel message
//! ```
//!
//! Every HTTP status the relay can return maps onto one of three outcomes the
//! forwarder acts on: accept the delivery, fail it for good, or retry later.
use async_trait::async_trait;
use nostr::hashes::{sha256::Hash as Sha256Hash, Hash};
use nostr::nips::nip98::HttpData;
use nostr::nips::nip98::HttpMethod;
use nostr::{Event, JsonUtil, NostrSigner, Url};
use reqwest::StatusCode;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Seam between the forwarder and the relay transport.
#[async_trait]
pub trait RelayClient: Send + Sync {
    /// Submit one signed event to `url`, authorizing the request with
    /// `signer` (the event's author), and classify the relay's answer.
    async fn submit(&self, url: &Url, event: &Event, signer: &Arc<dyn NostrSigner>)
        -> SubmitResult;
}

/// How the relay answered a submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitResult {
    /// The relay stored the event, or already had it.
    Accepted,
    /// The relay rejected the event and will keep rejecting it.
    Rejected(String),
    /// Transient failure; worth another attempt later.
    Retry(String),
}

const RESPONSE_LIMIT_BYTES: usize = 4096;
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(10);

/// [`RelayClient`] over HTTPS with NIP-98 request signing.
pub struct HttpRelayClient {
    client: reqwest::Client,
}

impl HttpRelayClient {
    /// Build a client; each submission brings its own signer.
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder().timeout(SUBMIT_TIMEOUT).build()?;
        Ok(Self { client })
    }
}

async fn authorization(
    url: &Url,
    body: &[u8],
    signer: &Arc<dyn NostrSigner>,
) -> Result<String, String> {
    HttpData::new(url.clone(), HttpMethod::POST)
        .payload(Sha256Hash::hash(body))
        .to_authorization(signer)
        .await
        .map_err(|e| format!("sign request: {e}"))
}

#[async_trait]
impl RelayClient for HttpRelayClient {
    async fn submit(
        &self,
        url: &Url,
        event: &Event,
        signer: &Arc<dyn NostrSigner>,
    ) -> SubmitResult {
        let body = event.as_json().into_bytes();
        let authorization = match authorization(url, &body, signer).await {
            Ok(h) => h,
            Err(e) => return SubmitResult::Rejected(e),
        };
        let response = self
            .client
            .post(url.clone())
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(r) => r,
            Err(e) => return SubmitResult::Retry(transport_error(&e)),
        };
        let status = response.status();
        let text = match bounded_body(response).await {
            Ok(text) => text,
            Err(e) => return SubmitResult::Retry(transport_error(&e)),
        };
        classify(status, &text)
    }
}

fn transport_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "relay request timed out".to_owned()
    } else if error.is_connect() {
        "relay connection failed".to_owned()
    } else {
        "relay request failed".to_owned()
    }
}

async fn bounded_body(mut response: reqwest::Response) -> Result<String, reqwest::Error> {
    let mut bytes = Vec::new();
    while bytes.len() < RESPONSE_LIMIT_BYTES {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        let room = RESPONSE_LIMIT_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[derive(Deserialize)]
struct SubmitResponse {
    accepted: Option<bool>,
    message: Option<String>,
    error: Option<String>,
}

/// Map a relay response onto a forwarder outcome. Success statuses still carry
/// an `accepted` flag because the relay reports a stored duplicate as not
/// accepted; a duplicate means an earlier attempt already landed.
pub fn classify(status: StatusCode, body: &str) -> SubmitResult {
    let parsed: Option<SubmitResponse> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|p| p.message.clone().or_else(|| p.error.clone()))
        .unwrap_or_else(|| format!("relay returned {}", status.as_u16()));
    match status {
        StatusCode::OK | StatusCode::ACCEPTED => match parsed.as_ref().and_then(|p| p.accepted) {
            Some(false) if !message.to_ascii_lowercase().contains("duplicate") => {
                SubmitResult::Rejected(message)
            }
            _ => SubmitResult::Accepted,
        },
        StatusCode::CONFLICT => SubmitResult::Accepted,
        StatusCode::BAD_REQUEST
        | StatusCode::UNAUTHORIZED
        | StatusCode::FORBIDDEN
        | StatusCode::NOT_FOUND => SubmitResult::Rejected(message),
        s if s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() => {
            SubmitResult::Retry(message)
        }
        _ => SubmitResult::Rejected(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use nostr::nips::nip98::verify_auth_header;
    use nostr::{EventBuilder, Keys, Kind, Timestamp};
    use std::sync::Mutex;

    #[test]
    fn classify_maps_relay_statuses() {
        let ok = r#"{"event_id":"ab","accepted":true,"message":""}"#;
        let dup =
            r#"{"event_id":"ab","accepted":false,"message":"duplicate: already have this event"}"#;
        let bad = r#"{"event_id":"ab","accepted":false,"message":"invalid: bad tag"}"#;
        assert_eq!(classify(StatusCode::OK, ok), SubmitResult::Accepted);
        assert_eq!(classify(StatusCode::OK, dup), SubmitResult::Accepted);
        assert_eq!(classify(StatusCode::OK, ""), SubmitResult::Accepted);
        assert_eq!(
            classify(StatusCode::OK, bad),
            SubmitResult::Rejected("invalid: bad tag".to_owned())
        );
        assert_eq!(classify(StatusCode::CONFLICT, ""), SubmitResult::Accepted);
        assert_eq!(
            classify(StatusCode::BAD_REQUEST, r#"{"error":"rejected"}"#),
            SubmitResult::Rejected("rejected".to_owned())
        );
        assert!(matches!(
            classify(StatusCode::FORBIDDEN, "nope"),
            SubmitResult::Rejected(m) if m == "relay returned 403"
        ));
        assert!(matches!(
            classify(StatusCode::TOO_MANY_REQUESTS, ""),
            SubmitResult::Retry(_)
        ));
        assert!(matches!(
            classify(StatusCode::BAD_GATEWAY, ""),
            SubmitResult::Retry(_)
        ));
        assert!(matches!(
            classify(StatusCode::IM_A_TEAPOT, ""),
            SubmitResult::Rejected(_)
        ));
    }

    type Request = (HeaderMap, Vec<u8>);

    #[derive(Clone)]
    struct Seen {
        relay_pubkey: nostr::PublicKey,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    async fn relay(
        State(seen): State<Seen>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> Json<serde_json::Value> {
        seen.requests.lock().unwrap().push((headers, body.to_vec()));
        Json(serde_json::json!({"event_id": "x", "accepted": true, "message": ""}))
    }

    #[tokio::test]
    async fn http_client_posts_signed_request() {
        let gateway = Keys::generate();
        let seen = Seen {
            relay_pubkey: gateway.public_key(),
            requests: Arc::default(),
        };
        let app = Router::new()
            .route("/events", post(relay))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let event = EventBuilder::new(Kind::Custom(9), "hi")
            .sign_with_keys(&gateway)
            .unwrap();
        let url = Url::parse(&format!("http://{addr}/events")).unwrap();
        let client = HttpRelayClient::new().unwrap();
        let signer: Arc<dyn NostrSigner> = Arc::new(gateway.clone());
        assert_eq!(
            client.submit(&url, &event, &signer).await,
            SubmitResult::Accepted
        );
        server.abort();

        let requests = seen.requests.lock().unwrap();
        let (headers, body) = &requests[0];
        assert_eq!(Event::from_json(body).unwrap().id, event.id);
        assert_eq!(headers["content-type"], "application/json");
        let auth = headers["authorization"].to_str().unwrap();
        let signer =
            verify_auth_header(auth, &url, HttpMethod::POST, Timestamp::now(), Some(body)).unwrap();
        assert_eq!(signer, seen.relay_pubkey);
        assert!(verify_auth_header(
            auth,
            &url,
            HttpMethod::POST,
            Timestamp::now(),
            Some(b"tampered")
        )
        .is_err());
    }

    #[tokio::test]
    async fn unreachable_relay_is_a_retry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "hi")
            .sign_with_keys(&keys)
            .unwrap();
        let url = Url::parse(&format!("http://{addr}/events")).unwrap();
        let client = HttpRelayClient::new().unwrap();
        let signer: Arc<dyn NostrSigner> = Arc::new(keys);
        assert!(matches!(
            client.submit(&url, &event, &signer).await,
            SubmitResult::Retry(_)
        ));
    }
}
