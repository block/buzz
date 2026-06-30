//! Harness-side Buzz relay client.
//!
//! Connects to the Buzz relay via NIP-01 WebSocket, authenticates via NIP-42,
//! discovers channels via REST API, and streams events back to the harness main
//! loop. Also publishes ephemeral events (typing indicators) via the same
//! WebSocket connection.
//!
//! ## Architecture
//!
//! A background tokio task owns the WebSocket stream. It:
//! - Responds to Ping frames with Pong (preventing relay disconnect on long turns)
//! - Forwards `BuzzEvent`s through an `mpsc` channel
//! - Handles reconnection with `since` filters to avoid event loss
//! - Responds to mid-session AUTH challenges
//! - Publishes ephemeral events (typing indicators) via `PublishEvent` commands
//!
//! `HarnessRelay` communicates with the background task via a `RelayCommand`
//! channel. `next_event()` reads from the event receiver.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

/// Default capacity of the event channel from background task to harness.
/// Override with `BUZZ_ACP_EVENT_BUFFER` env var at startup.
const EVENT_CHANNEL_CAPACITY_DEFAULT: usize = 256;
/// Capacity of the command channel from harness to background task.
const CMD_CHANNEL_CAPACITY: usize = 64;

/// Read the event channel capacity from the environment, falling back to the
/// compiled-in default. Parsed once at call-site (connect time).
fn event_channel_capacity() -> usize {
    std::env::var("BUZZ_ACP_EVENT_BUFFER")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.max(1)) // mpsc::channel panics on capacity 0
        .unwrap_or(EVENT_CHANNEL_CAPACITY_DEFAULT)
}
/// Maximum number of seen event IDs before the dedup set is rotated.
/// Two-generation dedup: each generation holds up to SEEN_ID_LIMIT/2 entries.
const SEEN_ID_LIMIT: usize = 12_000;

/// Interval between client-initiated WebSocket pings.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// If no pong is received within this duration after a ping, the connection is
/// considered dead and the background task triggers a reconnect.
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for individual ws.send() calls. Prevents a stalled socket from
/// wedging the background task indefinitely.
const WS_SEND_TIMEOUT_SECS: u64 = 10;
/// Diagnostic threshold: log when a connection has been stable for this long.
/// No backoff reset is implemented yet — this is a hook for future improvement.
const STABLE_CONNECTION_SECS: u64 = 60;
/// Seconds subtracted from `since` on resubscribe to tolerate clock skew.
const SINCE_SKEW_SECS: u64 = 5;
/// Timeout for the NIP-42 auth handshake steps.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// Timeout for the TCP + WebSocket handshake in `do_connect`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

use std::time::Instant;

use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION,
    KIND_TYPING_INDICATOR,
};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, RelayUrl, Tag};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::ChannelFilter;

/// Metadata about a channel, populated at discovery time.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub name: String,
    pub channel_type: String,
}

/// Build the discovered-channel subscribe set from the membership UUIDs and the
/// kind:39000 metadata events, **skipping any channel flagged `archived=true`**.
///
/// Archived channels (e.g. auto-archived by the ephemeral-channel reaper) are
/// unusable: re-offering one on reconnect draws a `CLOSED restricted` and would
/// re-form the reconnect loop. Dropping them here is the defense-in-depth
/// backstop to the relay-side live-subscription eviction — it covers a client
/// that was offline when the channel was reaped and so missed the CLOSED.
/// A channel with no metadata event defaults to a `stream` named `unknown`,
/// preserving prior behavior for non-archived channels.
fn merge_discovered_channels(
    channel_uuids: Vec<Uuid>,
    meta_events: &serde_json::Value,
) -> HashMap<Uuid, ChannelInfo> {
    let mut meta_map: HashMap<Uuid, (String, String)> = HashMap::new();
    let mut archived: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    if let Some(arr) = meta_events.as_array() {
        for ev in arr {
            let tags = match ev.get("tags").and_then(|t| t.as_array()) {
                Some(t) => t,
                None => continue,
            };
            let mut d_val = None;
            let mut name = None;
            let mut is_hidden = false;
            let mut is_private = false;
            let mut is_archived = false;
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    match arr.first().and_then(|v| v.as_str()) {
                        Some("d") => d_val = arr.get(1).and_then(|v| v.as_str()),
                        Some("name") => name = arr.get(1).and_then(|v| v.as_str()),
                        Some("hidden") => is_hidden = true,
                        Some("private") => is_private = true,
                        Some("archived") => {
                            is_archived = arr.get(1).and_then(|v| v.as_str()) == Some("true")
                        }
                        _ => {}
                    }
                }
            }
            if let Some(d) = d_val {
                if let Ok(uuid) = d.parse::<Uuid>() {
                    if is_archived {
                        archived.insert(uuid);
                        continue;
                    }
                    let ch_name = name.unwrap_or("unknown").to_string();
                    // DMs have the "hidden" tag; private channels have "private".
                    let ch_type = if is_hidden {
                        "dm".to_string()
                    } else if is_private {
                        "private".to_string()
                    } else {
                        "stream".to_string()
                    };
                    meta_map.insert(uuid, (ch_name, ch_type));
                }
            }
        }
    }

    let mut map = HashMap::with_capacity(channel_uuids.len());
    for uuid in channel_uuids {
        if archived.contains(&uuid) {
            continue;
        }
        let (name, channel_type) = meta_map
            .remove(&uuid)
            .unwrap_or_else(|| ("unknown".to_string(), "stream".to_string()));
        map.insert(uuid, ChannelInfo { name, channel_type });
    }
    map
}

/// Lightweight HTTP client for pre-prompt context fetches via the Nostr HTTP bridge.
///
/// Extracted from `HarnessRelay` fields so it can be shared (via `Arc`) with
/// spawned prompt tasks without giving them access to the WebSocket.
///
/// All reads go through `POST /query` with NIP-98 auth.
#[derive(Debug, Clone)]
pub struct RestClient {
    pub http: reqwest::Client,
    pub base_url: String,
    pub keys: Keys,
    /// Optional NIP-OA auth tag JSON for `x-auth-tag` header (relay membership delegation).
    pub auth_tag_json: Option<String>,
}

/// Whether an HTTP status code is retriable (transient server/rate-limit errors).
fn is_retriable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

/// Base retry delays for transient HTTP failures: 500ms, 1s, 2s.
/// Jitter (±20%) is applied at call time via `jittered_duration`.
const REST_RETRY_BASE_DELAYS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
];

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl RestClient {
    /// Sign a NIP-98 HTTP Auth event (kind:27235) for the given method/URL/body.
    ///
    /// Returns the `Authorization: Nostr <base64>` header value (without the
    /// `Nostr ` prefix — caller must prepend it or use `nip98_header`).
    fn sign_nip98(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<String, RelayError> {
        use base64::Engine;
        use sha2::{Digest, Sha256};

        let u_tag = Tag::parse(["u", url])
            .map_err(|e| RelayError::Http(format!("NIP-98 tag error: {e}")))?;
        let method_tag = Tag::parse(["method", method])
            .map_err(|e| RelayError::Http(format!("NIP-98 tag error: {e}")))?;
        // Nonce prevents replay rejection for rapid-fire requests with identical bodies.
        let nonce_tag = Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()])
            .map_err(|e| RelayError::Http(format!("NIP-98 tag error: {e}")))?;
        let mut tags = vec![u_tag, method_tag, nonce_tag];

        if let Some(b) = body {
            let hash = hex::encode(Sha256::digest(b));
            let payload_tag = Tag::parse(["payload", &hash])
                .map_err(|e| RelayError::Http(format!("NIP-98 tag error: {e}")))?;
            tags.push(payload_tag);
        }

        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(&self.keys)
            .map_err(|e| RelayError::Http(format!("NIP-98 sign error: {e}")))?;
        let event_json = serde_json::to_string(&event)
            .map_err(|e| RelayError::Http(format!("NIP-98 serialize error: {e}")))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(event_json))
    }

    /// Build the full `Authorization` header value: `Nostr <base64>`.
    fn nip98_header(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<String, RelayError> {
        Ok(format!("Nostr {}", self.sign_nip98(method, url, body)?))
    }

    /// Retry helper: executes `build_request` up to 4 times (1 attempt + 3 retries)
    /// on transient failures (429, 502, 503, 504, timeout, connect errors).
    ///
    /// NIP-98 auth events are re-signed on each attempt (they have a ±60s window).
    async fn request_with_retry<F, Fut>(
        &self,
        method: &str,
        path: &str,
        build_request: F,
    ) -> Result<reqwest::Response, RelayError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let mut last_err = None;

        for (attempt, delay) in std::iter::once(None)
            .chain(REST_RETRY_BASE_DELAYS.iter().map(|d| Some(*d)))
            .enumerate()
        {
            if let Some(base) = delay {
                let jittered = jittered_duration(base);
                tracing::debug!(
                    "retrying {method} {path} (attempt {attempt}) in {:.1}s",
                    jittered.as_secs_f64()
                );
                tokio::time::sleep(jittered).await;
            }

            match build_request().await {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) if is_retriable_status(resp.status()) => {
                    let status = resp.status();
                    tracing::warn!("{method} {path} returned retriable HTTP {status}");
                    last_err = Some(RelayError::Http(format!(
                        "{method} {path} returned HTTP {status}"
                    )));
                }
                Ok(resp) => {
                    return Err(RelayError::Http(format!(
                        "{method} {} returned HTTP {}",
                        path,
                        resp.status()
                    )));
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    tracing::warn!("{method} {path} network error: {e}");
                    last_err = Some(RelayError::Http(e.to_string()));
                }
                Err(e) => return Err(RelayError::Http(e.to_string())),
            }
        }

        Err(last_err
            .unwrap_or_else(|| RelayError::Http(format!("{method} {path} failed after retries"))))
    }

    /// POST with NIP-98 auth and retry. Re-signs on each attempt.
    async fn bridge_post(
        &self,
        path: &str,
        body_bytes: &[u8],
    ) -> Result<reqwest::Response, RelayError> {
        let url = format!("{}{}", self.base_url, path);
        let body_owned = body_bytes.to_vec();
        let auth_tag_header = self.auth_tag_json.clone();
        self.request_with_retry("POST", path, || {
            // NIP-98 is re-signed each attempt (fresh created_at).
            // sign_nip98 is infallible in practice (key is always valid).
            let auth = self
                .nip98_header("POST", &url, Some(&body_owned))
                .unwrap_or_default();
            let mut req = self
                .http
                .post(&url)
                .header("Authorization", auth)
                .header("Content-Type", "application/json");
            if let Some(ref tag) = auth_tag_header {
                req = req.header("x-auth-tag", tag);
            }
            req.body(body_owned.clone()).send()
        })
        .await
    }

    /// Query events via the HTTP bridge: `POST /query` with NIP-98 auth.
    ///
    /// Accepts a slice of `nostr::Filter` (serialized as JSON array).
    /// Returns the events as a `serde_json::Value` (JSON array of event objects).
    pub async fn query(&self, filters: &[nostr::Filter]) -> Result<Value, RelayError> {
        let body_bytes = serde_json::to_vec(filters)
            .map_err(|e| RelayError::Http(format!("filter serialize error: {e}")))?;
        let resp = self.bridge_post("/query", &body_bytes).await?;
        resp.json()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))
    }
}

/// Events the harness cares about.
#[derive(Debug, Clone)]
pub struct BuzzEvent {
    /// Which channel this event belongs to.
    pub channel_id: Uuid,
    /// The underlying Nostr event.
    pub event: Event,
}

/// Errors from relay operations.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Auth failed: {0}")]
    AuthFailed(String),

    #[error("No auth challenge received")]
    NoAuthChallenge,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout")]
    Timeout,

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Unexpected message: {0}")]
    UnexpectedMessage(String),
}

impl From<nostr::event::builder::Error> for RelayError {
    fn from(e: nostr::event::builder::Error) -> Self {
        RelayError::AuthFailed(e.to_string())
    }
}

/// A parsed NIP-01 relay message.
#[derive(Debug, Clone)]
enum RelayMessage {
    Event {
        subscription_id: String,
        event: Box<Event>,
    },
    Ok {
        event_id: String,
        accepted: bool,
        message: String,
    },
    Eose {
        subscription_id: String,
    },
    Closed {
        subscription_id: String,
        message: String,
    },
    Notice {
        message: String,
    },
    Auth {
        challenge: String,
    },
}

/// Subscription ID for the global membership notification subscription.
const MEMBERSHIP_NOTIF_SUB_ID: &str = "membership-notif";
/// Subscription ID for encrypted owner-to-agent observer control frames.
const OBSERVER_CONTROL_SUB_ID: &str = "agent-observer-control";

/// Commands sent from `HarnessRelay` to the background WebSocket task.
enum RelayCommand {
    /// Subscribe to a channel (sends a NIP-01 REQ) with the given filter.
    Subscribe {
        channel_id: Uuid,
        filter: ChannelFilter,
        replay_since: Option<u64>,
    },
    /// Unsubscribe from a channel (sends a NIP-01 CLOSE).
    Unsubscribe { channel_id: Uuid },
    /// Reconnect to the relay (re-authenticate and resubscribe).
    Reconnect,
    /// Shut down the background task.
    Shutdown,
    /// Subscribe to global membership notifications.
    SubscribeMembership,
    /// Subscribe to encrypted observer control frames addressed to this agent.
    SubscribeObserverControls,
    /// Publish a signed event to the relay (for typing indicators, etc.).
    PublishEvent { event: Box<Event> },
    /// Set the startup watermark timestamp for Finding #22.
    /// The background task uses this as the floor `since` for membership
    /// notification replay so events before startup are never re-delivered.
    SetStartupWatermark { ts: u64 },
}

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Harness-side relay client.
///
/// Connects to the Buzz relay, authenticates via NIP-42, and streams
/// matching events for subscribed channels.
///
/// A background tokio task owns the WebSocket connection and responds to
/// Ping frames, preventing disconnection during long agent turns.
pub struct HarnessRelay {
    /// Receiver for events forwarded by the background task.
    event_rx: mpsc::Receiver<Option<BuzzEvent>>,
    /// Receiver for encrypted observer control events addressed to this agent.
    observer_control_rx: Option<mpsc::Receiver<Event>>,
    /// Sender for commands to the background task.
    cmd_tx: mpsc::Sender<RelayCommand>,
    /// HTTP client for HTTP bridge calls.
    http: reqwest::Client,
    /// WebSocket URL of the relay.
    relay_url: String,
    /// Keys used for NIP-42 signing and NIP-98 HTTP auth.
    keys: Keys,
    /// Optional NIP-OA auth tag for relay membership delegation.
    auth_tag: Option<nostr::Tag>,
    /// Handle to the background task (for clean shutdown).
    /// Wrapped in `Option` so `shutdown()` can take ownership without conflicting
    /// with `Drop` (which only has `&mut self`).
    bg_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Cloneable publisher handle for signed events on the relay background socket.
#[derive(Clone)]
pub struct RelayEventPublisher {
    cmd_tx: mpsc::Sender<RelayCommand>,
}

impl RelayEventPublisher {
    /// Publish a signed event through the relay background task.
    pub async fn publish_event(&self, event: Event) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::PublishEvent {
                event: Box::new(event),
            })
            .await
            .map_err(|_| RelayError::ConnectionClosed)
    }
}

impl HarnessRelay {
    /// Connect to relay and authenticate via NIP-42.
    ///
    /// `auth_tag` is an optional NIP-OA owner attestation included in the AUTH
    /// event for relay membership delegation.
    pub async fn connect(
        relay_url: &str,
        keys: &Keys,
        agent_pubkey_hex: &str,
        auth_tag: Option<nostr::Tag>,
    ) -> Result<Self, RelayError> {
        // Perform the initial connection and auth handshake.
        // Finding #8: capture the handshake buffer and pass it to the background
        // task so buffered messages aren't silently discarded.
        let (ws, handshake_buffer) = do_connect(relay_url, keys, auth_tag.as_ref()).await?;

        let (event_tx, event_rx) = mpsc::channel::<Option<BuzzEvent>>(event_channel_capacity());
        let (observer_control_tx, observer_control_rx) =
            mpsc::channel::<Event>(event_channel_capacity());
        let (cmd_tx, cmd_rx) = mpsc::channel::<RelayCommand>(CMD_CHANNEL_CAPACITY);

        let bg_keys = keys.clone();
        let bg_relay_url = relay_url.to_string();
        let bg_agent_pubkey_hex = agent_pubkey_hex.to_string();
        let bg_auth_tag = auth_tag.clone();

        let bg_handle = tokio::spawn(async move {
            run_background_task(
                ws,
                handshake_buffer,
                event_tx,
                observer_control_tx,
                cmd_rx,
                bg_keys,
                bg_relay_url,
                bg_agent_pubkey_hex,
                bg_auth_tag,
            )
            .await;
        });

        Ok(Self {
            event_rx,
            observer_control_rx: Some(observer_control_rx),
            cmd_tx,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| RelayError::Http(format!("failed to build HTTP client: {e}")))?,
            relay_url: relay_url.to_string(),
            keys: keys.clone(),
            auth_tag,
            bg_handle: Some(bg_handle),
        })
    }

    /// Discover channels the agent is a member of.
    ///
    /// Queries kind:39002 (NIP-29 group members) events where `#p` includes
    /// the agent pubkey to find channel memberships, then queries kind:39000
    /// (group metadata) for channel names and types.
    pub async fn discover_channels(&self) -> Result<HashMap<Uuid, ChannelInfo>, RelayError> {
        use nostr::{Alphabet, SingleLetterTag};

        let rest = self.rest_client();
        let pk_hex = self.keys.public_key().to_hex();

        // Step 1: Find all channels where agent is a member (kind:39002 with #p tag).
        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let member_filter = nostr::Filter::new()
            .kind(Kind::Custom(
                buzz_core::kind::KIND_NIP29_GROUP_MEMBERS as u16,
            ))
            .custom_tags(p_tag, [pk_hex.as_str()]);
        let member_events = rest.query(&[member_filter]).await?;

        let member_arr = member_events
            .as_array()
            .ok_or_else(|| RelayError::Http("expected JSON array from /query (members)".into()))?;

        // Extract channel UUIDs from #d tags.
        let mut channel_uuids: Vec<Uuid> = Vec::new();
        for ev in member_arr {
            if let Some(tags) = ev.get("tags").and_then(|t| t.as_array()) {
                for tag in tags {
                    if let Some(arr) = tag.as_array() {
                        if arr.first().and_then(|v| v.as_str()) == Some("d") {
                            if let Some(d_val) = arr.get(1).and_then(|v| v.as_str()) {
                                if let Ok(uuid) = d_val.parse::<Uuid>() {
                                    channel_uuids.push(uuid);
                                }
                            }
                        }
                    }
                }
            }
        }

        if channel_uuids.is_empty() {
            debug!("discovered 0 channel(s)");
            return Ok(HashMap::new());
        }

        // Step 2: Fetch metadata (kind:39000) for discovered channels.
        let d_tag = SingleLetterTag::lowercase(Alphabet::D);
        let d_values: Vec<String> = channel_uuids.iter().map(|u| u.to_string()).collect();
        let meta_filter = nostr::Filter::new()
            .kind(Kind::Custom(
                buzz_core::kind::KIND_NIP29_GROUP_METADATA as u16,
            ))
            .custom_tags(d_tag, d_values);
        let meta_events = rest.query(&[meta_filter]).await?;

        // Step 3: Build the final subscribe set, skipping archived channels.
        let map = merge_discovered_channels(channel_uuids, &meta_events);

        debug!("discovered {} channel(s)", map.len());
        Ok(map)
    }

    /// Build a [`RestClient`] that shares this relay's HTTP credentials.
    ///
    /// The returned client is cheap to clone (wraps `reqwest::Client` which is
    /// internally `Arc`-ed) and safe to share across spawned tasks via `Arc`.
    pub fn rest_client(&self) -> RestClient {
        RestClient {
            http: self.http.clone(),
            base_url: relay_ws_to_http(&self.relay_url),
            keys: self.keys.clone(),
            auth_tag_json: self
                .auth_tag
                .as_ref()
                .and_then(|t| serde_json::to_string(t.as_slice()).ok()),
        }
    }

    /// Subscribe to events in a channel using the given filter.
    ///
    /// Sends a `Subscribe` command to the background task, which issues the
    /// NIP-01 `REQ` built from `filter`. Subscription ID is `ch-<uuid>`.
    pub async fn subscribe_channel(
        &mut self,
        channel_id: Uuid,
        filter: ChannelFilter,
    ) -> Result<(), RelayError> {
        self.subscribe_channel_from(channel_id, filter, None).await
    }

    /// Subscribe to events in a channel, replaying from a known timestamp.
    ///
    /// Used for channels discovered from membership notifications: the mention
    /// that invited an agent can be published immediately after the membership
    /// event, before this subscription is active. Replaying from the membership
    /// event timestamp closes that race.
    pub async fn subscribe_channel_from(
        &mut self,
        channel_id: Uuid,
        filter: ChannelFilter,
        replay_since: Option<u64>,
    ) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::Subscribe {
                channel_id,
                filter,
                replay_since,
            })
            .await
            .map_err(|_| RelayError::ConnectionClosed)?;
        debug!("queued subscribe for channel {channel_id}");
        Ok(())
    }

    /// Subscribe to membership notifications for this agent.
    pub async fn subscribe_membership_notifications(&mut self) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::SubscribeMembership)
            .await
            .map_err(|_| RelayError::ConnectionClosed)?;
        Ok(())
    }

    /// Subscribe to encrypted observer control frames addressed to this agent.
    pub async fn subscribe_observer_controls(&mut self) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::SubscribeObserverControls)
            .await
            .map_err(|_| RelayError::ConnectionClosed)?;
        Ok(())
    }

    /// Take the observer-control receiver for polling outside this relay object.
    pub fn take_observer_control_rx(&mut self) -> Option<mpsc::Receiver<Event>> {
        self.observer_control_rx.take()
    }

    /// Return a cloneable publisher handle for signed relay events.
    pub fn event_publisher(&self) -> RelayEventPublisher {
        RelayEventPublisher {
            cmd_tx: self.cmd_tx.clone(),
        }
    }

    /// Unsubscribe from a channel.
    pub async fn unsubscribe_channel(&mut self, channel_id: Uuid) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::Unsubscribe { channel_id })
            .await
            .map_err(|_| RelayError::ConnectionClosed)?;
        debug!("queued unsubscribe for channel {channel_id}");
        Ok(())
    }

    /// Wait for the next event from any subscribed channel.
    ///
    /// Reads from the background task's event channel. Returns `None` on
    /// connection loss — the caller should call [`reconnect`](Self::reconnect).
    pub async fn next_event(&mut self) -> Option<BuzzEvent> {
        // The background task sends `None` to signal connection loss.
        self.event_rx.recv().await.flatten()
    }

    /// Publish a signed event to the relay via the background WebSocket task.
    ///
    /// Blocks until the command channel has capacity. For ephemeral events
    /// (typing indicators) prefer [`try_publish_event`] which never blocks.
    #[allow(dead_code)] // Public API — callers outside the harness may use this
    pub async fn publish_event(&self, event: Event) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::PublishEvent {
                event: Box::new(event),
            })
            .await
            .map_err(|_| RelayError::ConnectionClosed)
    }

    /// Fire-and-forget publish — uses `try_send` so it never blocks the caller.
    ///
    /// Suitable for ephemeral commands like typing indicators where dropping
    /// the event on a full command channel is acceptable.
    pub fn try_publish_event(&self, event: Event) -> Result<(), RelayError> {
        self.cmd_tx
            .try_send(RelayCommand::PublishEvent {
                event: Box::new(event),
            })
            .map_err(|_| RelayError::ConnectionClosed)
    }

    /// Build a typing indicator event (kind:20002) for a channel.
    pub fn build_typing_event(
        &self,
        channel_id: Uuid,
        root_event_id: Option<&str>,
        parent_event_id: Option<&str>,
    ) -> Result<Event, RelayError> {
        let h_tag = Tag::parse(["h", &channel_id.to_string()])
            .map_err(|e| RelayError::AuthFailed(e.to_string()))?;
        let mut tags = vec![h_tag];
        if let Some(parent) = parent_event_id {
            if let Some(root) = root_event_id {
                if root != parent {
                    tags.push(
                        Tag::parse(["e", root, "", "root"])
                            .map_err(|e| RelayError::AuthFailed(e.to_string()))?,
                    );
                }
            }
            tags.push(
                Tag::parse(["e", parent, "", "reply"])
                    .map_err(|e| RelayError::AuthFailed(e.to_string()))?,
            );
        }
        let event = EventBuilder::new(Kind::Custom(KIND_TYPING_INDICATOR as u16), "")
            .tags(tags)
            .sign_with_keys(&self.keys)?;
        Ok(event)
    }

    /// Set the startup watermark timestamp (Finding #22).
    ///
    /// Call this once after `connect()` with the Unix timestamp captured just
    /// before the relay connection was established. The background task uses
    /// this as the floor `since` for membership notification replay so events
    /// predating this session are never re-delivered after reconnect.
    pub async fn set_startup_watermark(&self, ts: u64) -> Result<(), RelayError> {
        self.cmd_tx
            .send(RelayCommand::SetStartupWatermark { ts })
            .await
            .map_err(|_| RelayError::ConnectionClosed)
    }

    /// Reconnect after connection loss. Instructs the background task to
    /// re-authenticate and resubscribe to all previously active channels.
    pub async fn reconnect(&mut self) -> Result<(), RelayError> {
        warn!("relay connection lost — reconnecting…");
        self.cmd_tx
            .send(RelayCommand::Reconnect)
            .await
            .map_err(|_| RelayError::ConnectionClosed)?;
        Ok(())
    }
}

impl HarnessRelay {
    /// Graceful async shutdown — sends Shutdown command and waits up to 5s for
    /// the background task to finish. Use this from async contexts instead of
    /// relying on `Drop` (which aborts immediately).
    pub async fn shutdown(mut self) {
        let _ = self.cmd_tx.send(RelayCommand::Shutdown).await;
        if let Some(handle) = self.bg_handle.take() {
            let abort_handle = handle.abort_handle();
            if tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .is_err()
            {
                tracing::warn!("relay background task did not finish in 5s — aborting");
                abort_handle.abort();
            }
        }
    }
}

impl Drop for HarnessRelay {
    fn drop(&mut self) {
        // Best-effort shutdown signal; ignore errors (task may already be done).
        let _ = self.cmd_tx.try_send(RelayCommand::Shutdown);
        if let Some(handle) = self.bg_handle.take() {
            handle.abort();
        }
    }
}

/// Two-generation dedup set with bounded memory.
///
/// Mitigates the "amnesia window" caused by clearing the entire set at once.
/// When `current` reaches `limit/2` entries it is rotated into `previous`.
/// At any point we remember between `limit/2` and `limit` recent IDs.
/// The oldest `limit/2` IDs are forgotten on each rotation — this is the
/// inherent tradeoff of bounded-memory dedup. For the default limit of
/// 12,000, the worst case is that an ID seen 6,001+ inserts ago may be
/// replayed as new. This is acceptable for Nostr event dedup where the
/// `since` filter provides the primary replay protection.
struct TwoGenDedup {
    current: HashSet<String>,
    previous: HashSet<String>,
    limit: usize,
}

impl TwoGenDedup {
    fn new(limit: usize) -> Self {
        Self {
            current: HashSet::new(),
            previous: HashSet::new(),
            limit,
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.current.contains(id) || self.previous.contains(id)
    }

    /// Insert `id`. Returns `true` if it was new (not a duplicate).
    fn insert(&mut self, id: String) -> bool {
        if self.contains(&id) {
            return false;
        }
        self.current.insert(id);
        if self.current.len() >= self.limit / 2 {
            // Rotate: current → previous, start fresh current.
            self.previous = std::mem::take(&mut self.current);
        }
        true
    }

    /// Remove an ID (used to un-deduplicate a dropped event so it can be
    /// replayed after reconnect).
    fn remove(&mut self, id: &str) {
        self.current.remove(id);
        self.previous.remove(id);
    }
}

/// State maintained by the background WebSocket task.
struct BgState {
    /// Active subscriptions: channel_id → subscription_id string.
    active_subscriptions: HashMap<Uuid, String>,
    /// Most recent `created_at` timestamp seen per channel (for `since` filter).
    last_seen: HashMap<Uuid, u64>,
    /// Two-generation dedup set of event IDs seen.
    seen_ids: TwoGenDedup,
    /// Per-channel filter used on subscribe (for resubscribe after reconnect).
    active_filters: HashMap<Uuid, ChannelFilter>,
    /// Oldest timestamp of a membership notification that was dropped due to
    /// backpressure. If set, reconnect replay must start from this timestamp
    /// (minus skew) to re-deliver the lost event. Reset on successful reconnect.
    membership_dropped_since: Option<u64>,
    /// Newest successfully-enqueued membership notification timestamp.
    /// Used as the `since` for reconnect replay when no events were dropped.
    membership_last_seen: Option<u64>,
    /// Whether the membership notification subscription is active.
    membership_sub_active: bool,
    /// Whether the observer control subscription is active.
    observer_control_sub_active: bool,
    /// Oldest dropped channel-event timestamp per channel, keyed by channel_id.
    /// Mirrors `membership_dropped_since` but for ordinary channel events.
    /// On reconnect resubscribe, `since` = min(last_seen, channel_dropped_since).
    /// Cleared per-channel after a successful resubscribe.
    channel_dropped_since: HashMap<Uuid, u64>,
    /// Set by the backpressure handler when the event channel is full.
    /// The main loop checks this flag and triggers a proactive resubscribe
    /// (without waiting for a disconnect) so dropped events are replayed.
    proactive_resubscribe_needed: bool,
    /// Unix timestamp captured just before the relay connection was established
    /// (Finding #22). Used as the floor `since` for membership notification
    /// replay so events predating this session are never re-delivered.
    startup_watermark: Option<u64>,
    /// Replay floor captured when each channel was first subscribed.
    /// Used as the `since` fallback on reconnect for channels that have no
    /// `last_seen` or `channel_dropped_since`. This prevents channels joined
    /// after startup from replaying from an hours-old `startup_watermark`.
    /// Startup-era channels use the startup watermark; dynamic channels use
    /// the membership notification timestamp that caused the subscription.
    subscribe_since: HashMap<Uuid, u64>,
}

impl BgState {
    fn new() -> Self {
        Self {
            active_subscriptions: HashMap::new(),
            last_seen: HashMap::new(),
            seen_ids: TwoGenDedup::new(SEEN_ID_LIMIT),
            active_filters: HashMap::new(),
            membership_dropped_since: None,
            membership_last_seen: None,
            membership_sub_active: false,
            observer_control_sub_active: false,
            channel_dropped_since: HashMap::new(),
            proactive_resubscribe_needed: false,
            startup_watermark: None,
            subscribe_since: HashMap::new(),
        }
    }

    /// Record a received event for dedup and `since` tracking.
    /// Returns `true` if the event is new (not a duplicate).
    fn record_event(&mut self, channel_id: Uuid, event: &Event) -> bool {
        let id_hex = event.id.to_hex();

        // Two-generation dedup: no amnesia window on rotation.
        if !self.seen_ids.insert(id_hex) {
            return false;
        }

        // Update last_seen timestamp.
        let ts = event.created_at.as_secs();
        self.last_seen
            .entry(channel_id)
            .and_modify(|t| *t = (*t).max(ts))
            .or_insert(ts);

        true
    }

    /// Compute the `since` timestamp for a channel (re)subscribe.
    ///
    /// Picks the earliest of `last_seen` and `channel_dropped_since` so
    /// the replay window covers both successfully processed events and any
    /// that were dropped due to queue pressure. Falls back to the per-channel
    /// `subscribe_since` (set at first subscribe) or `startup_watermark`.
    fn channel_since(&self, channel_id: &Uuid) -> Option<u64> {
        let last_seen = self.last_seen.get(channel_id).copied();
        let dropped = self.channel_dropped_since.get(channel_id).copied();
        match (last_seen, dropped) {
            (Some(l), Some(d)) => Some(l.min(d)),
            (Some(l), None) => Some(l),
            (None, Some(d)) => Some(d),
            (None, None) => self
                .subscribe_since
                .get(channel_id)
                .copied()
                .or(self.startup_watermark),
        }
    }

    /// Clear all per-channel state for a channel that is being unsubscribed.
    /// Prevents stale replay on re-subscribe and avoids unbounded state growth
    /// for channels that are removed and never re-added.
    fn clear_channel_state(&mut self, channel_id: &Uuid) {
        self.last_seen.remove(channel_id);
        self.subscribe_since.remove(channel_id);
        self.channel_dropped_since.remove(channel_id);
        self.active_filters.remove(channel_id);
    }
}

/// Record a command's intent in state while disconnected (no WebSocket).
///
/// Subscribe/Unsubscribe/SubscribeMembership record intent so reconnect
/// restores the right subscriptions. SetStartupWatermark floors the replay
/// window. PublishEvent and Reconnect are no-ops while disconnected.
///
/// Callers MUST handle `Shutdown` before calling — reaching the Shutdown
/// arm here is a logic error.
fn apply_command_to_state(state: &mut BgState, cmd: RelayCommand) {
    match cmd {
        RelayCommand::Subscribe {
            channel_id,
            filter,
            replay_since,
        } => {
            state
                .active_subscriptions
                .insert(channel_id, channel_sub_id(channel_id));
            state.active_filters.insert(channel_id, filter);
            state.subscribe_since.entry(channel_id).or_insert_with(|| {
                // Use an explicit replay floor when available (dynamic
                // membership), otherwise startup_watermark closes the startup
                // blind spot between watermark capture and first REQ.
                replay_since
                    .or(state.startup_watermark)
                    .unwrap_or_else(unix_now_secs)
            });
        }
        RelayCommand::Unsubscribe { channel_id } => {
            state.active_subscriptions.remove(&channel_id);
            state.clear_channel_state(&channel_id);
        }
        RelayCommand::SubscribeMembership => {
            state.membership_sub_active = true;
        }
        RelayCommand::SubscribeObserverControls => {
            state.observer_control_sub_active = true;
        }
        RelayCommand::SetStartupWatermark { ts } => {
            state.startup_watermark = Some(ts);
            if state.membership_last_seen.is_none() {
                state.membership_last_seen = Some(ts);
            }
        }
        // Ephemeral events are meaningless while disconnected.
        RelayCommand::PublishEvent { .. } => {}
        // Already reconnecting — redundant.
        RelayCommand::Reconnect => {}
        // Callers MUST handle Shutdown before calling this function.
        RelayCommand::Shutdown => {
            debug_assert!(
                false,
                "Shutdown must be handled by caller, not apply_command_to_state"
            );
        }
    }
}

/// Execute a command on a live WebSocket connection.
///
/// Handles the five data commands: Subscribe, Unsubscribe,
/// SubscribeMembership, PublishEvent, SetStartupWatermark. Callers handle
/// Shutdown and Reconnect for control flow before dispatching here.
///
/// Returns `true` if the command succeeded (or was a no-op). Returns `false`
/// if a WebSocket send failed — the caller should treat this as a dead socket
/// and trigger reconnect. On failure, subscription intent is preserved in
/// state via [`apply_command_to_state`] so reconnect will restore it.
async fn execute_connected_command(
    ws: &mut WsStream,
    state: &mut BgState,
    agent_pubkey_hex: &str,
    cmd: RelayCommand,
) -> bool {
    match cmd {
        RelayCommand::Subscribe {
            channel_id,
            filter,
            replay_since,
        } => {
            // Seed subscribe_since BEFORE computing since — on first
            // subscribe, this provides the fallback timestamp that
            // closes the startup/dynamic-membership blind spot.
            state.subscribe_since.entry(channel_id).or_insert_with(|| {
                replay_since
                    .or(state.startup_watermark)
                    .unwrap_or_else(unix_now_secs)
            });
            let since = state
                .last_seen
                .get(&channel_id)
                .copied()
                .or_else(|| state.subscribe_since.get(&channel_id).copied());
            let sent =
                send_subscribe(ws, state, channel_id, agent_pubkey_hex, since, &filter).await;
            if sent {
                state
                    .active_subscriptions
                    .insert(channel_id, channel_sub_id(channel_id));
                state.active_filters.insert(channel_id, filter);
                true
            } else {
                // Send failed — record intent so reconnect restores it.
                warn!("subscribe REQ failed for channel {channel_id} — recording intent for reconnect");
                apply_command_to_state(
                    state,
                    RelayCommand::Subscribe {
                        channel_id,
                        filter,
                        replay_since,
                    },
                );
                false
            }
        }
        RelayCommand::Unsubscribe { channel_id } => {
            if let Some(sub_id) = state.active_subscriptions.remove(&channel_id) {
                let msg = json!(["CLOSE", sub_id]);
                if let Ok(text) = serde_json::to_string(&msg) {
                    // Best-effort CLOSE — don't fail the command if send fails,
                    // because the intent (unsubscribe) is already applied to state.
                    let _ =
                        ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await;
                }
                debug!("unsubscribed from channel {channel_id}");
            }
            state.clear_channel_state(&channel_id);
            true
        }
        RelayCommand::SubscribeMembership => {
            let since = state.membership_last_seen.or(state.startup_watermark);
            let sent = send_membership_subscribe(ws, agent_pubkey_hex, since).await;
            if sent {
                state.membership_sub_active = true;
                if state.membership_last_seen.is_none() {
                    state.membership_last_seen = since;
                }
                true
            } else {
                // Send failed — record intent so reconnect restores it.
                warn!("membership subscribe REQ failed — recording intent for reconnect");
                state.membership_sub_active = true;
                false
            }
        }
        RelayCommand::SubscribeObserverControls => {
            let sent = send_observer_control_subscribe(ws, agent_pubkey_hex).await;
            if sent {
                state.observer_control_sub_active = true;
                true
            } else {
                warn!("observer control subscribe REQ failed — recording intent for reconnect");
                state.observer_control_sub_active = true;
                false
            }
        }
        RelayCommand::PublishEvent { event } => {
            let msg = json!(["EVENT", event]);
            if let Ok(text) = serde_json::to_string(&msg) {
                if let Err(e) =
                    ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await
                {
                    // Ephemeral events (typing indicators) are best-effort.
                    // Log the failure but don't trigger reconnect — the next
                    // ping or read will detect the dead socket.
                    warn!("failed to publish event: {e}");
                }
            }
            true
        }
        RelayCommand::SetStartupWatermark { ts } => {
            state.startup_watermark = Some(ts);
            if state.membership_last_seen.is_none() {
                state.membership_last_seen = Some(ts);
            }
            debug!("startup watermark set to {ts}");
            true
        }
        // Control-flow commands — callers handle these before dispatching.
        RelayCommand::Shutdown | RelayCommand::Reconnect => {
            debug_assert!(
                false,
                "Shutdown/Reconnect must be handled by caller, not execute_connected_command"
            );
            true
        }
    }
}

/// The main background task loop.
///
/// Owns the WebSocket stream, responds to Pings, forwards events, and handles
/// reconnection.
#[allow(clippy::too_many_arguments)]
async fn run_background_task(
    mut ws: WsStream,
    initial_handshake_buffer: std::collections::VecDeque<RelayMessage>,
    event_tx: mpsc::Sender<Option<BuzzEvent>>,
    observer_control_tx: mpsc::Sender<Event>,
    mut cmd_rx: mpsc::Receiver<RelayCommand>,
    keys: Keys,
    relay_url: String,
    agent_pubkey_hex: String,
    auth_tag: Option<nostr::Tag>,
) {
    let mut state = BgState::new();

    // Finding #8: process any messages buffered during the initial auth handshake.
    // If a buffered message signals connection drop, trigger reconnect immediately.
    let handshake_ok = process_handshake_buffer(
        &mut ws,
        initial_handshake_buffer,
        &event_tx,
        &observer_control_tx,
        &mut state,
        &keys,
        &relay_url,
        &agent_pubkey_hex,
        auth_tag.as_ref(),
    )
    .await;
    if !handshake_ok {
        warn!("handshake buffer contained a drop signal — attempting autonomous reconnect");
        // Don't wait for a caller-driven Reconnect command — the caller was
        // never notified (no sentinel sent). Go straight to reconnect loop.
        let _ = event_tx.try_send(None);
        match try_autonomous_reconnect(
            &mut ws,
            &mut cmd_rx,
            &mut state,
            &keys,
            &relay_url,
            &agent_pubkey_hex,
            &event_tx,
            &observer_control_tx,
            auth_tag.as_ref(),
        )
        .await
        {
            ReconnectOutcome::Ok => {
                if matches!(
                    drain_post_reconnect(&mut ws, &mut cmd_rx, &mut state, &agent_pubkey_hex).await,
                    ReconnectOutcome::Shutdown
                ) {
                    return;
                }
            }
            ReconnectOutcome::Shutdown => return,
            ReconnectOutcome::Failed => {
                if matches!(
                    wait_for_reconnect(
                        &mut ws,
                        &mut cmd_rx,
                        &mut state,
                        &keys,
                        &relay_url,
                        &agent_pubkey_hex,
                        &event_tx,
                        &observer_control_tx,
                        true,
                        auth_tag.as_ref(),
                    )
                    .await,
                    ReconnectOutcome::Shutdown
                ) {
                    return;
                }
            }
        }
        // ping_sent, last_pong, connected_since are initialized below —
        // no reset needed here since they haven't been declared yet.
    }

    // Finding #31: client-initiated ping to detect silent connection death.
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_pong = Instant::now();
    let mut ping_sent = false;

    // Finding #42: track connection stability for backoff reset.
    let mut connected_since = Instant::now();
    let mut stable_logged = false;

    loop {
        // Finding #3: check proactive resubscribe flag before blocking on select!
        if state.proactive_resubscribe_needed {
            state.proactive_resubscribe_needed = false;
            info!("proactive resubscribe triggered by backpressure event loss");
            if !resubscribe_after_reconnect(&mut ws, &mut state, &agent_pubkey_hex).await {
                warn!("proactive resubscribe had failures — triggering reconnect");
                let _ = event_tx.try_send(None);
                match try_autonomous_reconnect(
                    &mut ws,
                    &mut cmd_rx,
                    &mut state,
                    &keys,
                    &relay_url,
                    &agent_pubkey_hex,
                    &event_tx,
                    &observer_control_tx,
                    auth_tag.as_ref(),
                )
                .await
                {
                    ReconnectOutcome::Ok => {
                        if matches!(
                            drain_post_reconnect(
                                &mut ws,
                                &mut cmd_rx,
                                &mut state,
                                &agent_pubkey_hex
                            )
                            .await,
                            ReconnectOutcome::Shutdown
                        ) {
                            return;
                        }
                    }
                    ReconnectOutcome::Shutdown => return,
                    ReconnectOutcome::Failed => {
                        if matches!(
                            wait_for_reconnect(
                                &mut ws,
                                &mut cmd_rx,
                                &mut state,
                                &keys,
                                &relay_url,
                                &agent_pubkey_hex,
                                &event_tx,
                                &observer_control_tx,
                                true,
                                auth_tag.as_ref(),
                            )
                            .await,
                            ReconnectOutcome::Shutdown
                        ) {
                            return;
                        }
                    }
                }
                ping_sent = false;
                last_pong = Instant::now();
                connected_since = Instant::now();
                stable_logged = false;
            }
        }

        tokio::select! {
                   raw = ws.next() => {
                       // Determine if the socket is lost.
                       let socket_lost = match raw {
                           Some(Ok(msg)) => {
                               // Finding #31: track pong replies directly, before dispatch.
                               if matches!(msg, Message::Pong(_)) {
                                   last_pong = Instant::now();
                                   ping_sent = false;
                                   false // pong is healthy — not a socket loss
                               } else {
                                   !handle_ws_message(
                                       msg,
                                       &mut ws,
                                       &event_tx,
                                       &observer_control_tx,
                                       &mut state,
                                       &keys,
                                       &relay_url,
                                       &agent_pubkey_hex,
                                       auth_tag.as_ref(),
                                   )
                                   .await
                               }
                           }
                           Some(Err(e)) => {
                               warn!("WebSocket error in background task: {e}");
                               true
                           }
                           None => {
                               debug!("WebSocket stream ended");
                               true
                           }
                       };

                       if socket_lost {
                           // Signal the caller, then attempt autonomous reconnect.
                           // Use try_send to avoid blocking on backpressure — recovery
                           // must not stall when the event channel is full.
                           let _ = event_tx.try_send(None);
                           let outcome = try_autonomous_reconnect(
                               &mut ws,
                               &mut cmd_rx,
                               &mut state,
                               &keys,
                               &relay_url,
                               &agent_pubkey_hex,
                               &event_tx,
                           &observer_control_tx,
            auth_tag.as_ref(),
                           )
                           .await;
                           match outcome {
                           ReconnectOutcome::Shutdown => return,
                           ReconnectOutcome::Ok => {
                               if matches!(
                                   drain_post_reconnect(&mut ws, &mut cmd_rx, &mut state, &agent_pubkey_hex).await,
                                   ReconnectOutcome::Shutdown
                               ) { return; }
                               // Reset ping state after reconnect.
                               ping_sent = false;
                               last_pong = Instant::now();
                               connected_since = Instant::now();
                               stable_logged = false;
                           }
                           ReconnectOutcome::Failed => {
                               if matches!(
                                   wait_for_reconnect(
                                       &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx, &observer_control_tx, true,
                        auth_tag.as_ref(),
                                   ).await,
                                   ReconnectOutcome::Shutdown
                               ) { return; }
                               ping_sent = false;
                               last_pong = Instant::now();
                               connected_since = Instant::now();
                               stable_logged = false;
                           }
                           } // end match outcome
                       }
                   }

                   cmd = cmd_rx.recv() => {
                       match cmd {
                           Some(RelayCommand::Reconnect) => {
                               if matches!(
                                   wait_for_reconnect(
                                       &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx, &observer_control_tx, true,
                        auth_tag.as_ref(),
                                   ).await,
                                   ReconnectOutcome::Shutdown
                               ) { return; }
                               ping_sent = false;
                               last_pong = Instant::now();
                               connected_since = Instant::now();
                               stable_logged = false;
                           }
                           Some(RelayCommand::Shutdown) | None => {
                               debug!("background task shutting down — sending close frame");
                               let _ = ws_send_timeout(
                                   &mut ws,
                                   Message::Close(None),
                                   WS_SEND_TIMEOUT_SECS,
                               )
                               .await;
                               return;
                           }
                           Some(cmd) => {
                               let ok = execute_connected_command(
                                   &mut ws,
                                   &mut state,
                                   &agent_pubkey_hex,
                                   cmd,
                               )
                               .await;
                               if !ok {
                                   // Send failed — socket is likely dead. Trigger reconnect.
                                   warn!("command send failed — triggering reconnect");
                                   let _ = event_tx.try_send(None);
                                   match try_autonomous_reconnect(
                                       &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx,
                                   &observer_control_tx,
            auth_tag.as_ref(),
                                   ).await {
                                       ReconnectOutcome::Shutdown => return,
                                       ReconnectOutcome::Ok => {
                                           if matches!(
                                               drain_post_reconnect(&mut ws, &mut cmd_rx, &mut state, &agent_pubkey_hex).await,
                                               ReconnectOutcome::Shutdown
                                           ) { return; }
                                       }
                                       ReconnectOutcome::Failed => {
                                           if matches!(
                                               wait_for_reconnect(
                                                   &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx, &observer_control_tx, true,
                        auth_tag.as_ref(),
                                               ).await,
                                               ReconnectOutcome::Shutdown
                                           ) { return; }
                                       }
                                   }
                                   ping_sent = false;
                                   last_pong = Instant::now();
                                   connected_since = Instant::now();
                                   stable_logged = false;
                               }
                           }
                       }
                   }

                   _ = ping_interval.tick() => {
                       if ping_sent && last_pong.elapsed() > PONG_TIMEOUT {
                           // No pong received after our last ping — connection is dead.
                           warn!("no pong received within {:?} — connection dead, reconnecting", PONG_TIMEOUT);
                           // Use try_send to avoid blocking on backpressure during recovery.
                           let _ = event_tx.try_send(None);
                           match try_autonomous_reconnect(
                               &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx,
                           &observer_control_tx,
            auth_tag.as_ref(),
                           ).await {
                               ReconnectOutcome::Shutdown => return,
                               ReconnectOutcome::Ok => {
                                   if matches!(
                                       drain_post_reconnect(&mut ws, &mut cmd_rx, &mut state, &agent_pubkey_hex).await,
                                       ReconnectOutcome::Shutdown
                                   ) { return; }
                               }
                               ReconnectOutcome::Failed => {
                                   if matches!(
                                       wait_for_reconnect(
                                           &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx, &observer_control_tx, true,
                        auth_tag.as_ref(),
                                       ).await,
                                       ReconnectOutcome::Shutdown
                                   ) { return; }
                               }
                           }
                           ping_sent = false;
                           last_pong = Instant::now();
                           connected_since = Instant::now();
                           stable_logged = false;
                       } else if !ping_sent {
                           if let Err(e) = ws_send_timeout(&mut ws, Message::Ping(vec![].into()), WS_SEND_TIMEOUT_SECS).await {
                               warn!("failed to send ping: {e} — triggering reconnect");
                               // Use try_send to avoid blocking on backpressure during recovery.
                               let _ = event_tx.try_send(None);
                               match try_autonomous_reconnect(
                                   &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx,
                               &observer_control_tx,
            auth_tag.as_ref(),
                               ).await {
                                   ReconnectOutcome::Shutdown => return,
                                   ReconnectOutcome::Ok => {
                                       if matches!(
                                           drain_post_reconnect(&mut ws, &mut cmd_rx, &mut state, &agent_pubkey_hex).await,
                                           ReconnectOutcome::Shutdown
                                       ) { return; }
                                   }
                                   ReconnectOutcome::Failed => {
                                       if matches!(
                                           wait_for_reconnect(
                                               &mut ws, &mut cmd_rx, &mut state, &keys, &relay_url,
        &agent_pubkey_hex, &event_tx, &observer_control_tx, true,
                        auth_tag.as_ref(),
                                           ).await,
                                           ReconnectOutcome::Shutdown
                                       ) { return; }
                                   }
                               }
                               ping_sent = false;
                               last_pong = Instant::now();
                               connected_since = Instant::now();
                               stable_logged = false;
                           } else {
                               ping_sent = true;
                               debug!("sent ping to relay");
                           }
                       }
                   }
               }

        // Finding #42: log when connection has been stable for STABLE_CONNECTION_SECS.
        // Log once when the connection has been stable. Diagnostic only.
        if !stable_logged && connected_since.elapsed() > Duration::from_secs(STABLE_CONNECTION_SECS)
        {
            stable_logged = true;
            debug!("connection stable for >{}s", STABLE_CONNECTION_SECS);
        }
    }
}

/// Handle a single WebSocket message in the background task.
///
/// Returns `false` if the connection has been lost (Close frame or unrecoverable
/// error), `true` otherwise.
#[allow(clippy::too_many_arguments)]
async fn handle_ws_message(
    msg: Message,
    ws: &mut WsStream,
    event_tx: &mpsc::Sender<Option<BuzzEvent>>,
    observer_control_tx: &mpsc::Sender<Event>,
    state: &mut BgState,
    keys: &Keys,
    relay_url: &str,
    agent_pubkey_hex: &str,
    auth_tag: Option<&nostr::Tag>,
) -> bool {
    match msg {
        Message::Text(text) => {
            let relay_msg = match parse_relay_message(&text) {
                Ok(m) => m,
                Err(e) => {
                    warn!("failed to parse relay message: {e} — raw: {text}");
                    return true;
                }
            };

            match relay_msg {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    if subscription_id == OBSERVER_CONTROL_SUB_ID {
                        match observer_control_tx.try_send(*event) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                warn!("observer control event dropped because control channel is full");
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => return false,
                        }
                    } else if subscription_id == MEMBERSHIP_NOTIF_SUB_ID {
                        // Membership notification — extract channel UUID from h tag.
                        let channel_uuid = match extract_h_tag_uuid(&event) {
                            Some(uuid) => uuid,
                            None => {
                                warn!("membership notification missing h tag — dropping");
                                return true;
                            }
                        };
                        // Dedup membership notifications through TwoGenDedup.
                        // We use seen_ids directly instead of record_event()
                        // because record_event() also updates last_seen, which
                        // would contaminate per-channel replay watermarks with
                        // membership-event timestamps and cause channel event
                        // loss on reconnect.
                        let event_id_hex = event.id.to_hex();
                        if !state.seen_ids.insert(event_id_hex.clone()) {
                            debug!(
                                channel_id = %channel_uuid,
                                event_id = %event_id_hex,
                                "duplicate membership notification — skipping"
                            );
                            return true;
                        }
                        let ts = event.created_at.as_secs();
                        let buzz_event = BuzzEvent {
                            channel_id: channel_uuid,
                            event: *event,
                        };
                        // Finding #3: warn at 80% capacity.
                        let cap = event_tx.max_capacity();
                        let used = cap - event_tx.capacity();
                        if used >= (cap * 4 / 5) {
                            warn!(
                                used,
                                capacity = cap,
                                "event channel at ≥80% capacity — backpressure imminent"
                            );
                        }
                        match event_tx.try_send(Some(buzz_event)) {
                            Ok(()) => {
                                state.membership_last_seen =
                                    Some(state.membership_last_seen.unwrap_or(0).max(ts));
                            }
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                // Remove from dedup so reconnect replay can
                                // re-deliver this event (it was never forwarded
                                // to the harness).
                                state.seen_ids.remove(&event_id_hex);
                                // Track the oldest dropped timestamp so reconnect
                                // replay starts early enough to re-deliver it.
                                state.membership_dropped_since =
                                    Some(state.membership_dropped_since.map_or(ts, |d| d.min(ts)));
                                // Finding #3: proactively trigger resubscribe without
                                // waiting for a disconnect.
                                state.proactive_resubscribe_needed = true;
                                warn!(
                                    channel_id = %channel_uuid,
                                    ts,
                                    "membership notification dropped (backpressure) — proactive resubscribe queued"
                                );
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => return false,
                        }
                    } else if let Some(channel_id) = channel_id_from_sub_id(&subscription_id) {
                        let ts = event.created_at.as_secs();
                        let event_id_hex = event.id.to_hex();
                        if state.record_event(channel_id, &event) {
                            let buzz_event = BuzzEvent {
                                channel_id,
                                event: *event,
                            };
                            // Finding #3: warn at 80% capacity.
                            let cap = event_tx.max_capacity();
                            let used = cap - event_tx.capacity();
                            if used >= (cap * 4 / 5) {
                                warn!(
                                    used,
                                    capacity = cap,
                                    "event channel at ≥80% capacity — backpressure imminent"
                                );
                            }
                            match event_tx.try_send(Some(buzz_event)) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    // Remove from dedup set so the replayed event
                                    // won't be rejected as a duplicate after reconnect.
                                    state.seen_ids.remove(&event_id_hex);
                                    // Track the oldest dropped timestamp so reconnect
                                    // replay starts early enough to re-deliver it.
                                    state
                                        .channel_dropped_since
                                        .entry(channel_id)
                                        .and_modify(|d| *d = (*d).min(ts))
                                        .or_insert(ts);
                                    // Finding #3: proactively trigger resubscribe.
                                    state.proactive_resubscribe_needed = true;
                                    warn!(
                                        channel_id = %channel_id,
                                        ts,
                                        "event channel full — dropping event for channel {channel_id} — proactive resubscribe queued"
                                    );
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    // Receiver dropped — shut down.
                                    return false;
                                }
                            }
                        } else {
                            debug!("dropping duplicate event for channel {channel_id}");
                        }
                    } else {
                        warn!("received EVENT for unknown subscription {subscription_id}");
                    }
                }
                RelayMessage::Eose { subscription_id } => {
                    debug!("EOSE for subscription {subscription_id}");
                }
                RelayMessage::Notice { message } => {
                    // Fix 4: NOTICE at warn level.
                    tracing::warn!("relay NOTICE: {message}");
                }
                RelayMessage::Closed {
                    subscription_id,
                    message,
                } => {
                    // A per-channel membership denial means THIS channel is
                    // forbidden, not the whole connection. Drop just this
                    // channel's subscription and keep the socket — otherwise the
                    // socket is torn down, the forbidden channel is resubscribed,
                    // and the same CLOSED arrives again: a tight reconnect loop.
                    if drop_channel_on_access_denied(state, &subscription_id, &message) {
                        return true;
                    }

                    // Finding #15: CLOSED needs cleanup and resubscribe, not just logging.
                    // Classify the error to decide how to respond.
                    let is_auth_error = message.starts_with("auth-required")
                        || message.starts_with("restricted")
                        || message.contains("auth");
                    warn!(
                        "subscription {subscription_id} closed by relay: {message}{}",
                        if is_auth_error {
                            " [auth error — reconnect required]"
                        } else {
                            ""
                        }
                    );

                    if is_auth_error {
                        // Auth errors require a full reconnect (re-handshake).
                        return false;
                    }

                    // Attempt targeted resubscribe. State is NOT cleared before
                    // the attempt — if the send fails and triggers reconnect,
                    // resubscribe_after_reconnect() needs the subscription to
                    // still be in state so it can restore it.
                    if subscription_id == OBSERVER_CONTROL_SUB_ID {
                        let sent = send_observer_control_subscribe(ws, agent_pubkey_hex).await;
                        if sent {
                            state.observer_control_sub_active = true;
                        } else {
                            warn!("observer control resubscribe failed after CLOSED — triggering reconnect");
                            return false;
                        }
                    } else if subscription_id == MEMBERSHIP_NOTIF_SUB_ID {
                        let since =
                            match (state.membership_dropped_since, state.membership_last_seen) {
                                (Some(d), Some(l)) => Some(d.min(l)),
                                (Some(d), None) => Some(d),
                                (None, Some(l)) => Some(l),
                                (None, None) => state.startup_watermark,
                            };
                        let sent = send_membership_subscribe(ws, agent_pubkey_hex, since).await;
                        if sent {
                            // Success — subscription is live again.
                            state.membership_dropped_since = None;
                        } else {
                            // Resubscribe failed — likely half-dead socket.
                            // Keep membership_sub_active = true so reconnect restores it.
                            warn!(
                                "membership resubscribe failed after CLOSED — triggering reconnect"
                            );
                            return false;
                        }
                    } else if let Some(channel_id) = channel_id_from_sub_id(&subscription_id) {
                        // Guard: only resubscribe if the channel is still active.
                        // A delayed CLOSED for an already-unsubscribed channel must
                        // NOT resurrect the subscription (especially with a default
                        // permissive filter, which would be a fail-open regression).
                        if !state.active_subscriptions.contains_key(&channel_id) {
                            debug!("ignoring CLOSED for already-unsubscribed channel {channel_id}");
                        } else {
                            let since = state.channel_since(&channel_id);
                            let filter = match state.active_filters.get(&channel_id).cloned() {
                                Some(f) => f,
                                None => {
                                    // Fail closed: missing filter state means the subscription
                                    // intent is inconsistent. Trigger reconnect rather than
                                    // resubscribing with a permissive wildcard.
                                    warn!("missing filter for channel {channel_id} after CLOSED — triggering reconnect (fail-closed)");
                                    return false;
                                }
                            };
                            let sent = send_subscribe(
                                ws,
                                state,
                                channel_id,
                                agent_pubkey_hex,
                                since,
                                &filter,
                            )
                            .await;
                            if sent {
                                // Success — update subscription ID (relay may assign new one).
                                state
                                    .active_subscriptions
                                    .insert(channel_id, channel_sub_id(channel_id));
                                state.channel_dropped_since.remove(&channel_id);
                            } else {
                                // Resubscribe failed — likely half-dead socket.
                                // Keep channel in active_subscriptions so reconnect restores it.
                                warn!("channel {channel_id} resubscribe failed after CLOSED — triggering reconnect");
                                return false;
                            }
                        } // end: channel is still active
                    } else {
                        warn!("CLOSED for unknown subscription {subscription_id} — ignoring");
                    }
                }
                RelayMessage::Auth { challenge } => {
                    // Finding #18: AUTH send failure must trigger reconnect.
                    debug!("received mid-session AUTH challenge — re-authenticating");
                    if let Err(e) =
                        send_auth_response(ws, &challenge, relay_url, keys, auth_tag).await
                    {
                        warn!("failed to respond to mid-session AUTH challenge: {e} — triggering reconnect");
                        return false;
                    }
                }
                RelayMessage::Ok {
                    event_id,
                    accepted,
                    message,
                } => {
                    if !accepted && message.starts_with("auth") {
                        // Finding #18: AUTH OK with accepted=false means auth was rejected.
                        warn!("mid-session AUTH rejected (event {event_id}): {message} — triggering reconnect");
                        return false;
                    }
                    debug!("OK for event {event_id}: accepted={accepted} message={message}");
                }
            }
            true
        }
        Message::Ping(data) => {
            if let Err(e) = ws_send_timeout(ws, Message::Pong(data), WS_SEND_TIMEOUT_SECS).await {
                warn!("failed to send pong: {e}");
                return false;
            }
            true
        }
        Message::Close(_) => {
            debug!("relay sent Close frame");
            false
        }
        // Binary, Pong, Frame — ignore
        _ => true,
    }
}

/// Process messages buffered during the NIP-42 auth handshake (Finding #8).
///
/// `do_connect` buffers any non-AUTH/non-OK messages it receives while waiting
/// for the challenge and OK. Those messages would otherwise be silently
/// discarded. We replay them through the normal handler here.
#[allow(clippy::too_many_arguments)]
/// Returns `false` if any buffered message signals the connection should be dropped.
async fn process_handshake_buffer(
    ws: &mut WsStream,
    buffer: std::collections::VecDeque<RelayMessage>,
    event_tx: &mpsc::Sender<Option<BuzzEvent>>,
    observer_control_tx: &mpsc::Sender<Event>,
    state: &mut BgState,
    keys: &Keys,
    relay_url: &str,
    agent_pubkey_hex: &str,
    auth_tag: Option<&nostr::Tag>,
) -> bool {
    if buffer.is_empty() {
        return true;
    }
    debug!("processing {} buffered handshake message(s)", buffer.len());
    for relay_msg in buffer {
        // Re-encode to text so we can reuse handle_ws_message.
        // This is slightly wasteful but keeps the handler as the single
        // source of truth for message dispatch.
        let text = match &relay_msg {
            RelayMessage::Event {
                subscription_id,
                event,
            } => serde_json::to_string(&json!(["EVENT", subscription_id, event])).ok(),
            RelayMessage::Eose { subscription_id } => {
                serde_json::to_string(&json!(["EOSE", subscription_id])).ok()
            }
            RelayMessage::Notice { message } => {
                serde_json::to_string(&json!(["NOTICE", message])).ok()
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => serde_json::to_string(&json!(["CLOSED", subscription_id, message])).ok(),
            RelayMessage::Ok {
                event_id,
                accepted,
                message,
            } => serde_json::to_string(&json!(["OK", event_id, accepted, message])).ok(),
            // AUTH in the buffer is stale — skip it.
            RelayMessage::Auth { .. } => None,
        };
        if let Some(text) = text {
            let should_continue = handle_ws_message(
                Message::Text(text.into()),
                ws,
                event_tx,
                observer_control_tx,
                state,
                keys,
                relay_url,
                agent_pubkey_hex,
                auth_tag,
            )
            .await;
            if !should_continue {
                return false;
            }
        }
    }
    true
}

/// Resubscribe all active channels and membership notifications after a
/// successful reconnect. Computes `since = min(last_seen, channel_dropped_since)`
/// per channel, and only clears the drop tracker when the REQ is confirmed sent.
///
/// Returns `true` if ALL subscriptions were sent successfully. Returns `false`
/// if any send failed — the caller should treat this as a failed reconnect
/// and retry, because a "connected" socket with missing subscriptions causes
/// silent event loss.
async fn resubscribe_after_reconnect(
    ws: &mut WsStream,
    state: &mut BgState,
    agent_pubkey_hex: &str,
) -> bool {
    let mut all_ok = true;
    let channels: Vec<Uuid> = state.active_subscriptions.keys().copied().collect();
    if !channels.is_empty() {
        info!(
            "resubscribing to {} channel(s) after reconnect",
            channels.len()
        );
        for channel_id in channels {
            let since = state.channel_since(&channel_id);
            let filter = match state.active_filters.get(&channel_id).cloned() {
                Some(f) => f,
                None => {
                    // Fail closed: missing filter state means the subscription
                    // intent is inconsistent. Skip rather than resubscribe with
                    // a permissive wildcard that would widen the subscription.
                    warn!("missing filter for channel {channel_id} — skipping resubscribe (fail-closed)");
                    all_ok = false;
                    continue;
                }
            };
            let sent =
                send_subscribe(ws, state, channel_id, agent_pubkey_hex, since, &filter).await;
            if sent {
                state.channel_dropped_since.remove(&channel_id);
            } else {
                warn!("failed to resubscribe channel {channel_id} after reconnect");
                all_ok = false;
            }
        }
    }

    if state.membership_sub_active {
        let replay_since = match (state.membership_dropped_since, state.membership_last_seen) {
            (Some(d), Some(l)) => Some(d.min(l)),
            (Some(d), None) => Some(d),
            (None, Some(l)) => Some(l),
            (None, None) => state.startup_watermark,
        };
        let sent = send_membership_subscribe(ws, agent_pubkey_hex, replay_since).await;
        if sent {
            state.membership_dropped_since = None;
        } else {
            warn!("failed to resubscribe membership after reconnect");
            all_ok = false;
        }
    }

    if state.observer_control_sub_active
        && !send_observer_control_subscribe(ws, agent_pubkey_hex).await
    {
        warn!("failed to resubscribe observer controls after reconnect");
        all_ok = false;
    }

    all_ok
}

/// Attempt autonomous reconnect on socket loss.
///
/// Finding #42: 5 attempts with 1s→2s→4s→8s→16s backoff (was 3 attempts).
/// Finding #27: ±20% jitter on each sleep.
/// Finding #8: process handshake buffer on success.
///
/// Outcome of an autonomous reconnect attempt.
enum ReconnectOutcome {
    /// Reconnected and resubscribed successfully.
    Ok,
    /// All attempts exhausted — caller should fall back to wait_for_reconnect.
    Failed,
    /// A Shutdown command was received during backoff — caller must return immediately.
    Shutdown,
}

/// Drain all pending commands after a successful reconnect.
///
/// Processes queued commands that arrived while reconnecting. Reconnect
/// commands are silently dropped (already reconnected). Shutdown causes an
/// immediate close-frame + return of `ReconnectOutcome::Shutdown`. All other
/// commands are executed on the live socket via [`execute_connected_command`].
/// If any send fails, remaining commands are recorded as intent via
/// [`apply_command_to_state`] and the drain continues (the caller's next
/// read/ping will detect the dead socket).
async fn drain_post_reconnect(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    agent_pubkey_hex: &str,
) -> ReconnectOutcome {
    let mut send_failed = false;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if send_failed {
            match cmd {
                RelayCommand::Shutdown => {
                    let _ = ws_send_timeout(ws, Message::Close(None), WS_SEND_TIMEOUT_SECS).await;
                    return ReconnectOutcome::Shutdown;
                }
                RelayCommand::Reconnect => {}
                cmd => apply_command_to_state(state, cmd),
            }
            continue;
        }
        match cmd {
            RelayCommand::Reconnect => {
                debug!("drained stale Reconnect after reconnect");
            }
            RelayCommand::Shutdown => {
                debug!("shutdown received during post-reconnect drain");
                let _ = ws_send_timeout(ws, Message::Close(None), WS_SEND_TIMEOUT_SECS).await;
                return ReconnectOutcome::Shutdown;
            }
            cmd => {
                let ok = execute_connected_command(ws, state, agent_pubkey_hex, cmd).await;
                if !ok {
                    warn!("send failed during post-reconnect drain — recording remaining commands as intent");
                    send_failed = true;
                }
            }
        }
    }
    ReconnectOutcome::Ok
}

/// Attempt autonomous reconnect on socket loss.
///
/// Returns [`ReconnectOutcome::Ok`] on success, [`ReconnectOutcome::Failed`]
/// if all attempts are exhausted, or [`ReconnectOutcome::Shutdown`] if a
/// Shutdown command was received during backoff sleep. Callers MUST check
/// for `Shutdown` and return immediately — do NOT fall through to
/// `wait_for_reconnect`, which would loop forever since the Shutdown command
/// was already consumed.
#[allow(clippy::too_many_arguments)]
async fn try_autonomous_reconnect(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    keys: &Keys,
    relay_url: &str,
    agent_pubkey_hex: &str,
    event_tx: &mpsc::Sender<Option<BuzzEvent>>,
    observer_control_tx: &mpsc::Sender<Event>,
    auth_tag: Option<&nostr::Tag>,
) -> ReconnectOutcome {
    // Finding #42: 5 attempts, up to 16s base backoff.
    let backoffs = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(8),
        Duration::from_secs(16),
    ];

    for (attempt, delay) in backoffs.iter().enumerate() {
        info!(
            "autonomous reconnect attempt {}/{} to {relay_url}…",
            attempt + 1,
            backoffs.len()
        );
        match do_connect(relay_url, keys, auth_tag).await {
            Ok((new_ws, handshake_buffer)) => {
                *ws = new_ws;
                info!("autonomous reconnect succeeded (attempt {})", attempt + 1);
                // Finding #8: process buffered messages from the handshake.
                let handshake_ok = process_handshake_buffer(
                    ws,
                    handshake_buffer,
                    event_tx,
                    observer_control_tx,
                    state,
                    keys,
                    relay_url,
                    agent_pubkey_hex,
                    auth_tag,
                )
                .await;
                if !handshake_ok {
                    warn!(
                        "handshake buffer drop signal after autonomous reconnect (attempt {})",
                        attempt + 1
                    );
                    // Fall through to backoff sleep instead of returning immediately.
                    // Returning false here would skip remaining attempts; continuing
                    // without sleep would drive a tight reconnect storm.
                } else if resubscribe_after_reconnect(ws, state, agent_pubkey_hex).await {
                    return ReconnectOutcome::Ok;
                } else {
                    warn!("resubscribe failed after autonomous reconnect — treating as failed attempt");
                    // Fall through to backoff sleep and retry.
                }
            }
            Err(e) => {
                warn!("autonomous reconnect attempt {} failed: {e}", attempt + 1);
            }
        }

        // Backoff sleep between attempts (shared by handshake-drop and connect-error).
        // Skip sleep on the final attempt — we'll fall through to the caller.
        // Use select! so Shutdown commands are honoured during sleep.
        if attempt + 1 < backoffs.len() {
            let jittered = jittered_duration(*delay);
            tracing::info!(
                "retrying autonomous reconnect in {:.1}s",
                jittered.as_secs_f64()
            );
            // Deadline-based sleep: commands processed during the wait don't
            // reset the timer (prevents PublishEvent traffic from collapsing backoff).
            let deadline = tokio::time::Instant::now() + jittered;
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(RelayCommand::Shutdown) | None => return ReconnectOutcome::Shutdown,
                            Some(cmd) => apply_command_to_state(state, cmd),
                        }
                    }
                }
            }
        }
    }

    ReconnectOutcome::Failed
}

/// Attempt reconnection with exponential backoff. Resubscribes all active
/// channels with `since` filters on success.
///
/// If `skip_drain` is `false`, drains the command channel until a `Reconnect`
/// command arrives (used when called from the WS-error path where the caller
/// hasn't sent Reconnect yet). If `true`, skips the drain and reconnects
/// immediately (used when called from the `RelayCommand::Reconnect` arm where
/// the command was already consumed).
#[allow(clippy::too_many_arguments)]
async fn wait_for_reconnect(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    keys: &Keys,
    relay_url: &str,
    agent_pubkey_hex: &str,
    event_tx: &mpsc::Sender<Option<BuzzEvent>>,
    observer_control_tx: &mpsc::Sender<Event>,
    skip_drain: bool,
    auth_tag: Option<&nostr::Tag>,
) -> ReconnectOutcome {
    if !skip_drain {
        // Drain commands until we get Reconnect (or Shutdown).
        // Other commands update state so reconnect reflects latest intent.
        loop {
            match cmd_rx.recv().await {
                Some(RelayCommand::Reconnect) => break,
                Some(RelayCommand::Shutdown) | None => return ReconnectOutcome::Shutdown,
                Some(cmd) => apply_command_to_state(state, cmd),
            }
        }
    }

    // Finding #42: 6 attempts with backoff up to 32s + jitter (Finding #27).
    // Finding #27: use tokio::select! so shutdown is honoured during sleep.
    let backoffs = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(8),
        Duration::from_secs(16),
        Duration::from_secs(32),
    ];
    let mut attempt = 0usize;
    let mut delay = Duration::from_secs(1);
    loop {
        info!("attempting relay reconnect to {relay_url}…");
        match do_connect(relay_url, keys, auth_tag).await {
            Ok((new_ws, handshake_buffer)) => {
                *ws = new_ws;
                info!("relay reconnected to {relay_url}");
                // Finding #8: process buffered messages from the handshake.
                let handshake_ok = process_handshake_buffer(
                    ws,
                    handshake_buffer,
                    event_tx,
                    observer_control_tx,
                    state,
                    keys,
                    relay_url,
                    agent_pubkey_hex,
                    auth_tag,
                )
                .await;
                if !handshake_ok {
                    warn!("handshake buffer contained a drop signal after reconnect — will retry with backoff");
                    // Fall through to the backoff sleep below instead of
                    // tight-looping. A relay that consistently fails the
                    // handshake would otherwise drive a reconnect storm.
                } else if resubscribe_after_reconnect(ws, state, agent_pubkey_hex).await {
                    // Drain any commands that arrived during the final
                    // do_connect() + resubscribe (which don't poll cmd_rx).
                    return drain_post_reconnect(ws, cmd_rx, state, agent_pubkey_hex).await;
                } else {
                    warn!("resubscribe failed after reconnect — will retry with backoff");
                    // Fall through to backoff sleep.
                }
            }
            Err(e) => {
                warn!("relay reconnect failed: {e}");
            }
        }

        // Backoff sleep — shared by both handshake-drop and connect-error paths.
        // Uses a deadline so commands processed during the wait don't reset
        // the timer. Without this, periodic PublishEvent traffic (typing
        // refresh every 3s) would collapse the jittered backoff into a
        // reconnect storm.
        let jittered = jittered_duration(delay);
        warn!("retrying reconnect in {:.1}s", jittered.as_secs_f64());
        let deadline = tokio::time::Instant::now() + jittered;
        let sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(RelayCommand::Shutdown) | None => return ReconnectOutcome::Shutdown,
                        Some(cmd) => apply_command_to_state(state, cmd),
                    }
                }
            }
        }
        attempt += 1;
        delay = if attempt < backoffs.len() {
            backoffs[attempt]
        } else {
            Duration::from_secs(60)
        };
    }
}

/// Send a NIP-01 REQ for a channel, built from a [`ChannelFilter`].
///
/// - `kinds` is included only when `filter.kinds` is `Some`; `None` = wildcard.
/// - `#p` is included only when `filter.require_mention` is `true`.
/// - `#h` is always included (channel-scoped subscription).
/// - On first subscribe (`since` is `None`) adds `since=now` to avoid replaying
///   history. On reconnect (`since` is `Some`) subtracts [`SINCE_SKEW_SECS`].
///
/// Returns `true` if the REQ was successfully written to the WebSocket.
async fn send_subscribe(
    ws: &mut WsStream,
    _state: &BgState,
    channel_id: Uuid,
    agent_pubkey_hex: &str,
    since: Option<u64>,
    filter: &ChannelFilter,
) -> bool {
    let sub_id = channel_sub_id(channel_id);

    let mut req_filter = serde_json::Map::new();

    // kinds — omit entirely for wildcard subscriptions.
    if let Some(ref kinds) = filter.kinds {
        req_filter.insert("kinds".into(), json!(kinds));
    }

    // #h — always present (channel scope).
    req_filter.insert("#h".into(), json!([channel_id.to_string()]));

    // #p — only when require_mention is true.
    if filter.require_mention {
        req_filter.insert("#p".into(), json!([agent_pubkey_hex]));
    }

    // since — on first subscribe use current time to skip history; on reconnect
    // subtract skew buffer to catch events missed during the disconnect window.
    let since_ts = match since {
        Some(ts) => ts.saturating_sub(SINCE_SKEW_SECS),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    req_filter.insert("since".into(), json!(since_ts));

    let req = json!(["REQ", sub_id, Value::Object(req_filter)]);

    match serde_json::to_string(&req) {
        Ok(text) => {
            match ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await {
                Ok(()) => {
                    debug!(
                        "subscribed to channel {channel_id}{}",
                        if since.is_some() {
                            " (with since filter)"
                        } else {
                            " (since=now)"
                        }
                    );
                    true
                }
                Err(e) => {
                    warn!("failed to send REQ for channel {channel_id}: {e}");
                    false
                }
            }
        }
        Err(e) => {
            warn!("failed to serialize REQ for channel {channel_id}: {e}");
            false
        }
    }
}

/// Send a NIP-01 REQ for membership notifications (kind:44100+44101, global, #p=[agent_pubkey]).
/// Returns `true` if the REQ was successfully written to the WebSocket.
async fn send_membership_subscribe(
    ws: &mut WsStream,
    agent_pubkey_hex: &str,
    since: Option<u64>,
) -> bool {
    let mut req_filter = serde_json::Map::new();
    req_filter.insert(
        "kinds".into(),
        json!([
            KIND_MEMBER_ADDED_NOTIFICATION,
            KIND_MEMBER_REMOVED_NOTIFICATION
        ]),
    );
    req_filter.insert("#p".into(), json!([agent_pubkey_hex]));

    let since_ts = match since {
        Some(ts) => ts.saturating_sub(SINCE_SKEW_SECS),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    req_filter.insert("since".into(), json!(since_ts));

    let req = json!(["REQ", MEMBERSHIP_NOTIF_SUB_ID, Value::Object(req_filter)]);
    match serde_json::to_string(&req) {
        Ok(text) => {
            match ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await {
                Ok(()) => {
                    debug!("subscribed to membership notifications (since={since_ts})");
                    true
                }
                Err(e) => {
                    warn!("failed to send membership notification REQ: {e}");
                    false
                }
            }
        }
        Err(e) => {
            warn!("failed to serialize membership notification REQ: {e}");
            false
        }
    }
}

/// Send a NIP-01 REQ for owner-to-agent observer control frames.
async fn send_observer_control_subscribe(ws: &mut WsStream, agent_pubkey_hex: &str) -> bool {
    let req = json!([
        "REQ",
        OBSERVER_CONTROL_SUB_ID,
        {
            "kinds": [KIND_AGENT_OBSERVER_FRAME],
            "#p": [agent_pubkey_hex],
            "since": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    ]);

    match serde_json::to_string(&req) {
        Ok(text) => {
            match ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await {
                Ok(()) => {
                    debug!("subscribed to observer control frames");
                    true
                }
                Err(e) => {
                    warn!("failed to send observer control REQ: {e}");
                    false
                }
            }
        }
        Err(e) => {
            warn!("failed to serialize observer control REQ: {e}");
            false
        }
    }
}

/// Send a WebSocket message with a hard timeout.
///
/// All `ws.send()` calls go through here so a stalled TCP socket can't wedge
/// the background task. On timeout the caller should break out of the loop to
/// trigger reconnect.
async fn ws_send_timeout(
    ws: &mut WsStream,
    msg: Message,
    timeout_secs: u64,
) -> Result<(), RelayError> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), ws.send(msg))
        .await
        .map_err(|_| RelayError::Timeout)?
        .map_err(|e| RelayError::WebSocket(Box::new(e)))
}

/// Add ±20% jitter to a backoff duration using the nanosecond sub-second
/// component of the system clock as a cheap entropy source (no `rand` dep).
fn jittered_duration(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // factor ∈ [0.8, 1.2)
    let factor = 0.8 + (nanos as f64 / u32::MAX as f64) * 0.4;
    base.mul_f64(factor)
}

/// Extract a channel UUID from the h tag of a Nostr event.
fn extract_h_tag_uuid(event: &nostr::Event) -> Option<Uuid> {
    event.tags.iter().find_map(|tag| {
        let tag_vec = tag.as_slice();
        if tag_vec.len() >= 2 && tag_vec[0] == "h" {
            tag_vec[1].parse::<Uuid>().ok()
        } else {
            None
        }
    })
}

/// Build and send a NIP-42 AUTH response event.
///
/// If `auth_tag` is provided (NIP-OA owner attestation), it is included in the
/// AUTH event so the relay can use it for membership delegation fallback.
async fn send_auth_response(
    ws: &mut WsStream,
    challenge: &str,
    relay_url: &str,
    keys: &Keys,
    auth_tag: Option<&nostr::Tag>,
) -> Result<(), RelayError> {
    let relay_nostr_url = RelayUrl::parse(relay_url)
        .map_err(|e| RelayError::Http(format!("invalid relay URL: {e}")))?;

    let auth_event = if let Some(tag) = auth_tag {
        // Cannot use EventBuilder::auth() shortcut — it doesn't accept extra tags.
        let tags = vec![
            nostr::Tag::parse(["relay", relay_url])
                .map_err(|e| RelayError::Http(format!("tag parse error: {e}")))?,
            nostr::Tag::parse(["challenge", challenge])
                .map_err(|e| RelayError::Http(format!("tag parse error: {e}")))?,
            tag.clone(),
        ];
        EventBuilder::new(nostr::Kind::Authentication, "")
            .tags(tags)
            .sign_with_keys(keys)?
    } else {
        EventBuilder::auth(challenge, relay_nostr_url).sign_with_keys(keys)?
    };

    let auth_msg = serde_json::to_string(&json!(["AUTH", auth_event]))?;
    ws_send_timeout(ws, Message::Text(auth_msg.into()), WS_SEND_TIMEOUT_SECS).await?;
    debug!("sent AUTH response for challenge");
    Ok(())
}

/// Convert a WebSocket URL to its HTTP equivalent.
///
/// `ws://host:port` → `http://host:port`
/// `wss://host:port` → `https://host:port`
/// Trailing slashes are stripped.
pub(crate) fn relay_ws_to_http(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Build the subscription ID for a channel: `ch-<uuid>`.
pub(crate) fn channel_sub_id(channel_id: Uuid) -> String {
    format!("ch-{channel_id}")
}

/// Extract a channel UUID from a subscription ID of the form `ch-<uuid>`.
/// Returns `None` if the format doesn't match or the UUID is invalid.
fn channel_id_from_sub_id(sub_id: &str) -> Option<Uuid> {
    sub_id
        .strip_prefix("ch-")
        .and_then(|s| s.parse::<Uuid>().ok())
}

/// Per-channel CLOSED denials: the channel is forbidden but the connection is
/// fine. Match these EXACT strings, never a `starts_with("restricted")` prefix —
/// a prefix would also swallow connection-level `restricted: insufficient scope`,
/// dropping a channel instead of reconnecting. The only CLOSED senders of these
/// strings are `req.rs:153` (not a channel member) and `side_effects.rs:71`
/// (channel access revoked, via member eviction / open→private flip).
/// `ingest.rs` returns these as EVENT-publish `OK(false)`, never as a
/// subscription CLOSED, so it is not a source here.
const CHANNEL_ACCESS_DENIED_REASONS: &[&str] = &[
    "restricted: not a channel member",
    "restricted: channel access revoked",
];

/// Handle a CLOSED that denies access to a single channel: drop just that
/// channel's subscription (the proven Unsubscribe cleanup) and keep the socket.
///
/// Returns `true` when the CLOSED was an exact per-channel denial on a `ch-`
/// subscription and the channel was dropped — the caller keeps the connection
/// with no reconnect. Returns `false` for everything else (connection-level
/// `restricted: insufficient scope`, `auth-required`, non-channel subs), which
/// falls through to the existing reconnect path.
///
/// An already-removed channel is a harmless no-op: the remove/clear simply
/// affect nothing, and the dropped channel is never re-subscribed, so the loop
/// cannot re-form.
fn drop_channel_on_access_denied(state: &mut BgState, sub_id: &str, message: &str) -> bool {
    if !CHANNEL_ACCESS_DENIED_REASONS.contains(&message) {
        return false;
    }
    let Some(channel_id) = channel_id_from_sub_id(sub_id) else {
        return false;
    };
    warn!(
        "channel {channel_id} access denied by relay: {message} — dropping subscription, keeping connection"
    );
    state.active_subscriptions.remove(&channel_id);
    state.clear_channel_state(&channel_id);
    true
}

/// Apply the appropriate auth header to a reqwest request builder.
/// Parse a raw relay text frame into a typed [`RelayMessage`].
#[allow(private_interfaces)]
pub(crate) fn parse_relay_message(text: &str) -> Result<RelayMessage, RelayError> {
    let arr: Vec<Value> = serde_json::from_str(text)?;

    let msg_type = arr
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?;

    match msg_type {
        "EVENT" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let event: Event = serde_json::from_value(
                arr.get(2)
                    .cloned()
                    .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?,
            )?;
            Ok(RelayMessage::Event {
                subscription_id: sub_id,
                event: Box::new(event),
            })
        }
        "OK" => {
            let event_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            let message = arr
                .get(3)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Ok {
                event_id,
                accepted,
                message,
            })
        }
        "EOSE" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Eose {
                subscription_id: sub_id,
            })
        }
        "CLOSED" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let message = arr
                .get(2)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Closed {
                subscription_id: sub_id,
                message,
            })
        }
        "NOTICE" => {
            let message = arr
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Notice { message })
        }
        "AUTH" => {
            let challenge = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Auth { challenge })
        }
        other => Err(RelayError::UnexpectedMessage(format!(
            "unknown message type: {other}"
        ))),
    }
}

/// Perform a single WebSocket connect + NIP-42 auth handshake.
///
/// Returns `(ws, buffer)` on success.
async fn do_connect(
    relay_url: &str,
    keys: &Keys,
    auth_tag: Option<&nostr::Tag>,
) -> Result<(WsStream, VecDeque<RelayMessage>), RelayError> {
    let parsed = relay_url
        .parse::<url::Url>()
        .map_err(|e| RelayError::Http(format!("invalid relay URL: {e}")))?;

    let (ws, _response) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(parsed.as_str()))
        .await
        .map_err(|_| RelayError::ConnectionClosed)? // timeout → treat as connection failure
        .map_err(|e| RelayError::WebSocket(Box::new(e)))?;
    debug!("connected to relay at {relay_url}");

    let mut ws = ws;
    let mut buffer: VecDeque<RelayMessage> = VecDeque::new();

    let challenge = wait_for_auth_challenge(&mut ws, &mut buffer, AUTH_TIMEOUT).await?;

    send_auth_response(&mut ws, &challenge, relay_url, keys, auth_tag).await?;

    let event_id = {
        // We need the event_id that was just sent. Re-derive it by signing again
        // just to get the ID — but that's wasteful. Instead, parse the last sent
        // message. Simpler: wait_for_ok accepts any OK (we just sent one event).
        // The event_id in the OK will match whatever we sent.
        // We'll accept the first OK we receive.
        let ok = wait_for_any_ok(&mut ws, &mut buffer, AUTH_TIMEOUT).await?;
        if !ok.accepted {
            return Err(RelayError::AuthFailed(ok.message));
        }
        ok.event_id
    };

    debug!("NIP-42 authentication successful (event {event_id})");
    Ok((ws, buffer))
}

/// Wait for an `AUTH` challenge from the relay, buffering any other messages.
async fn wait_for_auth_challenge(
    ws: &mut WsStream,
    buffer: &mut VecDeque<RelayMessage>,
    timeout_dur: Duration,
) -> Result<String, RelayError> {
    // Check if there's already one buffered.
    if let Some(idx) = buffer
        .iter()
        .position(|m| matches!(m, RelayMessage::Auth { .. }))
    {
        if let Some(RelayMessage::Auth { challenge }) = buffer.remove(idx) {
            return Ok(challenge);
        }
    }

    let deadline = tokio::time::Instant::now() + timeout_dur;

    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            return Err(RelayError::NoAuthChallenge);
        }

        let raw = timeout(remaining, ws.next())
            .await
            .map_err(|_| RelayError::NoAuthChallenge)?
            .ok_or(RelayError::ConnectionClosed)?
            .map_err(|e| RelayError::WebSocket(Box::new(e)))?;

        match raw {
            Message::Text(text) => {
                let msg = parse_relay_message(&text)?;
                match msg {
                    RelayMessage::Auth { challenge } => return Ok(challenge),
                    other => buffer.push_back(other),
                }
            }
            Message::Ping(data) => {
                ws_send_timeout(ws, Message::Pong(data), WS_SEND_TIMEOUT_SECS)
                    .await
                    .map_err(|_| RelayError::Timeout)?;
            }
            Message::Close(_) => return Err(RelayError::ConnectionClosed),
            _ => {}
        }
    }
}

/// Response from an `OK` relay message.
struct OkResponse {
    event_id: String,
    accepted: bool,
    message: String,
}

/// Wait for the first `OK` message from the relay (used after sending AUTH).
async fn wait_for_any_ok(
    ws: &mut WsStream,
    buffer: &mut VecDeque<RelayMessage>,
    timeout_dur: Duration,
) -> Result<OkResponse, RelayError> {
    // Check if there's already one buffered.
    if let Some(idx) = buffer
        .iter()
        .position(|m| matches!(m, RelayMessage::Ok { .. }))
    {
        if let Some(RelayMessage::Ok {
            event_id,
            accepted,
            message,
        }) = buffer.remove(idx)
        {
            return Ok(OkResponse {
                event_id,
                accepted,
                message,
            });
        }
    }

    let deadline = tokio::time::Instant::now() + timeout_dur;

    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);

        if remaining.is_zero() {
            return Err(RelayError::Timeout);
        }

        let raw = timeout(remaining, ws.next())
            .await
            .map_err(|_| RelayError::Timeout)?
            .ok_or(RelayError::ConnectionClosed)?
            .map_err(|e| RelayError::WebSocket(Box::new(e)))?;

        match raw {
            Message::Text(text) => {
                let msg = parse_relay_message(&text)?;
                match msg {
                    RelayMessage::Ok {
                        event_id,
                        accepted,
                        message,
                    } => {
                        return Ok(OkResponse {
                            event_id,
                            accepted,
                            message,
                        });
                    }
                    other => buffer.push_back(other),
                }
            }
            Message::Ping(data) => {
                ws_send_timeout(ws, Message::Pong(data), WS_SEND_TIMEOUT_SECS)
                    .await
                    .map_err(|_| RelayError::Timeout)?;
            }
            Message::Close(_) => return Err(RelayError::ConnectionClosed),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_ws_to_http_plain() {
        assert_eq!(
            relay_ws_to_http("ws://localhost:3000"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn relay_ws_to_http_secure() {
        assert_eq!(
            relay_ws_to_http("wss://relay.example.com"),
            "https://relay.example.com"
        );
    }

    #[test]
    fn relay_ws_to_http_strips_trailing_slash() {
        assert_eq!(
            relay_ws_to_http("ws://localhost:3000/"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn relay_ws_to_http_with_path() {
        assert_eq!(
            relay_ws_to_http("wss://relay.example.com/nostr"),
            "https://relay.example.com/nostr"
        );
    }

    #[test]
    fn relay_ws_to_http_with_port_and_path() {
        assert_eq!(
            relay_ws_to_http("wss://relay.example.com:4000/ws"),
            "https://relay.example.com:4000/ws"
        );
    }

    #[test]
    fn channel_sub_id_format() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(
            channel_sub_id(uuid),
            "ch-550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn channel_id_from_sub_id_roundtrip() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let sub_id = channel_sub_id(uuid);
        let recovered = channel_id_from_sub_id(&sub_id).unwrap();
        assert_eq!(recovered, uuid);
    }

    #[test]
    fn channel_id_from_sub_id_invalid_prefix() {
        assert!(channel_id_from_sub_id("sub-550e8400-e29b-41d4-a716-446655440000").is_none());
    }

    #[test]
    fn channel_id_from_sub_id_invalid_uuid() {
        assert!(channel_id_from_sub_id("ch-not-a-uuid").is_none());
    }

    #[test]
    fn channel_id_from_sub_id_empty() {
        assert!(channel_id_from_sub_id("").is_none());
    }

    fn meta_event(uuid: Uuid, name: &str, extra: &[&str]) -> serde_json::Value {
        let mut tags = vec![
            serde_json::json!(["d", uuid.to_string()]),
            serde_json::json!(["name", name]),
        ];
        // `extra` is a flat list of single-value tag names (e.g. archived=true).
        for pair in extra.chunks(2) {
            match pair {
                [k, v] => tags.push(serde_json::json!([k, v])),
                [k] => tags.push(serde_json::json!([k])),
                _ => {}
            }
        }
        serde_json::json!({ "tags": tags })
    }

    #[test]
    fn merge_discovered_channels_skips_archived_metadata() {
        let live = Uuid::new_v4();
        let archived = Uuid::new_v4();
        let meta = serde_json::json!([
            meta_event(live, "live", &[]),
            meta_event(archived, "dead", &["archived", "true"]),
        ]);

        let map = merge_discovered_channels(vec![live, archived], &meta);

        assert!(map.contains_key(&live), "non-archived channel is kept");
        assert!(
            !map.contains_key(&archived),
            "archived=true channel is skipped from the subscribe set"
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn merge_discovered_channels_skips_archived_even_when_still_a_member() {
        // The offline feeder: the agent is still listed as a member
        // (uuid present in channel_uuids, the kind:39002 membership set), but the
        // channel was reaped while the agent was offline. Even though the agent
        // missed the eviction CLOSED, the archived=true kind:39000 makes the
        // client skip re-subscribing on reconnect — proving (b) closes the loop
        // independently of the relay-side eviction.
        let reaped = Uuid::new_v4();
        let meta = serde_json::json!([meta_event(reaped, "reaped", &["archived", "true"])]);

        let map = merge_discovered_channels(vec![reaped], &meta);

        assert!(
            map.is_empty(),
            "a still-member but archived channel is not re-subscribed"
        );
    }

    #[test]
    fn merge_discovered_channels_archived_false_is_kept() {
        // An explicit archived=false (e.g. after unarchive) must NOT be skipped.
        let ch = Uuid::new_v4();
        let meta = serde_json::json!([meta_event(ch, "back", &["archived", "false"])]);

        let map = merge_discovered_channels(vec![ch], &meta);

        assert!(map.contains_key(&ch), "archived=false is treated as live");
    }

    #[test]
    fn parse_ok_accepted() {
        let text = r#"["OK","abc123",true,""]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Ok {
                event_id,
                accepted,
                message,
            } => {
                assert_eq!(event_id, "abc123");
                assert!(accepted);
                assert_eq!(message, "");
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn parse_ok_rejected() {
        let text = r#"["OK","abc123",false,"blocked: spam"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Ok {
                event_id,
                accepted,
                message,
            } => {
                assert_eq!(event_id, "abc123");
                assert!(!accepted);
                assert_eq!(message, "blocked: spam");
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn parse_eose() {
        let text = r#"["EOSE","sub-1"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Eose { subscription_id } => {
                assert_eq!(subscription_id, "sub-1");
            }
            _ => panic!("expected Eose"),
        }
    }

    #[test]
    fn parse_notice() {
        let text = r#"["NOTICE","hello from relay"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Notice { message } => {
                assert_eq!(message, "hello from relay");
            }
            _ => panic!("expected Notice"),
        }
    }

    #[test]
    fn parse_notice_empty() {
        let text = r#"["NOTICE"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Notice { message } => {
                assert_eq!(message, "");
            }
            _ => panic!("expected Notice"),
        }
    }

    #[test]
    fn parse_auth() {
        let text = r#"["AUTH","some-challenge-string"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Auth { challenge } => {
                assert_eq!(challenge, "some-challenge-string");
            }
            _ => panic!("expected Auth"),
        }
    }

    #[test]
    fn parse_closed() {
        let text = r#"["CLOSED","sub-2","error: rate-limited"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, "sub-2");
                assert_eq!(message, "error: rate-limited");
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn parse_closed_no_message() {
        let text = r#"["CLOSED","sub-3"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, "sub-3");
                assert_eq!(message, "");
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn parse_unknown_type_returns_error() {
        let text = r#"["UNKNOWN","data"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
        match result.unwrap_err() {
            RelayError::UnexpectedMessage(msg) => {
                assert!(msg.contains("unknown message type"));
            }
            e => panic!("expected UnexpectedMessage, got {e:?}"),
        }
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let text = "not json at all";
        let result = parse_relay_message(text);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RelayError::Json(_)));
    }

    #[test]
    fn parse_empty_array_returns_error() {
        let text = "[]";
        let result = parse_relay_message(text);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RelayError::UnexpectedMessage(_)
        ));
    }

    #[test]
    fn parse_auth_missing_challenge_returns_error() {
        let text = r#"["AUTH"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
    }

    #[test]
    fn parse_eose_missing_sub_id_returns_error() {
        let text = r#"["EOSE"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
    }

    #[test]
    fn subscription_id_starts_with_ch_prefix() {
        let uuid = Uuid::new_v4();
        let sub_id = channel_sub_id(uuid);
        assert!(sub_id.starts_with("ch-"));
    }

    #[test]
    fn subscription_id_contains_full_uuid() {
        let uuid = Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap();
        let sub_id = channel_sub_id(uuid);
        assert_eq!(sub_id, "ch-12345678-1234-5678-1234-567812345678");
    }

    /// Build a real signed Nostr event for testing BgState.
    ///
    /// Uses `custom_created_at` so tests can control the timestamp.
    /// The event ID is determined by the nostr signing process — we don't
    /// control it, but we return it so callers can use it for dedup tests.
    fn make_test_event(keys: &nostr::Keys, created_at_secs: u64) -> Event {
        let ts = nostr::Timestamp::from(created_at_secs);
        EventBuilder::new(nostr::Kind::TextNote, "test")
            .tags([])
            .custom_created_at(ts)
            .sign_with_keys(keys)
            .expect("signing should succeed")
    }

    #[test]
    fn bg_state_dedup_first_event_accepted() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys = nostr::Keys::generate();
        let event = make_test_event(&keys, 1_000_000);
        assert!(
            state.record_event(channel_id, &event),
            "first event should be accepted"
        );
    }

    #[test]
    fn bg_state_dedup_duplicate_rejected() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys = nostr::Keys::generate();
        let event = make_test_event(&keys, 1_000_000);
        assert!(
            state.record_event(channel_id, &event),
            "first should be accepted"
        );
        assert!(
            !state.record_event(channel_id, &event),
            "duplicate should be rejected"
        );
    }

    #[test]
    fn bg_state_dedup_different_ids_both_accepted() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        // Two different keys → two different event IDs.
        let keys1 = nostr::Keys::generate();
        let keys2 = nostr::Keys::generate();
        let event1 = make_test_event(&keys1, 1_000_000);
        let event2 = make_test_event(&keys2, 1_000_001);
        assert!(state.record_event(channel_id, &event1));
        assert!(state.record_event(channel_id, &event2));
    }

    #[test]
    fn bg_state_last_seen_set_on_first_event() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys = nostr::Keys::generate();
        let event = make_test_event(&keys, 1_700_000);
        state.record_event(channel_id, &event);
        assert_eq!(state.last_seen.get(&channel_id).copied(), Some(1_700_000));
    }

    #[test]
    fn bg_state_last_seen_advances_on_newer_event() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys1 = nostr::Keys::generate();
        let keys2 = nostr::Keys::generate();
        let event1 = make_test_event(&keys1, 1_700_000);
        let event2 = make_test_event(&keys2, 1_800_000);
        state.record_event(channel_id, &event1);
        state.record_event(channel_id, &event2);
        assert_eq!(state.last_seen.get(&channel_id).copied(), Some(1_800_000));
    }

    #[test]
    fn bg_state_last_seen_does_not_regress_on_older_event() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys1 = nostr::Keys::generate();
        let keys2 = nostr::Keys::generate();
        let event_new = make_test_event(&keys1, 1_800_000);
        let event_old = make_test_event(&keys2, 1_700_000);
        state.record_event(channel_id, &event_new);
        state.record_event(channel_id, &event_old);
        // last_seen should remain at the higher timestamp
        assert_eq!(state.last_seen.get(&channel_id).copied(), Some(1_800_000));
    }

    #[test]
    fn bg_state_last_seen_independent_per_channel() {
        let mut state = BgState::new();
        let ch1 = Uuid::new_v4();
        let ch2 = Uuid::new_v4();
        let keys1 = nostr::Keys::generate();
        let keys2 = nostr::Keys::generate();
        let event1 = make_test_event(&keys1, 1_000_000);
        let event2 = make_test_event(&keys2, 2_000_000);
        state.record_event(ch1, &event1);
        state.record_event(ch2, &event2);
        assert_eq!(state.last_seen.get(&ch1).copied(), Some(1_000_000));
        assert_eq!(state.last_seen.get(&ch2).copied(), Some(2_000_000));
    }

    /// Two-generation dedup: no amnesia window on rotation.
    ///
    /// The old implementation cleared the entire set at 12_001, creating a gap
    /// where all previously-seen IDs became eligible again. The new TwoGenDedup
    /// rotates at SEEN_ID_LIMIT/2 = 6_000, keeping the previous generation so
    /// IDs from both generations are still recognised as duplicates.
    #[test]
    fn bg_state_two_gen_dedup_no_amnesia_on_rotation() {
        let mut dedup = TwoGenDedup::new(SEEN_ID_LIMIT);

        // Fill current generation to the rotation threshold (limit/2 = 6_000).
        // After inserting the 6_000th item, current rotates into previous.
        let mut ids: Vec<String> = Vec::new();
        for i in 0u64..6_000 {
            let id = format!("{:0>64x}", i);
            ids.push(id.clone());
            dedup.insert(id);
        }

        // All 6_000 IDs were rotated into `previous`. `current` is now empty.
        // They must still be recognised as duplicates.
        for id in &ids {
            assert!(
                dedup.contains(id),
                "rotated ID {id} should still be a duplicate"
            );
        }

        // New IDs after rotation must be accepted.
        let new_id = format!("{:0>64x}", 99_999u64);
        assert!(
            dedup.insert(new_id.clone()),
            "new ID after rotation should be accepted"
        );
        assert!(
            dedup.contains(&new_id),
            "new ID should be found after insert"
        );
    }

    #[test]
    fn bg_state_two_gen_dedup_duplicate_rejected_across_generations() {
        let mut dedup = TwoGenDedup::new(12);
        // limit/2 = 6, so rotation happens at 6 inserts.
        for i in 0u64..6 {
            dedup.insert(format!("id-{i}"));
        }
        // id-0 is now in `previous` (rotated). Inserting it again must return false.
        assert!(
            !dedup.insert("id-0".to_string()),
            "cross-generation duplicate must be rejected"
        );
    }

    #[test]
    fn bg_state_seen_ids_cleared_at_limit() {
        // Compatibility test: BgState.record_event still deduplicates correctly
        // after the TwoGenDedup rotation threshold is crossed.
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        // Insert SEEN_ID_LIMIT/2 synthetic IDs to trigger the first rotation.
        for i in 0u64..(SEEN_ID_LIMIT as u64 / 2) {
            state.seen_ids.insert(format!("{:0>64x}", i));
        }

        // The first generation has been rotated into `previous`. All IDs are
        // still present across the two generations — no amnesia window.
        assert!(
            state
                .seen_ids
                .contains("0000000000000000000000000000000000000000000000000000000000000000"),
            "first ID should still be recognised after rotation"
        );

        // A new real event should be accepted (not a duplicate).
        let keys = nostr::Keys::generate();
        let event = make_test_event(&keys, 1_000_000);
        assert!(
            state.record_event(channel_id, &event),
            "new event after rotation should be accepted"
        );

        // The same event must be rejected as a duplicate.
        assert!(
            !state.record_event(channel_id, &event),
            "duplicate event after rotation should be rejected"
        );
    }

    /// Test 8: channel_dropped_since records the OLDEST dropped timestamp.
    ///
    /// Simulates the backpressure path directly on BgState:
    /// - First drop at ts=1000 → entry is 1000
    /// - Second drop at ts=2000 (later) → entry stays 1000 (min)
    /// - Third drop at ts=500 (earlier) → entry updates to 500 (min)
    #[test]
    fn acp_records_channel_dropped_since_on_backpressure() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        // Simulate the backpressure path: record ts=1000.
        let ts1: u64 = 1_000;
        state
            .channel_dropped_since
            .entry(channel_id)
            .and_modify(|d| *d = (*d).min(ts1))
            .or_insert(ts1);
        assert_eq!(
            state.channel_dropped_since.get(&channel_id).copied(),
            Some(1_000),
            "first drop should record ts=1000"
        );

        // Later timestamp (2000) — entry should stay at 1000.
        let ts2: u64 = 2_000;
        state
            .channel_dropped_since
            .entry(channel_id)
            .and_modify(|d| *d = (*d).min(ts2))
            .or_insert(ts2);
        assert_eq!(
            state.channel_dropped_since.get(&channel_id).copied(),
            Some(1_000),
            "later drop should not overwrite earlier timestamp"
        );

        // Earlier timestamp (500) — entry should update to 500.
        let ts3: u64 = 500;
        state
            .channel_dropped_since
            .entry(channel_id)
            .and_modify(|d| *d = (*d).min(ts3))
            .or_insert(ts3);
        assert_eq!(
            state.channel_dropped_since.get(&channel_id).copied(),
            Some(500),
            "earlier drop should update entry to 500"
        );
    }

    /// Test 9: reconnect since filter = min(last_seen, channel_dropped_since) - SINCE_SKEW_SECS.
    ///
    /// With last_seen=1000 and channel_dropped_since=900, the effective since
    /// passed to send_subscribe should be min(1000, 900) - SINCE_SKEW_SECS = 895.
    #[test]
    fn acp_reconnect_uses_dropped_since_for_replay() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        // Set up state: last_seen=1000, channel_dropped_since=900.
        state.last_seen.insert(channel_id, 1_000);
        state.channel_dropped_since.insert(channel_id, 900);

        // Compute the since value the reconnect path would use.
        let since = state.channel_since(&channel_id);

        // The since passed to send_subscribe (which subtracts SINCE_SKEW_SECS internally).
        assert_eq!(since, Some(900), "since should be min(1000, 900) = 900");

        // After subtracting skew (as send_subscribe does), the REQ filter value is:
        let req_since = since.unwrap().saturating_sub(SINCE_SKEW_SECS);
        assert_eq!(
            req_since, 895,
            "REQ since filter should be 900 - {} = 895",
            SINCE_SKEW_SECS
        );

        // Simulate clearing after resubscribe.
        state.channel_dropped_since.remove(&channel_id);
        assert!(
            !state.channel_dropped_since.contains_key(&channel_id),
            "channel_dropped_since should be cleared after resubscribe"
        );
    }

    #[test]
    fn dynamic_subscribe_records_membership_replay_floor() {
        let mut state = BgState::new();
        state.startup_watermark = Some(2_000);
        let channel_id = Uuid::new_v4();
        let membership_ts = 10_000;
        let filter = ChannelFilter {
            kinds: Some(vec![9]),
            require_mention: true,
        };

        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter,
                replay_since: Some(membership_ts),
            },
        );

        assert_eq!(
            state.subscribe_since.get(&channel_id).copied(),
            Some(membership_ts),
            "dynamic channel subscriptions should replay from the membership notification, not startup"
        );
        assert_eq!(
            state.channel_since(&channel_id),
            Some(membership_ts),
            "channel_since should use the dynamic replay floor until an event is seen"
        );
    }

    /// Membership dedup must NOT contaminate per-channel `last_seen`.
    /// Using `record_event()` for membership notifications would update
    /// `last_seen[channel_uuid]`, causing channel resubscribe to use a
    /// membership timestamp as the `since` filter — skipping channel events.
    /// The fix uses `seen_ids.insert()` directly.
    #[test]
    fn membership_dedup_does_not_touch_last_seen() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys = nostr::Keys::generate();

        // Simulate: a channel event sets last_seen to 1000.
        let event1 = make_test_event(&keys, 1_000);
        assert!(state.record_event(channel_id, &event1));
        assert_eq!(state.last_seen.get(&channel_id).copied(), Some(1_000));

        // Simulate: a membership notification for the same channel at ts=2000.
        // This should go through seen_ids only, NOT update last_seen.
        let membership_event = make_test_event(&keys, 2_000);
        let membership_id = membership_event.id.to_hex();
        assert!(
            state.seen_ids.insert(membership_id),
            "membership event should be accepted by dedup"
        );
        // last_seen must still be 1000, not 2000.
        assert_eq!(
            state.last_seen.get(&channel_id).copied(),
            Some(1_000),
            "membership dedup must not contaminate last_seen"
        );
    }

    /// On membership backpressure (TrySendError::Full), the dedup ID must
    /// be removed from seen_ids so reconnect replay can re-deliver the event.
    /// Without this, a dropped membership notification would be permanently
    /// rejected as a duplicate on replay.
    #[test]
    fn membership_backpressure_removes_dedup_id() {
        let mut state = BgState::new();
        let keys = nostr::Keys::generate();

        let event = make_test_event(&keys, 1_000);
        let event_id_hex = event.id.to_hex();

        // Insert into dedup (simulating the pre-try_send path).
        assert!(state.seen_ids.insert(event_id_hex.clone()));
        assert!(state.seen_ids.contains(&event_id_hex));

        // Simulate backpressure: remove the ID (matching the production code).
        state.seen_ids.remove(&event_id_hex);

        // The ID should now be accepted again on replay.
        assert!(
            state.seen_ids.insert(event_id_hex),
            "after backpressure removal, replay must be accepted"
        );
    }

    /// Subscribe a channel via the production command path so the test exercises
    /// real subscription state (active_subscriptions + active_filters + since).
    fn subscribe_channel(state: &mut BgState, channel_id: Uuid) {
        apply_command_to_state(
            state,
            RelayCommand::Subscribe {
                channel_id,
                filter: ChannelFilter {
                    kinds: Some(vec![9]),
                    require_mention: false,
                },
                replay_since: Some(1_000),
            },
        );
    }

    #[test]
    fn not_a_channel_member_drops_channel_without_reconnect() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        subscribe_channel(&mut state, channel_id);

        let handled = drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "restricted: not a channel member",
        );

        assert!(handled, "per-channel denial must be handled (no reconnect)");
        assert!(
            !state.active_subscriptions.contains_key(&channel_id),
            "the forbidden channel's subscription must be dropped"
        );
        assert!(
            !state.active_filters.contains_key(&channel_id),
            "channel state must be cleared (Unsubscribe cleanup)"
        );
    }

    #[test]
    fn channel_access_revoked_drops_channel_without_reconnect() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        subscribe_channel(&mut state, channel_id);

        let handled = drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "restricted: channel access revoked",
        );

        assert!(handled, "per-channel denial must be handled (no reconnect)");
        assert!(!state.active_subscriptions.contains_key(&channel_id));
        assert!(!state.active_filters.contains_key(&channel_id));
    }

    #[test]
    fn insufficient_scope_is_not_dropped_and_reconnects() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        subscribe_channel(&mut state, channel_id);

        let handled = drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "restricted: insufficient scope",
        );

        assert!(
            !handled,
            "connection-level insufficient-scope must fall through to reconnect, not drop the channel"
        );
        assert!(
            state.active_subscriptions.contains_key(&channel_id),
            "the channel must survive so reconnect can restore it"
        );
    }

    #[test]
    fn auth_required_is_not_dropped_and_reconnects() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        subscribe_channel(&mut state, channel_id);

        let handled = drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "auth-required: not authenticated",
        );

        assert!(
            !handled,
            "auth-required must fall through to reconnect, not drop the channel"
        );
        assert!(state.active_subscriptions.contains_key(&channel_id));
    }

    #[test]
    fn already_removed_channel_is_a_no_op() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        // Channel was never subscribed (or already dropped) — a delayed CLOSED.

        let handled = drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "restricted: not a channel member",
        );

        assert!(
            handled,
            "an exact per-channel denial is still handled (keep socket) even if the channel is gone"
        );
        assert!(
            !state.active_subscriptions.contains_key(&channel_id),
            "no-op: nothing to remove and nothing resurrected"
        );
    }

    #[test]
    fn dropped_channel_is_not_resubscribed_so_loop_cannot_re_form() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        subscribe_channel(&mut state, channel_id);

        drop_channel_on_access_denied(
            &mut state,
            &channel_sub_id(channel_id),
            "restricted: not a channel member",
        );

        // Simulate a reconnect: only channels still in active_subscriptions are
        // restored. The dropped channel must not be among them — otherwise the
        // forbidden channel would be resubscribed and earn the same CLOSED again.
        let resubscribed: Vec<Uuid> = state.active_subscriptions.keys().copied().collect();
        assert!(
            !resubscribed.contains(&channel_id),
            "the dropped channel must not be resubscribed — the loop cannot re-form"
        );
    }
}
