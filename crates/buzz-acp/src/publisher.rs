//! Harness-owned, least-authority message publisher for model-facing MCP tools.
//!
//! The ACP harness retains the Nostr signing key. Model-controlled MCP and shell
//! processes receive only a random loopback capability that can request one
//! typed operation: publish a channel message or threaded reply.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use buzz_sdk::ThreadRef;
use nostr::{Event, EventId, Filter, Kind};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::relay::RestClient;

const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = MAX_CONTENT_BYTES + 8 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct PublisherAccess {
    pub endpoint: String,
    pub capability: String,
}

#[derive(Clone)]
pub(crate) struct PublisherIssuer {
    endpoint: String,
    grants: Arc<RwLock<HashMap<String, Option<Uuid>>>>,
}

pub(crate) struct PublisherLease {
    access: PublisherAccess,
    grants: Arc<RwLock<HashMap<String, Option<Uuid>>>>,
}

impl PublisherLease {
    pub fn access(&self) -> &PublisherAccess {
        &self.access
    }
}

impl Drop for PublisherLease {
    fn drop(&mut self) {
        if let Ok(mut grants) = self.grants.write() {
            grants.remove(&self.access.capability);
        }
    }
}

impl PublisherIssuer {
    /// Issue one capability for one ACP session. Channel sessions are confined
    /// to their channel; the heartbeat session receives a wildcard grant so it
    /// can answer actionable mentions discovered across channels.
    pub fn issue(&self, channel: Option<Uuid>) -> PublisherLease {
        // Two UUIDv4 values provide a capability with ample entropy without
        // adding another randomness dependency.
        let capability = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        if let Ok(mut grants) = self.grants.write() {
            grants.insert(capability.clone(), channel);
        }
        PublisherLease {
            access: PublisherAccess {
                endpoint: self.endpoint.clone(),
                capability,
            },
            grants: Arc::clone(&self.grants),
        }
    }

    #[cfg(test)]
    pub fn capability_is_active(&self, capability: &str) -> bool {
        self.grants
            .read()
            .is_ok_and(|grants| grants.contains_key(capability))
    }
}

pub(crate) struct PublisherBroker {
    issuer: PublisherIssuer,
    task: JoinHandle<()>,
}

impl PublisherBroker {
    pub async fn start(rest_client: RestClient) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = listener.local_addr()?.to_string();
        let grants = Arc::new(RwLock::new(HashMap::new()));
        let issuer = PublisherIssuer {
            endpoint,
            grants: Arc::clone(&grants),
        };

        let task = tokio::spawn(async move {
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(error) => {
                        tracing::warn!("publisher broker accept failed: {error}");
                        break;
                    }
                };
                if !peer.ip().is_loopback() {
                    tracing::warn!(%peer, "publisher broker rejected non-loopback peer");
                    continue;
                }
                let grants = Arc::clone(&grants);
                let client = rest_client.clone();
                tokio::spawn(async move {
                    if let Err(error) = tokio::time::timeout(
                        CONNECTION_TIMEOUT,
                        serve_connection(stream, grants, client),
                    )
                    .await
                    {
                        tracing::warn!("publisher broker request timed out: {error}");
                    }
                });
            }
        });

        Ok(Self { issuer, task })
    }

    pub fn issuer(&self) -> PublisherIssuer {
        self.issuer.clone()
    }
}

impl Drop for PublisherBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Deserialize)]
struct PublishRequest {
    capability: String,
    channel: String,
    content: String,
    #[serde(default)]
    reply_to: Option<String>,
    #[serde(default)]
    mentions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublishResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl PublishResponse {
    fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            event_id: None,
            accepted: None,
            message: None,
            error: Some(error.into()),
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    grants: Arc<RwLock<HashMap<String, Option<Uuid>>>>,
    rest_client: RestClient,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).take((MAX_REQUEST_BYTES + 1) as u64);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;

    let response = if bytes.len() > MAX_REQUEST_BYTES {
        PublishResponse::error("publisher request exceeds size limit")
    } else {
        match serde_json::from_slice::<PublishRequest>(&bytes) {
            Ok(request) => publish_message(&grants, &rest_client, request).await,
            Err(error) => PublishResponse::error(format!("invalid publisher request: {error}")),
        }
    };

    let payload = serde_json::to_vec(&response).unwrap_or_else(|_| {
        br#"{"ok":false,"error":"publisher response serialization failed"}"#.to_vec()
    });
    write_half.write_all(&payload).await?;
    write_half.shutdown().await
}

async fn publish_message(
    grants: &RwLock<HashMap<String, Option<Uuid>>>,
    rest_client: &RestClient,
    request: PublishRequest,
) -> PublishResponse {
    if request.content.trim().is_empty() {
        return PublishResponse::error("content must not be empty");
    }
    if request.content.len() > MAX_CONTENT_BYTES {
        return PublishResponse::error(format!("content exceeds {MAX_CONTENT_BYTES} bytes"));
    }
    let channel = match Uuid::parse_str(&request.channel) {
        Ok(channel) => channel,
        Err(_) => return PublishResponse::error("channel must be a valid UUID"),
    };
    let grant = grants
        .read()
        .ok()
        .and_then(|grants| grants.get(&request.capability).copied());
    match grant {
        Some(Some(allowed_channel)) if allowed_channel != channel => {
            return PublishResponse::error(format!(
                "publisher capability is not authorized for channel {channel}"
            ))
        }
        Some(_) => {}
        None => return PublishResponse::error("invalid or expired publisher capability"),
    }

    let thread_ref = match request.reply_to.as_deref() {
        Some(parent) => match resolve_thread_ref(rest_client, parent, channel).await {
            Ok(thread_ref) => Some(thread_ref),
            Err(error) => return PublishResponse::error(error),
        },
        None => None,
    };

    let mention_refs = request
        .mentions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let builder = match buzz_sdk::build_message(
        channel,
        &request.content,
        thread_ref.as_ref(),
        &mention_refs,
        false,
        &[],
    ) {
        Ok(builder) => builder,
        Err(error) => return PublishResponse::error(format!("message validation failed: {error}")),
    };
    let event = match builder.sign_with_keys(&rest_client.keys) {
        Ok(event) => event,
        Err(error) => return PublishResponse::error(format!("message signing failed: {error}")),
    };
    let event_id = event.id.to_hex();
    let relay_response = match rest_client.submit_event(&event).await {
        Ok(response) => response,
        Err(error) => return PublishResponse::error(format!("relay publish failed: {error}")),
    };
    let accepted = relay_response
        .get("accepted")
        .and_then(|value| value.as_bool());
    let message = relay_response
        .get("message")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let ok = accepted != Some(false);

    PublishResponse {
        ok,
        event_id: Some(event_id),
        accepted,
        message,
        error: (!ok).then(|| "relay rejected message".to_string()),
    }
}

async fn resolve_thread_ref(
    rest_client: &RestClient,
    parent_hex: &str,
    expected_channel: Uuid,
) -> Result<ThreadRef, String> {
    let parent_event_id = EventId::from_hex(parent_hex)
        .map_err(|_| "reply_to must be a 64-character event id".to_string())?;
    let filter = Filter::new().id(parent_event_id).kinds([
        Kind::Custom(9),
        Kind::Custom(45001),
        Kind::Custom(45003),
    ]);
    let response = rest_client
        .query(&[filter])
        .await
        .map_err(|error| format!("failed to load reply parent: {error}"))?;
    let parent: Event = response
        .as_array()
        .and_then(|events| events.first())
        .cloned()
        .ok_or_else(|| format!("reply parent {parent_hex} was not found"))
        .and_then(|value| {
            serde_json::from_value(value)
                .map_err(|error| format!("reply parent is malformed: {error}"))
        })?;

    thread_ref_from_parent(&parent, expected_channel)
}

fn thread_ref_from_parent(parent: &Event, expected_channel: Uuid) -> Result<ThreadRef, String> {
    let parent_channel = parent.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some("h"))
            .then(|| parts.get(1).and_then(|value| Uuid::parse_str(value).ok()))
            .flatten()
    });
    match parent_channel {
        Some(channel) if channel == expected_channel => {}
        Some(channel) => {
            return Err(format!(
                "reply parent belongs to channel {channel}, not {expected_channel}"
            ))
        }
        None => return Err("reply parent has no valid channel tag".to_string()),
    }

    let mut root = None;
    let mut reply = None;
    for tag in parent.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e") || parts.len() < 4 {
            continue;
        }
        let Some(event_id) = parts.get(1).and_then(|value| EventId::from_hex(value).ok()) else {
            continue;
        };
        match parts.get(3).map(String::as_str) {
            Some("root") => root = Some(event_id),
            Some("reply") => reply = Some(event_id),
            _ => {}
        }
    }
    Ok(ThreadRef {
        root_event_id: root.or(reply).unwrap_or(parent.id),
        parent_event_id: parent.id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag};
    use serde_json::Value;

    #[test]
    fn direct_reply_uses_parent_as_root_and_parent() {
        let channel = Uuid::new_v4();
        let channel_tag = Tag::parse(["h", &channel.to_string()]).expect("channel tag");
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([channel_tag])
            .sign_with_keys(&Keys::generate())
            .expect("sign parent");
        let thread_ref = thread_ref_from_parent(&parent, channel).expect("thread ref");
        assert_eq!(thread_ref.root_event_id, parent.id);
        assert_eq!(thread_ref.parent_event_id, parent.id);
    }

    #[test]
    fn nested_reply_preserves_existing_root() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let channel_tag = Tag::parse(["h", &channel.to_string()]).expect("channel tag");
        let root = EventBuilder::new(Kind::Custom(9), "root")
            .tags([channel_tag.clone()])
            .sign_with_keys(&keys)
            .expect("sign root");
        let root_tag = Tag::parse(["e", &root.id.to_hex(), "", "root"]).expect("root tag");
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([channel_tag, root_tag])
            .sign_with_keys(&keys)
            .expect("sign parent");

        let thread_ref = thread_ref_from_parent(&parent, channel).expect("thread ref");
        assert_eq!(thread_ref.root_event_id, root.id);
        assert_eq!(thread_ref.parent_event_id, parent.id);
    }

    #[test]
    fn reply_parent_from_another_channel_is_rejected() {
        let parent_channel = Uuid::new_v4();
        let requested_channel = Uuid::new_v4();
        let channel_tag = Tag::parse(["h", &parent_channel.to_string()]).expect("channel tag");
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([channel_tag])
            .sign_with_keys(&Keys::generate())
            .expect("sign parent");

        let error = match thread_ref_from_parent(&parent, requested_channel) {
            Ok(_) => panic!("cross-channel reply must fail"),
            Err(error) => error,
        };
        assert!(error.contains(&parent_channel.to_string()));
        assert!(error.contains(&requested_channel.to_string()));
    }

    #[tokio::test]
    async fn invalid_typed_fields_fail_before_relay_contact() {
        let channel = Uuid::new_v4();
        let grants = RwLock::new(HashMap::from([("capability".to_string(), Some(channel))]));
        let rest_client = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:1".to_string(),
            keys: Keys::generate(),
            auth_tag_json: None,
        };

        let invalid_channel = publish_message(
            &grants,
            &rest_client,
            PublishRequest {
                capability: "capability".to_string(),
                channel: "not-a-uuid".to_string(),
                content: "hello".to_string(),
                reply_to: None,
                mentions: vec![],
            },
        )
        .await;
        assert!(!invalid_channel.ok);
        assert_eq!(
            invalid_channel.error.as_deref(),
            Some("channel must be a valid UUID")
        );

        let invalid_reply = publish_message(
            &grants,
            &rest_client,
            PublishRequest {
                capability: "capability".to_string(),
                channel: channel.to_string(),
                content: "hello".to_string(),
                reply_to: Some("not-an-event-id".to_string()),
                mentions: vec![],
            },
        )
        .await;
        assert!(!invalid_reply.ok);
        assert_eq!(
            invalid_reply.error.as_deref(),
            Some("reply_to must be a 64-character event id")
        );
    }

    #[tokio::test]
    async fn broker_publishes_signed_typed_message_without_returning_signature() {
        let http_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake relay");
        let http_addr = http_listener.local_addr().expect("fake relay address");
        let received_event = tokio::spawn(async move {
            let (mut stream, _) = http_listener.accept().await.expect("accept relay request");
            let mut request = Vec::new();
            let mut buf = [0_u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut buf).await.expect("read relay request");
                assert!(read > 0, "relay request closed before headers");
                request.extend_from_slice(&buf[..read]);
                if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .expect("content-length header");
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buf).await.expect("read relay body");
                assert!(read > 0, "relay request closed before body");
                request.extend_from_slice(&buf[..read]);
            }
            let event: Event =
                serde_json::from_slice(&request[header_end..header_end + content_length])
                    .expect("signed event body");
            let response = br#"{"accepted":true,"message":"stored"}"#;
            let http_response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.len()
            );
            stream
                .write_all(http_response.as_bytes())
                .await
                .expect("write relay headers");
            stream
                .write_all(response)
                .await
                .expect("write relay response");
            event
        });

        let keys = Keys::generate();
        let rest_client = RestClient {
            http: reqwest::Client::new(),
            base_url: format!("http://{http_addr}"),
            keys: keys.clone(),
            auth_tag_json: None,
        };
        let broker = PublisherBroker::start(rest_client)
            .await
            .expect("start publisher broker");
        let channel = Uuid::new_v4();
        let lease = broker.issuer().issue(Some(channel));
        let mut stream = TcpStream::connect(&lease.access().endpoint)
            .await
            .expect("connect publisher broker");
        let request = serde_json::json!({
            "capability": lease.access().capability,
            "channel": channel,
            "content": "typed hello",
            "mentions": []
        });
        stream
            .write_all(&serde_json::to_vec(&request).expect("serialize request"))
            .await
            .expect("write publisher request");
        stream.shutdown().await.expect("finish publisher request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read publisher response");
        let response: Value = serde_json::from_slice(&response).expect("publisher JSON response");

        assert_eq!(response["ok"], true);
        assert_eq!(response["accepted"], true);
        assert!(response.get("signature").is_none());
        let event = received_event.await.expect("fake relay task");
        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.content, "typed hello");
        assert_eq!(event.kind, Kind::Custom(9));
        assert!(event.verify().is_ok());
    }

    #[tokio::test]
    async fn broker_rejects_expired_capability_without_contacting_relay() {
        let rest_client = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:1".to_string(),
            keys: Keys::generate(),
            auth_tag_json: None,
        };
        let broker = PublisherBroker::start(rest_client)
            .await
            .expect("start publisher broker");
        let lease = broker.issuer().issue(Some(Uuid::new_v4()));
        let endpoint = lease.access().endpoint.clone();
        let expired_capability = lease.access().capability.clone();
        drop(lease);
        let mut stream = TcpStream::connect(&endpoint)
            .await
            .expect("connect publisher broker");
        let request = serde_json::json!({
            "capability": expired_capability,
            "channel": Uuid::new_v4(),
            "content": "must not publish"
        });
        stream
            .write_all(&serde_json::to_vec(&request).expect("serialize request"))
            .await
            .expect("write publisher request");
        stream.shutdown().await.expect("finish publisher request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read publisher response");
        let response: Value = serde_json::from_slice(&response).expect("publisher JSON response");

        assert_eq!(response["ok"], false);
        assert_eq!(response["error"], "invalid or expired publisher capability");
        assert!(response.get("event_id").is_none());
    }

    #[tokio::test]
    async fn channel_session_capability_cannot_publish_to_another_channel() {
        let rest_client = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:1".to_string(),
            keys: Keys::generate(),
            auth_tag_json: None,
        };
        let broker = PublisherBroker::start(rest_client)
            .await
            .expect("start publisher broker");
        let allowed_channel = Uuid::new_v4();
        let requested_channel = Uuid::new_v4();
        let lease = broker.issuer().issue(Some(allowed_channel));
        let mut stream = TcpStream::connect(&lease.access().endpoint)
            .await
            .expect("connect publisher broker");
        let request = serde_json::json!({
            "capability": lease.access().capability,
            "channel": requested_channel,
            "content": "must not publish"
        });
        stream
            .write_all(&serde_json::to_vec(&request).expect("serialize request"))
            .await
            .expect("write publisher request");
        stream.shutdown().await.expect("finish publisher request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read publisher response");
        let response: Value = serde_json::from_slice(&response).expect("publisher JSON response");

        assert_eq!(response["ok"], false);
        assert_eq!(
            response["error"],
            format!("publisher capability is not authorized for channel {requested_channel}")
        );
        assert!(response.get("event_id").is_none());
    }
}
