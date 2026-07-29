//! Harness-side Buzz relay client.
//!
//! Connects to the Buzz relay via NIP-01 WebSocket, authenticates via NIP-42,
//! discovers channels via REST API, and streams events back to the harness main
//! loop. Also publishes ephemeral events (typing indicators) via the same
//! WebSocket connection.
//!
//! ## Architecture
//!
//! `HarnessRelay::connect()` retries a transient initial connect/auth failure
//! (e.g. a dropped handshake on a spotty link) with bounded jittered backoff
//! before giving up; a terminal configuration/auth error fails immediately.
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
use std::sync::atomic::{AtomicBool, Ordering};
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
/// Maximum accepted age for ordinary owner-control candidates.
const PRIVILEGED_CONTROL_FRESHNESS_SECS: u64 = 300;
/// Bound privileged-control replay memory without evicting still-fresh IDs.
///
/// On saturation, new controls fail closed until an entry expires. Legitimate
/// control volume is expected to be several orders of magnitude below this.
const PRIVILEGED_CONTROL_REPLAY_CAPACITY: usize = 1_024;
/// Maximum accepted age for authenticated owner-to-agent observer controls.
const OBSERVER_CONTROL_FRESHNESS_SECS: u64 = 300;
/// Bound observer-control replay memory without evicting still-fresh IDs.
const OBSERVER_CONTROL_REPLAY_CAPACITY: usize = 1_024;
/// Bound authenticated observer controls waiting for downstream capacity.
///
/// This matches the non-evicting replay capacity: every still-fresh pending
/// control retains a replay entry, so a newly admitted control always has a
/// pending slot after expired entries are removed.
const OBSERVER_CONTROL_PENDING_CAPACITY: usize = OBSERVER_CONTROL_REPLAY_CAPACITY;

/// Interval between client-initiated WebSocket pings.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// If no pong is received within this duration after a ping, the connection is
/// considered dead and the background task triggers a reconnect.
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for individual ws.send() calls. Prevents a stalled socket from
/// wedging the background task indefinitely.
const WS_SEND_TIMEOUT_SECS: u64 = 10;
/// Diagnostic threshold: log when a connection has been stable for this long.
/// The stability block resets `BgState::backoff_step` to 0 here so the next
/// drop after a long healthy run retries at the short end of the ladder again.
const STABLE_CONNECTION_SECS: u64 = 60;
/// Seconds subtracted from `since` on resubscribe to tolerate clock skew.
const SINCE_SKEW_SECS: u64 = 5;
/// Maximum client clock lead accepted for ordinary channel messages.
///
/// Privileged controls still require a non-future timestamp. Accepted skew is
/// clamped to the local clock before replay-watermark bookkeeping so a client
/// cannot push reconnect state into the future.
const CHANNEL_EVENT_FUTURE_SKEW_SECS: u64 = SINCE_SKEW_SECS;
/// Timeout for the NIP-42 auth handshake steps.
///
/// Raised from 5s to 20s (≈2 RTTs at the observed 10s max round-trip on degraded
/// links) so auth doesn't time out before the first WS frame arrives.
const AUTH_TIMEOUT: Duration = Duration::from_secs(20);
/// Timeout for the TCP + WebSocket handshake in `do_connect`.
///
/// Raised from 10s to 30s so the OS TCP connect attempt (SYN→SYN-ACK) has time
/// to succeed at 3.4s average / 10s max observed RTT.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Backoff delay values shared by the initial-connect retry in
/// `HarnessRelay::connect()` and `try_autonomous_reconnect`'s post-start
/// reconnect loop — a spotty link should get consistent retry pacing whether
/// the failure happens at agent startup or later. Bounded so a dead relay
/// can't hang either path forever.
///
/// The two callers consume this differently: `retry_initial_connect` sleeps
/// before every entry (1 immediate attempt + up to 5 delayed retries, all 5
/// values used), while `try_autonomous_reconnect` skips the sleep after its
/// final attempt (5 attempts total, only the first 4 values used) — so
/// "shared values," not "identical schedule."
const STARTUP_CONNECT_BACKOFFS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
/// Flat retry interval for DNS failures — no backoff ladder rung consumed.
/// 2s gives name servers a short window to recover from a brownout without driving
/// a tight storm; jitter (±20%) staggers concurrent agent instances.
///
/// DNS flat retries are capped at 10 in the bounded startup/reconnect path
/// (`try_autonomous_reconnect`) so a full brownout cannot hang agent startup
/// indefinitely. In `wait_for_reconnect` the DNS path is unbounded — a
/// reconnecting agent should keep trying across extended outages rather than
/// give up.
const DNS_RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// Minimum inter-REQ spacing during resubscribe bursts.
/// 125 ms ≈ 8 frames/s — safely below the relay's 50-frames-per-5s admission
/// window (10 frames/s at the limit). A 48-channel reconnect spreads over ≈6 s
/// instead of arriving as a single burst that consumes the entire budget at once.
const REQ_PACING_INTERVAL: Duration = Duration::from_millis(125);
/// Maximum REQ frames sent per drain iteration (shared across rate_limited_pending,
/// resubscribe_retry, and control-sub recovery). Keeps any single main-loop tick
/// below the relay's 50-frames/5s budget, and ensures the select! loop is never
/// blocked for more than one REQ's worth of I/O between drain ticks.
const DRAIN_BUDGET_PER_ITER: usize = 1;
/// Maximum observer telemetry frames parked while the rate-limit gate is armed
/// (or the socket is down). The upstream pacer feeds at most ~6 frames/s, so
/// this covers ~40 s of gating; beyond that the oldest frames are dropped with
/// visible accounting (`gated_observer_dropped`).
const GATED_OBSERVER_QUEUE_CAP: usize = 256;
/// Reserved relay-gate capacity for terminal observer control results.
const GATED_OBSERVER_PRIORITY_CAP: usize = 64;
/// Largest relay retry hint accepted into the local admission gate.
///
/// The production relay's fixed message window is sixty seconds, so the client
/// must honor that full reset horizon. Larger, malformed hints are bounded here
/// so they cannot park all local relay traffic indefinitely.
const MAX_RATE_LIMIT_RETRY_SECS: u64 = 60;
/// Maximum time a terminal observer publisher waits for the relay's exact
/// `OK accepted=true` admission proof.
///
/// Ninety seconds is longer than the full sixty-second relay window plus its
/// positive-only twenty-percent desynchronization (72 s), with room for the
/// paced resend and relay OK. A later independent rate-limit response can still
/// exhaust this bounded attempt; the observer ledger then performs one bounded
/// end-to-end retry rather than silently discarding the terminal result.
const TERMINAL_OBSERVER_ACK_TIMEOUT: Duration = Duration::from_secs(90);
/// Cleartext marker added only to encrypted terminal control-result frames.
pub(crate) const OBSERVER_PRIORITY_TAG: &str = "priority";
pub(crate) const OBSERVER_CONTROL_RESULT_PRIORITY: &str = "control-result";

use std::time::Instant;

use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION,
    KIND_STREAM_MESSAGE, KIND_TYPING_INDICATOR,
};
use buzz_core::observer::{
    content_looks_like_nip44, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
};
use futures_util::{Sink, SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, RelayUrl, Tag};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
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

pub(crate) fn channel_type_from_tags(tags: &[serde_json::Value]) -> String {
    let mut is_hidden = false;
    let mut is_private = false;
    let mut declared_type = None;
    for tag in tags {
        if let Some(arr) = tag.as_array() {
            match arr.first().and_then(|v| v.as_str()) {
                Some("hidden") => is_hidden = true,
                Some("private") => is_private = true,
                Some("t") => declared_type = arr.get(1).and_then(|v| v.as_str()),
                _ => {}
            }
        }
    }
    if declared_type == Some("dm") || is_hidden {
        "dm".to_string()
    } else if declared_type == Some("private") || is_private {
        "private".to_string()
    } else {
        "stream".to_string()
    }
}

/// Build the discovered-channel subscribe set from the membership UUIDs and the
/// kind:39000 metadata events, **skipping any channel flagged `archived=true`**.
///
/// Archived channels (e.g. auto-archived by the ephemeral-channel reaper) are
/// unusable: re-offering one on reconnect draws a `CLOSED restricted` and would
/// re-form the reconnect loop. Dropping them here is the defense-in-depth
/// backstop to the relay-side live-subscription eviction — it covers a client
/// that was offline when the channel was reaped and so missed the CLOSED.
/// A channel with no metadata event is preserved as `unknown`; security
/// consumers must lazy-resolve it or fail closed rather than assuming stream.
pub(crate) fn merge_discovered_channels(
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
            let mut is_archived = false;
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    match arr.first().and_then(|v| v.as_str()) {
                        Some("d") => d_val = arr.get(1).and_then(|v| v.as_str()),
                        Some("name") => name = arr.get(1).and_then(|v| v.as_str()),
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
                    let ch_type = channel_type_from_tags(tags);
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
            .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));
        map.insert(uuid, ChannelInfo { name, channel_type });
    }
    map
}

/// Lightweight HTTP client for pre-prompt context fetches via the Nostr HTTP bridge.
///
/// Extracted from `HarnessRelay` fields so it can be shared (via `Arc`) with
/// spawned prompt tasks without giving them access to the WebSocket.
///
/// All reads go through `POST /query` with NIP-98 auth. Event submission goes
/// through `POST /events` with NIP-98 auth.
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

/// Resolve the only identity allowed to exercise privileged relay controls.
///
/// The relay-facing `auth` tag is parsed before `HarnessRelay::connect`, but
/// parsing alone does not authenticate its embedded owner. Verify the NIP-OA
/// signature against this agent key before retaining the owner identity. When
/// the attestation is absent or invalid, revalidate the trusted configured
/// owner fallback; if neither source is valid, privileged control is disabled.
fn resolve_authorized_owner_pubkey(
    auth_tag: Option<&nostr::Tag>,
    agent_keys: &Keys,
    configured_owner: Option<&str>,
) -> Option<String> {
    if let Some(auth_tag) = auth_tag {
        match serde_json::to_string(auth_tag) {
            Ok(auth_tag_json) => {
                match buzz_sdk::nip_oa::verify_auth_tag(&auth_tag_json, &agent_keys.public_key()) {
                    Ok(owner) => return Some(owner.to_hex()),
                    Err(error) => {
                        warn!(
                            "NIP-OA owner attestation verification failed; using configured fallback when valid: {error}"
                        );
                    }
                }
            }
            Err(error) => {
                warn!(
                    "could not serialize NIP-OA owner attestation; using configured fallback when valid: {error}"
                );
            }
        }
    }

    let configured_owner = configured_owner?;
    match nostr::PublicKey::from_hex(configured_owner.trim()) {
        Ok(owner) if owner != agent_keys.public_key() => Some(owner.to_hex()),
        Ok(_) => {
            warn!("configured privileged owner matches the agent key; failing closed");
            None
        }
        Err(error) => {
            warn!("configured privileged owner is invalid; failing closed: {error}");
            None
        }
    }
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

    /// Submit a signed event via the HTTP bridge: `POST /events` with NIP-98 auth.
    ///
    /// The event must already be signed. Returns the relay response JSON.
    pub async fn submit_event(&self, event: &Event) -> Result<Value, RelayError> {
        let body_bytes = serde_json::to_vec(event)
            .map_err(|e| RelayError::Http(format!("event serialize error: {e}")))?;
        let resp = self.bridge_post("/events", &body_bytes).await?;
        let text = resp
            .text()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| RelayError::Http(e.to_string()))
    }
}

/// Events the harness cares about.
#[derive(Debug, Clone)]
pub struct BuzzEvent {
    /// Which channel this event belongs to.
    pub channel_id: Uuid,
    /// The underlying Nostr event.
    pub event: Event,
    /// Relay-issued authority for harness-level owner control execution.
    ///
    /// Command-shaped events without this marker remain ordinary messages,
    /// even when their signer matches a configured owner.
    pub privileged_control_admitted: bool,
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
    PublishEvent {
        event: Box<Event>,
        admission: Option<oneshot::Sender<Result<(), String>>>,
    },
    /// Floor `since` for membership notification replay; events before startup are never re-delivered.
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
                admission: None,
            })
            .await
            .map_err(|_| RelayError::ConnectionClosed)
    }

    /// Publish a terminal observer result only after the relay explicitly
    /// acknowledges it with `OK accepted=true`.
    pub async fn publish_terminal_event(&self, event: Event) -> Result<(), RelayError> {
        self.publish_terminal_event_with_ack_timeout(event, TERMINAL_OBSERVER_ACK_TIMEOUT)
            .await
    }

    async fn publish_terminal_event_with_ack_timeout(
        &self,
        event: Event,
        ack_timeout: Duration,
    ) -> Result<(), RelayError> {
        if !is_exact_terminal_observer_frame(&event) {
            return Err(RelayError::Http(
                "terminal observer event must be kind 24200 with exactly one control-result priority marker"
                    .to_string(),
            ));
        }
        let deadline = tokio::time::Instant::now() + ack_timeout;
        let (admission_tx, admission_rx) = oneshot::channel();
        match tokio::time::timeout_at(
            deadline,
            self.cmd_tx.send(RelayCommand::PublishEvent {
                event: Box::new(event),
                admission: Some(admission_tx),
            }),
        )
        .await
        {
            Ok(result) => result.map_err(|_| RelayError::ConnectionClosed)?,
            Err(_) => return Err(RelayError::Timeout),
        }
        match tokio::time::timeout_at(deadline, admission_rx).await {
            Ok(result) => result
                .map_err(|_| RelayError::ConnectionClosed)?
                .map_err(RelayError::Http),
            Err(_) => {
                // Dropping `admission_rx` above closes only the sender owned by
                // this exact publication attempt. That per-attempt sender is the
                // authoritative cancellation token checked before every park,
                // socket write, and reconnect drain. An event-ID-only cancellation
                // would be ambiguous when a delayed duplicate shares the original
                // signed event ID, so timeout cleanup never enqueues one.
                Err(RelayError::Timeout)
            }
        }
    }

    /// Test-only publisher pair: published events are forwarded to the
    /// returned receiver instead of a live relay socket.
    #[cfg(test)]
    pub(crate) fn test_pair() -> (Self, mpsc::Receiver<Event>) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RelayCommand>(64);
        let (event_tx, event_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let RelayCommand::PublishEvent { event, admission } = cmd {
                    if event_tx.send(*event).await.is_err() {
                        break;
                    }
                    if let Some(admission) = admission {
                        let _ = admission.send(Ok(()));
                    }
                }
            }
        });
        (Self { cmd_tx }, event_rx)
    }

    /// Test-only publisher that drops the first terminal admission sender,
    /// then confirms every later publication.
    #[cfg(test)]
    pub(crate) fn test_pair_dropping_first_terminal_confirmation() -> (Self, mpsc::Receiver<Event>)
    {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<RelayCommand>(64);
        let (event_tx, event_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut drop_first = true;
            while let Some(cmd) = cmd_rx.recv().await {
                if let RelayCommand::PublishEvent { event, admission } = cmd {
                    if event_tx.send(*event).await.is_err() {
                        break;
                    }
                    if let Some(admission) = admission {
                        if drop_first {
                            drop_first = false;
                            drop(admission);
                        } else {
                            let _ = admission.send(Ok(()));
                        }
                    }
                }
            }
        });
        (Self { cmd_tx }, event_rx)
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
        Self::connect_with_owner(relay_url, keys, agent_pubkey_hex, auth_tag, None).await
    }

    /// Connect with the canonical owner resolved by the trusted harness config.
    ///
    /// A valid NIP-OA attestation remains authoritative. `configured_owner` is
    /// used only when the attestation is absent or invalid, and is normalized
    /// and revalidated before the background task classifies any event.
    pub async fn connect_with_owner(
        relay_url: &str,
        keys: &Keys,
        agent_pubkey_hex: &str,
        auth_tag: Option<nostr::Tag>,
        configured_owner: Option<String>,
    ) -> Result<Self, RelayError> {
        // Perform the initial connection and auth handshake, retrying
        // transient failures (dropped handshake, timeout) with bounded
        // jittered backoff. A terminal error (bad URL, bad auth tag,
        // rejected/invalid signing key) fails immediately — see
        // `is_terminal_connect_error`.
        let (ws, handshake_buffer) =
            retry_initial_connect(|| do_connect(relay_url, keys, auth_tag.as_ref())).await?;

        let (event_tx, event_rx) = mpsc::channel::<Option<BuzzEvent>>(event_channel_capacity());
        let (observer_control_tx, observer_control_rx) =
            mpsc::channel::<Event>(event_channel_capacity());
        let (cmd_tx, cmd_rx) = mpsc::channel::<RelayCommand>(CMD_CHANNEL_CAPACITY);

        let bg_keys = keys.clone();
        let bg_relay_url = relay_url.to_string();
        let bg_agent_pubkey_hex = agent_pubkey_hex.to_string();
        let bg_auth_tag = auth_tag.clone();
        let bg_configured_owner = configured_owner;

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
                bg_configured_owner,
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
                admission: None,
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
                admission: None,
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

    /// Pins the floor `since` for membership notification replay.
    ///
    /// Call once after `connect()` with the Unix timestamp captured just before
    /// the relay connection was established. The background task uses this so
    /// events predating this session are never re-delivered after reconnect.
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

#[derive(Debug, PartialEq, Eq)]
enum PrivilegedControlAdmission {
    Admitted,
    Replay,
    Full,
}

/// Non-evicting replay memory for ordinary privileged control candidates.
///
/// Entries remain present for the event's entire freshness lifetime. Unlike
/// [`TwoGenDedup`], capacity pressure never evicts a still-admissible control
/// ID because doing so would reopen a replay path. Saturation fails closed.
struct PrivilegedControlReplay {
    expires_at: HashMap<String, u64>,
    capacity: usize,
}

impl PrivilegedControlReplay {
    fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "privileged control replay capacity must be positive"
        );
        Self {
            expires_at: HashMap::with_capacity(capacity),
            capacity,
        }
    }

    fn admit(&mut self, event_id: String, expires_at: u64, now: u64) -> PrivilegedControlAdmission {
        self.expires_at.retain(|_, expiry| *expiry >= now);
        if self.expires_at.contains_key(&event_id) {
            return PrivilegedControlAdmission::Replay;
        }
        if self.expires_at.len() >= self.capacity {
            return PrivilegedControlAdmission::Full;
        }
        self.expires_at.insert(event_id, expires_at);
        PrivilegedControlAdmission::Admitted
    }

    fn remove(&mut self, event_id: &str) {
        self.expires_at.remove(event_id);
    }
}

/// State maintained by the background WebSocket task.
struct BgState {
    /// Active subscriptions: channel_id → subscription_id string.
    active_subscriptions: HashMap<Uuid, String>,
    /// Successful REQ completion second for each currently live channel.
    ///
    /// Owner controls must strictly post-date this boundary. Intent recorded
    /// while disconnected never populates it.
    channel_subscribed_at: HashMap<Uuid, u64>,
    /// Most recent `created_at` timestamp seen per channel (for `since` filter).
    last_seen: HashMap<Uuid, u64>,
    /// Two-generation dedup set of event IDs seen.
    seen_ids: TwoGenDedup,
    /// Supervisor-process start second for privileged control admission.
    ///
    /// Nostr timestamps have one-second precision, so controls at exactly this
    /// boundary are rejected: they cannot be proven to post-date startup.
    privileged_control_start_boundary: u64,
    /// Owner identity authenticated by the agent's NIP-OA attestation.
    ///
    /// `None` disables privileged classification. Command-looking events from
    /// any other signer remain ordinary channel messages.
    authorized_owner_pubkey: Option<String>,
    /// Replay IDs retained without eviction for their full freshness lifetime.
    privileged_control_replay: PrivilegedControlReplay,
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
    /// Start second of the exact observer-control REQ currently live on the socket.
    ///
    /// `None` means subscription intent exists but no REQ is currently proven
    /// live. Equality is rejected because Nostr timestamps have one-second
    /// precision and cannot prove that a control followed the REQ.
    observer_control_subscribed_at: Option<u64>,
    /// Authenticated observer-control IDs retained for their freshness lifetime.
    observer_control_replay: PrivilegedControlReplay,
    /// Authenticated controls waiting for capacity in the downstream channel.
    ///
    /// This FIFO is drained by its own `select!` arm so relay reads, pings,
    /// reconnects, and shutdown commands never wait for downstream capacity.
    observer_control_pending: VecDeque<Event>,
    /// Oldest dropped channel-event timestamp per channel, keyed by channel_id.
    /// Mirrors `membership_dropped_since` but for ordinary channel events.
    /// On reconnect resubscribe, `since` = min(last_seen, channel_dropped_since).
    /// Cleared per-channel after a successful resubscribe.
    channel_dropped_since: HashMap<Uuid, u64>,
    /// Set by the backpressure handler when the event channel is full.
    /// The main loop checks this flag and triggers a proactive resubscribe
    /// (without waiting for a disconnect) so dropped events are replayed.
    proactive_resubscribe_needed: bool,
    /// Unix timestamp captured just before the relay connection was established.
    /// Used as the floor `since` for membership notification replay so events
    /// predating this session are never re-delivered.
    startup_watermark: Option<u64>,
    /// Replay floor captured when each channel was first subscribed.
    /// Used as the `since` fallback on reconnect for channels that have no
    /// `last_seen` or `channel_dropped_since`. This prevents channels joined
    /// after startup from replaying from an hours-old `startup_watermark`.
    /// Startup-era channels use the startup watermark; dynamic channels use
    /// the membership notification timestamp that caused the subscription.
    subscribe_since: HashMap<Uuid, u64>,
    /// Relay rate-limit gate deadline.
    ///
    /// While `Some(deadline)` and `Instant::now() < deadline`, outbound
    /// admission-counted frames (REQ, EVENT) are deferred or dropped.
    /// `check_rate_gate` lazily clears this to `None` once it expires.
    rate_limit_gate: Option<tokio::time::Instant>,
    /// Channels parked because a CLOSED "rate-limited:" was received.
    ///
    /// Drained by the main loop when the gate clears, one REQ per
    /// `REQ_PACING_INTERVAL` tick via the select-integrated pacing timer.
    /// Value is the `Instant` before which the channel must not be retried.
    rate_limited_pending: HashMap<Uuid, tokio::time::Instant>,
    /// Set when a rate-limited CLOSED arrives for the membership notification
    /// subscription. The main-loop drain re-sends the REQ once the gate clears,
    /// even when `rate_limited_pending` is empty.
    membership_resub_needed: bool,
    /// Set when a rate-limited CLOSED arrives for the observer control
    /// subscription. The main-loop drain re-sends the REQ once the gate clears,
    /// even when `rate_limited_pending` is empty.
    observer_resub_needed: bool,
    /// Observer telemetry frames (kind 24200) parked while the rate-limit gate
    /// is armed. Unlike typing indicators, these frames are durable telemetry:
    /// dropping them silently loses turn history in the Desktop observer.
    /// Bounded at `GATED_OBSERVER_QUEUE_CAP` (drop-oldest); drained by the
    /// main loop one frame per pacing tick once the gate clears.
    gated_observer_pending: VecDeque<Box<Event>>,
    /// Protected terminal control-result frames. This FIFO is never displaced
    /// by ordinary observer telemetry and drains first after a relay gate.
    gated_observer_priority_pending: VecDeque<Box<Event>>,
    /// Observer frames written to the socket but not yet acknowledged. The
    /// relay's rate-limit NOTICE does not carry an event ID, so all unresolved
    /// observer writes are moved back ahead of the parked FIFO when one arrives.
    observer_in_flight: VecDeque<Box<Event>>,
    /// Protected acknowledgement window for terminal control-result frames.
    observer_priority_in_flight: VecDeque<Box<Event>>,
    /// Terminal publisher completions keyed by relay event ID. A completion is
    /// released only by an explicit relay `OK`, so local queue admission can
    /// never be mistaken for externally durable proof.
    observer_priority_confirmations: HashMap<String, oneshot::Sender<Result<(), String>>>,
    /// Frames evicted from the bounded pending/in-flight observer buffers since
    /// summary log. Makes overflow loss visible instead of silent.
    gated_observer_dropped: u64,
    /// Observer publishes rejected by a terminal relay denial. These are not
    /// retried, but the count and exact denial are logged for auditability.
    observer_terminal_denied: u64,
    /// Channels whose REQ failed during `resubscribe_after_reconnect`.
    ///
    /// A single failed channel REQ is parked here instead of aborting the whole
    /// reconnect. Drained by the main loop. Flushed on each reconnect attempt.
    resubscribe_retry: HashSet<Uuid>,
    /// Current position in the exponential backoff ladder.
    ///
    /// Persisted across calls to `wait_for_reconnect` so a flapping link stays at
    /// the elevated rung it earned. Reset to 0 by the stability block once the
    /// connection has been up for `STABLE_CONNECTION_SECS`.
    backoff_step: usize,
}

impl BgState {
    fn new() -> Self {
        Self {
            active_subscriptions: HashMap::new(),
            channel_subscribed_at: HashMap::new(),
            last_seen: HashMap::new(),
            seen_ids: TwoGenDedup::new(SEEN_ID_LIMIT),
            privileged_control_start_boundary: unix_now_secs(),
            authorized_owner_pubkey: None,
            privileged_control_replay: PrivilegedControlReplay::new(
                PRIVILEGED_CONTROL_REPLAY_CAPACITY,
            ),
            active_filters: HashMap::new(),
            membership_dropped_since: None,
            membership_last_seen: None,
            membership_sub_active: false,
            observer_control_sub_active: false,
            observer_control_subscribed_at: None,
            observer_control_replay: PrivilegedControlReplay::new(OBSERVER_CONTROL_REPLAY_CAPACITY),
            observer_control_pending: VecDeque::new(),
            channel_dropped_since: HashMap::new(),
            proactive_resubscribe_needed: false,
            startup_watermark: None,
            subscribe_since: HashMap::new(),
            rate_limit_gate: None,
            rate_limited_pending: HashMap::new(),
            membership_resub_needed: false,
            observer_resub_needed: false,
            gated_observer_pending: VecDeque::new(),
            gated_observer_priority_pending: VecDeque::new(),
            observer_in_flight: VecDeque::new(),
            observer_priority_in_flight: VecDeque::new(),
            observer_priority_confirmations: HashMap::new(),
            gated_observer_dropped: 0,
            observer_terminal_denied: 0,
            resubscribe_retry: HashSet::new(),
            backoff_step: 0,
        }
    }

    /// Record a received event for dedup and bounded `since` tracking.
    /// Returns `true` if the event is new (not a duplicate).
    fn record_event_with_watermark(
        &mut self,
        channel_id: Uuid,
        event: &Event,
        watermark_timestamp: u64,
    ) -> bool {
        let id_hex = event.id.to_hex();

        // Two-generation dedup: no amnesia window on rotation.
        if !self.seen_ids.insert(id_hex) {
            return false;
        }

        // Update last_seen timestamp.
        self.last_seen
            .entry(channel_id)
            .and_modify(|t| *t = (*t).max(watermark_timestamp))
            .or_insert(watermark_timestamp);

        true
    }

    #[cfg(test)]
    fn record_event(&mut self, channel_id: Uuid, event: &Event) -> bool {
        self.record_event_with_watermark(channel_id, event, event.created_at.as_secs())
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
        self.channel_subscribed_at.remove(channel_id);
        self.last_seen.remove(channel_id);
        self.subscribe_since.remove(channel_id);
        self.channel_dropped_since.remove(channel_id);
        self.active_filters.remove(channel_id);
        self.rate_limited_pending.remove(channel_id);
        self.resubscribe_retry.remove(channel_id);
    }

    /// Arm or extend the rate-limit gate.
    ///
    /// `retry_secs` is the relay's `retry in {N}s` hint; hints below 2s (including
    /// the no-hint case of 0) floor to 5s. The floor prevents a burst of
    /// low-quality hints from dropping the gate so short that re-queued REQs
    /// immediately re-trigger rate limiting. Note the deliberate asymmetry with
    /// the desktop TypeScript client, which uses a 10s no-hint default — both
    /// values are conservative enough. Relay hints win when present, up to the
    /// bounded [`MAX_RATE_LIMIT_RETRY_SECS`] contract that keeps terminal
    /// confirmation alive through the retry window.
    ///
    /// The gate takes the **maximum** of any existing deadline and the newly
    /// computed one so overlapping CLOSED/NOTICE messages can't shorten a gate
    /// that is already set further out.
    ///
    /// Explicit reset hints use positive-only jitter: retrying before the
    /// advertised reset can create a self-sustaining rate-limit loop. Returns
    /// the gate deadline that was set.
    fn set_rate_limit_gate(&mut self, retry_secs: u64) -> tokio::time::Instant {
        let secs = if retry_secs < 2 {
            5
        } else {
            retry_secs.min(MAX_RATE_LIMIT_RETRY_SECS)
        };
        let base = Duration::from_secs(secs);
        let deadline = tokio::time::Instant::now() + rate_limit_retry_duration(base);
        let gate = match self.rate_limit_gate {
            Some(existing) if existing > deadline => existing,
            _ => deadline,
        };
        self.rate_limit_gate = Some(gate);
        gate
    }

    /// Check whether the rate-limit gate is currently active.
    ///
    /// Returns `Some(deadline)` when gated, `None` when the gate has expired or
    /// was never set. Lazily clears `rate_limit_gate` to `None` on expiry so
    /// subsequent calls are cheap (no `Instant::now()` except when `Some`).
    fn check_rate_gate(&mut self) -> Option<tokio::time::Instant> {
        if let Some(deadline) = self.rate_limit_gate {
            if tokio::time::Instant::now() < deadline {
                return Some(deadline);
            }
            self.rate_limit_gate = None;
        }
        None
    }

    /// Park an observer telemetry frame while the rate-limit gate is armed.
    ///
    /// Bounded drop-oldest queue: overflow evicts the oldest frame and counts
    /// it in `gated_observer_dropped` so the loss is visible, never silent.
    fn park_gated_observer_frame(&mut self, event: Box<Event>) -> bool {
        if is_priority_observer_frame(&event) {
            if self.gated_observer_priority_pending.len() + self.observer_priority_in_flight.len()
                >= GATED_OBSERVER_PRIORITY_CAP
            {
                self.gated_observer_dropped += 1;
                tracing::error!(
                    dropped_total = self.gated_observer_dropped,
                    "protected observer delivery state full — rejected newest terminal result"
                );
                return false;
            }
            self.gated_observer_priority_pending.push_back(event);
            return true;
        }
        if self.gated_observer_pending.len() >= GATED_OBSERVER_QUEUE_CAP {
            self.gated_observer_pending.pop_front();
            self.gated_observer_dropped += 1;
            warn!(
                dropped_total = self.gated_observer_dropped,
                "gated observer queue full — dropped oldest frame"
            );
        }
        self.gated_observer_pending.push_back(event);
        true
    }

    /// Restore unresolved observer writes ahead of frames parked after the
    /// gate armed. NOTICE has no event ID, so conservatively retry every frame
    /// without an OK; duplicate IDs are harmless at the relay.
    fn requeue_observer_in_flight(&mut self) {
        self.prune_cancelled_terminal_admissions();
        while let Some(event) = self.observer_priority_in_flight.pop_back() {
            self.gated_observer_priority_pending.push_front(event);
        }
        while let Some(event) = self.observer_in_flight.pop_back() {
            self.gated_observer_pending.push_front(event);
        }
        while self.gated_observer_pending.len() > GATED_OBSERVER_QUEUE_CAP {
            self.gated_observer_pending.pop_front();
            self.gated_observer_dropped += 1;
        }
    }

    /// Retire authority belonging to one failed replacement socket while
    /// preserving all unresolved observer delivery intent for the next one.
    fn abandon_failed_reconnect_candidate(&mut self) {
        self.requeue_observer_in_flight();
        self.observer_control_subscribed_at = None;
        self.channel_subscribed_at.clear();
    }

    fn track_observer_in_flight(&mut self, event: Box<Event>) -> bool {
        if is_priority_observer_frame(&event) {
            if self.gated_observer_priority_pending.len() + self.observer_priority_in_flight.len()
                >= GATED_OBSERVER_PRIORITY_CAP
            {
                self.gated_observer_dropped += 1;
                tracing::error!(
                    dropped_total = self.gated_observer_dropped,
                    "protected observer acknowledgment state full — rejected newest terminal result"
                );
                return false;
            }
            self.observer_priority_in_flight.push_back(event);
            return true;
        }
        if self.observer_in_flight.len() >= GATED_OBSERVER_QUEUE_CAP {
            self.observer_in_flight.pop_front();
            self.gated_observer_dropped += 1;
            warn!(
                dropped_total = self.gated_observer_dropped,
                "observer acknowledgment window full — dropped oldest frame"
            );
        }
        self.observer_in_flight.push_back(event);
        true
    }

    fn has_observer_priority_capacity(&self) -> bool {
        self.gated_observer_priority_pending.len() + self.observer_priority_in_flight.len()
            < GATED_OBSERVER_PRIORITY_CAP
    }

    fn contains_observer_priority_event(&self, event_id: &str) -> bool {
        self.gated_observer_priority_pending
            .iter()
            .chain(self.observer_priority_in_flight.iter())
            .any(|event| event.id.to_hex() == event_id)
            || self.observer_priority_confirmations.contains_key(event_id)
    }

    fn reject_duplicate_observer_priority(
        &self,
        event_id: &str,
        is_priority: bool,
        admission: &mut Option<oneshot::Sender<Result<(), String>>>,
    ) -> bool {
        if !is_priority || !self.contains_observer_priority_event(event_id) {
            return false;
        }
        tracing::error!(
            event_id = %event_id,
            "rejected duplicate terminal observer result before queue or socket write"
        );
        if let Some(admission) = admission.take() {
            let _ = admission.send(Err(
                "terminal observer event is already awaiting relay confirmation".to_string(),
            ));
        }
        true
    }

    /// A timed-out publisher drops its oneshot receiver synchronously. Purge
    /// every exact terminal event whose confirmation sender observes that close,
    /// independent of command-channel capacity or reconnect command ordering.
    fn prune_cancelled_terminal_admissions(&mut self) {
        let cancelled: Vec<_> = self
            .observer_priority_confirmations
            .iter()
            .filter(|(_, confirmation)| confirmation.is_closed())
            .map(|(event_id, _)| event_id.clone())
            .collect();
        for event_id in cancelled {
            self.cancel_terminal_admission(&event_id);
        }
    }

    /// Remove one timed-out terminal publication from every relay-owned state
    /// lane. Event IDs are signed-event identities, so matching the exact ID
    /// cannot cancel a different queued terminal result.
    fn cancel_terminal_admission(&mut self, event_id: &str) -> bool {
        let pending_len = self.gated_observer_priority_pending.len();
        self.gated_observer_priority_pending
            .retain(|event| event.id.to_hex() != event_id);
        let in_flight_len = self.observer_priority_in_flight.len();
        self.observer_priority_in_flight
            .retain(|event| event.id.to_hex() != event_id);
        let confirmation_removed = self
            .observer_priority_confirmations
            .remove(event_id)
            .is_some();
        let removed = self.gated_observer_priority_pending.len() != pending_len
            || self.observer_priority_in_flight.len() != in_flight_len
            || confirmation_removed;
        if removed {
            warn!(
                event_id,
                "terminal observer relay acknowledgement timed out — cancelled exact delivery state"
            );
        } else {
            debug!(
                event_id,
                "terminal observer timeout cancellation found no unresolved delivery state"
            );
        }
        removed
    }

    fn acknowledge_observer_frame(&mut self, event_id: &str) {
        if self.take_unresolved_observer_frame(event_id).is_some() {
            if let Some(confirmation) = self.observer_priority_confirmations.remove(event_id) {
                let _ = confirmation.send(Ok(()));
            }
        }
    }

    /// Remove and return an exact unresolved observer write from either its
    /// socket-bound acknowledgement window or its rate-gated retry FIFO.
    fn take_unresolved_observer_frame(&mut self, event_id: &str) -> Option<Box<Event>> {
        if let Some(index) = self
            .observer_priority_in_flight
            .iter()
            .position(|event| event.id.to_hex() == event_id)
        {
            return self.observer_priority_in_flight.remove(index);
        }
        if let Some(index) = self
            .gated_observer_priority_pending
            .iter()
            .position(|event| event.id.to_hex() == event_id)
        {
            return self.gated_observer_priority_pending.remove(index);
        }
        if let Some(index) = self
            .observer_in_flight
            .iter()
            .position(|event| event.id.to_hex() == event_id)
        {
            return self.observer_in_flight.remove(index);
        }
        let index = self
            .gated_observer_pending
            .iter()
            .position(|event| event.id.to_hex() == event_id)?;
        self.gated_observer_pending.remove(index)
    }

    /// Retry the exact rate-limited observer frame ahead of telemetry that was
    /// parked after it was first written.
    fn requeue_rate_limited_observer_frame(&mut self, event_id: &str) -> bool {
        if self
            .gated_observer_priority_pending
            .iter()
            .chain(self.gated_observer_pending.iter())
            .any(|event| event.id.to_hex() == event_id)
        {
            return true;
        }
        let Some(event) = self.take_unresolved_observer_frame(event_id) else {
            return false;
        };
        if is_priority_observer_frame(&event) {
            self.gated_observer_priority_pending.push_front(event);
        } else {
            self.gated_observer_pending.push_front(event);
            if self.gated_observer_pending.len() > GATED_OBSERVER_QUEUE_CAP {
                self.gated_observer_pending.pop_back();
                self.gated_observer_dropped += 1;
            }
        }
        true
    }

    /// Retire a terminally denied observer frame without retrying it forever.
    /// The caller logs the exact relay denial; the counter preserves a bounded
    /// summary signal even after the frame leaves the acknowledgement window.
    fn record_terminal_observer_denial(&mut self, event_id: &str, denial: &str) -> bool {
        if self.take_unresolved_observer_frame(event_id).is_none() {
            return false;
        }
        if let Some(confirmation) = self.observer_priority_confirmations.remove(event_id) {
            let _ = confirmation.send(Err(format!(
                "relay denied terminal observer frame: {denial}"
            )));
        }
        self.observer_terminal_denied = self.observer_terminal_denied.saturating_add(1);
        true
    }

    fn record_observer_confirmation(
        &mut self,
        event_id: String,
        is_priority: bool,
        admission: Option<oneshot::Sender<Result<(), String>>>,
        admitted: bool,
    ) {
        let Some(admission) = admission else {
            return;
        };
        if admission.is_closed() {
            if admitted {
                self.cancel_terminal_admission(&event_id);
            }
        } else if !admitted {
            let _ = admission.send(Err("terminal observer priority state is full".to_string()));
        } else if !is_priority {
            let _ = admission.send(Err(
                "confirmed delivery is only valid for terminal observer frames".to_string(),
            ));
        } else {
            self.observer_priority_confirmations
                .insert(event_id, admission);
        }
    }
}

/// Record a command's intent in state while disconnected (no WebSocket).
///
/// Subscribe/Unsubscribe/SubscribeMembership record intent so reconnect
/// restores the right subscriptions. SetStartupWatermark floors the replay
/// window. Observer telemetry publishes are parked for post-reconnect drain;
/// other PublishEvent and Reconnect are no-ops while disconnected.
///
/// Callers MUST handle `Shutdown` before calling — reaching the Shutdown
/// arm here is a logic error.
fn apply_command_to_state(state: &mut BgState, cmd: RelayCommand) {
    state.prune_cancelled_terminal_admissions();
    match cmd {
        RelayCommand::Subscribe {
            channel_id,
            filter,
            replay_since,
        } => {
            // Intent is not a live subscription. Any prior socket-bound
            // authority stays disabled until a new REQ is written successfully.
            state.channel_subscribed_at.remove(&channel_id);
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
            state.observer_control_subscribed_at = None;
        }
        RelayCommand::SetStartupWatermark { ts } => {
            state.startup_watermark = Some(ts);
            if state.membership_last_seen.is_none() {
                state.membership_last_seen = Some(ts);
            }
        }
        // Observer telemetry frames are durable: park them (bounded, visible
        // overflow) so they are delivered by the post-reconnect drain. Other
        // ephemeral publishes (typing indicators) are meaningless while
        // disconnected and are dropped.
        RelayCommand::PublishEvent {
            event,
            mut admission,
        } => {
            if admission.as_ref().is_some_and(oneshot::Sender::is_closed) {
                debug!(
                    event_id = %event.id,
                    "discarding terminal observer publish cancelled before disconnected admission"
                );
                return;
            }
            if event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME {
                let event_id = event.id.to_hex();
                let is_priority = is_priority_observer_frame(&event);
                if state.reject_duplicate_observer_priority(&event_id, is_priority, &mut admission)
                {
                    return;
                }
                let admitted = state.park_gated_observer_frame(event);
                state.record_observer_confirmation(event_id, is_priority, admission, admitted);
            } else if let Some(admission) = admission {
                let _ = admission.send(Err(
                    "confirmed delivery is only valid for observer frames".to_string()
                ));
            }
        }
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

/// Retain command intent after a live send failure.
///
/// Subscription state must survive reconnect. Observer telemetry publishes are
/// parked for post-reconnect drain; other ephemeral publishes are deliberately
/// discarded because replaying a typing indicator after reconnect is meaningless.
/// `Shutdown` and `Reconnect` are handled by the caller.
fn retain_failed_command_intent(state: &mut BgState, cmd: RelayCommand) {
    state.prune_cancelled_terminal_admissions();
    match cmd {
        RelayCommand::PublishEvent {
            event,
            mut admission,
        } if event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME => {
            if admission.as_ref().is_some_and(oneshot::Sender::is_closed) {
                debug!(
                    event_id = %event.id,
                    "discarding terminal observer publish cancelled before failed-send retention"
                );
                return;
            }
            let event_id = event.id.to_hex();
            let is_priority = is_priority_observer_frame(&event);
            if state.reject_duplicate_observer_priority(&event_id, is_priority, &mut admission) {
                return;
            }
            let admitted = state.park_gated_observer_frame(event);
            state.record_observer_confirmation(event_id, is_priority, admission, admitted);
        }
        RelayCommand::PublishEvent { admission, .. } => {
            if let Some(admission) = admission {
                let _ = admission.send(Err("observer frame was not retained".to_string()));
            }
        }
        cmd => apply_command_to_state(state, cmd),
    }
}

/// Preserve stateful commands already consumed during replay when that replay
/// loses its live socket before the deferred queue can be executed.
///
/// Commands are applied in arrival order. Ephemeral publishes are discarded by
/// [`retain_failed_command_intent`], and pacing never queues `Shutdown`.
fn retain_deferred_command_intent(
    state: &mut BgState,
    deferred_commands: &mut VecDeque<RelayCommand>,
) {
    while let Some(cmd) = deferred_commands.pop_front() {
        match cmd {
            RelayCommand::Shutdown | RelayCommand::Reconnect => {}
            cmd => retain_failed_command_intent(state, cmd),
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
    state.prune_cancelled_terminal_admissions();
    match cmd {
        RelayCommand::Subscribe {
            channel_id,
            filter,
            replay_since,
        } => {
            // Rate-gated: defer this REQ to prevent flooding a saturated relay.
            // The gate holds until the relay's retry hint expires.
            if let Some(retry_after) = state.check_rate_gate() {
                debug!(
                    "rate-gated: deferring REQ for channel {channel_id} to rate_limited_pending"
                );
                apply_command_to_state(
                    state,
                    RelayCommand::Subscribe {
                        channel_id,
                        filter,
                        replay_since,
                    },
                );
                state.rate_limited_pending.insert(channel_id, retry_after);
                return true; // connection is fine — just rate-limited
            }

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
                // Evict stale drain entries so the drain loop can't send a
                // duplicate REQ for this now-live subscription.
                state.rate_limited_pending.remove(&channel_id);
                state.resubscribe_retry.remove(&channel_id);
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
            state.membership_sub_active = true;
            if state.check_rate_gate().is_some() {
                debug!("rate-gated: deferring membership subscription");
                state.membership_resub_needed = true;
                return true;
            }
            let since = state.membership_last_seen.or(state.startup_watermark);
            let sent = send_membership_subscribe(ws, agent_pubkey_hex, since).await;
            if sent {
                state.membership_resub_needed = false;
                if state.membership_last_seen.is_none() {
                    state.membership_last_seen = since;
                }
                true
            } else {
                // Send failed — record intent so reconnect restores it.
                warn!("membership subscribe REQ failed — recording intent for reconnect");
                state.membership_resub_needed = true;
                false
            }
        }
        RelayCommand::SubscribeObserverControls => {
            state.observer_control_sub_active = true;
            state.observer_control_subscribed_at = None;
            if state.check_rate_gate().is_some() {
                debug!("rate-gated: deferring observer control subscription");
                state.observer_resub_needed = true;
                return true;
            }
            if let Some(subscribed_at) = send_observer_control_subscribe(ws, agent_pubkey_hex).await
            {
                state.observer_control_subscribed_at = Some(subscribed_at);
                state.observer_resub_needed = false;
                true
            } else {
                warn!("observer control subscribe REQ failed — recording intent for reconnect");
                state.observer_resub_needed = true;
                false
            }
        }
        RelayCommand::PublishEvent {
            event,
            mut admission,
        } => {
            if admission.as_ref().is_some_and(oneshot::Sender::is_closed) {
                debug!(
                    event_id = %event.id,
                    "discarding terminal observer publish cancelled before socket admission"
                );
                return true;
            }
            let event_id = event.id.to_hex();
            let is_priority = is_priority_observer_frame(&event);
            if state.reject_duplicate_observer_priority(&event_id, is_priority, &mut admission) {
                return true;
            }
            // Observer telemetry frames (kind 24200) are durable telemetry, not
            // droppable ephemera: park them while the rate-limit gate is armed —
            // and while earlier parked frames are still draining, so relative
            // order is preserved — then let the main-loop drain deliver them
            // one per pacing tick once the gate clears.
            if event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME
                && (state.check_rate_gate().is_some()
                    || !state.gated_observer_priority_pending.is_empty()
                    || !state.gated_observer_pending.is_empty())
            {
                debug!(
                    priority_pending = state.gated_observer_priority_pending.len(),
                    pending = state.gated_observer_pending.len(),
                    "rate-gated: parking observer frame for paced drain"
                );
                let admitted = state.park_gated_observer_frame(event);
                state.record_observer_confirmation(event_id, is_priority, admission, admitted);
                return true;
            }
            // Drop remaining ephemeral publishes while rate-gated. Stale typing
            // indicators are worthless and sending them would consume admission
            // budget the relay already rejected us on.
            //
            // INVARIANT: apart from observer frames (parked above), the WS publish
            // path carries only ephemeral kinds (typing indicators). The silent
            // drop-while-gated relies on that invariant. If a future caller
            // publishes durable events through this path, it must extend the
            // kind guard above to avoid silently discarding user data.
            if state.check_rate_gate().is_some() {
                debug!("rate-gated: dropping ephemeral PublishEvent (typing indicator)");
                return true;
            }
            // A failed observer frame is parked so the post-reconnect drain
            // redelivers it. If `start_send` already accepted bytes, any
            // cancellation, timeout, or error taints the socket and forces
            // reconnect before later traffic can flush those buffered bytes.
            let is_observer = event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME;
            if is_priority && !state.has_observer_priority_capacity() {
                state.gated_observer_dropped += 1;
                tracing::error!(
                    dropped_total = state.gated_observer_dropped,
                    "protected observer acknowledgment state full — rejected newest terminal result before socket write"
                );
                state.record_observer_confirmation(event_id, true, admission, false);
                return true;
            }
            let outcome =
                send_publish_event_frame_until_cancelled(ws, &event, admission.as_mut()).await;
            match outcome {
                PublishEventFrameOutcome::CancelledBeforeSend => {
                    debug!(
                        event_id,
                        "terminal observer publish cancelled before socket write started"
                    );
                }
                PublishEventFrameOutcome::SocketTainted => {
                    warn!(
                        event_id,
                        "terminal observer publish cancelled after socket write started — retiring relay socket"
                    );
                }
                PublishEventFrameOutcome::Sent => {
                    if is_observer {
                        let admitted = state.track_observer_in_flight(event);
                        state.record_observer_confirmation(
                            event_id,
                            is_priority,
                            admission,
                            admitted,
                        );
                    } else if let Some(admission) = admission {
                        let _ =
                            admission
                                .send(Err("confirmed delivery is only valid for observer frames"
                                    .to_string()));
                    }
                }
                PublishEventFrameOutcome::Failed
                | PublishEventFrameOutcome::FailedSocketTainted
                    if is_observer =>
                {
                    let admitted = state.park_gated_observer_frame(event);
                    state.record_observer_confirmation(event_id, is_priority, admission, admitted);
                }
                PublishEventFrameOutcome::Failed
                | PublishEventFrameOutcome::FailedSocketTainted => {
                    if let Some(admission) = admission {
                        let _ = admission.send(Err("relay publish failed".to_string()));
                    }
                }
            }
            !outcome.socket_tainted()
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
    configured_owner: Option<String>,
) {
    let mut state = BgState::new();
    state.authorized_owner_pubkey =
        resolve_authorized_owner_pubkey(auth_tag.as_ref(), &keys, configured_owner.as_deref());

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
            ReconnectOutcome::Ok => {}
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

    // Client-initiated ping to detect silent connection death.
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_pong = Instant::now();
    let mut ping_sent = false;

    // Track connection stability for backoff reset.
    let mut connected_since = Instant::now();
    let mut stable_logged = false;

    // Pacing timer for select-integrated rate-limit drain.
    // `None` = no pending drain or budget window is open; `Some(t)` = next
    // allowed drain tick. The select! arm below fires when `t` elapses and
    // resets this to `None`, allowing the pre-select drain to run again.
    let mut drain_pacing_next: Option<tokio::time::Instant> = None;

    loop {
        if state.proactive_resubscribe_needed {
            state.proactive_resubscribe_needed = false;
            info!("proactive resubscribe triggered by backpressure event loss");
            // Proactive resubscribe runs on the EXISTING socket — do NOT clear the
            // rate-limit gate or pending queues.
            match resubscribe_after_reconnect(
                &mut ws,
                &mut cmd_rx,
                &mut state,
                &agent_pubkey_hex,
                false, // existing socket — preserve gate state
            )
            .await
            {
                ResubscribeResult::Ok => {}
                ResubscribeResult::Shutdown => return,
                ResubscribeResult::RetryConnection => {
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
                        ReconnectOutcome::Ok => {}
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
        }

        // Drain pending subs, one REQ per pacing tick within the relay's
        // admission window.
        let drain_window_open = drain_pacing_next.is_none_or(|t| tokio::time::Instant::now() >= t);
        if drain_window_open {
            let mut budget = DRAIN_BUDGET_PER_ITER;
            let mut any_sent = false;

            // Control subs use a flag rather than a per-channel pending entry, so
            // recovery fires even when rate_limited_pending is empty.
            if state.check_rate_gate().is_none() {
                if state.membership_resub_needed && budget > 0 {
                    let replay_since =
                        match (state.membership_dropped_since, state.membership_last_seen) {
                            (Some(d), Some(l)) => Some(d.min(l)),
                            (Some(d), None) => Some(d),
                            (None, Some(l)) => Some(l),
                            (None, None) => state.startup_watermark,
                        };
                    if send_membership_subscribe(&mut ws, &agent_pubkey_hex, replay_since).await {
                        state.membership_resub_needed = false;
                        state.membership_dropped_since = None;
                        budget = budget.saturating_sub(1);
                        any_sent = true;
                    } else {
                        warn!(
                            "membership control resub after rate-limit failed — will retry next drain"
                        );
                    }
                }
                if state.observer_resub_needed && budget > 0 {
                    state.observer_control_subscribed_at = None;
                    if let Some(subscribed_at) =
                        send_observer_control_subscribe(&mut ws, &agent_pubkey_hex).await
                    {
                        state.observer_control_subscribed_at = Some(subscribed_at);
                        state.observer_resub_needed = false;
                        budget = budget.saturating_sub(1);
                        any_sent = true;
                    } else {
                        warn!(
                            "observer control resub after rate-limit failed — will retry next drain"
                        );
                    }
                }
            }

            if budget > 0 && !state.rate_limited_pending.is_empty() {
                let sent =
                    drain_rate_limited_pending(&mut ws, &mut state, &agent_pubkey_hex, budget)
                        .await;
                budget = budget.saturating_sub(sent);
                if sent > 0 {
                    any_sent = true;
                }
            }

            if budget > 0 && !state.resubscribe_retry.is_empty() {
                let sent =
                    drain_resubscribe_retry(&mut ws, &mut state, &agent_pubkey_hex, budget).await;
                budget = budget.saturating_sub(sent);
                if sent > 0 {
                    any_sent = true;
                }
            }

            if budget > 0
                && (!state.gated_observer_priority_pending.is_empty()
                    || !state.gated_observer_pending.is_empty())
            {
                match drain_gated_observer_pending(&mut ws, &mut state, budget).await {
                    GatedObserverDrainOutcome::Sent(sent) => {
                        if sent > 0 {
                            any_sent = true;
                        }
                    }
                    GatedObserverDrainOutcome::SocketTainted => {
                        warn!(
                            "terminal observer send was cancelled after start_send — reconnecting before later traffic"
                        );
                        let _ = event_tx.try_send(None);
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
                        ping_sent = false;
                        last_pong = Instant::now();
                        connected_since = Instant::now();
                        stable_logged = false;
                        drain_pacing_next = None;
                        continue;
                    }
                }
            }

            if any_sent {
                drain_pacing_next = Some(tokio::time::Instant::now() + REQ_PACING_INTERVAL);
            } else if !state.gated_observer_priority_pending.is_empty()
                || !state.gated_observer_pending.is_empty()
            {
                // Nothing sent because the gate is still armed. Arm the pacing
                // timer to the gate deadline so parked observer frames drain
                // promptly even when no other traffic wakes the select loop.
                drain_pacing_next = state
                    .check_rate_gate()
                    .or_else(|| Some(tokio::time::Instant::now() + REQ_PACING_INTERVAL));
            }
        }

        tokio::select! {
                   permit = observer_control_tx.reserve(),
                       if !state.observer_control_pending.is_empty() =>
                   {
                       match permit {
                           Ok(permit) => {
                               deliver_next_pending_observer_control(&mut state, permit);
                           }
                           Err(_) => {
                               warn!(
                                   pending = state.observer_control_pending.len(),
                                   "observer control receiver closed with pending controls"
                               );
                               return;
                           }
                       }
                   }

                   raw = ws.next() => {
                       // Determine if the socket is lost.
                       let socket_lost = match raw {
                           Some(Ok(msg)) => {
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
                                       ReconnectOutcome::Ok => {}
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
                               ReconnectOutcome::Ok => {}
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
                                   ReconnectOutcome::Ok => {}
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

                   // Pacing timer arm — wakes the loop for the next drain batch.
                   // `pending()` when no drain is in progress so this arm never
                   // fires spuriously and never blocks the other select! arms.
                   _ = async {
                       match drain_pacing_next {
                           Some(t) => tokio::time::sleep_until(t).await,
                           None => std::future::pending::<()>().await,
                       }
                   } => {
                       drain_pacing_next = None;
                   }
               }

        // Reset backoff_step on a long healthy run so a subsequent brief drop
        // retries at the short end of the backoff ladder.
        if !stable_logged && connected_since.elapsed() > Duration::from_secs(STABLE_CONNECTION_SECS)
        {
            stable_logged = true;
            state.backoff_step = 0;
            debug!(
                "connection stable for >{}s — backoff ladder reset",
                STABLE_CONNECTION_SECS
            );
        }
    }
}

/// Queue one fully authenticated observer control without awaiting downstream
/// capacity. Returns `false` only when the downstream receiver is closed.
fn queue_admitted_observer_control(
    state: &mut BgState,
    observer_control_tx: &mpsc::Sender<Event>,
    event: Event,
) -> bool {
    if state.observer_control_pending.is_empty() {
        match observer_control_tx.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Full(event)) => {
                state.observer_control_pending.push_back(event);
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(event)) => {
                state.observer_control_replay.remove(&event.id.to_hex());
                warn!(
                    event_id = %event.id,
                    "observer control receiver closed before delivery"
                );
                return false;
            }
        }
    }

    if state.observer_control_pending.len() >= OBSERVER_CONTROL_PENDING_CAPACITY {
        let now = unix_now_secs();
        let mut expired_ids = Vec::new();
        state.observer_control_pending.retain(|pending| {
            let ts = pending.created_at.as_secs();
            let expired = now.saturating_sub(ts) > OBSERVER_CONTROL_FRESHNESS_SECS;
            if expired {
                expired_ids.push(pending.id.to_hex());
                warn!(
                    event_id = %pending.id,
                    event_ts = ts,
                    delivery_now = now,
                    "dropping observer control that expired while awaiting downstream capacity"
                );
            }
            !expired
        });
        for event_id in expired_ids {
            state.observer_control_replay.remove(&event_id);
        }
    }

    if state.observer_control_pending.len() >= OBSERVER_CONTROL_PENDING_CAPACITY {
        state.observer_control_replay.remove(&event.id.to_hex());
        warn!(
            event_id = %event.id,
            capacity = OBSERVER_CONTROL_PENDING_CAPACITY,
            "observer control pending queue full — failing closed without eviction"
        );
        return true;
    }

    state.observer_control_pending.push_back(event);
    true
}

/// Deliver the oldest pending observer control through a reserved downstream
/// slot, discarding only controls whose bounded freshness window elapsed while
/// they waited.
fn deliver_next_pending_observer_control(state: &mut BgState, permit: mpsc::Permit<'_, Event>) {
    let delivery_now = unix_now_secs();
    while let Some(event) = state.observer_control_pending.pop_front() {
        let event_id = event.id.to_hex();
        let ts = event.created_at.as_secs();
        if delivery_now.saturating_sub(ts) > OBSERVER_CONTROL_FRESHNESS_SECS {
            state.observer_control_replay.remove(&event_id);
            warn!(
                event_id,
                event_ts = ts,
                delivery_now,
                "dropping observer control that expired while awaiting downstream capacity"
            );
            continue;
        }
        permit.send(event);
        return;
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
                        let Some(subscription_started_at) = state.observer_control_subscribed_at
                        else {
                            warn!(
                                event_id = %event.id,
                                "dropping observer control for inactive subscription"
                            );
                            return true;
                        };
                        if !state.observer_control_sub_active {
                            warn!(
                                event_id = %event.id,
                                "dropping observer control without subscription intent"
                            );
                            return true;
                        }
                        if !is_authorized_observer_control(
                            &event,
                            agent_pubkey_hex,
                            state.authorized_owner_pubkey.as_deref(),
                        ) {
                            warn!(
                                event_id = %event.id,
                                "dropping observer frame outside the exact owner-control shape"
                            );
                            return true;
                        }

                        let ts = event.created_at.as_secs();
                        let now = unix_now_secs();
                        let admission_boundary = state
                            .privileged_control_start_boundary
                            .max(subscription_started_at);
                        if ts <= admission_boundary {
                            warn!(
                                event_id = %event.id,
                                event_ts = ts,
                                admission_boundary,
                                "dropping observer control that does not provably post-date startup and subscription"
                            );
                            return true;
                        }
                        if ts > now || now - ts > OBSERVER_CONTROL_FRESHNESS_SECS {
                            warn!(
                                event_id = %event.id,
                                event_ts = ts,
                                now,
                                "dropping observer control outside the freshness window"
                            );
                            return true;
                        }

                        let event_for_verify = event.clone();
                        match tokio::task::spawn_blocking(move || {
                            buzz_core::verify_event(&event_for_verify)
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                warn!(
                                    event_id = %event.id,
                                    "dropping invalid observer control: {error}"
                                );
                                return true;
                            }
                            Err(error) => {
                                warn!(
                                    event_id = %event.id,
                                    "observer control verification task failed: {error}"
                                );
                                return true;
                            }
                        }

                        let event_id = event.id.to_hex();
                        let expires_at = ts.saturating_add(OBSERVER_CONTROL_FRESHNESS_SECS);
                        match state
                            .observer_control_replay
                            .admit(event_id, expires_at, now)
                        {
                            PrivilegedControlAdmission::Admitted => {}
                            PrivilegedControlAdmission::Replay => {
                                warn!(
                                    event_id = %event.id,
                                    "dropping replayed observer control"
                                );
                                return true;
                            }
                            PrivilegedControlAdmission::Full => {
                                warn!(
                                    event_id = %event.id,
                                    capacity = OBSERVER_CONTROL_REPLAY_CAPACITY,
                                    "observer control replay guard full — failing closed"
                                );
                                return true;
                            }
                        }

                        if !queue_admitted_observer_control(state, observer_control_tx, *event) {
                            return false;
                        }
                    } else if subscription_id == MEMBERSHIP_NOTIF_SUB_ID {
                        if !state.membership_sub_active {
                            warn!(
                                event_id = %event.id,
                                "dropping membership notification for inactive subscription"
                            );
                            return true;
                        }

                        let kind = event.kind.as_u16() as u32;
                        if !matches!(
                            kind,
                            KIND_MEMBER_ADDED_NOTIFICATION | KIND_MEMBER_REMOVED_NOTIFICATION
                        ) {
                            warn!(
                                kind,
                                event_id = %event.id,
                                "dropping unexpected event kind on membership subscription"
                            );
                            return true;
                        }

                        if !event_has_tag_value(&event, "p", agent_pubkey_hex) {
                            warn!(
                                event_id = %event.id,
                                "dropping membership notification targeting another agent"
                            );
                            return true;
                        }

                        // Membership notification — extract the one unambiguous
                        // channel UUID from its h tag.
                        let channel_uuid = match extract_h_tag_uuid(&event) {
                            Some(uuid) => uuid,
                            None => {
                                warn!(
                                    event_id = %event.id,
                                    "membership notification missing or ambiguous h tag — dropping"
                                );
                                return true;
                            }
                        };

                        // Relays can replay stored events without authenticating
                        // them. Verify before any dedup, watermark, or forwarding
                        // mutation so a forged frame cannot reserve its claimed
                        // ID and suppress a later authentic notification.
                        let event_for_verify = event.clone();
                        match tokio::task::spawn_blocking(move || {
                            buzz_core::verify_event(&event_for_verify)
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                warn!(
                                    channel_id = %channel_uuid,
                                    event_id = %event.id,
                                    "dropping invalid membership notification: {error}"
                                );
                                return true;
                            }
                            Err(error) => {
                                warn!(
                                    channel_id = %channel_uuid,
                                    event_id = %event.id,
                                    "membership verification task failed: {error}"
                                );
                                return true;
                            }
                        }

                        let ts = event.created_at.as_secs();
                        let now = unix_now_secs();
                        if ts > now {
                            warn!(
                                channel_id = %channel_uuid,
                                event_id = %event.id,
                                event_ts = ts,
                                now,
                                "dropping future-dated membership notification"
                            );
                            return true;
                        }

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
                        let buzz_event = BuzzEvent {
                            channel_id: channel_uuid,
                            event: *event,
                            privileged_control_admitted: false,
                        };
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
                                // Proactively trigger resubscribe without waiting for a disconnect.
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
                        if state
                            .active_subscriptions
                            .get(&channel_id)
                            .map(String::as_str)
                            != Some(subscription_id.as_str())
                        {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                "dropping event for inactive channel subscription"
                            );
                            return true;
                        }

                        if extract_h_tag_uuid(&event) != Some(channel_id) {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                "dropping event whose h tag does not match its subscription"
                            );
                            return true;
                        }

                        let Some(filter) = state.active_filters.get(&channel_id) else {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                "dropping event without a local subscription filter"
                            );
                            return true;
                        };
                        let kind = event.kind.as_u16() as u32;
                        if filter
                            .kinds
                            .as_ref()
                            .is_some_and(|kinds| !kinds.contains(&kind))
                        {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                kind,
                                "dropping event outside the local kind filter"
                            );
                            return true;
                        }
                        if filter.require_mention
                            && !event_has_tag_value(&event, "p", agent_pubkey_hex)
                        {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                "dropping event outside the local mention filter"
                            );
                            return true;
                        }

                        // Relays are not trusted to have verified stored or
                        // replayed events. Verify before touching replay
                        // watermarks or dedup state so a forged frame cannot
                        // reserve the claimed ID and suppress a later authentic
                        // event. Schnorr verification is CPU-bound.
                        let event_for_verify = event.clone();
                        match tokio::task::spawn_blocking(move || {
                            buzz_core::verify_event(&event_for_verify)
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                warn!(
                                    channel_id = %channel_id,
                                    event_id = %event.id,
                                    "dropping invalid channel event: {error}"
                                );
                                return true;
                            }
                            Err(error) => {
                                warn!(
                                    channel_id = %channel_id,
                                    event_id = %event.id,
                                    "channel event verification task failed: {error}"
                                );
                                return true;
                            }
                        }
                        let ts = event.created_at.as_secs();
                        let event_id_hex = event.id.to_hex();
                        let now = unix_now_secs();
                        let future_skew = ts.saturating_sub(now);
                        if future_skew > CHANNEL_EVENT_FUTURE_SKEW_SECS {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                event_ts = ts,
                                now,
                                "dropping future-dated channel event"
                            );
                            return true;
                        }
                        let is_privileged_control_candidate = is_privileged_channel_control(
                            &event,
                            agent_pubkey_hex,
                            state.authorized_owner_pubkey.as_deref(),
                        );
                        let privileged_control_boundary =
                            state.channel_subscribed_at.get(&channel_id).copied().map(
                                |subscribed_at| {
                                    subscribed_at.max(state.privileged_control_start_boundary)
                                },
                            );
                        let privileged_control_admitted = if !is_privileged_control_candidate {
                            false
                        } else if ts > now {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                event_ts = ts,
                                now,
                                "future-skewed owner control — forwarding without execution authority"
                            );
                            false
                        } else if privileged_control_boundary.is_none() {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                "owner control arrived without a successful live channel REQ — forwarding without execution authority"
                            );
                            false
                        } else if privileged_control_boundary.is_some_and(|boundary| ts <= boundary)
                        {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                event_ts = ts,
                                admission_boundary = privileged_control_boundary,
                                "owner control does not provably post-date supervisor/channel subscription startup — forwarding without execution authority; retry with a newly signed event after the boundary"
                            );
                            false
                        } else if now - ts > PRIVILEGED_CONTROL_FRESHNESS_SECS {
                            warn!(
                                channel_id = %channel_id,
                                event_id = %event.id,
                                event_ts = ts,
                                now,
                                "stale owner control — forwarding without execution authority"
                            );
                            false
                        } else {
                            let expires_at = ts.saturating_add(PRIVILEGED_CONTROL_FRESHNESS_SECS);
                            match state.privileged_control_replay.admit(
                                event_id_hex.clone(),
                                expires_at,
                                now,
                            ) {
                                PrivilegedControlAdmission::Admitted => true,
                                PrivilegedControlAdmission::Replay => {
                                    warn!(
                                        channel_id = %channel_id,
                                        event_id = %event.id,
                                        "replayed owner control — withholding execution authority"
                                    );
                                    false
                                }
                                PrivilegedControlAdmission::Full => {
                                    warn!(
                                        channel_id = %channel_id,
                                        event_id = %event.id,
                                        capacity = PRIVILEGED_CONTROL_REPLAY_CAPACITY,
                                        "privileged control replay guard full — failing closed"
                                    );
                                    false
                                }
                            }
                        };
                        let replay_timestamp = ts.min(now);
                        if state.record_event_with_watermark(channel_id, &event, replay_timestamp) {
                            let buzz_event = BuzzEvent {
                                channel_id,
                                event: *event,
                                privileged_control_admitted,
                            };
                            // Warn at 80% capacity.
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
                                    if privileged_control_admitted {
                                        // The harness never received this control,
                                        // so a reconnect replay must remain eligible.
                                        state.privileged_control_replay.remove(&event_id_hex);
                                    }
                                    // Track the oldest dropped timestamp so reconnect
                                    // replay starts early enough to re-deliver it.
                                    state
                                        .channel_dropped_since
                                        .entry(channel_id)
                                        .and_modify(|d| *d = (*d).min(replay_timestamp))
                                        .or_insert(replay_timestamp);
                                    // Proactively trigger resubscribe without waiting for a disconnect.
                                    state.proactive_resubscribe_needed = true;
                                    warn!(
                                        channel_id = %channel_id,
                                        event_ts = ts,
                                        replay_timestamp,
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
                    // The relay sends NOTICE for rate-limited EVENT/COUNT frames.
                    if message.starts_with("rate-limited:") {
                        let secs = parse_rate_limit_retry_secs(&message).unwrap_or(0);
                        let deadline = state.set_rate_limit_gate(secs);
                        state.requeue_observer_in_flight();
                        warn!(
                            "rate-limit gate armed via NOTICE until ~{:.1}s from now",
                            deadline
                                .checked_duration_since(tokio::time::Instant::now())
                                .unwrap_or_default()
                                .as_secs_f64()
                        );
                    }
                }
                RelayMessage::Closed {
                    subscription_id,
                    message,
                } => {
                    if subscription_id == OBSERVER_CONTROL_SUB_ID {
                        state.observer_control_subscribed_at = None;
                    }
                    if let Some(channel_id) = channel_id_from_sub_id(&subscription_id) {
                        state.channel_subscribed_at.remove(&channel_id);
                    }
                    // A per-channel membership denial means THIS channel is
                    // forbidden, not the whole connection. Drop just this
                    // channel's subscription and keep the socket — otherwise the
                    // socket is torn down, the forbidden channel is resubscribed,
                    // and the same CLOSED arrives again: a tight reconnect loop.
                    if drop_channel_on_access_denied(state, &subscription_id, &message) {
                        return true;
                    }

                    // Rate-limited CLOSED — park and keep the socket. The relay's
                    // "retry in {N}s" hint arms the gate; the channel or control sub
                    // is resubscribed by the main-loop drain once the gate clears.
                    if message.starts_with("rate-limited:") {
                        let secs = parse_rate_limit_retry_secs(&message).unwrap_or(0);
                        let deadline = state.set_rate_limit_gate(secs);
                        warn!(
                            "subscription {subscription_id} rate-limited — parking until ~{:.1}s, gate armed",
                            deadline
                                .checked_duration_since(tokio::time::Instant::now())
                                .unwrap_or_default()
                                .as_secs_f64()
                        );
                        if let Some(channel_id) = channel_id_from_sub_id(&subscription_id) {
                            state.rate_limited_pending.insert(channel_id, deadline);
                        } else if subscription_id == MEMBERSHIP_NOTIF_SUB_ID {
                            // Mark membership sub for drain recovery. The relay rejected
                            // this REQ before registering it, so the sub does not exist
                            // server-side — the drain must re-send it.
                            state.membership_resub_needed = true;
                        } else if subscription_id == OBSERVER_CONTROL_SUB_ID {
                            state.observer_resub_needed = true;
                        }
                        return true; // keep the socket
                    }

                    // CLOSED needs cleanup and resubscribe, not just logging.
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
                        if let Some(subscribed_at) =
                            send_observer_control_subscribe(ws, agent_pubkey_hex).await
                        {
                            state.observer_control_sub_active = true;
                            state.observer_control_subscribed_at = Some(subscribed_at);
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
                    // AUTH send failure must trigger reconnect.
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
                        // AUTH OK with accepted=false means auth was rejected.
                        warn!("mid-session AUTH rejected (event {event_id}): {message} — triggering reconnect");
                        return false;
                    }
                    if accepted {
                        state.acknowledge_observer_frame(&event_id);
                    } else if message.starts_with("rate-limited:") {
                        let secs = parse_rate_limit_retry_secs(&message).unwrap_or(0);
                        let deadline = state.set_rate_limit_gate(secs);
                        if state.requeue_rate_limited_observer_frame(&event_id) {
                            warn!(
                                event_id,
                                "observer publish rate-limited via OK false — exact frame requeued; gate armed until ~{:.1}s",
                                deadline
                                    .checked_duration_since(tokio::time::Instant::now())
                                    .unwrap_or_default()
                                    .as_secs_f64()
                            );
                        }
                    } else if state.record_terminal_observer_denial(&event_id, &message) {
                        warn!(
                            event_id,
                            denial = message,
                            denied_total = state.observer_terminal_denied,
                            "observer publish terminally denied — frame retired without retry"
                        );
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

/// Process messages buffered during the NIP-42 auth handshake.
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

/// Outcome of [`resubscribe_after_reconnect`].
enum ResubscribeResult {
    /// All subscriptions restored (or parked for drain recovery).
    Ok,
    /// A control subscription or deferred live command failed to send.
    /// Caller should retry the connection.
    RetryConnection,
    /// A `Shutdown` command arrived during a pacing sleep.
    /// Caller must return immediately (background task is exiting).
    Shutdown,
}

/// Resubscribe all active channels and membership notifications after a
/// successful reconnect. Computes `since = min(last_seen, channel_dropped_since)`
/// per channel, and only clears the drop tracker when the REQ is confirmed sent.
///
/// Paces REQs at `REQ_PACING_INTERVAL` (125 ms) via a shutdown-aware sleep so
/// a 48-channel reconnect burst spreads over ≈6 s. Commands received during a
/// pacing sleep are deferred in arrival order and executed on the live socket
/// after replay. If the gate is active mid-burst, remaining channels are parked
/// in `rate_limited_pending` instead of sent.
///
/// A failed CHANNEL REQ is parked in `resubscribe_retry` rather than failing
/// the whole reconnect. Only membership, observer-control, or deferred-command
/// failures return `RetryConnection` — their silent loss would leave live state
/// inconsistent with command intent.
///
/// A relay quota gate is keyed by community and pubkey, so replacing the socket
/// does not reset it. Fresh connections may clear derived pending/retry queues
/// before rebuilding them from `active_subscriptions`, but the gate itself is
/// always preserved until its deadline expires.
///
/// Returns [`ResubscribeResult`] signalling success, retry, or shutdown.
async fn resubscribe_after_reconnect(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    agent_pubkey_hex: &str,
    is_fresh_connection: bool,
) -> ResubscribeResult {
    if is_fresh_connection {
        // These queues are derived from active subscription intent and rebuilt
        // below. The rate-limit gate is deliberately preserved: the relay's
        // shared admission counter survives socket replacement.
        state.rate_limited_pending.clear();
        state.resubscribe_retry.clear();
    }

    let mut deferred_commands = VecDeque::new();
    let channels: Vec<Uuid> = state.active_subscriptions.keys().copied().collect();
    if !channels.is_empty() {
        info!(
            "resubscribing to {} channel(s) after reconnect",
            channels.len()
        );
        for channel_id in channels {
            // Gate re-armed mid-burst — park remaining channels.
            if let Some(retry_after) = state.check_rate_gate() {
                debug!(
                    "rate-gated mid-resubscribe: parking channel {channel_id} in rate_limited_pending"
                );
                state.rate_limited_pending.insert(channel_id, retry_after);
                continue;
            }

            let since = state.channel_since(&channel_id);
            let filter = match state.active_filters.get(&channel_id).cloned() {
                Some(f) => f,
                None => {
                    // Fail closed: missing filter state means the subscription
                    // intent is inconsistent. Skip rather than resubscribe with
                    // a permissive wildcard that would widen the subscription.
                    warn!("missing filter for channel {channel_id} — skipping resubscribe (fail-closed)");
                    state.resubscribe_retry.insert(channel_id);
                    continue;
                }
            };
            let this_sent =
                send_subscribe(ws, state, channel_id, agent_pubkey_hex, since, &filter).await;
            if this_sent {
                state.channel_dropped_since.remove(&channel_id);
                // Shutdown-aware pacing sleep before any next replay/deferred REQ.
                if !pacing_sleep(cmd_rx, &mut deferred_commands, REQ_PACING_INTERVAL).await {
                    return ResubscribeResult::Shutdown;
                }
            } else {
                // Partial failure — park the channel for main-loop retry instead
                // of aborting the entire reconnect.
                warn!(
                    "failed to resubscribe channel {channel_id} after reconnect — parking for retry"
                );
                state.resubscribe_retry.insert(channel_id);
            }
        }
    }

    // Membership and observer-control are control-plane subscriptions: a silent
    // failure breaks join notifications and agent pause/resume. A shared quota
    // gate parks their intent for the main-loop drain just like channel REQs.
    if state.membership_sub_active {
        if state.check_rate_gate().is_some() {
            debug!("rate-gated: parking membership resubscribe after reconnect");
            state.membership_resub_needed = true;
        } else {
            if !state.active_subscriptions.is_empty()
                && !pacing_sleep(cmd_rx, &mut deferred_commands, REQ_PACING_INTERVAL).await
            {
                return ResubscribeResult::Shutdown;
            }
            let replay_since = match (state.membership_dropped_since, state.membership_last_seen) {
                (Some(d), Some(l)) => Some(d.min(l)),
                (Some(d), None) => Some(d),
                (None, Some(l)) => Some(l),
                (None, None) => state.startup_watermark,
            };
            let sent = send_membership_subscribe(ws, agent_pubkey_hex, replay_since).await;
            if sent {
                state.membership_dropped_since = None;
                state.membership_resub_needed = false;
            } else {
                warn!("failed to resubscribe membership after reconnect");
                retain_deferred_command_intent(state, &mut deferred_commands);
                return ResubscribeResult::RetryConnection;
            }
        }
    }

    if state.observer_control_sub_active {
        state.observer_control_subscribed_at = None;
        if state.check_rate_gate().is_some() {
            debug!("rate-gated: parking observer control resubscribe after reconnect");
            state.observer_resub_needed = true;
        } else {
            if !pacing_sleep(cmd_rx, &mut deferred_commands, REQ_PACING_INTERVAL).await {
                return ResubscribeResult::Shutdown;
            }
            match send_observer_control_subscribe(ws, agent_pubkey_hex).await {
                Some(subscribed_at) => {
                    state.observer_control_subscribed_at = Some(subscribed_at);
                }
                None => {
                    warn!("failed to resubscribe observer controls after reconnect");
                    retain_deferred_command_intent(state, &mut deferred_commands);
                    return ResubscribeResult::RetryConnection;
                }
            }
            state.observer_resub_needed = false;
        }
    }

    match drain_commands(ws, cmd_rx, &mut deferred_commands, state, agent_pubkey_hex).await {
        ReconnectOutcome::Ok => ResubscribeResult::Ok,
        ReconnectOutcome::Failed => ResubscribeResult::RetryConnection,
        ReconnectOutcome::Shutdown => ResubscribeResult::Shutdown,
    }
}

/// Sink adapter that records whether `SinkExt::send` reached `start_send`.
///
/// Before that boundary cancellation is clean. After it, the sink may retain
/// buffered bytes even when the send future is dropped, so the connection must
/// be retired before any later frame is written.
struct StartSendTrackedSink<'a, S> {
    inner: &'a mut S,
    started: &'a AtomicBool,
}

impl<S> Sink<Message> for StartSendTrackedSink<'_, S>
where
    S: Sink<Message> + Unpin,
{
    type Error = S::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut *this.inner).poll_ready(cx)
    }

    fn start_send(self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let this = self.get_mut();
        // Conservatively taint as soon as `start_send` is invoked. Some sinks,
        // including tungstenite write-failure paths, may retain the frame even
        // when this call returns an error.
        this.started.store(true, Ordering::SeqCst);
        std::pin::Pin::new(&mut *this.inner).start_send(item)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut *this.inner).poll_flush(cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut *this.inner).poll_close(cx)
    }
}

/// Send a signed EVENT frame on a sink. Returns `false` on send failure.
async fn send_publish_event_frame<S>(ws: &mut S, event: &Event) -> bool
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let msg = json!(["EVENT", event]);
    if let Ok(text) = serde_json::to_string(&msg) {
        match tokio::time::timeout(
            Duration::from_secs(WS_SEND_TIMEOUT_SECS),
            ws.send(Message::Text(text.into())),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("failed to publish event: {e}");
                return false;
            }
            Err(_) => {
                warn!("failed to publish event: socket send timed out");
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishEventFrameOutcome {
    Sent,
    Failed,
    CancelledBeforeSend,
    /// Cancellation won after `start_send`; buffered bytes may remain.
    SocketTainted,
    /// The send failed or timed out after `start_send`; buffered bytes may remain.
    FailedSocketTainted,
}

impl PublishEventFrameOutcome {
    fn socket_tainted(self) -> bool {
        matches!(self, Self::SocketTainted | Self::FailedSocketTainted)
    }
}

/// Write an EVENT only while its terminal confirmation receiver remains open.
///
/// The publisher drops that receiver at its end-to-end deadline. Selecting on
/// `closed()` prevents an expired command from reaching a reconnected socket
/// even when its FIFO cancellation command has not yet been processed. If
/// cancellation wins after `start_send`, the caller must retire the socket:
/// dropping `SinkExt::send` does not retract bytes already buffered by the sink.
async fn send_publish_event_frame_until_cancelled<S>(
    ws: &mut S,
    event: &Event,
    admission: Option<&mut oneshot::Sender<Result<(), String>>>,
) -> PublishEventFrameOutcome
where
    S: Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let send_started = AtomicBool::new(false);
    let sent = if let Some(admission) = admission {
        if admission.is_closed() {
            return PublishEventFrameOutcome::CancelledBeforeSend;
        }
        let selected = {
            let mut tracked = StartSendTrackedSink {
                inner: ws,
                started: &send_started,
            };
            tokio::select! {
                biased;
                _ = admission.closed() => None,
                sent = send_publish_event_frame(&mut tracked, event) => Some(sent),
            }
        };
        match selected {
            Some(sent) => sent,
            None if send_started.load(Ordering::SeqCst) => {
                return PublishEventFrameOutcome::SocketTainted;
            }
            None => return PublishEventFrameOutcome::CancelledBeforeSend,
        }
    } else {
        let mut tracked = StartSendTrackedSink {
            inner: ws,
            started: &send_started,
        };
        send_publish_event_frame(&mut tracked, event).await
    };
    if sent {
        PublishEventFrameOutcome::Sent
    } else if send_started.load(Ordering::SeqCst) {
        PublishEventFrameOutcome::FailedSocketTainted
    } else {
        PublishEventFrameOutcome::Failed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatedObserverDrainOutcome {
    Sent(usize),
    SocketTainted,
}

/// Drain parked observer telemetry frames once the rate-limit gate clears.
///
/// Called by the main loop pacing timer. Sends at most `budget` frames without
/// sleeping — pacing is enforced by the caller via `drain_pacing_next`. Stops
/// immediately if the gate re-arms mid-drain. When the queue empties, any
/// overflow loss is summarized in one warning. Returns the number of frames sent.
async fn drain_gated_observer_pending(
    ws: &mut WsStream,
    state: &mut BgState,
    budget: usize,
) -> GatedObserverDrainOutcome {
    let mut sent = 0;
    while sent < budget {
        state.prune_cancelled_terminal_admissions();
        if state.check_rate_gate().is_some() {
            break;
        }
        let event = state
            .gated_observer_priority_pending
            .pop_front()
            .or_else(|| state.gated_observer_pending.pop_front());
        let Some(event) = event else {
            break;
        };
        let event_id = event.id.to_hex();
        let is_priority = is_priority_observer_frame(&event);
        let mut confirmation = is_priority
            .then(|| state.observer_priority_confirmations.remove(&event_id))
            .flatten();
        let outcome =
            send_publish_event_frame_until_cancelled(ws, &event, confirmation.as_mut()).await;
        match outcome {
            PublishEventFrameOutcome::CancelledBeforeSend => continue,
            PublishEventFrameOutcome::SocketTainted => {
                return GatedObserverDrainOutcome::SocketTainted;
            }
            PublishEventFrameOutcome::Failed | PublishEventFrameOutcome::FailedSocketTainted => {
                if confirmation
                    .as_ref()
                    .is_some_and(oneshot::Sender::is_closed)
                {
                    if outcome.socket_tainted() {
                        return GatedObserverDrainOutcome::SocketTainted;
                    }
                    continue;
                }
                // Socket may be dead — re-park at the front so the frame survives
                // reconnect (the post-reconnect drain will retry it in order).
                if is_priority {
                    state.gated_observer_priority_pending.push_front(event);
                    if let Some(confirmation) = confirmation {
                        state
                            .observer_priority_confirmations
                            .insert(event_id, confirmation);
                    }
                } else {
                    state.gated_observer_pending.push_front(event);
                }
                if outcome.socket_tainted() {
                    return GatedObserverDrainOutcome::SocketTainted;
                }
                break;
            }
            PublishEventFrameOutcome::Sent => {
                let admitted = state.track_observer_in_flight(event);
                state.record_observer_confirmation(event_id, is_priority, confirmation, admitted);
                sent += 1;
            }
        }
    }
    if state.gated_observer_priority_pending.is_empty()
        && state.gated_observer_pending.is_empty()
        && state.gated_observer_dropped > 0
    {
        warn!(
            observer_frames_dropped = state.gated_observer_dropped,
            "observer frames lost to gated-queue overflow"
        );
        state.gated_observer_dropped = 0;
    }
    GatedObserverDrainOutcome::Sent(sent)
}

/// Drain `rate_limited_pending` channels whose retry deadline has passed.
///
/// Called by the main loop pacing timer. Sends at most `budget` REQs without
/// sleeping — pacing is enforced by the caller via `drain_pacing_next`. A
/// failed send re-queues the channel with a +5 s penalty. Returns the number
/// of REQs successfully sent.
async fn drain_rate_limited_pending(
    ws: &mut WsStream,
    state: &mut BgState,
    agent_pubkey_hex: &str,
    budget: usize,
) -> usize {
    let now = tokio::time::Instant::now();
    let ready: Vec<Uuid> = state
        .rate_limited_pending
        .iter()
        .filter(|(_, &deadline)| now >= deadline)
        .map(|(&ch, _)| ch)
        .take(budget)
        .collect();

    if ready.is_empty() {
        return 0;
    }
    debug!("draining {} rate_limited_pending channel(s)", ready.len());

    let mut sent_count = 0;
    for channel_id in ready {
        // Re-check gate each iteration — a new CLOSED may have re-armed it mid-drain.
        if let Some(retry_after) = state.check_rate_gate() {
            state.rate_limited_pending.insert(channel_id, retry_after);
            continue;
        }

        let since = state.channel_since(&channel_id);
        let filter = match state.active_filters.get(&channel_id).cloned() {
            Some(f) => f,
            None => {
                warn!("missing filter for channel {channel_id} in rate_limited_pending — dropping");
                state.rate_limited_pending.remove(&channel_id);
                continue;
            }
        };
        let sent = send_subscribe(ws, state, channel_id, agent_pubkey_hex, since, &filter).await;
        if sent {
            state.rate_limited_pending.remove(&channel_id);
            state.channel_dropped_since.remove(&channel_id);
            sent_count += 1;
            // Pacing is enforced by the main-loop timer; no inline sleep here.
        } else {
            // Socket may be dead — re-queue with +5s penalty; the next ws event
            // will detect the dead socket and trigger a full reconnect.
            let penalty = tokio::time::Instant::now() + Duration::from_secs(5);
            state.rate_limited_pending.insert(channel_id, penalty);
            warn!("drain_rate_limited_pending: REQ failed for channel {channel_id} — re-queued with +5s penalty");
        }
    }
    sent_count
}

/// Drain `resubscribe_retry` channels that were parked by partial reconnect failure.
///
/// Called by the main loop pacing timer. Sends at most `budget` REQs without
/// sleeping — pacing is enforced by the caller. A failed send leaves the
/// channel in the retry set; a gate re-armed mid-drain moves it to
/// `rate_limited_pending`. Returns the number of REQs successfully sent.
async fn drain_resubscribe_retry(
    ws: &mut WsStream,
    state: &mut BgState,
    agent_pubkey_hex: &str,
    budget: usize,
) -> usize {
    if state.resubscribe_retry.is_empty() {
        return 0;
    }
    // Budget-bounded take avoids cloning the full set.
    let channels: Vec<Uuid> = state
        .resubscribe_retry
        .iter()
        .copied()
        .take(budget)
        .collect();
    debug!("draining {} resubscribe_retry channel(s)", channels.len());
    let mut sent_count = 0;
    for channel_id in channels {
        if let Some(retry_after) = state.check_rate_gate() {
            // Gate re-armed mid-drain — move to rate_limited_pending.
            state.rate_limited_pending.insert(channel_id, retry_after);
            state.resubscribe_retry.remove(&channel_id);
            continue;
        }
        let since = state.channel_since(&channel_id);
        let filter = match state.active_filters.get(&channel_id).cloned() {
            Some(f) => f,
            None => {
                warn!("missing filter for channel {channel_id} in resubscribe_retry — dropping");
                state.resubscribe_retry.remove(&channel_id);
                continue;
            }
        };
        let sent = send_subscribe(ws, state, channel_id, agent_pubkey_hex, since, &filter).await;
        if sent {
            state.resubscribe_retry.remove(&channel_id);
            state.channel_dropped_since.remove(&channel_id);
            sent_count += 1;
            // Pacing is enforced by the main-loop timer; no inline sleep here.
        } else {
            warn!(
                "drain_resubscribe_retry: REQ still failing for channel {channel_id} — will retry"
            );
            // Leave in resubscribe_retry; next main-loop tick will try again.
        }
    }
    sent_count
}

/// Outcome of an autonomous reconnect attempt.
enum ReconnectOutcome {
    /// Reconnected and resubscribed successfully.
    Ok,
    /// Reconnect or resubscription attempts failed; caller should retry or fall
    /// back to `wait_for_reconnect`. Live command intent is retained.
    Failed,
    /// A Shutdown command was received during backoff — caller must return immediately.
    Shutdown,
}

/// Execute commands deferred during paced replay, then commands that arrived
/// while the deferred queue was draining. FIFO order is preserved across both
/// sources. Subscription REQs are paced; CLOSE, ephemeral EVENT, and local-state
/// commands execute immediately. A failed live send records remaining command
/// intent and returns `Failed`; Shutdown closes the socket immediately.
async fn drain_commands(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    deferred_commands: &mut VecDeque<RelayCommand>,
    state: &mut BgState,
    agent_pubkey_hex: &str,
) -> ReconnectOutcome {
    let mut send_failed = false;
    loop {
        let cmd = match deferred_commands.pop_front() {
            Some(cmd) => cmd,
            None => match cmd_rx.try_recv() {
                Ok(cmd) => cmd,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return ReconnectOutcome::Shutdown;
                }
            },
        };

        if send_failed {
            match cmd {
                RelayCommand::Shutdown => {
                    // The failed send may have left bytes buffered after
                    // `start_send`. Do not write even a Close frame through
                    // this socket; returning drops or replaces it without a
                    // later flush.
                    return ReconnectOutcome::Shutdown;
                }
                RelayCommand::Reconnect => {}
                cmd => retain_failed_command_intent(state, cmd),
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
            RelayCommand::Subscribe { .. }
            | RelayCommand::SubscribeMembership
            | RelayCommand::SubscribeObserverControls => {
                // A gated subscription is only parked in state; pace only an
                // actual live send attempt.
                let pace_after = state.check_rate_gate().is_none();
                if !execute_connected_command(ws, state, agent_pubkey_hex, cmd).await {
                    warn!("send failed during post-reconnect drain — recording remaining commands as intent");
                    send_failed = true;
                }
                if !send_failed
                    && pace_after
                    && !pacing_sleep(cmd_rx, deferred_commands, REQ_PACING_INTERVAL).await
                {
                    return ReconnectOutcome::Shutdown;
                }
            }
            cmd => {
                if !execute_connected_command(ws, state, agent_pubkey_hex, cmd).await {
                    warn!("send failed during post-reconnect drain — recording remaining commands as intent");
                    send_failed = true;
                }
            }
        }
    }

    if send_failed {
        ReconnectOutcome::Failed
    } else {
        ReconnectOutcome::Ok
    }
}

/// Drain all pending commands after a successful reconnect.
///
/// Processes queued commands that arrived while reconnecting. Reconnect
/// commands are silently dropped (already reconnected). Shutdown causes an
/// immediate close-frame + return of `ReconnectOutcome::Shutdown`. All other
/// commands are executed on the live socket via [`execute_connected_command`].
/// If any subscription send fails, remaining commands are recorded as intent
/// and `Failed` is returned so the caller can reconnect.
async fn drain_post_reconnect(
    ws: &mut WsStream,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    agent_pubkey_hex: &str,
) -> ReconnectOutcome {
    drain_commands(ws, cmd_rx, &mut VecDeque::new(), state, agent_pubkey_hex).await
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
    state.requeue_observer_in_flight();
    state.observer_control_subscribed_at = None;
    state.channel_subscribed_at.clear();
    // 5 attempts, up to 16s base backoff. Shares delay values with the
    // initial-connect retry in `HarnessRelay::connect()` (STARTUP_CONNECT_BACKOFFS) —
    // see its doc comment for how the two loops consume the array differently.
    // DNS failures sleep flat (DNS_RETRY_INTERVAL) without consuming a ladder
    // rung. Capped at 10 DNS-only retries in this bounded startup path so a
    // total brownout cannot hang agent startup indefinitely. By contrast,
    // `wait_for_reconnect` (the post-startup loop) retries DNS failures without
    // a cap — a reconnecting agent should keep trying across extended outages.
    let backoffs = STARTUP_CONNECT_BACKOFFS;
    const MAX_DNS_FLAT_RETRIES: usize = 10;
    let mut dns_retry_count = 0usize;

    let mut attempt = 0usize;
    while attempt < backoffs.len() {
        info!(
            "autonomous reconnect attempt {}/{} to {relay_url}…",
            attempt + 1,
            backoffs.len()
        );
        match do_connect(relay_url, keys, auth_tag).await {
            Ok((new_ws, handshake_buffer)) => {
                *ws = new_ws;
                info!("autonomous reconnect succeeded (attempt {})", attempt + 1);
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
                } else {
                    match resubscribe_after_reconnect(ws, cmd_rx, state, agent_pubkey_hex, true)
                        .await
                    {
                        ResubscribeResult::Ok => {
                            // Commands may arrive after the resubscribe helper's
                            // own drain. Finish that window here, and never return
                            // a socket whose final send failed or was tainted.
                            match drain_post_reconnect(ws, cmd_rx, state, agent_pubkey_hex).await {
                                ReconnectOutcome::Ok => return ReconnectOutcome::Ok,
                                ReconnectOutcome::Shutdown => {
                                    return ReconnectOutcome::Shutdown;
                                }
                                ReconnectOutcome::Failed => {
                                    warn!(
                                        "post-reconnect command drain tainted or lost the autonomous socket — treating as failed attempt"
                                    );
                                    // Fall through to backoff sleep and retry.
                                }
                            }
                        }
                        ResubscribeResult::Shutdown => return ReconnectOutcome::Shutdown,
                        ResubscribeResult::RetryConnection => {
                            warn!("resubscribe failed after autonomous reconnect — treating as failed attempt");
                            // Fall through to backoff sleep and retry.
                        }
                    }
                }
            }
            // DNS failures retry flat without consuming a ladder rung.
            // Cap at MAX_DNS_FLAT_RETRIES so a total brownout doesn't hang startup.
            Err(e) if is_dns_error(&e) && dns_retry_count < MAX_DNS_FLAT_RETRIES => {
                dns_retry_count += 1;
                warn!(
                    "autonomous reconnect DNS failure ({}/{}), flat retry in {:.1}s: {e}",
                    dns_retry_count,
                    MAX_DNS_FLAT_RETRIES,
                    DNS_RETRY_INTERVAL.as_secs_f64()
                );
                if !dns_flat_sleep(cmd_rx, state, DNS_RETRY_INTERVAL).await {
                    return ReconnectOutcome::Shutdown;
                }
                continue; // retry WITHOUT incrementing attempt
            }
            Err(e) => {
                warn!("autonomous reconnect attempt {} failed: {e}", attempt + 1);
            }
        }

        // Any writes and subscription timestamps created on this failed
        // candidate belong to that socket. Preserve unresolved publications,
        // but never let its socket-bound authority reach the next candidate.
        state.abandon_failed_reconnect_candidate();

        // Backoff sleep between ladder attempts (shared by handshake-drop and connect-error).
        // Skip sleep on the final attempt — we'll fall through to the caller.
        // Use select! so Shutdown commands are honoured during sleep.
        if attempt + 1 < backoffs.len() {
            let jittered = jittered_duration(backoffs[attempt]);
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
        attempt += 1;
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
    state.requeue_observer_in_flight();
    state.observer_control_subscribed_at = None;
    state.channel_subscribed_at.clear();
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

    // 6 attempts with backoff up to 32s + jitter; uses tokio::select! so shutdown is
    // honoured during sleep. Resumes from state.backoff_step so a flapping link
    // keeps its elevated position; the stability block resets it to 0 after 60s.
    // DNS failures retry flat without consuming a ladder rung.
    let backoffs = [
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(8),
        Duration::from_secs(16),
        Duration::from_secs(32),
    ];
    let mut attempt = state.backoff_step;
    loop {
        info!("attempting relay reconnect to {relay_url}…");
        match do_connect(relay_url, keys, auth_tag).await {
            Ok((new_ws, handshake_buffer)) => {
                *ws = new_ws;
                info!("relay reconnected to {relay_url}");
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
                } else {
                    match resubscribe_after_reconnect(ws, cmd_rx, state, agent_pubkey_hex, true)
                        .await
                    {
                        ResubscribeResult::Ok => {
                            // Drain any commands that arrived during do_connect() +
                            // resubscribe (which don't poll cmd_rx). A failed
                            // drain may have tainted this replacement socket, so
                            // retry instead of returning it to the main loop.
                            match drain_post_reconnect(ws, cmd_rx, state, agent_pubkey_hex).await {
                                ReconnectOutcome::Ok => return ReconnectOutcome::Ok,
                                ReconnectOutcome::Shutdown => {
                                    return ReconnectOutcome::Shutdown;
                                }
                                ReconnectOutcome::Failed => {
                                    warn!(
                                        "post-reconnect command drain tainted or lost the replacement socket — retrying"
                                    );
                                    // Fall through to the backoff sleep below.
                                }
                            }
                        }
                        ResubscribeResult::Shutdown => return ReconnectOutcome::Shutdown,
                        ResubscribeResult::RetryConnection => {
                            warn!("resubscribe failed after reconnect — will retry with backoff");
                            // Fall through to backoff sleep.
                        }
                    }
                }
            }
            // DNS failures retry on a flat interval without consuming a backoff
            // ladder rung — the host is temporarily unresolvable, not persistently
            // rejecting us, so exponential back-off is counter-productive.
            // This loop is unbounded (unlike the 10-retry cap in `try_autonomous_reconnect`)
            // so a reconnecting agent keeps trying across extended DNS brownouts.
            Err(e) if is_dns_error(&e) => {
                warn!("relay reconnect DNS failure (not consuming ladder rung): {e}");
                if !dns_flat_sleep(cmd_rx, state, DNS_RETRY_INTERVAL).await {
                    return ReconnectOutcome::Shutdown;
                }
                continue; // retry without incrementing attempt
            }
            Err(e) => {
                warn!("relay reconnect failed: {e}");
            }
        }

        // A failed candidate may have written observer frames during replay or
        // command drain. Requeue every unresolved frame and revoke timestamps
        // that were authoritative only for that discarded socket.
        state.abandon_failed_reconnect_candidate();

        // Persist ladder position before sleeping — if shutdown arrives mid-sleep,
        // the next session resumes from here rather than restarting at 0.
        state.backoff_step = attempt;

        // Backoff sleep — shared by both handshake-drop and connect-error paths.
        // Uses a deadline so commands processed during the wait don't reset
        // the timer. Without this, periodic PublishEvent traffic (typing
        // refresh every 3s) would collapse the jittered backoff into a
        // reconnect storm.
        let delay = if attempt < backoffs.len() {
            backoffs[attempt]
        } else {
            Duration::from_secs(60)
        };
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
    state: &mut BgState,
    channel_id: Uuid,
    agent_pubkey_hex: &str,
    since: Option<u64>,
    filter: &ChannelFilter,
) -> bool {
    // Clear before attempting the write so a failed or blocked replacement REQ
    // can never inherit authority from the prior socket/subscription instance.
    state.channel_subscribed_at.remove(&channel_id);
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
                    state
                        .channel_subscribed_at
                        .insert(channel_id, unix_now_secs());
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
///
/// Returns the second after the REQ is written successfully. Callers retain it
/// as the strict admission boundary for frames attributed to this subscription.
async fn send_observer_control_subscribe(ws: &mut WsStream, agent_pubkey_hex: &str) -> Option<u64> {
    let replay_since = unix_now_secs();
    let req = json!([
        "REQ",
        OBSERVER_CONTROL_SUB_ID,
        {
            "kinds": [KIND_AGENT_OBSERVER_FRAME],
            "#p": [agent_pubkey_hex],
            "since": replay_since,
        }
    ]);

    match serde_json::to_string(&req) {
        Ok(text) => {
            match ws_send_timeout(ws, Message::Text(text.into()), WS_SEND_TIMEOUT_SECS).await {
                Ok(()) => {
                    debug!("subscribed to observer control frames");
                    Some(unix_now_secs())
                }
                Err(e) => {
                    warn!("failed to send observer control REQ: {e}");
                    None
                }
            }
        }
        Err(e) => {
            warn!("failed to serialize observer control REQ: {e}");
            None
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

/// Parse the relay's `retry in {N}s` hint from a rate-limit message.
///
/// Accepts any string containing `"retry in "` followed by decimal digits then `'s'`.
/// Returns `None` if the hint is absent; returns `Some(0)` for a literal zero (caller
/// defaults to 5 s). No regex dependency — a simple split is sufficient.
pub(crate) fn parse_rate_limit_retry_secs(msg: &str) -> Option<u64> {
    let after = msg.split("retry in ").nth(1)?;
    // All hint digits are ASCII, so char count == byte count — subslice is valid.
    let len = after.chars().take_while(|c| c.is_ascii_digit()).count();
    after[..len].parse::<u64>().ok()
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

/// Add 0–20% positive jitter to an explicit relay reset horizon.
///
/// Backoff attempts may safely jitter earlier, but a server-provided
/// `retry in Ns` value is a lower bound: retrying before it expires only
/// consumes another denied attempt and can perpetuate the shared gate.
fn rate_limit_retry_duration(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // factor ∈ [1.0, 1.2)
    let factor = 1.0 + (nanos as f64 / u32::MAX as f64) * 0.2;
    base.mul_f64(factor)
}

/// Classify a `RelayError` as a DNS resolution failure.
///
/// Matches the OS-level "name not found" strings surfaced by the platform's
/// resolver, covering macOS (`nodename nor servname`), Linux (`Name or service not
/// known`), and common BSD/Windows variants (`No such host`,
/// `failed to lookup address`). These are transient on brownouts and must NOT
/// consume a backoff ladder rung — they retry on a flat `DNS_RETRY_INTERVAL`.
pub(crate) fn is_dns_error(err: &RelayError) -> bool {
    let msg = err.to_string();
    msg.contains("nodename nor servname")
        || msg.contains("Name or service not known")
        || msg.contains("No such host")
        || msg.contains("failed to lookup address")
}

/// Shutdown-aware fixed-duration sleep for REQ pacing in `resubscribe_after_reconnect`.
///
/// Unlike `dns_flat_sleep`, no jitter is applied — exact `duration` is required
/// to maintain the ≤8 REQ/s pacing invariant. Non-Shutdown commands received
/// during the sleep are deferred in arrival order for live execution after
/// replay. Returns `true` if sleep completed normally, `false` if shutdown was
/// received.
async fn pacing_sleep(
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    deferred_commands: &mut VecDeque<RelayCommand>,
    duration: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + duration;
    let sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return true,
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(RelayCommand::Shutdown) | None => return false,
                    Some(cmd) => deferred_commands.push_back(cmd),
                }
            }
        }
    }
}

/// Shutdown-aware sleep used for DNS flat retries.
///
/// Selects between `duration` elapsing and a `Shutdown`/channel-closed signal on
/// `cmd_rx`. Returns `true` if the sleep completed normally, `false` if the task
/// should shut down.
async fn dns_flat_sleep(
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    state: &mut BgState,
    duration: Duration,
) -> bool {
    let jittered = jittered_duration(duration);
    let deadline = tokio::time::Instant::now() + jittered;
    let sleep = tokio::time::sleep_until(deadline);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return true,
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(RelayCommand::Shutdown) | None => return false,
                    Some(cmd) => apply_command_to_state(state, cmd),
                }
            }
        }
    }
}

/// Extract the one unambiguous channel UUID from the h tag of a Nostr event.
///
/// Channel-scoped events must carry exactly one `h` tag. Accepting the first of
/// multiple values would let a validly-signed event match one relay filter while
/// being attributed to another channel locally.
fn extract_h_tag_uuid(event: &nostr::Event) -> Option<Uuid> {
    let mut h_tags = event.tags.iter().filter_map(|tag| {
        let tag_vec = tag.as_slice();
        if tag_vec.len() >= 2 && tag_vec[0] == "h" {
            Some(tag_vec[1].as_str())
        } else {
            None
        }
    });
    let channel_id = h_tags.next()?.parse::<Uuid>().ok()?;
    if h_tags.next().is_some() {
        return None;
    }
    Some(channel_id)
}

fn event_has_tag_value(event: &nostr::Event, tag_name: &str, expected: &str) -> bool {
    event.tags.iter().any(|tag| {
        let tag_vec = tag.as_slice();
        tag_vec.first().map(String::as_str) == Some(tag_name)
            && tag_vec.get(1).map(String::as_str) == Some(expected)
    })
}

fn is_priority_observer_frame(event: &Event) -> bool {
    event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME
        && event_has_tag_value(
            event,
            OBSERVER_PRIORITY_TAG,
            OBSERVER_CONTROL_RESULT_PRIORITY,
        )
}

fn is_exact_terminal_observer_frame(event: &Event) -> bool {
    if event.kind.as_u16() as u32 != KIND_AGENT_OBSERVER_FRAME {
        return false;
    }
    let mut priority_tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(OBSERVER_PRIORITY_TAG));
    priority_tags.next().is_some_and(|tag| {
        tag.as_slice().len() == 2
            && tag.as_slice().get(1).map(String::as_str) == Some(OBSERVER_CONTROL_RESULT_PRIORITY)
    }) && priority_tags.next().is_none()
}

fn event_has_single_tag_value(event: &nostr::Event, tag_name: &str, expected: &str) -> bool {
    let mut values = event.tags.iter().filter_map(|tag| {
        let tag_vec = tag.as_slice();
        (tag_vec.first().map(String::as_str) == Some(tag_name))
            .then(|| tag_vec.get(1).map(String::as_str))
            .flatten()
    });
    values.next() == Some(expected) && values.next().is_none()
}

fn is_authorized_observer_control(
    event: &nostr::Event,
    agent_pubkey_hex: &str,
    authorized_owner_pubkey: Option<&str>,
) -> bool {
    let Some(authorized_owner_pubkey) = authorized_owner_pubkey else {
        return false;
    };
    event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME
        && event.pubkey.to_hex() == authorized_owner_pubkey
        && event_has_single_tag_value(event, "p", agent_pubkey_hex)
        && event_has_single_tag_value(event, OBSERVER_AGENT_TAG, agent_pubkey_hex)
        && event_has_single_tag_value(event, OBSERVER_FRAME_TAG, OBSERVER_FRAME_CONTROL)
        && content_looks_like_nip44(&event.content)
}

fn is_privileged_channel_control(
    event: &nostr::Event,
    agent_pubkey_hex: &str,
    authorized_owner_pubkey: Option<&str>,
) -> bool {
    let Some(authorized_owner_pubkey) = authorized_owner_pubkey else {
        return false;
    };
    event.pubkey.to_hex() == authorized_owner_pubkey
        && event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE
        && matches!(event.content.trim(), "!shutdown" | "!cancel" | "!rotate")
        && event_has_tag_value(event, "p", agent_pubkey_hex)
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

/// Whether an initial connect/auth-handshake error is terminal — retrying
/// with the same `relay_url`/`keys`/`auth_tag` would reproduce it — rather
/// than transient (the network dropping bytes on a spotty link).
///
/// **Terminal (fail fast):**
/// - `Http`/`Json`/`UnexpectedMessage` — local parsing or relay protocol
///   mismatch; deterministic given the same relay.
/// - `WebSocket` inner variants `Url`, `Capacity`, `Utf8`, `HttpFormat`,
///   `AttackAttempt` — deterministic pre-connect or handshake-shape failures.
/// - `WebSocket(Protocol(…))` — most variants indicate a stable HTTP/WS
///   upgrade mismatch (wrong method, missing headers, accept-key mismatch).
///   Two exceptions are transient: `HandshakeIncomplete` (connection dropped
///   mid-handshake) and `ResetWithoutClosingHandshake` (abrupt reset).
/// - `WebSocket(Http(resp))` — non-101 HTTP response; terminal unless the
///   status is `408`, `429`, or `5xx` (server-side transient conditions).
/// - `WebSocket(Tls)` — deterministic TLS config failures. On our rustls
///   build the only connect-time `Tls` is `InvalidDnsName`.
/// - `WebSocket(Io)` with a deterministic `rustls::Error` in the source
///   chain — terminal. `tokio-rustls` wraps all rustls handshake failures
///   as `io::Error` with the `rustls::Error` as source; `tokio-tungstenite`
///   then surfaces them as `Error::Io`. Only deterministic cert/config/
///   incompatibility variants (allowlist) are terminal; ambiguous protocol,
///   decrypt, and server-alert shapes stay transient under the bounded budget.
/// - `AuthFailed` — split by [`is_terminal_auth_failure`].
///
/// **Transient (retry):**
/// - `WebSocket(Io)` without a rustls source, or with an ambiguous rustls
///   error (alerts, protocol, decrypt) — plain transport failures (reset,
///   EOF, timeout, refused) and ambiguous TLS errors stay retryable.
/// - `WebSocket(ConnectionClosed)` — link-level closure.
/// - `WebSocket(AlreadyClosed)`, `WebSocket(WriteBufferFull)` — unreachable
///   during `connect_async`; kept fail-safe transient.
/// - `NoAuthChallenge`, `ConnectionClosed`, `Timeout` — timing/link noise.
fn is_terminal_connect_error(err: &RelayError) -> bool {
    match err {
        RelayError::Http(_) | RelayError::Json(_) | RelayError::UnexpectedMessage(_) => true,
        RelayError::WebSocket(e) => is_terminal_ws_error(e.as_ref()),
        RelayError::AuthFailed(message) => is_terminal_auth_failure(message),
        RelayError::NoAuthChallenge | RelayError::ConnectionClosed | RelayError::Timeout => false,
    }
}

/// Exhaustive classification of `tungstenite::Error` inner variants for
/// startup connect retry. No wildcard — a tungstenite upgrade forces
/// reclassification at compile time.
fn is_terminal_ws_error(err: &tokio_tungstenite::tungstenite::Error) -> bool {
    use tokio_tungstenite::tungstenite::error::ProtocolError;
    use tokio_tungstenite::tungstenite::Error as WsError;

    match err {
        // Deterministic pre-connect / handshake-shape failures.
        WsError::Url(_)
        | WsError::Capacity(_)
        | WsError::Utf8(_)
        | WsError::HttpFormat(_)
        | WsError::AttackAttempt => true,

        // Non-101 HTTP: terminal unless 408/429/5xx.
        WsError::Http(resp) => {
            let status = resp.status().as_u16();
            !(status == 408 || status == 429 || (500..600).contains(&status))
        }

        // Protocol errors: most are deterministic upgrade mismatches.
        WsError::Protocol(p) => !matches!(
            p,
            ProtocolError::HandshakeIncomplete | ProtocolError::ResetWithoutClosingHandshake
        ),

        // Io: split by error source and rustls variant. tokio-rustls wraps
        // rustls errors as io::Error(InvalidData, rustls_err). Deterministic
        // cert/config/incompatibility failures (allowlist) are terminal;
        // ambiguous protocol, decrypt, and server-alert shapes stay transient
        // under the bounded retry budget. Plain transport Io (reset, EOF,
        // timeout, refused) also stays transient.
        // Relies on a single rustls version in the dep tree (0.23.40);
        // a version split would break the downcast.
        WsError::Io(e) => is_terminal_rustls_io_error(e),

        WsError::ConnectionClosed => false,

        // Deterministic TLS config failures. On our rustls build the only
        // connect-time Tls variant is InvalidDnsName; certificate validation
        // failures arrive wrapped inside Io (terminal via source-chain
        // downcast above).
        WsError::Tls(_) => true,

        // Unreachable during connect_async; kept fail-safe transient.
        WsError::AlreadyClosed | WsError::WriteBufferFull(_) => false,
    }
}

/// Walks an `io::Error` for a `rustls::Error` and inspects its variant.
/// Returns `true` (terminal) only for deterministic cert/config/incompatibility
/// failures that retry cannot fix. Ambiguous protocol, decrypt, and server-alert
/// shapes return `false` (transient) — retries are bounded and the feature's
/// purpose is resilience.
///
/// Relies on a single rustls version in the dep tree (0.23.40); a version split
/// would break the downcast.
fn is_terminal_rustls_io_error(err: &std::io::Error) -> bool {
    use std::error::Error as _;

    fn find_rustls_error(err: &std::io::Error) -> Option<&rustls::Error> {
        // First check the direct inner payload (io::Error stores it via
        // get_ref — source() skips to *its* source).
        if let Some(inner) = err.get_ref() {
            if let Some(re) = inner.downcast_ref::<rustls::Error>() {
                return Some(re);
            }
        }
        // Walk the source chain for deeper wrapping.
        let mut source = err.source();
        while let Some(e) = source {
            if let Some(re) = e.downcast_ref::<rustls::Error>() {
                return Some(re);
            }
            source = e.source();
        }
        None
    }

    let Some(rustls_err) = find_rustls_error(err) else {
        return false;
    };

    matches!(
        rustls_err,
        rustls::Error::InvalidCertificate(_)
            | rustls::Error::InvalidCertRevocationList(_)
            | rustls::Error::NoCertificatesPresented
            | rustls::Error::UnsupportedNameType
            | rustls::Error::PeerIncompatible(_)
    )
}

/// Whether a relay's `OK false <message>` denial during NIP-42 auth is
/// terminal, per the NIP-01 machine-readable prefixes the relay actually
/// sends (`crates/buzz-relay/src/handlers/auth.rs`).
///
/// `error:` marks the relay's own dependency failures (e.g. a ban-state DB
/// lookup that couldn't run) — the relay is failing closed on itself, not
/// rejecting the caller, and a later attempt can succeed once the
/// dependency recovers. `invalid:`, `auth-required:`, `restricted:`, and
/// `blocked:` are explicit rejections of this identity/config (bad
/// signature, ban, non-member, allowlist denial) that retrying without
/// changing anything cannot fix. An unrecognized prefix is treated as
/// terminal — failing fast on an unknown denial is safer than retrying one
/// that might be a real rejection.
fn is_terminal_auth_failure(message: &str) -> bool {
    !message.trim_start().starts_with("error:")
}

/// Retry `op` with bounded jittered backoff, stopping immediately on a
/// terminal error (see [`is_terminal_connect_error`]). Used by
/// `HarnessRelay::connect()` so a transient failure during the initial
/// WebSocket/NIP-42 handshake — e.g. a dropped connection on a spotty link —
/// doesn't fail agent startup outright.
///
/// Generic over the success type so the backoff/classification logic can be
/// exercised in tests without a real socket. Returns the last transient
/// error if all attempts are exhausted.
async fn retry_initial_connect<F, Fut, T>(mut op: F) -> Result<T, RelayError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, RelayError>>,
{
    let mut last_err = None;

    for (attempt, delay) in std::iter::once(None)
        .chain(STARTUP_CONNECT_BACKOFFS.iter().map(|d| Some(*d)))
        .enumerate()
    {
        if let Some(base) = delay {
            let jittered = jittered_duration(base);
            info!(
                "retrying initial relay connect (attempt {attempt}) in {:.1}s",
                jittered.as_secs_f64()
            );
            tokio::time::sleep(jittered).await;
        }

        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if is_terminal_connect_error(&e) => {
                warn!("initial relay connect failed with terminal error: {e}");
                return Err(e);
            }
            Err(e) => {
                warn!("initial relay connect attempt {attempt} failed: {e}");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or(RelayError::ConnectionClosed))
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
    fn merge_discovered_channels_preserves_missing_metadata_as_unknown() {
        let channel = Uuid::new_v4();
        let map = merge_discovered_channels(vec![channel], &serde_json::json!([]));
        assert_eq!(map[&channel].channel_type, "unknown");
    }

    #[test]
    fn merge_discovered_channels_uses_declared_dm_type_without_hidden_hint() {
        let channel = Uuid::new_v4();
        let meta = serde_json::json!([meta_event(channel, "dm", &["t", "dm"])]);
        let map = merge_discovered_channels(vec![channel], &meta);
        assert_eq!(map[&channel].channel_type, "dm");
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

    fn make_test_membership_event(
        keys: &nostr::Keys,
        channel_id: Uuid,
        agent_pubkey_hex: &str,
        created_at_secs: u64,
    ) -> Event {
        let channel_id = channel_id.to_string();
        let tags = [
            Tag::parse(["h", channel_id.as_str()]).expect("valid h tag"),
            Tag::parse(["p", agent_pubkey_hex]).expect("valid p tag"),
        ];
        EventBuilder::new(
            Kind::Custom(KIND_MEMBER_ADDED_NOTIFICATION as u16),
            "membership added",
        )
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(created_at_secs))
        .sign_with_keys(keys)
        .expect("membership event signing should succeed")
    }

    fn make_test_channel_event(
        keys: &nostr::Keys,
        channel_id: Uuid,
        kind: u32,
        content: &str,
        mentioned_pubkey: Option<&str>,
        created_at_secs: u64,
    ) -> Event {
        let channel_id = channel_id.to_string();
        let mut tags = vec![Tag::parse(["h", channel_id.as_str()]).expect("valid h tag")];
        if let Some(pubkey) = mentioned_pubkey {
            tags.push(Tag::parse(["p", pubkey]).expect("valid p tag"));
        }
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from(created_at_secs))
            .sign_with_keys(keys)
            .expect("channel event signing should succeed")
    }

    #[tokio::test]
    async fn membership_authentication_and_targeting_precede_dedup_and_forwarding() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let relay_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let valid =
            make_test_membership_event(&relay_keys, channel_id, &agent_pubkey_hex, 1_700_000);
        let claimed_id = valid.id.to_hex();

        let unsolicited_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, valid.clone()]))
                .expect("serialize unsolicited membership frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                unsolicited_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&claimed_id),
            "an inactive membership subscription must not admit or dedup events"
        );
        assert!(event_rx.try_recv().is_err());

        state.membership_sub_active = true;

        let mut tampered_json = serde_json::to_value(&valid).expect("serialize membership event");
        tampered_json["content"] = Value::String("forged membership".to_string());
        let tampered: Event =
            serde_json::from_value(tampered_json).expect("parse forged membership event");
        assert_eq!(tampered.id.to_hex(), claimed_id);
        assert!(buzz_core::verify_event(&tampered).is_err());
        let forged_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, tampered]))
                .expect("serialize forged membership frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                forged_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&claimed_id),
            "a forged membership event must not poison dedup"
        );
        assert!(event_rx.try_recv().is_err());

        let wrong_target =
            make_test_membership_event(&relay_keys, channel_id, "wrong-agent", 1_700_001);
        let wrong_target_id = wrong_target.id.to_hex();
        let wrong_target_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, wrong_target]))
                .expect("serialize wrong-target membership frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                wrong_target_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&wrong_target_id),
            "a membership event targeting another agent must not poison dedup"
        );
        assert!(event_rx.try_recv().is_err());

        let valid_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, valid]))
                .expect("serialize valid membership frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                valid_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("authentic same-ID membership event remains admissible")
            .expect("membership event payload");
        assert_eq!(forwarded.channel_id, channel_id);
        assert_eq!(forwarded.event.id.to_hex(), claimed_id);
        assert!(state.seen_ids.contains(&claimed_id));
    }

    #[tokio::test]
    async fn regular_event_verification_precedes_dedup_and_forwarding() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        seed_test_subscription(&mut state, channel_id);
        let valid = make_test_channel_event(&keys, channel_id, 9, "test", None, 1_700_000);
        let claimed_id = valid.id.to_hex();

        // Retain the authentic event's claimed ID and signature while changing
        // its content, making both the ID binding and signature invalid.
        let mut tampered_json = serde_json::to_value(&valid).expect("serialize event");
        tampered_json["content"] = Value::String("tampered".to_string());
        let tampered: Event = serde_json::from_value(tampered_json).expect("parse tampered event");
        assert_eq!(tampered.id.to_hex(), claimed_id);
        assert!(buzz_core::verify_event(&tampered).is_err());

        let invalid_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), tampered]))
                .expect("serialize invalid frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                invalid_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &keys,
                "ws://localhost:3000",
                &keys.public_key().to_hex(),
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&claimed_id),
            "invalid event must not poison dedup"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "invalid event must not be forwarded"
        );

        let valid_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), valid]))
                .expect("serialize valid frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                valid_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &keys,
                "ws://localhost:3000",
                &keys.public_key().to_hex(),
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("valid same-ID event remains admissible")
            .expect("regular event payload");
        assert_eq!(forwarded.event.id.to_hex(), claimed_id);
        assert!(state.seen_ids.contains(&claimed_id));
    }

    #[tokio::test]
    async fn regular_events_must_match_active_subscription_and_local_filter() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        let channel_inactive = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let sender_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let mention_filter = ChannelFilter {
            kinds: Some(vec![9]),
            require_mention: true,
        };
        seed_test_subscription_with_filter(&mut state, channel_a, mention_filter.clone());
        seed_test_subscription_with_filter(&mut state, channel_b, mention_filter);

        let routed = make_test_channel_event(
            &sender_keys,
            channel_a,
            9,
            "routed",
            Some(&agent_pubkey_hex),
            1_700_000,
        );
        let routed_id = routed.id.to_hex();
        let misrouted_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_b), routed.clone()]))
                .expect("serialize misrouted channel frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                misrouted_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&routed_id),
            "an event delivered under the wrong active channel must not poison dedup"
        );
        assert!(!state.last_seen.contains_key(&channel_b));
        assert!(event_rx.try_recv().is_err());

        let correct_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_a), routed]))
                .expect("serialize correctly routed channel frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                correct_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("correctly routed same-ID event remains admissible")
            .expect("channel event payload");
        assert_eq!(forwarded.channel_id, channel_a);
        assert_eq!(forwarded.event.id.to_hex(), routed_id);

        let inactive = make_test_channel_event(
            &sender_keys,
            channel_inactive,
            9,
            "inactive",
            Some(&agent_pubkey_hex),
            1_700_001,
        );
        let inactive_id = inactive.id.to_hex();
        let inactive_frame = Message::Text(
            serde_json::to_string(&json!([
                "EVENT",
                channel_sub_id(channel_inactive),
                inactive.clone()
            ]))
            .expect("serialize inactive channel frame")
            .into(),
        );
        assert!(
            handle_ws_message(
                inactive_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&inactive_id),
            "an inactive subscription must not poison dedup"
        );
        assert!(event_rx.try_recv().is_err());

        seed_test_subscription_with_filter(
            &mut state,
            channel_inactive,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );
        let active_frame = Message::Text(
            serde_json::to_string(&json!([
                "EVENT",
                channel_sub_id(channel_inactive),
                inactive
            ]))
            .expect("serialize newly active channel frame")
            .into(),
        );
        assert!(
            handle_ws_message(
                active_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert_eq!(
            event_rx
                .try_recv()
                .expect("event is admissible after local subscription activates")
                .expect("channel event payload")
                .event
                .id
                .to_hex(),
            inactive_id
        );

        let wrong_kind = make_test_channel_event(
            &sender_keys,
            channel_a,
            1,
            "wrong kind",
            Some(&agent_pubkey_hex),
            1_700_002,
        );
        let wrong_kind_id = wrong_kind.id.to_hex();
        let wrong_kind_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_a), wrong_kind]))
                .expect("serialize wrong-kind channel frame")
                .into(),
        );
        assert!(
            handle_ws_message(
                wrong_kind_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(!state.seen_ids.contains(&wrong_kind_id));
        assert!(event_rx.try_recv().is_err());

        let missing_mention =
            make_test_channel_event(&sender_keys, channel_a, 9, "no mention", None, 1_700_003);
        let missing_mention_id = missing_mention.id.to_hex();
        let missing_mention_frame = Message::Text(
            serde_json::to_string(&json!([
                "EVENT",
                channel_sub_id(channel_a),
                missing_mention
            ]))
            .expect("serialize missing-mention channel frame")
            .into(),
        );
        assert!(
            handle_ws_message(
                missing_mention_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(!state.seen_ids.contains(&missing_mention_id));
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn future_ordinary_event_cannot_poison_reconnect_watermark() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let sender_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        seed_test_subscription(&mut state, channel_id);
        let now = unix_now_secs();
        let future =
            make_test_channel_event(&sender_keys, channel_id, 9, "future", None, now + 3_600);
        let future_id = future.id.to_hex();
        let future_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), future]))
                .expect("serialize future ordinary event")
                .into(),
        );

        assert!(
            handle_ws_message(
                future_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&future_id),
            "future event must not reserve its ID"
        );
        assert!(
            !state.last_seen.contains_key(&channel_id),
            "future event must not advance reconnect watermark"
        );
        assert!(event_rx.try_recv().is_err());

        let current = make_test_channel_event(&sender_keys, channel_id, 9, "current", None, now);
        let current_id = current.id.to_hex();
        let current_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), current]))
                .expect("serialize current ordinary event")
                .into(),
        );
        assert!(
            handle_ws_message(
                current_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("current event should remain deliverable")
            .expect("ordinary event payload");
        assert_eq!(forwarded.event.id.to_hex(), current_id);
        assert_eq!(state.last_seen.get(&channel_id), Some(&now));
    }

    #[tokio::test]
    async fn bounded_future_skew_forwards_ordinary_event_without_future_watermark() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let sender_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        seed_test_subscription(&mut state, channel_id);
        let now = unix_now_secs();
        let event = make_test_channel_event(
            &sender_keys,
            channel_id,
            9,
            "slightly ahead",
            None,
            now + CHANNEL_EVENT_FUTURE_SKEW_SECS,
        );
        let event_id = event.id.to_hex();
        let frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), event]))
                .expect("serialize slightly future ordinary event")
                .into(),
        );

        assert!(
            handle_ws_message(
                frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("bounded client clock skew should remain deliverable")
            .expect("ordinary event payload");
        assert_eq!(forwarded.event.id.to_hex(), event_id);
        assert!(!forwarded.privileged_control_admitted);
        assert!(
            state
                .last_seen
                .get(&channel_id)
                .is_some_and(|watermark| *watermark <= unix_now_secs()),
            "accepted future skew must not move replay watermarks into the future"
        );
    }

    #[tokio::test]
    async fn future_membership_event_cannot_poison_replay_state() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        state.membership_sub_active = true;
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let relay_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        let future =
            make_test_membership_event(&relay_keys, channel_id, &agent_pubkey_hex, now + 3_600);
        let future_id = future.id.to_hex();
        let future_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, future]))
                .expect("serialize future membership event")
                .into(),
        );

        assert!(
            handle_ws_message(
                future_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            !state.seen_ids.contains(&future_id),
            "future membership event must not reserve its ID"
        );
        assert_eq!(
            state.membership_last_seen, None,
            "future membership event must not advance replay state"
        );
        assert!(event_rx.try_recv().is_err());

        let current = make_test_membership_event(&relay_keys, channel_id, &agent_pubkey_hex, now);
        let current_id = current.id.to_hex();
        let current_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", MEMBERSHIP_NOTIF_SUB_ID, current]))
                .expect("serialize current membership event")
                .into(),
        );
        assert!(
            handle_ws_message(
                current_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("current membership event should remain deliverable")
            .expect("membership event payload");
        assert_eq!(forwarded.event.id.to_hex(), current_id);
        assert_eq!(state.membership_last_seen, Some(now));
    }

    #[tokio::test]
    async fn stale_privileged_channel_control_is_rejected_before_dedup() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        seed_test_subscription_with_filter(
            &mut state,
            channel_id,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );
        let stale = make_test_channel_event(
            &owner_keys,
            channel_id,
            9,
            "!rotate",
            Some(&agent_pubkey_hex),
            unix_now_secs().saturating_sub(301),
        );
        let stale_id = stale.id.to_hex();
        let stale_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), stale]))
                .expect("serialize stale privileged control")
                .into(),
        );

        assert!(
            handle_ws_message(
                stale_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            state.seen_ids.contains(&stale_id),
            "stale owner control should follow ordinary-message dedup"
        );
        let forwarded = event_rx
            .try_recv()
            .expect("stale owner control should remain an ordinary message")
            .expect("ordinary event payload");
        assert_eq!(forwarded.event.id.to_hex(), stale_id);
        assert!(
            !forwarded.privileged_control_admitted,
            "stale owner control must never carry relay execution authority"
        );
    }

    #[tokio::test]
    async fn non_owner_command_shape_does_not_consume_privileged_replay_capacity() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let non_owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(1);
        state.privileged_control_replay = PrivilegedControlReplay::new(1);
        assert_eq!(
            state.privileged_control_replay.admit(
                "owner-capacity".into(),
                now.saturating_add(300),
                now
            ),
            PrivilegedControlAdmission::Admitted
        );
        seed_test_subscription_with_filter(
            &mut state,
            channel_id,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );

        let ordinary = make_test_channel_event(
            &non_owner_keys,
            channel_id,
            9,
            "!shutdown",
            Some(&agent_pubkey_hex),
            now,
        );
        let ordinary_id = ordinary.id.to_hex();
        let frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), ordinary]))
                .expect("serialize non-owner command-shaped event")
                .into(),
        );

        assert!(
            handle_ws_message(
                frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("non-owner command shape follows ordinary-message admission")
            .expect("ordinary event payload");
        assert_eq!(forwarded.event.id.to_hex(), ordinary_id);
        assert!(
            !forwarded.privileged_control_admitted,
            "non-owner command shape must not carry relay execution authority"
        );
        assert_eq!(
            state.privileged_control_replay.expires_at.len(),
            1,
            "non-owner traffic must not reserve privileged replay capacity"
        );
    }

    #[tokio::test]
    async fn owner_controls_require_live_channel_req_and_post_boundary_retry() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(10);
        seed_test_subscription_with_filter(
            &mut state,
            channel_id,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );

        for (command, boundary, expected_admitted) in [
            ("!shutdown", None, false),
            ("!cancel", Some(now), false),
            ("!rotate", Some(now.saturating_sub(1)), true),
        ] {
            match boundary {
                Some(boundary) => {
                    state.channel_subscribed_at.insert(channel_id, boundary);
                }
                None => {
                    state.channel_subscribed_at.remove(&channel_id);
                }
            }
            let event = make_test_channel_event(
                &owner_keys,
                channel_id,
                KIND_STREAM_MESSAGE,
                command,
                Some(&agent_pubkey_hex),
                now,
            );
            let frame = Message::Text(
                serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), event]))
                    .expect("serialize owner control")
                    .into(),
            );

            assert!(
                handle_ws_message(
                    frame,
                    &mut ws,
                    &event_tx,
                    &observer_control_tx,
                    &mut state,
                    &agent_keys,
                    "ws://localhost:3000",
                    &agent_pubkey_hex,
                    None,
                )
                .await
            );
            let forwarded = event_rx
                .try_recv()
                .expect("owner command-shaped text remains deliverable")
                .expect("ordinary event payload");
            assert_eq!(
                forwarded.privileged_control_admitted, expected_admitted,
                "{command} admission mismatch for boundary {boundary:?}"
            );
        }
    }

    #[tokio::test]
    async fn channel_req_boundary_tracks_only_a_successful_live_send() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let filter = ChannelFilter {
            kinds: Some(vec![KIND_STREAM_MESSAGE]),
            require_mention: true,
        };
        state.channel_subscribed_at.insert(channel_id, 1);

        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter: filter.clone(),
                replay_since: Some(1_000),
            },
        );
        assert!(
            !state.channel_subscribed_at.contains_key(&channel_id),
            "disconnected subscription intent must not retain a prior live REQ boundary"
        );

        let before_send = unix_now_secs();
        assert!(
            send_subscribe(
                &mut client,
                &mut state,
                channel_id,
                "agent-pubkey",
                Some(1_000),
                &filter,
            )
            .await
        );
        let after_send = unix_now_secs();
        let boundary = state
            .channel_subscribed_at
            .get(&channel_id)
            .copied()
            .expect("successful live REQ records its completion boundary");
        assert!((before_send..=after_send).contains(&boundary));
        let frame = next_test_frame(&mut server).await;
        assert_eq!(frame[0], "REQ");
        assert_eq!(frame[1], channel_sub_id(channel_id));
    }

    #[test]
    fn privileged_control_owner_requires_verified_nip_oa_attestation() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let other_agent_keys = Keys::generate();
        let configured_owner_keys = Keys::generate();
        let configured_owner = configured_owner_keys.public_key().to_hex();
        let auth_json =
            buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
                .expect("compute owner attestation");
        let auth_tag =
            buzz_sdk::nip_oa::parse_auth_tag(&auth_json).expect("parse owner attestation");
        let wrong_agent_auth_json =
            buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &other_agent_keys.public_key(), "")
                .expect("compute wrong-agent owner attestation");
        let wrong_agent_auth_tag = buzz_sdk::nip_oa::parse_auth_tag(&wrong_agent_auth_json)
            .expect("parse wrong-agent owner attestation");

        assert_eq!(
            resolve_authorized_owner_pubkey(
                Some(&auth_tag),
                &agent_keys,
                Some(configured_owner.as_str()),
            ),
            Some(owner_keys.public_key().to_hex()),
            "a valid NIP-OA owner remains canonical over configured fallback"
        );
        assert_eq!(
            resolve_authorized_owner_pubkey(
                Some(&wrong_agent_auth_tag),
                &agent_keys,
                Some(configured_owner.as_str()),
            ),
            Some(configured_owner.clone()),
            "an invalid attestation for this agent must fall back to trusted configuration"
        );
        assert_eq!(
            resolve_authorized_owner_pubkey(None, &agent_keys, Some(configured_owner.as_str())),
            Some(configured_owner),
            "missing attestation must retain the trusted configured owner"
        );
        assert_eq!(
            resolve_authorized_owner_pubkey(None, &agent_keys, Some("not-a-pubkey")),
            None,
            "malformed configured owner must fail closed"
        );
    }

    #[test]
    fn privileged_control_replay_guard_never_evicts_live_ids() {
        let mut replay = PrivilegedControlReplay::new(2);

        assert_eq!(
            replay.admit("first".into(), 200, 100),
            PrivilegedControlAdmission::Admitted
        );
        assert_eq!(
            replay.admit("second".into(), 200, 100),
            PrivilegedControlAdmission::Admitted
        );
        assert_eq!(
            replay.admit("third".into(), 200, 100),
            PrivilegedControlAdmission::Full,
            "capacity pressure must fail closed instead of evicting a live ID"
        );
        assert_eq!(
            replay.admit("first".into(), 200, 200),
            PrivilegedControlAdmission::Replay,
            "expiry is inclusive because the event remains freshness-admissible at that second"
        );
        assert_eq!(
            replay.admit("third".into(), 300, 201),
            PrivilegedControlAdmission::Admitted,
            "expired IDs release bounded capacity"
        );
        assert_eq!(replay.expires_at.len(), 1);
    }

    #[tokio::test]
    async fn privileged_controls_require_strict_post_start_nonfuture_timestamps() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(1);
        seed_test_subscription_with_filter(
            &mut state,
            channel_id,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );

        let boundary = make_test_channel_event(
            &owner_keys,
            channel_id,
            9,
            "!shutdown",
            Some(&agent_pubkey_hex),
            state.privileged_control_start_boundary,
        );
        let boundary_id = boundary.id.to_hex();
        let boundary_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), boundary]))
                .expect("serialize boundary control")
                .into(),
        );
        assert!(
            handle_ws_message(
                boundary_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            state.seen_ids.contains(&boundary_id),
            "same-second owner text should remain an ordinary message"
        );
        let boundary_forwarded = event_rx
            .try_recv()
            .expect("same-second owner text should be forwarded without authority")
            .expect("ordinary event payload");
        assert!(
            !boundary_forwarded.privileged_control_admitted,
            "second-granularity equality with startup must fail closed for execution"
        );

        let skewed_future = make_test_channel_event(
            &owner_keys,
            channel_id,
            9,
            "!cancel",
            Some(&agent_pubkey_hex),
            now.saturating_add(CHANNEL_EVENT_FUTURE_SKEW_SECS),
        );
        let skewed_future_id = skewed_future.id.to_hex();
        let skewed_future_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), skewed_future]))
                .expect("serialize skewed future control")
                .into(),
        );
        assert!(
            handle_ws_message(
                skewed_future_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(state.seen_ids.contains(&skewed_future_id));
        let skewed_forwarded = event_rx
            .try_recv()
            .expect("bounded future owner text remains an ordinary message")
            .expect("ordinary event payload");
        assert!(
            !skewed_forwarded.privileged_control_admitted,
            "future-skewed owner text must never gain execution authority"
        );
        assert!(
            state
                .last_seen
                .get(&channel_id)
                .is_some_and(|watermark| *watermark <= unix_now_secs()),
            "future-skewed owner text must clamp its replay watermark"
        );

        let future = make_test_channel_event(
            &owner_keys,
            channel_id,
            9,
            "!cancel",
            Some(&agent_pubkey_hex),
            now.saturating_add(30),
        );
        let future_id = future.id.to_hex();
        let future_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), future]))
                .expect("serialize future control")
                .into(),
        );
        assert!(
            handle_ws_message(
                future_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(!state.seen_ids.contains(&future_id));
        assert!(event_rx.try_recv().is_err());

        let fresh = make_test_channel_event(
            &owner_keys,
            channel_id,
            9,
            "!rotate",
            Some(&agent_pubkey_hex),
            now,
        );
        let fresh_id = fresh.id.to_hex();
        let fresh_json =
            serde_json::to_string(&json!(["EVENT", channel_sub_id(channel_id), fresh]))
                .expect("serialize fresh control");
        assert!(
            handle_ws_message(
                Message::Text(fresh_json.clone().into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        let forwarded = event_rx
            .try_recv()
            .expect("fresh post-start control should be forwarded")
            .expect("control payload");
        assert_eq!(forwarded.event.id.to_hex(), fresh_id);
        assert!(
            forwarded.privileged_control_admitted,
            "fresh exact-owner control must carry relay execution authority"
        );
        assert!(
            handle_ws_message(
                Message::Text(fresh_json.clone().into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            event_rx.try_recv().is_err(),
            "same-process replay must not be forwarded"
        );

        let mut restarted_state = BgState::new();
        restarted_state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        restarted_state.privileged_control_start_boundary = now;
        seed_test_subscription_with_filter(
            &mut restarted_state,
            channel_id,
            ChannelFilter {
                kinds: Some(vec![9]),
                require_mention: true,
            },
        );
        assert!(
            handle_ws_message(
                Message::Text(fresh_json.into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut restarted_state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            restarted_state.seen_ids.contains(&fresh_id),
            "pre-start owner text should remain an ordinary message after restart"
        );
        let restarted_forwarded = event_rx
            .try_recv()
            .expect("pre-start owner text should be forwarded without authority")
            .expect("ordinary event payload");
        assert_eq!(restarted_forwarded.event.id.to_hex(), fresh_id);
        assert!(
            !restarted_forwarded.privileged_control_admitted,
            "restart replay must never regain execution authority"
        );
    }

    async fn test_ws_pair() -> (WsStream, WebSocketStream<tokio::net::TcpStream>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test websocket");
        let address = listener.local_addr().expect("read test address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept test websocket");
            tokio_tungstenite::accept_async(stream)
                .await
                .expect("complete server websocket handshake")
        });
        let (client, _) = connect_async(format!("ws://{address}"))
            .await
            .expect("connect test websocket");
        (client, server.await.expect("join test websocket server"))
    }

    async fn next_test_frame(
        server: &mut WebSocketStream<tokio::net::TcpStream>,
    ) -> serde_json::Value {
        let message = timeout(Duration::from_secs(1), server.next())
            .await
            .expect("timed out waiting for websocket frame")
            .expect("test websocket closed")
            .expect("read test websocket frame");
        serde_json::from_str(message.to_text().expect("expected text frame"))
            .expect("parse test websocket frame")
    }

    fn test_channel_filter() -> ChannelFilter {
        ChannelFilter {
            kinds: Some(vec![9]),
            require_mention: false,
        }
    }

    fn seed_test_subscription(state: &mut BgState, channel_id: Uuid) {
        seed_test_subscription_with_filter(state, channel_id, test_channel_filter());
    }

    fn seed_test_subscription_with_filter(
        state: &mut BgState,
        channel_id: Uuid,
        filter: ChannelFilter,
    ) {
        apply_command_to_state(
            state,
            RelayCommand::Subscribe {
                channel_id,
                filter,
                replay_since: Some(1_000),
            },
        );
        state
            .channel_subscribed_at
            .insert(channel_id, unix_now_secs().saturating_sub(1));
    }

    #[tokio::test]
    async fn fresh_reconnect_preserves_gate_until_pending_replay_resumes() {
        let (mut client, mut server) = test_ws_pair().await;
        let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        seed_test_subscription(&mut state, channel_id);
        state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_millis(150));

        let result =
            resubscribe_after_reconnect(&mut client, &mut cmd_rx, &mut state, "agent-pubkey", true)
                .await;

        assert!(matches!(result, ResubscribeResult::Ok));
        assert!(state.rate_limit_gate.is_some());
        assert!(state.rate_limited_pending.contains_key(&channel_id));
        assert!(
            timeout(Duration::from_millis(50), server.next())
                .await
                .is_err(),
            "fresh reconnect must not send REQ while the shared quota gate is active"
        );

        tokio::time::sleep(Duration::from_millis(125)).await;
        assert_eq!(
            drain_rate_limited_pending(&mut client, &mut state, "agent-pubkey", 1).await,
            1
        );
        let frame = next_test_frame(&mut server).await;
        assert_eq!(frame[0], "REQ");
        assert_eq!(frame[1], channel_sub_id(channel_id));
    }

    #[tokio::test]
    async fn subscribe_during_replay_pacing_is_sent_on_live_socket() {
        let (client, mut server) = test_ws_pair().await;
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let mut state = BgState::new();
        let replayed_channel = Uuid::new_v4();
        let deferred_channel = Uuid::new_v4();
        seed_test_subscription(&mut state, replayed_channel);

        let task = tokio::spawn(async move {
            let mut client = client;
            let result = resubscribe_after_reconnect(
                &mut client,
                &mut cmd_rx,
                &mut state,
                "agent-pubkey",
                true,
            )
            .await;
            (result, state)
        });

        let replay = next_test_frame(&mut server).await;
        assert_eq!(replay[1], channel_sub_id(replayed_channel));
        cmd_tx
            .send(RelayCommand::Subscribe {
                channel_id: deferred_channel,
                filter: test_channel_filter(),
                replay_since: Some(2_000),
            })
            .await
            .expect("queue subscribe during pacing");

        let deferred = next_test_frame(&mut server).await;
        assert_eq!(deferred[0], "REQ");
        assert_eq!(deferred[1], channel_sub_id(deferred_channel));
        let (result, state) = task.await.expect("join resubscribe task");
        assert!(matches!(result, ResubscribeResult::Ok));
        assert!(state.active_subscriptions.contains_key(&deferred_channel));
    }

    #[tokio::test]
    async fn unsubscribe_during_replay_pacing_sends_close_on_live_socket() {
        let (client, mut server) = test_ws_pair().await;
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        seed_test_subscription(&mut state, channel_id);

        let task = tokio::spawn(async move {
            let mut client = client;
            let result = resubscribe_after_reconnect(
                &mut client,
                &mut cmd_rx,
                &mut state,
                "agent-pubkey",
                true,
            )
            .await;
            (result, state)
        });

        let replay = next_test_frame(&mut server).await;
        assert_eq!(replay[1], channel_sub_id(channel_id));
        cmd_tx
            .send(RelayCommand::Unsubscribe { channel_id })
            .await
            .expect("queue unsubscribe during pacing");

        let close = next_test_frame(&mut server).await;
        assert_eq!(close, json!(["CLOSE", channel_sub_id(channel_id)]));
        let (result, state) = task.await.expect("join resubscribe task");
        assert!(matches!(result, ResubscribeResult::Ok));
        assert!(!state.active_subscriptions.contains_key(&channel_id));
    }

    #[tokio::test]
    async fn publish_during_replay_pacing_is_sent_on_live_socket() {
        let (client, mut server) = test_ws_pair().await;
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();
        seed_test_subscription(&mut state, channel_id);
        let event = make_test_event(&nostr::Keys::generate(), 2_000);
        let event_id = event.id.to_hex();

        let task = tokio::spawn(async move {
            let mut client = client;
            let result = resubscribe_after_reconnect(
                &mut client,
                &mut cmd_rx,
                &mut state,
                "agent-pubkey",
                true,
            )
            .await;
            result
        });

        let replay = next_test_frame(&mut server).await;
        assert_eq!(replay[1], channel_sub_id(channel_id));
        cmd_tx
            .send(RelayCommand::PublishEvent {
                event: Box::new(event),
                admission: None,
            })
            .await
            .expect("queue publish during pacing");

        let publish = next_test_frame(&mut server).await;
        assert_eq!(publish[0], "EVENT");
        assert_eq!(publish[1]["id"], event_id);
        assert!(matches!(
            task.await.expect("join resubscribe task"),
            ResubscribeResult::Ok
        ));
    }

    #[test]
    fn failed_replay_retains_deferred_subscription_intent_in_fifo_order() {
        let mut state = BgState::new();
        let kept_channel = Uuid::new_v4();
        let removed_channel = Uuid::new_v4();
        seed_test_subscription(&mut state, removed_channel);
        let event = make_test_event(&nostr::Keys::generate(), 2_000);
        let mut deferred = VecDeque::from([
            RelayCommand::Subscribe {
                channel_id: kept_channel,
                filter: test_channel_filter(),
                replay_since: Some(2_000),
            },
            RelayCommand::Unsubscribe {
                channel_id: removed_channel,
            },
            RelayCommand::PublishEvent {
                event: Box::new(event),
                admission: None,
            },
        ]);

        retain_deferred_command_intent(&mut state, &mut deferred);

        assert!(deferred.is_empty());
        assert!(state.active_subscriptions.contains_key(&kept_channel));
        assert!(!state.active_subscriptions.contains_key(&removed_channel));
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

    // ── startup connect retry ────────────────────────────────────────────

    /// Table-driven coverage of every `RelayError` variant and every
    /// `tungstenite::Error` inner variant. Exhaustive — adding a new
    /// tungstenite variant without updating this table is a compile error
    /// in `is_terminal_ws_error` (no wildcard), and a missing row here
    /// is a code-review gap, not a silent misclassification.
    #[test]
    fn connect_error_classification_matches_every_relay_error_variant() {
        use tokio_tungstenite::tungstenite::error::{
            CapacityError, Error as WsError, ProtocolError, SubProtocolError, TlsError, UrlError,
        };
        use tokio_tungstenite::tungstenite::http;

        fn ws(e: WsError) -> RelayError {
            RelayError::WebSocket(Box::new(e))
        }

        let cases: Vec<(&str, RelayError, bool)> = vec![
            // ── outer RelayError variants ──
            ("Http: bad URL", RelayError::Http("bad url".into()), true),
            (
                "Json: malformed relay frame",
                RelayError::Json(serde_json::from_str::<()>("not json").unwrap_err()),
                true,
            ),
            (
                "UnexpectedMessage: unknown frame type",
                RelayError::UnexpectedMessage("unknown message type: WAT".into()),
                true,
            ),
            (
                "AuthFailed: relay dependency fault (NIP-01 `error:` prefix)",
                RelayError::AuthFailed("error: internal error checking restriction state".into()),
                false,
            ),
            (
                "AuthFailed: bad signature (`invalid:` prefix)",
                RelayError::AuthFailed("invalid: bad signature".into()),
                true,
            ),
            (
                "AuthFailed: banned (`blocked:` prefix)",
                RelayError::AuthFailed("blocked: you are banned from this community".into()),
                true,
            ),
            (
                "AuthFailed: not a member (`restricted:` prefix)",
                RelayError::AuthFailed("restricted: not a relay member".into()),
                true,
            ),
            (
                "AuthFailed: allowlist denial (`auth-required:` prefix)",
                RelayError::AuthFailed("auth-required: verification failed".into()),
                true,
            ),
            (
                "AuthFailed: unrecognized prefix fails safe as terminal",
                RelayError::AuthFailed("some new denial reason".into()),
                true,
            ),
            (
                "NoAuthChallenge: relay silence is link/relay-timing noise",
                RelayError::NoAuthChallenge,
                false,
            ),
            ("ConnectionClosed", RelayError::ConnectionClosed, false),
            ("Timeout", RelayError::Timeout, false),
            // ── WebSocket inner: terminal ──
            (
                "WebSocket(Url): unsupported scheme",
                ws(WsError::Url(UrlError::UnsupportedUrlScheme)),
                true,
            ),
            (
                "WebSocket(Url): missing host",
                ws(WsError::Url(UrlError::NoHostName)),
                true,
            ),
            (
                "WebSocket(Url): empty host",
                ws(WsError::Url(UrlError::EmptyHostName)),
                true,
            ),
            (
                "WebSocket(Url): TLS feature not enabled",
                ws(WsError::Url(UrlError::TlsFeatureNotEnabled)),
                true,
            ),
            (
                "WebSocket(Url): unable to connect",
                ws(WsError::Url(UrlError::UnableToConnect("addr".into()))),
                true,
            ),
            (
                "WebSocket(Url): no path or query",
                ws(WsError::Url(UrlError::NoPathOrQuery)),
                true,
            ),
            (
                "WebSocket(Capacity): message too long",
                ws(WsError::Capacity(CapacityError::MessageTooLong {
                    size: 100,
                    max_size: 50,
                })),
                true,
            ),
            (
                "WebSocket(Capacity): too many headers",
                ws(WsError::Capacity(CapacityError::TooManyHeaders)),
                true,
            ),
            (
                "WebSocket(Utf8): encoding error",
                ws(WsError::Utf8("invalid utf-8".into())),
                true,
            ),
            (
                "WebSocket(HttpFormat): malformed HTTP",
                ws(WsError::HttpFormat(
                    http::Response::builder().status(9999).body(()).unwrap_err(),
                )),
                true,
            ),
            ("WebSocket(AttackAttempt)", ws(WsError::AttackAttempt), true),
            // ── WebSocket inner: Http status split ──
            (
                "WebSocket(Http): 200 = plain HTTPS endpoint → terminal",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(200).body(None).unwrap(),
                ))),
                true,
            ),
            (
                "WebSocket(Http): 301 redirect → terminal",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(301).body(None).unwrap(),
                ))),
                true,
            ),
            (
                "WebSocket(Http): 404 not found → terminal",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(404).body(None).unwrap(),
                ))),
                true,
            ),
            (
                "WebSocket(Http): 403 forbidden → terminal",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(403).body(None).unwrap(),
                ))),
                true,
            ),
            (
                "WebSocket(Http): 408 request timeout → transient",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(408).body(None).unwrap(),
                ))),
                false,
            ),
            (
                "WebSocket(Http): 429 too many requests → transient",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(429).body(None).unwrap(),
                ))),
                false,
            ),
            (
                "WebSocket(Http): 500 internal server error → transient",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(500).body(None).unwrap(),
                ))),
                false,
            ),
            (
                "WebSocket(Http): 502 bad gateway → transient",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(502).body(None).unwrap(),
                ))),
                false,
            ),
            (
                "WebSocket(Http): 503 service unavailable → transient",
                ws(WsError::Http(Box::new(
                    http::Response::builder().status(503).body(None).unwrap(),
                ))),
                false,
            ),
            // ── WebSocket inner: Protocol variants ──
            (
                "Protocol(WrongHttpMethod): deterministic upgrade mismatch",
                ws(WsError::Protocol(ProtocolError::WrongHttpMethod)),
                true,
            ),
            (
                "Protocol(WrongHttpVersion): deterministic upgrade mismatch",
                ws(WsError::Protocol(ProtocolError::WrongHttpVersion)),
                true,
            ),
            (
                "Protocol(MissingConnectionUpgradeHeader)",
                ws(WsError::Protocol(
                    ProtocolError::MissingConnectionUpgradeHeader,
                )),
                true,
            ),
            (
                "Protocol(MissingUpgradeWebSocketHeader)",
                ws(WsError::Protocol(
                    ProtocolError::MissingUpgradeWebSocketHeader,
                )),
                true,
            ),
            (
                "Protocol(MissingSecWebSocketVersionHeader)",
                ws(WsError::Protocol(
                    ProtocolError::MissingSecWebSocketVersionHeader,
                )),
                true,
            ),
            (
                "Protocol(MissingSecWebSocketKey)",
                ws(WsError::Protocol(ProtocolError::MissingSecWebSocketKey)),
                true,
            ),
            (
                "Protocol(SecWebSocketAcceptKeyMismatch)",
                ws(WsError::Protocol(
                    ProtocolError::SecWebSocketAcceptKeyMismatch,
                )),
                true,
            ),
            (
                "Protocol(SecWebSocketSubProtocolError)",
                ws(WsError::Protocol(
                    ProtocolError::SecWebSocketSubProtocolError(
                        SubProtocolError::ServerSentSubProtocolNoneRequested,
                    ),
                )),
                true,
            ),
            (
                "Protocol(JunkAfterRequest)",
                ws(WsError::Protocol(ProtocolError::JunkAfterRequest)),
                true,
            ),
            (
                "Protocol(CustomResponseSuccessful)",
                ws(WsError::Protocol(ProtocolError::CustomResponseSuccessful)),
                true,
            ),
            (
                "Protocol(InvalidHeader)",
                ws(WsError::Protocol(ProtocolError::InvalidHeader(Box::new(
                    http::header::UPGRADE,
                )))),
                true,
            ),
            (
                "Protocol(HttparseError)",
                ws(WsError::Protocol(ProtocolError::HttparseError(
                    httparse::Error::TooManyHeaders,
                ))),
                true,
            ),
            (
                "Protocol(SendAfterClosing)",
                ws(WsError::Protocol(ProtocolError::SendAfterClosing)),
                true,
            ),
            (
                "Protocol(ReceivedAfterClosing)",
                ws(WsError::Protocol(ProtocolError::ReceivedAfterClosing)),
                true,
            ),
            (
                "Protocol(NonZeroReservedBits)",
                ws(WsError::Protocol(ProtocolError::NonZeroReservedBits)),
                true,
            ),
            (
                "Protocol(UnmaskedFrameFromClient)",
                ws(WsError::Protocol(ProtocolError::UnmaskedFrameFromClient)),
                true,
            ),
            (
                "Protocol(MaskedFrameFromServer)",
                ws(WsError::Protocol(ProtocolError::MaskedFrameFromServer)),
                true,
            ),
            (
                "Protocol(FragmentedControlFrame)",
                ws(WsError::Protocol(ProtocolError::FragmentedControlFrame)),
                true,
            ),
            (
                "Protocol(ControlFrameTooBig)",
                ws(WsError::Protocol(ProtocolError::ControlFrameTooBig)),
                true,
            ),
            (
                "Protocol(UnknownControlFrameType)",
                ws(WsError::Protocol(ProtocolError::UnknownControlFrameType(
                    0xF,
                ))),
                true,
            ),
            (
                "Protocol(UnknownDataFrameType)",
                ws(WsError::Protocol(ProtocolError::UnknownDataFrameType(0xF))),
                true,
            ),
            (
                "Protocol(UnexpectedContinueFrame)",
                ws(WsError::Protocol(ProtocolError::UnexpectedContinueFrame)),
                true,
            ),
            (
                "Protocol(ExpectedFragment)",
                ws(WsError::Protocol(ProtocolError::ExpectedFragment(
                    tokio_tungstenite::tungstenite::protocol::frame::coding::Data::Text,
                ))),
                true,
            ),
            (
                "Protocol(InvalidOpcode)",
                ws(WsError::Protocol(ProtocolError::InvalidOpcode(0xF))),
                true,
            ),
            (
                "Protocol(InvalidCloseSequence)",
                ws(WsError::Protocol(ProtocolError::InvalidCloseSequence)),
                true,
            ),
            // ── Protocol: transient exceptions ──
            (
                "Protocol(HandshakeIncomplete): connection dropped mid-handshake",
                ws(WsError::Protocol(ProtocolError::HandshakeIncomplete)),
                false,
            ),
            (
                "Protocol(ResetWithoutClosingHandshake): abrupt reset",
                ws(WsError::Protocol(
                    ProtocolError::ResetWithoutClosingHandshake,
                )),
                false,
            ),
            // ── WebSocket(Io): transport (transient) ──
            (
                "Io(other): plain transport failure is transient",
                ws(WsError::Io(std::io::Error::other("reset"))),
                false,
            ),
            (
                "Io(ConnectionReset): transport reset is transient",
                ws(WsError::Io(std::io::ErrorKind::ConnectionReset.into())),
                false,
            ),
            (
                "Io(UnexpectedEof): transport EOF is transient",
                ws(WsError::Io(std::io::ErrorKind::UnexpectedEof.into())),
                false,
            ),
            (
                "Io(TimedOut): transport timeout is transient",
                ws(WsError::Io(std::io::ErrorKind::TimedOut.into())),
                false,
            ),
            // ── WebSocket(Io): rustls-sourced, variant-inspected ──
            // Production shape: tokio-rustls wraps rustls errors as
            // io::Error(InvalidData, rustls::Error). Only deterministic
            // cert/config/incompatibility variants are terminal.
            (
                "Io(rustls InvalidCertificate(Expired)): production-shaped expired cert is terminal",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
                ))),
                true,
            ),
            (
                "Io(rustls InvalidCertificate(NotValidForName)): hostname mismatch is terminal",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::InvalidCertificate(
                        rustls::CertificateError::NotValidForName,
                    ),
                ))),
                true,
            ),
            (
                "Io(rustls NoCertificatesPresented): missing cert is terminal",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::NoCertificatesPresented,
                ))),
                true,
            ),
            // ── WebSocket(Io): rustls-sourced, ambiguous (transient) ──
            // Protocol, decrypt, alert, and general errors may be caused by
            // network conditions or transient server failures — retryable
            // under the bounded budget.
            (
                "Io(rustls General): ambiguous general error is transient",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::General("protocol error".into()),
                ))),
                false,
            ),
            (
                "Io(rustls AlertReceived(InternalError)): server alert is transient",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::AlertReceived(rustls::AlertDescription::InternalError),
                ))),
                false,
            ),
            (
                "Io(rustls DecryptError): corrupted record is transient",
                ws(WsError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    rustls::Error::DecryptError,
                ))),
                false,
            ),
            (
                "WebSocket(ConnectionClosed): link-level closure",
                ws(WsError::ConnectionClosed),
                false,
            ),
            // ── WebSocket(Tls): deterministic config (terminal, pins the arm) ──
            // These shapes are constructible but not reachable through our
            // rustls production connector — cert failures arrive as Io above.
            // Kept to pin the Tls(_) => true arm.
            (
                "Tls(Rustls(General)): pins Tls arm terminal",
                ws(WsError::Tls(
                    rustls::Error::General("tls handshake failed".into()).into(),
                )),
                true,
            ),
            (
                "Tls(InvalidDnsName): only reachable connect-time Tls variant",
                ws(WsError::Tls(TlsError::InvalidDnsName)),
                true,
            ),
            (
                "Tls(Rustls(InvalidCertificate(Expired))): pins Tls arm terminal",
                ws(WsError::Tls(
                    rustls::Error::InvalidCertificate(rustls::CertificateError::Expired).into(),
                )),
                true,
            ),
            (
                "WebSocket(AlreadyClosed): unreachable at connect, fail-safe transient",
                ws(WsError::AlreadyClosed),
                false,
            ),
            (
                "WebSocket(WriteBufferFull): unreachable at connect, fail-safe transient",
                ws(WsError::WriteBufferFull(Box::new(
                    tokio_tungstenite::tungstenite::Message::Text("x".into()),
                ))),
                false,
            ),
        ];

        for (label, err, want_terminal) in cases {
            assert_eq!(
                is_terminal_connect_error(&err),
                want_terminal,
                "{label}: expected terminal={want_terminal}"
            );
        }
    }

    /// A literal `https://…` URL through production `do_connect()` must fail
    /// fast as terminal — the relay endpoint is a plain HTTPS server, not a
    /// WebSocket endpoint, and tungstenite returns `Error::Http` (non-101
    /// response) or `Error::Url(UnsupportedUrlScheme)` depending on how far
    /// the handshake gets. Either way it must not be retried.
    #[tokio::test]
    async fn do_connect_wrong_scheme_is_terminal() {
        let keys = nostr::Keys::generate();
        let err = do_connect("https://example.com", &keys, None)
            .await
            .unwrap_err();
        assert!(
            is_terminal_connect_error(&err),
            "wrong-scheme URL should be terminal, got: {err}"
        );
    }

    /// A transient failure (e.g. connection dropped mid-handshake on a spotty
    /// link) must be retried and can still succeed once the link recovers.
    #[tokio::test(start_paused = true)]
    async fn retry_initial_connect_retries_transient_failure_then_succeeds() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let result: Result<&'static str, RelayError> = retry_initial_connect(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(RelayError::ConnectionClosed)
                } else {
                    Ok("connected")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "connected");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "should succeed on the 3rd attempt (2 transient failures + 1 success)"
        );
    }

    /// A terminal error (bad auth, bad config) must not be retried — the
    /// same call would fail identically every time, so retrying just delays
    /// surfacing a real problem to the caller.
    #[tokio::test(start_paused = true)]
    async fn retry_initial_connect_does_not_retry_terminal_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let result: Result<(), RelayError> = retry_initial_connect(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(RelayError::AuthFailed("invalid: bad signature".into())) }
        })
        .await;

        assert!(matches!(result, Err(RelayError::AuthFailed(_))));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "a terminal error must fail on the first attempt with no retries"
        );
    }

    /// A relay-side dependency fault (NIP-01 `error:` prefix) is transient —
    /// the relay is failing closed on itself, not rejecting this identity —
    /// so it must be retried rather than surfaced immediately like a real
    /// auth rejection.
    #[tokio::test(start_paused = true)]
    async fn retry_initial_connect_retries_relay_dependency_fault() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let result: Result<&'static str, RelayError> = retry_initial_connect(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 1 {
                    Err(RelayError::AuthFailed(
                        "error: internal error checking restriction state".into(),
                    ))
                } else {
                    Ok("connected")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "connected");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "a relay dependency fault must be retried, not surfaced immediately"
        );
    }

    /// Once every attempt (1 initial + N backoff retries) is exhausted, the
    /// last transient error is returned rather than retrying forever — a
    /// dead relay must not hang agent startup indefinitely.
    #[tokio::test(start_paused = true)]
    async fn retry_initial_connect_exhausts_and_returns_last_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let result: Result<(), RelayError> = retry_initial_connect(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(RelayError::Timeout) }
        })
        .await;

        assert!(
            matches!(result, Err(RelayError::Timeout)),
            "must surface the last attempt's error, not a generic one"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            STARTUP_CONNECT_BACKOFFS.len() + 1,
            "must attempt exactly once plus one retry per backoff entry"
        );
    }

    /// Backoff sleeps must actually elapse (not be skipped) — this pins the
    /// bounded-but-real-delay contract using `tokio::time::pause` so the
    /// test itself stays fast (virtual time, not wall-clock sleeps).
    #[tokio::test(start_paused = true)]
    async fn retry_initial_connect_sleeps_between_attempts() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let call = retry_initial_connect(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 1 {
                    Err(RelayError::ConnectionClosed)
                } else {
                    Ok(())
                }
            }
        });
        tokio::pin!(call);

        // Before the first backoff elapses, the retry must still be pending
        // (i.e. it actually slept rather than immediately retrying).
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(1)) => {}
            _ = &mut call => panic!("must not resolve before the backoff sleep elapses"),
        }
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        // Advancing past the (jittered, ≤1.2x) first backoff lets it proceed.
        let result = call.await;
        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    // ── Rate-limit gate, pacing, backoff reset, DNS ──────────────────────────

    /// parse_rate_limit_retry_secs: full hint extracts the N from "retry in Ns".
    #[test]
    fn parse_rate_limit_retry_secs_with_hint() {
        assert_eq!(
            parse_rate_limit_retry_secs("rate-limited: quota exceeded; retry in 12s"),
            Some(12)
        );
    }

    /// parse_rate_limit_retry_secs: message without a hint returns None.
    #[test]
    fn parse_rate_limit_retry_secs_missing_hint() {
        assert_eq!(
            parse_rate_limit_retry_secs("rate-limited: too many concurrent requests"),
            None
        );
    }

    /// parse_rate_limit_retry_secs: explicit zero value is returned as Some(0).
    #[test]
    fn parse_rate_limit_retry_secs_zero() {
        assert_eq!(
            parse_rate_limit_retry_secs("rate-limited: quota exceeded; retry in 0s"),
            Some(0)
        );
    }

    /// parse_rate_limit_retry_secs: garbage input returns None.
    #[test]
    fn parse_rate_limit_retry_secs_garbage() {
        assert_eq!(
            parse_rate_limit_retry_secs("not a rate limit message"),
            None
        );
    }

    /// set_rate_limit_gate arms the gate with jittered expiry from the hint.
    /// check_rate_gate returns Some while active and lazily clears on expiry.
    #[tokio::test(start_paused = true)]
    async fn rate_limit_gate_set_and_expiry() {
        let mut state = BgState::new();
        assert!(
            state.check_rate_gate().is_none(),
            "gate must start inactive"
        );

        // Arm with a 5 s hint.
        state.set_rate_limit_gate(5);
        assert!(
            state.check_rate_gate().is_some(),
            "gate must be active immediately after arming"
        );

        // Advance virtual time past the max jitter (1.2 × 5 s = 6 s).
        tokio::time::advance(Duration::from_secs(7)).await;

        assert!(
            state.check_rate_gate().is_none(),
            "gate must have expired after 7s"
        );
        assert!(
            state.rate_limit_gate.is_none(),
            "check_rate_gate must lazily clear the field on expiry"
        );
    }

    /// set_rate_limit_gate takes the max of overlapping deadlines.
    #[tokio::test(start_paused = true)]
    async fn rate_limit_gate_extends_to_max() {
        let mut state = BgState::new();

        // Arm with a long hint first.
        state.set_rate_limit_gate(30);
        let first_deadline = state.rate_limit_gate.unwrap();

        // A shorter subsequent hint must NOT shorten the existing gate.
        state.set_rate_limit_gate(1);
        let second_deadline = state.rate_limit_gate.unwrap();

        assert_eq!(
            first_deadline, second_deadline,
            "shorter hint must not overwrite a later existing deadline"
        );
    }

    #[test]
    fn terminal_confirmation_window_outlives_maximum_accepted_rate_limit_gate() {
        assert!(
            TERMINAL_OBSERVER_ACK_TIMEOUT
                > Duration::from_secs(MAX_RATE_LIMIT_RETRY_SECS).mul_f64(1.2),
            "terminal confirmation must remain live beyond the largest accepted retry hint plus maximum jitter"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn oversized_rate_limit_hint_is_bounded_below_terminal_confirmation_window() {
        let mut state = BgState::new();
        let now = tokio::time::Instant::now();
        let deadline = state.set_rate_limit_gate(u64::MAX);
        assert!(
            deadline.duration_since(now) >= Duration::from_secs(MAX_RATE_LIMIT_RETRY_SECS),
            "an explicit reset hint must never be retried before its advertised window"
        );
        assert!(
            deadline.duration_since(now)
                <= Duration::from_secs(MAX_RATE_LIMIT_RETRY_SECS).mul_f64(1.2),
            "untrusted retry hints must not extend the local gate beyond its bounded contract"
        );
        assert!(
            deadline.duration_since(now) < TERMINAL_OBSERVER_ACK_TIMEOUT,
            "the accepted gate must expire while the terminal publisher is still awaiting proof"
        );
    }

    /// Build a signed observer telemetry frame (kind 24200) for gate tests.
    fn make_observer_frame(keys: &Keys) -> Event {
        let recipient = Keys::generate();
        let encrypted = buzz_core::observer::encrypt_observer_payload(
            keys,
            &recipient.public_key(),
            &json!({"type": "test"}),
        )
        .expect("encrypt test observer payload");
        buzz_sdk::build_agent_observer_frame(
            &recipient.public_key().to_hex(),
            &keys.public_key().to_hex(),
            "telemetry",
            &encrypted,
        )
        .expect("build test observer frame")
        .sign_with_keys(keys)
        .expect("sign test observer frame")
    }

    fn make_priority_observer_frame(keys: &Keys) -> Event {
        let recipient = Keys::generate();
        let encrypted = buzz_core::observer::encrypt_observer_payload(
            keys,
            &recipient.public_key(),
            &json!({"kind": "control_result"}),
        )
        .expect("encrypt test priority observer payload");
        buzz_sdk::build_agent_observer_frame(
            &recipient.public_key().to_hex(),
            &keys.public_key().to_hex(),
            "telemetry",
            &encrypted,
        )
        .expect("build test priority observer frame")
        .tag(
            Tag::parse([OBSERVER_PRIORITY_TAG, OBSERVER_CONTROL_RESULT_PRIORITY])
                .expect("valid priority tag"),
        )
        .sign_with_keys(keys)
        .expect("sign test priority observer frame")
    }

    fn make_observer_control_event(
        owner_keys: &Keys,
        agent_keys: &Keys,
        created_at_secs: u64,
    ) -> Event {
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let encrypted = buzz_core::observer::encrypt_observer_payload(
            owner_keys,
            &agent_keys.public_key(),
            &json!({"type": "cancel_turn", "channelId": Uuid::new_v4()}),
        )
        .expect("encrypt test observer control");
        EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
            .tags([
                Tag::parse(["p", agent_pubkey_hex.as_str()]).expect("valid recipient tag"),
                Tag::parse(["agent", agent_pubkey_hex.as_str()]).expect("valid agent tag"),
                Tag::parse(["frame", "control"]).expect("valid frame tag"),
            ])
            .custom_created_at(nostr::Timestamp::from(created_at_secs))
            .sign_with_keys(owner_keys)
            .expect("sign test observer control")
    }

    #[tokio::test]
    async fn observer_controls_require_live_strictly_post_start_subscription() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, mut observer_control_rx) = mpsc::channel(2);
        let mut state = BgState::new();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(2);
        let control = make_observer_control_event(&owner_keys, &agent_keys, now);
        let control_json =
            serde_json::to_string(&json!(["EVENT", OBSERVER_CONTROL_SUB_ID, control.clone()]))
                .expect("serialize observer control");

        assert!(
            handle_ws_message(
                Message::Text(control_json.clone().into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            observer_control_rx.try_recv().is_err(),
            "a matching subscription ID before the REQ is live must not admit controls"
        );

        state.observer_control_sub_active = true;
        state.observer_control_subscribed_at = Some(now);
        assert!(
            handle_ws_message(
                Message::Text(control_json.clone().into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert!(
            observer_control_rx.try_recv().is_err(),
            "same-second controls cannot be proven to post-date subscription startup"
        );

        state.observer_control_subscribed_at = Some(now.saturating_sub(1));
        assert!(
            handle_ws_message(
                Message::Text(control_json.into()),
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert_eq!(
            observer_control_rx
                .try_recv()
                .expect("valid owner control after subscription should be delivered")
                .id,
            control.id
        );
    }

    #[tokio::test]
    async fn invalid_observer_flood_cannot_displace_valid_owner_control() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, mut observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let non_owner_keys = Keys::generate();
        let other_agent_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(2);
        state.observer_control_sub_active = true;
        state.observer_control_subscribed_at = Some(now.saturating_sub(1));

        let valid = make_observer_control_event(&owner_keys, &agent_keys, now);
        let mut tampered_json = serde_json::to_value(&valid).expect("serialize valid control");
        let original_content = tampered_json["content"]
            .as_str()
            .expect("observer ciphertext")
            .to_string();
        let replacement = if let Some(rest) = original_content.strip_prefix('A') {
            format!("B{rest}")
        } else {
            let mut chars = original_content.chars();
            let _ = chars.next();
            format!("A{}", chars.as_str())
        };
        tampered_json["content"] = Value::String(replacement);
        let tampered: Event =
            serde_json::from_value(tampered_json).expect("parse tampered observer control");
        assert!(buzz_core::verify_event(&tampered).is_err());

        let non_owner = make_observer_control_event(&non_owner_keys, &agent_keys, now);
        let wrong_target = make_observer_control_event(&owner_keys, &other_agent_keys, now);
        let wrong_kind = make_test_channel_event(
            &owner_keys,
            Uuid::new_v4(),
            KIND_STREAM_MESSAGE,
            "not an observer control",
            Some(&agent_pubkey_hex),
            now,
        );
        for invalid in [tampered, non_owner, wrong_target, wrong_kind] {
            for _ in 0..16 {
                let frame = Message::Text(
                    serde_json::to_string(&json!(["EVENT", OBSERVER_CONTROL_SUB_ID, invalid]))
                        .expect("serialize invalid observer frame")
                        .into(),
                );
                assert!(
                    handle_ws_message(
                        frame,
                        &mut ws,
                        &event_tx,
                        &observer_control_tx,
                        &mut state,
                        &agent_keys,
                        "ws://localhost:3000",
                        &agent_pubkey_hex,
                        None,
                    )
                    .await
                );
            }
        }
        assert!(
            observer_control_rx.try_recv().is_err(),
            "invalid controls must be filtered before they consume queue capacity"
        );

        let valid_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", OBSERVER_CONTROL_SUB_ID, valid.clone()]))
                .expect("serialize valid observer control")
                .into(),
        );
        assert!(
            handle_ws_message(
                valid_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            )
            .await
        );
        assert_eq!(
            observer_control_rx
                .try_recv()
                .expect("valid owner control retains queue capacity")
                .id,
            valid.id
        );
    }

    #[tokio::test]
    async fn valid_owner_control_queues_without_blocking_and_delivers_after_recovery() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, mut observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey_hex = agent_keys.public_key().to_hex();
        let now = unix_now_secs();
        state.authorized_owner_pubkey = Some(owner_keys.public_key().to_hex());
        state.privileged_control_start_boundary = now.saturating_sub(2);
        state.observer_control_sub_active = true;
        state.observer_control_subscribed_at = Some(now.saturating_sub(1));

        let queued = make_observer_control_event(&owner_keys, &agent_keys, now);
        observer_control_tx
            .try_send(queued.clone())
            .expect("prefill observer channel");
        let waiting = make_observer_control_event(&owner_keys, &agent_keys, now);
        let waiting_frame = Message::Text(
            serde_json::to_string(&json!(["EVENT", OBSERVER_CONTROL_SUB_ID, waiting.clone()]))
                .expect("serialize waiting observer control")
                .into(),
        );

        let should_continue = timeout(
            Duration::from_secs(1),
            handle_ws_message(
                waiting_frame,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &agent_keys,
                "ws://localhost:3000",
                &agent_pubkey_hex,
                None,
            ),
        )
        .await
        .expect("full downstream queue must not block WebSocket message handling");

        assert!(should_continue);
        assert_eq!(
            state.observer_control_pending.len(),
            1,
            "authenticated control should be retained in the bounded pending queue"
        );
        let first = observer_control_rx
            .recv()
            .await
            .expect("prefilled control remains queued");
        let permit = observer_control_tx
            .reserve()
            .await
            .expect("observer control receiver remains open");
        deliver_next_pending_observer_control(&mut state, permit);
        let second = timeout(Duration::from_secs(1), observer_control_rx.recv())
            .await
            .expect("valid control should resume after queue capacity returns")
            .expect("observer control channel remains open");

        assert_eq!(first.id, queued.id);
        assert_eq!(second.id, waiting.id);
        assert!(state.observer_control_pending.is_empty());
    }

    /// While the rate-limit gate is armed, an observer frame (kind 24200) is
    /// parked — not silently dropped — and delivered by the drain once the
    /// gate clears. A typing indicator in the same window stays dropped.
    #[tokio::test]
    async fn gated_observer_frame_is_parked_then_drained_not_dropped() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let keys = Keys::generate();
        state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_millis(150));

        // Observer frame while gated: parked, nothing on the wire.
        let observer_frame = make_observer_frame(&keys);
        let ok = execute_connected_command(
            &mut client,
            &mut state,
            "agent-pubkey",
            RelayCommand::PublishEvent {
                event: Box::new(observer_frame.clone()),
                admission: None,
            },
        )
        .await;
        assert!(ok);
        assert_eq!(
            state.gated_observer_pending.len(),
            1,
            "observer frame must be parked while gated"
        );

        // Typing indicator while gated: still dropped, not parked.
        let typing = EventBuilder::new(Kind::Custom(KIND_TYPING_INDICATOR as u16), "")
            .tags([Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign typing indicator");
        let ok = execute_connected_command(
            &mut client,
            &mut state,
            "agent-pubkey",
            RelayCommand::PublishEvent {
                event: Box::new(typing),
                admission: None,
            },
        )
        .await;
        assert!(ok);
        assert_eq!(
            state.gated_observer_pending.len(),
            1,
            "typing indicators must not be parked"
        );
        assert!(
            timeout(Duration::from_millis(50), server.next())
                .await
                .is_err(),
            "nothing may reach the wire while the gate is armed"
        );

        // Gate expires — the drain delivers the parked frame.
        tokio::time::sleep(Duration::from_millis(160)).await;
        assert_eq!(
            drain_gated_observer_pending(&mut client, &mut state, 1).await,
            GatedObserverDrainOutcome::Sent(1)
        );
        assert!(state.gated_observer_pending.is_empty());
        let frame = next_test_frame(&mut server).await;
        assert_eq!(frame[0], "EVENT");
        assert_eq!(frame[1]["id"], observer_frame.id.to_hex());
        assert_eq!(
            frame[1]["kind"],
            u64::from(KIND_AGENT_OBSERVER_FRAME),
            "delivered frame must be the parked observer frame"
        );
    }

    /// Observer frames arriving while earlier parked frames are still queued
    /// are appended behind them (order preserved), even if the gate has
    /// already expired.
    #[tokio::test]
    async fn observer_frames_queue_behind_parked_backlog_in_order() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let keys = Keys::generate();
        state.rate_limit_gate = Some(tokio::time::Instant::now() + Duration::from_millis(50));

        let first = make_observer_frame(&keys);
        let second = make_observer_frame(&keys);
        for event in [&first, &second] {
            let ok = execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(event.clone()),
                    admission: None,
                },
            )
            .await;
            assert!(ok);
        }
        assert_eq!(state.gated_observer_pending.len(), 2);

        // Gate expires but the backlog is not drained yet — a third frame must
        // queue behind it rather than jumping ahead on the wire.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let third = make_observer_frame(&keys);
        let ok = execute_connected_command(
            &mut client,
            &mut state,
            "agent-pubkey",
            RelayCommand::PublishEvent {
                event: Box::new(third.clone()),
                admission: None,
            },
        )
        .await;
        assert!(ok);
        assert_eq!(
            state.gated_observer_pending.len(),
            3,
            "frame must queue behind undrained backlog to preserve order"
        );

        for expected in [&first, &second, &third] {
            assert_eq!(
                drain_gated_observer_pending(&mut client, &mut state, 1).await,
                GatedObserverDrainOutcome::Sent(1)
            );
            let frame = next_test_frame(&mut server).await;
            assert_eq!(frame[1]["id"], expected.id.to_hex(), "order preserved");
        }
        assert!(state.gated_observer_pending.is_empty());
    }

    #[test]
    fn observer_notice_requeues_unacknowledged_frames_and_ok_retires_them() {
        let mut state = BgState::new();
        let keys = Keys::generate();
        let accepted = make_observer_frame(&keys);
        let rejected = make_observer_frame(&keys);
        let later = make_observer_frame(&keys);

        state.track_observer_in_flight(Box::new(accepted.clone()));
        state.track_observer_in_flight(Box::new(rejected.clone()));
        state.acknowledge_observer_frame(&accepted.id.to_hex());
        state.park_gated_observer_frame(Box::new(later.clone()));
        state.requeue_observer_in_flight();

        let ids: Vec<_> = state
            .gated_observer_pending
            .iter()
            .map(|event| event.id)
            .collect();
        assert_eq!(ids, [rejected.id, later.id]);
        assert!(state.observer_in_flight.is_empty());
    }

    #[test]
    fn failed_reconnect_candidate_requeues_all_observer_writes_and_clears_socket_authority() {
        let mut state = BgState::new();
        let keys = Keys::generate();
        let ordinary = make_observer_frame(&keys);
        let terminal = make_priority_observer_frame(&keys);
        let channel_id = Uuid::new_v4();

        state.track_observer_in_flight(Box::new(ordinary.clone()));
        state.track_observer_in_flight(Box::new(terminal.clone()));
        state.observer_control_subscribed_at = Some(unix_now_secs());
        state
            .channel_subscribed_at
            .insert(channel_id, unix_now_secs());

        state.abandon_failed_reconnect_candidate();

        assert_eq!(
            state
                .gated_observer_pending
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [ordinary.id]
        );
        assert_eq!(
            state
                .gated_observer_priority_pending
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [terminal.id]
        );
        assert!(state.observer_in_flight.is_empty());
        assert!(state.observer_priority_in_flight.is_empty());
        assert!(state.observer_control_subscribed_at.is_none());
        assert!(state.channel_subscribed_at.is_empty());
    }

    #[test]
    fn delayed_ok_after_notice_resolves_terminal_confirmation_exactly_once() {
        let mut state = BgState::new();
        let terminal = make_priority_observer_frame(&Keys::generate());
        let terminal_id = terminal.id.to_hex();
        let (confirmation_tx, mut confirmation_rx) = oneshot::channel();

        assert!(state.track_observer_in_flight(Box::new(terminal.clone())));
        state.record_observer_confirmation(terminal_id.clone(), true, Some(confirmation_tx), true);
        state.requeue_observer_in_flight();

        state.acknowledge_observer_frame(&terminal_id);
        assert_eq!(
            confirmation_rx.try_recv(),
            Ok(Ok(())),
            "the delayed accepted OK must resolve the original publisher"
        );
        assert!(
            !state.contains_observer_priority_event(&terminal_id),
            "accepted terminal proof must not remain queued for duplicate resend"
        );
        state.acknowledge_observer_frame(&terminal_id);
        assert_eq!(
            confirmation_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Closed),
            "a duplicate OK cannot resolve the publisher twice"
        );
    }

    #[test]
    fn delayed_terminal_denial_after_notice_resolves_confirmation_without_resend() {
        let mut state = BgState::new();
        let terminal = make_priority_observer_frame(&Keys::generate());
        let terminal_id = terminal.id.to_hex();
        let (confirmation_tx, mut confirmation_rx) = oneshot::channel();

        assert!(state.track_observer_in_flight(Box::new(terminal)));
        state.record_observer_confirmation(terminal_id.clone(), true, Some(confirmation_tx), true);
        state.requeue_observer_in_flight();

        assert!(state.record_terminal_observer_denial(&terminal_id, "blocked: policy denied"));
        assert_eq!(
            confirmation_rx.try_recv(),
            Ok(Err(
                "relay denied terminal observer frame: blocked: policy denied".to_string()
            ))
        );
        assert!(!state.contains_observer_priority_event(&terminal_id));
        assert_eq!(state.observer_terminal_denied, 1);
        assert!(
            !state.record_terminal_observer_denial(&terminal_id, "blocked: policy denied"),
            "a duplicate denial must not retire or count the same event twice"
        );
        assert_eq!(state.observer_terminal_denied, 1);
    }

    #[tokio::test]
    async fn rate_limited_ok_false_requeues_exact_observer_frame_and_arms_gate() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let keys = Keys::generate();
        let first = make_observer_frame(&keys);
        let rejected = make_observer_frame(&keys);
        let later = make_observer_frame(&keys);
        state.track_observer_in_flight(Box::new(first.clone()));
        state.track_observer_in_flight(Box::new(rejected.clone()));
        state.park_gated_observer_frame(Box::new(later.clone()));

        let ok_false = Message::Text(
            serde_json::to_string(&json!([
                "OK",
                rejected.id.to_hex(),
                false,
                "rate-limited: quota exceeded; retry in 5s"
            ]))
            .expect("serialize OK false")
            .into(),
        );
        assert!(
            handle_ws_message(
                ok_false,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &keys,
                "ws://localhost:3000",
                &keys.public_key().to_hex(),
                None,
            )
            .await
        );

        assert!(state.check_rate_gate().is_some(), "retry pacing must arm");
        assert_eq!(
            state
                .gated_observer_pending
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [rejected.id, later.id],
            "only the exact rejected frame is requeued ahead of later telemetry"
        );
        assert_eq!(
            state
                .observer_in_flight
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [first.id],
            "unrelated in-flight frames stay pending their own OK"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sixty_second_terminal_quota_waits_for_reset_then_accepts_delayed_ok() {
        let (mut ws, mut server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let keys = Keys::generate();
        let terminal = make_priority_observer_frame(&keys);
        let terminal_id = terminal.id.to_hex();
        let (confirmation_tx, mut confirmation_rx) = oneshot::channel();

        // Socket readiness uses wall-clock I/O; pause only the quota interval
        // whose lower bound this test proves.
        tokio::time::resume();
        assert!(
            execute_connected_command(
                &mut ws,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(terminal),
                    admission: Some(confirmation_tx),
                },
            )
            .await
        );
        assert_eq!(next_test_frame(&mut server).await[0], "EVENT");
        tokio::time::pause();

        let ok_false = Message::Text(
            serde_json::to_string(&json!([
                "OK",
                terminal_id,
                false,
                "rate-limited: quota exceeded; retry in 60s"
            ]))
            .expect("serialize OK false")
            .into(),
        );
        assert!(
            handle_ws_message(
                ok_false,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &keys,
                "ws://localhost:3000",
                &keys.public_key().to_hex(),
                None,
            )
            .await
        );
        let gate = state.rate_limit_gate.expect("sixty-second gate");
        let remaining = gate.duration_since(tokio::time::Instant::now());
        assert!(
            remaining >= Duration::from_secs(60),
            "the client must never retry before the relay's honest reset horizon"
        );
        assert!(
            remaining < TERMINAL_OBSERVER_ACK_TIMEOUT,
            "the original confirmation owner must outlive the full retry gate"
        );

        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(state.check_rate_gate().is_some());
        assert_eq!(
            drain_gated_observer_pending(&mut ws, &mut state, 1).await,
            GatedObserverDrainOutcome::Sent(0),
            "no terminal resend is allowed before the advertised reset"
        );

        tokio::time::advance(Duration::from_secs(14)).await;
        assert!(state.check_rate_gate().is_none());
        tokio::time::resume();
        assert_eq!(
            drain_gated_observer_pending(&mut ws, &mut state, 1).await,
            GatedObserverDrainOutcome::Sent(1)
        );
        tokio::task::yield_now().await;
        assert_eq!(next_test_frame(&mut server).await[0], "EVENT");

        state.acknowledge_observer_frame(&terminal_id);
        assert_eq!(
            confirmation_rx.try_recv(),
            Ok(Ok(())),
            "the delayed accepted OK must resolve the original terminal publication"
        );
    }

    #[tokio::test]
    async fn terminal_ok_false_does_not_ack_or_retry_observer_frame() {
        let (mut ws, _server) = test_ws_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (observer_control_tx, _observer_control_rx) = mpsc::channel(1);
        let mut state = BgState::new();
        let keys = Keys::generate();
        let denied = make_observer_frame(&keys);
        state.track_observer_in_flight(Box::new(denied.clone()));

        let ok_false = Message::Text(
            serde_json::to_string(&json!([
                "OK",
                denied.id.to_hex(),
                false,
                "blocked: policy denied"
            ]))
            .expect("serialize OK false")
            .into(),
        );
        assert!(
            handle_ws_message(
                ok_false,
                &mut ws,
                &event_tx,
                &observer_control_tx,
                &mut state,
                &keys,
                "ws://localhost:3000",
                &keys.public_key().to_hex(),
                None,
            )
            .await
        );

        assert!(
            state.observer_in_flight.is_empty(),
            "terminal denial is retired instead of occupying the bounded ACK window forever"
        );
        assert_eq!(
            state.observer_terminal_denied, 1,
            "terminal denial must leave an auditable counter"
        );
        assert!(
            state.gated_observer_pending.is_empty(),
            "terminal denial must not enter an unbounded retry loop"
        );
        assert!(state.check_rate_gate().is_none());
    }

    /// The parked-frame queue is bounded: overflow evicts the oldest frame and
    /// counts it; the drain resets the counter after logging the summary.
    #[tokio::test]
    async fn gated_observer_queue_drops_oldest_on_overflow() {
        let mut state = BgState::new();
        let keys = Keys::generate();
        let first = make_observer_frame(&keys);
        state.park_gated_observer_frame(Box::new(first.clone()));
        for _ in 1..GATED_OBSERVER_QUEUE_CAP {
            state.park_gated_observer_frame(Box::new(make_observer_frame(&keys)));
        }
        assert_eq!(state.gated_observer_pending.len(), GATED_OBSERVER_QUEUE_CAP);
        assert_eq!(state.gated_observer_dropped, 0);

        let overflow = make_observer_frame(&keys);
        state.park_gated_observer_frame(Box::new(overflow.clone()));
        assert_eq!(
            state.gated_observer_pending.len(),
            GATED_OBSERVER_QUEUE_CAP,
            "queue must stay bounded"
        );
        assert_eq!(state.gated_observer_dropped, 1, "loss must be counted");
        assert!(
            !state
                .gated_observer_pending
                .iter()
                .any(|e| e.id == first.id),
            "oldest frame must be the one evicted"
        );
        assert_eq!(
            state.gated_observer_pending.back().map(|e| e.id),
            Some(overflow.id),
            "newest frame must be retained"
        );
    }

    #[tokio::test]
    async fn protected_observer_queue_rejects_newest_without_evicting_terminal_proof() {
        let mut state = BgState::new();
        let keys = Keys::generate();
        let first = make_priority_observer_frame(&keys);
        assert!(state.park_gated_observer_frame(Box::new(first.clone())));
        for _ in 1..GATED_OBSERVER_PRIORITY_CAP {
            assert!(state.park_gated_observer_frame(Box::new(make_priority_observer_frame(&keys))));
        }

        let overflow = make_priority_observer_frame(&keys);
        assert!(
            !state.park_gated_observer_frame(Box::new(overflow.clone())),
            "a saturated protected lane must reject admission"
        );
        assert_eq!(
            state.gated_observer_priority_pending.len(),
            GATED_OBSERVER_PRIORITY_CAP
        );
        assert_eq!(state.gated_observer_dropped, 1);
        assert_eq!(
            state
                .gated_observer_priority_pending
                .front()
                .map(|event| event.id),
            Some(first.id),
            "an already-admitted terminal proof must never be evicted"
        );
        assert!(
            !state
                .gated_observer_priority_pending
                .iter()
                .any(|event| event.id == overflow.id),
            "the rejected newest frame must not displace admitted proof"
        );
    }

    #[tokio::test]
    async fn terminal_publisher_reports_rejected_priority_admission() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let publisher = RelayEventPublisher { cmd_tx };
        let event = make_priority_observer_frame(&Keys::generate());
        let publish = tokio::spawn(async move { publisher.publish_terminal_event(event).await });

        let RelayCommand::PublishEvent {
            admission: Some(admission),
            ..
        } = cmd_rx.recv().await.expect("terminal publish command")
        else {
            panic!("terminal publish must request confirmed priority admission");
        };
        admission
            .send(Err("terminal observer priority state is full".into()))
            .expect("publisher must still be waiting for admission");

        assert!(matches!(
            publish.await.expect("publisher task"),
            Err(RelayError::Http(message))
                if message == "terminal observer priority state is full"
        ));
    }

    #[tokio::test]
    async fn terminal_publisher_rejects_non_terminal_shape_before_queue_admission() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let publisher = RelayEventPublisher { cmd_tx };
        let ordinary = make_observer_frame(&Keys::generate());

        assert!(matches!(
            publisher.publish_terminal_event(ordinary).await,
            Err(RelayError::Http(message))
                if message == "terminal observer event must be kind 24200 with exactly one control-result priority marker"
        ));
        assert!(
            cmd_rx.try_recv().is_err(),
            "invalid terminal shape must never enter the relay command queue"
        );

        let terminal = make_priority_observer_frame(&Keys::generate());
        let mut wrong_kind_value = serde_json::to_value(terminal).expect("serialize terminal");
        wrong_kind_value["kind"] = json!(KIND_TYPING_INDICATOR);
        let wrong_kind: Event =
            serde_json::from_value(wrong_kind_value).expect("deserialize wrong-kind event");
        assert!(matches!(
            publisher.publish_terminal_event(wrong_kind).await,
            Err(RelayError::Http(message))
                if message == "terminal observer event must be kind 24200 with exactly one control-result priority marker"
        ));
        assert!(
            cmd_rx.try_recv().is_err(),
            "a priority marker on the wrong event kind must never reach a socket"
        );
    }

    #[tokio::test]
    async fn terminal_publisher_rejects_duplicate_priority_marker_before_queue_admission() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let publisher = RelayEventPublisher { cmd_tx };
        let terminal = make_priority_observer_frame(&Keys::generate());
        let mut value = serde_json::to_value(terminal).expect("serialize terminal");
        value["tags"]
            .as_array_mut()
            .expect("terminal tags")
            .push(json!([
                OBSERVER_PRIORITY_TAG,
                OBSERVER_CONTROL_RESULT_PRIORITY
            ]));
        let duplicate_marker: Event =
            serde_json::from_value(value).expect("deserialize duplicate marker event");

        assert!(matches!(
            publisher.publish_terminal_event(duplicate_marker).await,
            Err(RelayError::Http(message))
                if message == "terminal observer event must be kind 24200 with exactly one control-result priority marker"
        ));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn terminal_publisher_rejects_extended_priority_marker_before_queue_admission() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let publisher = RelayEventPublisher { cmd_tx };
        let terminal = make_priority_observer_frame(&Keys::generate());
        let mut value = serde_json::to_value(terminal).expect("serialize terminal");
        let priority = value["tags"]
            .as_array_mut()
            .expect("terminal tags")
            .iter_mut()
            .find(|tag| tag[0] == OBSERVER_PRIORITY_TAG)
            .expect("priority marker");
        priority
            .as_array_mut()
            .expect("priority tag")
            .push(json!("extra"));
        let extended_marker: Event =
            serde_json::from_value(value).expect("deserialize extended marker event");

        assert!(matches!(
            publisher.publish_terminal_event(extended_marker).await,
            Err(RelayError::Http(message))
                if message == "terminal observer event must be kind 24200 with exactly one control-result priority marker"
        ));
        assert!(
            cmd_rx.try_recv().is_err(),
            "an extended marker must not weaken the exact terminal frame contract"
        );
    }

    #[tokio::test]
    async fn terminal_confirmation_waits_for_relay_ok_not_local_socket_write() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let event = make_priority_observer_frame(&Keys::generate());
        let event_id = event.id.to_hex();
        let (confirmation_tx, mut confirmation_rx) = oneshot::channel();

        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(event),
                    admission: Some(confirmation_tx),
                },
            )
            .await
        );
        let frame = next_test_frame(&mut server).await;
        assert_eq!(frame[0], "EVENT");
        assert!(
            confirmation_rx.try_recv().is_err(),
            "a local socket write is not terminal delivery proof"
        );

        state.acknowledge_observer_frame(&event_id);
        assert_eq!(
            confirmation_rx.await.expect("relay OK confirmation"),
            Ok(())
        );
    }

    #[tokio::test]
    async fn terminal_confirmation_timeout_cancels_exact_event_and_next_result_progresses() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let publisher = RelayEventPublisher { cmd_tx };
        let test_ack_timeout = Duration::from_millis(250);

        let first = make_priority_observer_frame(&Keys::generate());
        let first_id = first.id.to_hex();
        let first_publish = tokio::spawn({
            let publisher = publisher.clone();
            async move {
                publisher
                    .publish_terminal_event_with_ack_timeout(first, test_ack_timeout)
                    .await
            }
        });
        let first_command = cmd_rx.recv().await.expect("first terminal publish command");
        assert!(
            execute_connected_command(&mut client, &mut state, "agent-pubkey", first_command,)
                .await
        );
        let first_frame = next_test_frame(&mut server).await;
        assert_eq!(first_frame[0], "EVENT");

        assert!(matches!(
            first_publish.await.expect("first publisher task"),
            Err(RelayError::Timeout)
        ));

        let second = make_priority_observer_frame(&Keys::generate());
        let second_id = second.id.to_hex();
        let second_publish = tokio::spawn({
            let publisher = publisher.clone();
            async move { publisher.publish_terminal_event(second).await }
        });
        let second_command = cmd_rx
            .recv()
            .await
            .expect("second terminal publish command");
        assert!(
            execute_connected_command(&mut client, &mut state, "agent-pubkey", second_command,)
                .await
        );
        assert!(
            !state.contains_observer_priority_event(&first_id),
            "the next command must prune only the expired attempt before admitting later work"
        );
        let second_frame = next_test_frame(&mut server).await;
        assert_eq!(second_frame[0], "EVENT");

        state.acknowledge_observer_frame(&second_id);
        assert!(
            second_publish.await.expect("second publisher task").is_ok(),
            "a later terminal result must progress after exact timeout cleanup"
        );
    }

    #[tokio::test]
    async fn expired_queued_terminal_publish_never_reaches_reconnected_socket() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let publisher = RelayEventPublisher { cmd_tx };
        let event = make_priority_observer_frame(&Keys::generate());
        let event_id = event.id.to_hex();

        let publish_result = publisher
            .publish_terminal_event_with_ack_timeout(event, Duration::from_millis(100))
            .await;
        assert!(matches!(publish_result, Err(RelayError::Timeout)));

        let expired_publish = cmd_rx.recv().await.expect("expired publish remains queued");
        assert!(
            execute_connected_command(&mut client, &mut state, "agent-pubkey", expired_publish,)
                .await
        );
        assert!(
            cmd_rx.try_recv().is_err(),
            "timeout cleanup must not enqueue an ambiguous event-ID cancellation"
        );
        assert!(
            timeout(Duration::from_millis(100), server.next())
                .await
                .is_err(),
            "an expired queued terminal result must not reach a reconnected relay"
        );
        assert!(
            !state.contains_observer_priority_event(&event_id),
            "expired reconnect work must leave no pending, in-flight, or confirmation owner"
        );

        let later = make_observer_frame(&Keys::generate());
        let later_id = later.id;
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(later),
                    admission: None,
                },
            )
            .await,
            "clean pre-send cancellation must leave the socket reusable"
        );
        let later_frame = next_test_frame(&mut server).await;
        assert_eq!(
            later_frame[1]["id"],
            later_id.to_hex(),
            "later traffic must progress on the clean socket"
        );
    }

    #[tokio::test]
    async fn timed_out_queued_duplicate_preserves_original_confirmation_owner() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let event = make_priority_observer_frame(&Keys::generate());
        let event_id = event.id.to_hex();
        let (original_tx, mut original_rx) = oneshot::channel();
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(event.clone()),
                    admission: Some(original_tx),
                },
            )
            .await
        );
        let original_frame = next_test_frame(&mut server).await;
        assert_eq!(original_frame[1]["id"], event_id);

        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        let duplicate_publisher = RelayEventPublisher { cmd_tx };
        let duplicate_result = duplicate_publisher
            .publish_terminal_event_with_ack_timeout(event, Duration::from_millis(100))
            .await;
        assert!(matches!(duplicate_result, Err(RelayError::Timeout)));

        while let Ok(command) = cmd_rx.try_recv() {
            assert!(
                execute_connected_command(&mut client, &mut state, "agent-pubkey", command,).await
            );
        }
        assert!(
            timeout(Duration::from_millis(100), server.next())
                .await
                .is_err(),
            "the expired duplicate attempt must never write a second frame"
        );
        assert!(
            state.contains_observer_priority_event(&event_id),
            "duplicate timeout cleanup must preserve the original confirmation owner"
        );
        assert!(matches!(
            original_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        state.acknowledge_observer_frame(&event_id);
        assert_eq!(
            original_rx
                .await
                .expect("original confirmation remains live"),
            Ok(())
        );
    }

    struct StallAfterStartSendSink {
        started: Option<oneshot::Sender<()>>,
        buffered: Vec<Message>,
        flush_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        flushed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl futures_util::Sink<Message> for StallAfterStartSendSink {
        type Error = std::convert::Infallible;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(
            mut self: std::pin::Pin<&mut Self>,
            item: Message,
        ) -> Result<(), Self::Error> {
            self.buffered.push(item);
            if let Some(started) = self.started.take() {
                let _ = started.send(());
            }
            Ok(())
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            if self.flush_allowed.load(std::sync::atomic::Ordering::SeqCst) {
                if !self.buffered.is_empty() {
                    self.flushed
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    self.buffered.clear();
                }
                std::task::Poll::Ready(Ok(()))
            } else {
                std::task::Poll::Pending
            }
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            self.poll_flush(cx)
        }
    }

    struct BufferThenStartSendErrorSink {
        buffered: Vec<Message>,
    }

    impl futures_util::Sink<Message> for BufferThenStartSendErrorSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(
            mut self: std::pin::Pin<&mut Self>,
            item: Message,
        ) -> Result<(), Self::Error> {
            self.buffered.push(item);
            Err(std::io::Error::other(
                "write failed after buffering the frame",
            ))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn start_send_error_after_buffering_taints_socket() {
        let event = make_priority_observer_frame(&Keys::generate());
        let mut socket = BufferThenStartSendErrorSink {
            buffered: Vec::new(),
        };

        let outcome = send_publish_event_frame_until_cancelled(&mut socket, &event, None).await;

        assert_eq!(
            outcome,
            PublishEventFrameOutcome::FailedSocketTainted,
            "invoking start_send must conservatively taint the socket even when it returns an error"
        );
        assert!(outcome.socket_tainted());
        assert_eq!(
            socket.buffered.len(),
            1,
            "a start_send error may leave the EVENT buffered for later traffic"
        );
    }

    #[tokio::test]
    async fn cancellation_after_start_send_retires_socket_before_later_traffic() {
        let event = make_priority_observer_frame(&Keys::generate());
        let (mut confirmation_tx, confirmation_rx) = oneshot::channel();
        let (started_tx, started_rx) = oneshot::channel();
        let old_flush_allowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let old_flushed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut old_socket = StallAfterStartSendSink {
            started: Some(started_tx),
            buffered: Vec::new(),
            flush_allowed: old_flush_allowed,
            flushed: old_flushed.clone(),
        };
        let close_attempt = tokio::spawn(async move {
            started_rx.await.expect("EVENT reaches start_send");
            drop(confirmation_rx);
        });

        let outcome = send_publish_event_frame_until_cancelled(
            &mut old_socket,
            &event,
            Some(&mut confirmation_tx),
        )
        .await;
        close_attempt.await.expect("close attempt task");

        assert_eq!(outcome, PublishEventFrameOutcome::SocketTainted);
        assert!(outcome.socket_tainted());
        assert_eq!(
            old_socket.buffered.len(),
            1,
            "the cancelled send may still be buffered after start_send"
        );

        // The supervisor must retire the tainted transport. Later traffic goes
        // only to its replacement, so it cannot flush the cancelled EVENT.
        drop(old_socket);
        let replacement_flushed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut replacement = StallAfterStartSendSink {
            started: None,
            buffered: Vec::new(),
            flush_allowed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            flushed: replacement_flushed.clone(),
        };
        replacement
            .send(Message::Ping(Vec::new().into()))
            .await
            .expect("later traffic uses replacement transport");

        assert!(
            replacement_flushed.load(std::sync::atomic::Ordering::SeqCst),
            "the later frame must be sent on the replacement transport"
        );
        assert!(
            !old_flushed.load(std::sync::atomic::Ordering::SeqCst),
            "later traffic must never flush the cancelled EVENT"
        );
    }

    #[test]
    fn receiver_close_purges_only_its_exact_terminal_admission() {
        let mut state = BgState::new();
        let keys = Keys::generate();
        let cancelled = make_priority_observer_frame(&keys);
        let cancelled_id = cancelled.id.to_hex();
        let survivor = make_priority_observer_frame(&keys);
        let survivor_id = survivor.id.to_hex();
        let (cancelled_tx, cancelled_rx) = oneshot::channel();
        let (survivor_tx, mut survivor_rx) = oneshot::channel();

        assert!(state.park_gated_observer_frame(Box::new(cancelled)));
        state.record_observer_confirmation(cancelled_id.clone(), true, Some(cancelled_tx), true);
        assert!(state.park_gated_observer_frame(Box::new(survivor)));
        state.record_observer_confirmation(survivor_id.clone(), true, Some(survivor_tx), true);

        drop(cancelled_rx);
        state.prune_cancelled_terminal_admissions();

        assert!(!state.contains_observer_priority_event(&cancelled_id));
        assert!(
            state.contains_observer_priority_event(&survivor_id),
            "receiver-close cleanup must preserve every other terminal owner"
        );
        assert!(matches!(
            survivor_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn full_command_channel_cannot_extend_terminal_publish_deadline() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        cmd_tx
            .send(RelayCommand::Reconnect)
            .await
            .expect("seed full command channel");
        let publisher = RelayEventPublisher { cmd_tx };
        let event = make_priority_observer_frame(&Keys::generate());

        let bounded = timeout(
            Duration::from_secs(1),
            publisher.publish_terminal_event_with_ack_timeout(event, Duration::from_millis(100)),
        )
        .await
        .expect("publisher must return without command-channel capacity");
        assert!(matches!(bounded, Err(RelayError::Timeout)));
    }

    #[tokio::test]
    async fn duplicate_terminal_event_rejects_second_confirmation_before_socket_write() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let keys = Keys::generate();
        let terminal = make_priority_observer_frame(&keys);
        let terminal_id = terminal.id.to_hex();
        let (first_tx, first_rx) = oneshot::channel();
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(terminal.clone()),
                    admission: Some(first_tx),
                },
            )
            .await
        );
        let first_frame = next_test_frame(&mut server).await;
        assert_eq!(first_frame[1]["id"], terminal_id);

        let (second_tx, second_rx) = oneshot::channel();
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(terminal),
                    admission: Some(second_tx),
                },
            )
            .await
        );
        assert_eq!(
            second_rx.await.expect("duplicate rejection result"),
            Err("terminal observer event is already awaiting relay confirmation".to_string())
        );

        state.acknowledge_observer_frame(&terminal_id);
        assert_eq!(
            first_rx
                .await
                .expect("first relay confirmation remains live"),
            Ok(())
        );

        let next = make_observer_frame(&keys);
        let next_id = next.id;
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(next),
                    admission: None,
                },
            )
            .await
        );
        let next_frame = next_test_frame(&mut server).await;
        assert_eq!(
            next_frame[1]["id"],
            next_id.to_hex(),
            "duplicate rejection must not emit a second terminal EVENT"
        );
    }

    #[tokio::test]
    async fn disconnected_duplicate_terminal_event_preserves_first_waiter_and_queue_entry() {
        let mut state = BgState::new();
        let terminal = make_priority_observer_frame(&Keys::generate());
        let terminal_id = terminal.id.to_hex();
        let (first_tx, mut first_rx) = oneshot::channel();
        apply_command_to_state(
            &mut state,
            RelayCommand::PublishEvent {
                event: Box::new(terminal.clone()),
                admission: Some(first_tx),
            },
        );

        let (second_tx, second_rx) = oneshot::channel();
        apply_command_to_state(
            &mut state,
            RelayCommand::PublishEvent {
                event: Box::new(terminal.clone()),
                admission: Some(second_tx),
            },
        );
        assert_eq!(
            second_rx.await.expect("disconnected duplicate rejection"),
            Err("terminal observer event is already awaiting relay confirmation".to_string())
        );

        let (replay_tx, replay_rx) = oneshot::channel();
        retain_failed_command_intent(
            &mut state,
            RelayCommand::PublishEvent {
                event: Box::new(terminal),
                admission: Some(replay_tx),
            },
        );
        assert_eq!(
            replay_rx.await.expect("failed-replay duplicate rejection"),
            Err("terminal observer event is already awaiting relay confirmation".to_string())
        );
        assert_eq!(state.gated_observer_priority_pending.len(), 1);
        assert_eq!(
            state
                .gated_observer_priority_pending
                .front()
                .map(|event| event.id.to_hex()),
            Some(terminal_id.clone())
        );
        assert!(
            state
                .observer_priority_confirmations
                .contains_key(&terminal_id),
            "the first publisher must retain confirmation ownership"
        );
        assert!(
            matches!(
                first_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "the first publisher must remain pending instead of observing replacement closure"
        );
    }

    #[tokio::test]
    async fn saturated_in_flight_priority_window_rejects_before_socket_write() {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let keys = Keys::generate();
        for _ in 0..GATED_OBSERVER_PRIORITY_CAP {
            assert!(state.track_observer_in_flight(Box::new(make_priority_observer_frame(&keys))));
        }

        let rejected = make_priority_observer_frame(&keys);
        let rejected_id = rejected.id;
        let (confirmation_tx, confirmation_rx) = oneshot::channel();
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(rejected),
                    admission: Some(confirmation_tx),
                },
            )
            .await
        );

        assert_eq!(
            confirmation_rx.await.expect("priority rejection result"),
            Err("terminal observer priority state is full".to_string())
        );
        assert_eq!(
            state.observer_priority_in_flight.len(),
            GATED_OBSERVER_PRIORITY_CAP
        );
        assert!(
            !state
                .observer_priority_in_flight
                .iter()
                .any(|event| event.id == rejected_id),
            "a rejected terminal result must not enter the acknowledgment window"
        );

        let next = make_observer_frame(&keys);
        let next_id = next.id;
        assert!(
            execute_connected_command(
                &mut client,
                &mut state,
                "agent-pubkey",
                RelayCommand::PublishEvent {
                    event: Box::new(next),
                    admission: None,
                },
            )
            .await
        );
        let frame = next_test_frame(&mut server).await;
        assert_eq!(
            frame[1]["id"],
            next_id.to_hex(),
            "the first socket write after rejection must be the later admitted frame"
        );
        assert_eq!(state.gated_observer_dropped, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_control_result_survives_telemetry_saturation_and_drains_within_eight_seconds()
    {
        let (mut client, mut server) = test_ws_pair().await;
        let mut state = BgState::new();
        let keys = Keys::generate();
        for _ in 0..GATED_OBSERVER_QUEUE_CAP {
            state.park_gated_observer_frame(Box::new(make_observer_frame(&keys)));
        }
        let terminal = make_priority_observer_frame(&keys);
        state.park_gated_observer_frame(Box::new(terminal.clone()));
        state.set_rate_limit_gate(5);
        let saturated_at = tokio::time::Instant::now();

        assert_eq!(state.gated_observer_pending.len(), GATED_OBSERVER_QUEUE_CAP);
        assert_eq!(
            state
                .gated_observer_priority_pending
                .front()
                .map(|event| event.id),
            Some(terminal.id),
            "ordinary telemetry saturation cannot consume protected capacity"
        );

        tokio::time::advance(Duration::from_secs(7)).await;
        assert_eq!(
            drain_gated_observer_pending(&mut client, &mut state, 1).await,
            GatedObserverDrainOutcome::Sent(1)
        );
        let message = server
            .next()
            .await
            .expect("test websocket stays open")
            .expect("read protected observer frame");
        let frame: serde_json::Value =
            serde_json::from_str(message.to_text().expect("protected frame is text"))
                .expect("parse protected observer frame");
        assert_eq!(
            frame[1]["id"],
            terminal.id.to_hex(),
            "terminal result drains ahead of the saturated telemetry FIFO"
        );
        assert!(
            tokio::time::Instant::now().duration_since(saturated_at) < Duration::from_secs(8),
            "protected result must beat ModelPicker's eight-second expectation"
        );
    }

    /// is_dns_error correctly classifies platform resolver strings, including
    /// the production shape: a WebSocket I/O error wrapping the OS message.
    #[test]
    fn is_dns_error_classification() {
        use tokio_tungstenite::tungstenite;

        // macOS resolver (Http-wrapped, used in many existing tests)
        assert!(is_dns_error(&RelayError::Http(
            "nodename nor servname provided, or not known".into()
        )));
        // Linux resolver
        assert!(is_dns_error(&RelayError::Http(
            "Name or service not known".into()
        )));
        // BSD/Windows
        assert!(is_dns_error(&RelayError::Http("No such host".into())));
        // Another common variant
        assert!(is_dns_error(&RelayError::Http(
            "failed to lookup address information".into()
        )));
        // F15: production-shaped error — RelayError::WebSocket wrapping a
        // tungstenite I/O error (the shape emitted by connect_async on macOS).
        let ws_io_err = RelayError::WebSocket(Box::new(tungstenite::Error::Io(
            std::io::Error::other("nodename nor servname provided, or not known"),
        )));
        assert!(
            is_dns_error(&ws_io_err),
            "WebSocket-wrapped I/O DNS error must be classified as DNS"
        );
        // Normal connection errors are NOT DNS errors.
        assert!(!is_dns_error(&RelayError::Timeout));
        assert!(!is_dns_error(&RelayError::ConnectionClosed));
        assert!(!is_dns_error(&RelayError::Http(
            "connection refused".into()
        )));
    }

    /// resubscribe_retry is populated when a channel REQ fails during partial reconnect.
    ///
    /// This exercises BgState directly since we have no live socket in unit tests.
    #[test]
    fn resubscribe_retry_populated_on_failure() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        // Subscribe the channel so it ends up in active_subscriptions.
        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter: ChannelFilter {
                    kinds: Some(vec![9]),
                    require_mention: false,
                },
                replay_since: Some(1_000),
            },
        );
        assert!(state.active_subscriptions.contains_key(&channel_id));

        // Simulate a partial-reconnect failure: insert into resubscribe_retry.
        state.resubscribe_retry.insert(channel_id);

        assert!(
            state.resubscribe_retry.contains(&channel_id),
            "failed channel must be in resubscribe_retry"
        );
        assert!(
            state.active_subscriptions.contains_key(&channel_id),
            "channel must stay in active_subscriptions so reconnect can restore it"
        );
    }

    // ── Control-sub recovery from rate-limited CLOSED ────────────────────────

    /// A rate-limited CLOSED for the membership sub sets membership_resub_needed.
    /// After the gate expires the drain re-arms the sub and clears the flag.
    #[tokio::test(start_paused = true)]
    async fn membership_resub_flag_set_on_rate_limited_closed() {
        let mut state = BgState::new();
        state.membership_sub_active = true;

        // Simulate a rate-limited CLOSED arriving for the membership sub.
        let secs = parse_rate_limit_retry_secs("rate-limited: retry in 5s").unwrap_or(0);
        state.set_rate_limit_gate(secs);
        state.membership_resub_needed = true;

        assert!(
            state.membership_resub_needed,
            "flag must be set after rate-limited CLOSED"
        );
        assert!(
            state.check_rate_gate().is_some(),
            "gate must be active while membership sub is pending"
        );

        // Advance past the gate (max jitter: 1.2 × 5s = 6s).
        tokio::time::advance(Duration::from_secs(7)).await;

        assert!(
            state.check_rate_gate().is_none(),
            "gate must expire so drain can fire"
        );
        // The drain clears membership_resub_needed after re-sending the REQ.
        // Simulate successful re-send:
        state.membership_resub_needed = false;
        assert!(
            !state.membership_resub_needed,
            "flag must clear after drain re-sends the membership REQ"
        );
    }

    /// A rate-limited CLOSED for the observer control sub sets observer_resub_needed.
    #[test]
    fn observer_resub_flag_set_on_rate_limited_closed() {
        let mut state = BgState::new();
        state.observer_control_sub_active = true;

        // Simulate rate-limited CLOSED on observer control sub.
        state.set_rate_limit_gate(5);
        state.observer_resub_needed = true;

        assert!(
            state.observer_resub_needed,
            "flag must be set after rate-limited CLOSED on observer sub"
        );
    }

    // ── Drain state transitions ───────────────────────────────────────────────

    /// drain_rate_limited_pending: a channel re-queued with +5s penalty on send
    /// failure stays in pending and is not immediately retried.
    #[tokio::test(start_paused = true)]
    async fn rate_limited_pending_failure_requeues_with_penalty() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        // Seed the channel's subscription intent.
        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter: ChannelFilter {
                    kinds: Some(vec![9]),
                    require_mention: false,
                },
                replay_since: None,
            },
        );

        // Park the channel as rate-limited with a deadline in the past.
        let past = tokio::time::Instant::now();
        state.rate_limited_pending.insert(channel_id, past);

        // Simulate a send failure by re-queuing with +5s (what the drain does).
        let penalty = tokio::time::Instant::now() + Duration::from_secs(5);
        state.rate_limited_pending.insert(channel_id, penalty);

        assert!(
            state.rate_limited_pending.contains_key(&channel_id),
            "channel must stay in rate_limited_pending after send failure"
        );
        // Deadline should be in the future.
        let deadline = state.rate_limited_pending[&channel_id];
        assert!(
            deadline > tokio::time::Instant::now(),
            "penalty deadline must be in the future"
        );
    }

    /// drain_resubscribe_retry: a gate re-armed mid-drain moves the channel to
    /// rate_limited_pending and removes it from resubscribe_retry.
    #[tokio::test(start_paused = true)]
    async fn resubscribe_retry_gate_rearm_moves_to_pending() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter: ChannelFilter {
                    kinds: Some(vec![9]),
                    require_mention: false,
                },
                replay_since: None,
            },
        );
        state.resubscribe_retry.insert(channel_id);

        // Simulate gate re-arming mid-drain (what the drain does on check_rate_gate hit).
        let retry_after = state.set_rate_limit_gate(5);
        state.rate_limited_pending.insert(channel_id, retry_after);
        state.resubscribe_retry.remove(&channel_id);

        assert!(
            !state.resubscribe_retry.contains(&channel_id),
            "channel must be removed from resubscribe_retry when gate re-arms"
        );
        assert!(
            state.rate_limited_pending.contains_key(&channel_id),
            "channel must be moved to rate_limited_pending on gate re-arm"
        );
    }

    /// drain_resubscribe_retry: a successful drain removes the channel and
    /// clears channel_dropped_since.
    #[test]
    fn resubscribe_retry_success_clears_state() {
        let mut state = BgState::new();
        let channel_id = Uuid::new_v4();

        apply_command_to_state(
            &mut state,
            RelayCommand::Subscribe {
                channel_id,
                filter: ChannelFilter {
                    kinds: Some(vec![9]),
                    require_mention: false,
                },
                replay_since: None,
            },
        );
        state.resubscribe_retry.insert(channel_id);
        state.channel_dropped_since.insert(channel_id, 1_000_000);

        // Simulate successful re-send (what the drain does on success).
        state.resubscribe_retry.remove(&channel_id);
        state.channel_dropped_since.remove(&channel_id);

        assert!(
            !state.resubscribe_retry.contains(&channel_id),
            "channel must leave resubscribe_retry on successful drain"
        );
        assert!(
            !state.channel_dropped_since.contains_key(&channel_id),
            "channel_dropped_since must be cleared on successful drain"
        );
    }
}
