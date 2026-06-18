//! Rust-owned relay-mesh coordinator.
//!
//! Owns the control plane that used to live in React (`useMeshRelayOrchestrator`)
//! and the TS start helper (`startRelayMeshClientForTarget`). One runtime-owned
//! actor, started when the desktop's relay identity is set — independent of any
//! UI mount or mesh activity. This is what kills the cold-launch race: the
//! call-me-now listener's lifetime is tied to the runtime, not to a renderer
//! mounting with a pubkey.
//!
//! Two responsibilities:
//!   1. `spawn_listener` — a long-lived task that holds an authenticated WS to
//!      Buzz's relay (generalizing the proven `commands::pairing` NIP-42
//!      machinery), subscribes `kind:24622 #p=self`, and dials each paired
//!      call-me-now back into the local mesh runtime. Idempotent: one listener
//!      per process; re-entrant calls return the live handle.
//!   2. `start_client` — publishes a `kind:24621` connect-request with a fresh
//!      attempt id and drives bounded publish+dial retry inside the relay's 60s
//!      call-me-now TTL. Both fresh-create and saved/restore start paths route
//!      through here, so there is no behavioral fork.
//!
//! `start_client` blocks on `listener_active` being `true` before publishing,
//! so a 24621 can never be emitted before this desktop is able to receive its
//! own paired 24622. The race is closed by construction, not convention.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::JsonUtil;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::sync::watch;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use buzz_core_pkg::kind::{
    KIND_MESH_CALL_ME_NOW, KIND_MESH_CONNECT_REQUEST, KIND_MESH_STATUS_REPORT,
};

use crate::app_state::AppState;

/// Relay's call-me-now TTL. Retry deadline lives inside this window: once the
/// paired 24622 expires, re-dialing the same attempt is pointless.
const CALL_ME_NOW_TTL: Duration = Duration::from_secs(60);
/// Backoff between connect-request attempts. Hole-punch is lossy; a single
/// publish is flaky by construction even when both ends are correct.
const RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// Handle to the runtime-owned mesh control plane. Stored on [`AppState`].
pub struct MeshCoordinator {
    /// `true` once the call-me-now listener holds a live, authenticated
    /// subscription. `start_client` awaits this before publishing a 24621.
    /// Observable so guardrail tests can assert listener-before-publish
    /// without depending on wall-clock timing.
    listener_active: watch::Receiver<bool>,
    _listener: tokio::task::JoinHandle<()>,
    _status_publisher: tokio::task::JoinHandle<()>,
}

impl MeshCoordinator {
    /// Whether the call-me-now listener is currently live. Exposed for
    /// instrumentation and tests.
    pub fn listener_active(&self) -> bool {
        *self.listener_active.borrow()
    }

    /// Await the listener becoming active, bounded by `timeout`. Returns
    /// `Err` if it does not come up in time (relay unreachable / auth failed),
    /// so `start_client` fails loudly rather than publishing into the void.
    async fn await_listener(&self, timeout: Duration) -> Result<(), String> {
        if self.listener_active() {
            return Ok(());
        }
        let mut rx = self.listener_active.clone();
        tokio::time::timeout(timeout, async {
            while !*rx.borrow() {
                if rx.changed().await.is_err() {
                    return Err("mesh listener task ended before becoming active".to_string());
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| "timed out waiting for mesh call-me-now listener to come up".to_string())?
    }
}

/// Start the runtime-owned relay-mesh coordinator if it is not already running.
/// Idempotent: a second call with a coordinator already present is a no-op.
///
/// Known limitation: the listener subscribes with the identity active at spawn
/// time and is never restarted. If the workspace identity changes mid-session
/// the subscription keeps filtering on the old pubkey; an app restart picks up
/// the new one. Acceptable for now — identity changes are rare and already
/// disruptive — but revisit if identity switching becomes a first-class flow.
///
/// Spawned at identity-set time from `lib.rs` setup, *before* any restore or
/// create attempt can enqueue a connect-request. Holds the `AppHandle` and
/// fetches `AppState` per session (the codebase manages `AppState` by value;
/// long-lived tasks never hold an `Arc<AppState>` across awaits).
pub async fn spawn_listener(app: AppHandle) {
    {
        let state = app.state::<AppState>();
        let guard = state.mesh_coordinator.lock().await;
        if guard.is_some() {
            return;
        }
    }
    let (active_tx, active_rx) = watch::channel(false);
    let listener_app = app.clone();
    let listener = tokio::spawn(async move {
        listener_loop(listener_app, active_tx).await;
    });
    let publisher_app = app.clone();
    let publisher = tokio::spawn(async move {
        status_publisher_loop(publisher_app).await;
    });
    let state = app.state::<AppState>();
    let mut guard = state.mesh_coordinator.lock().await;
    if guard.is_none() {
        *guard = Some(MeshCoordinator {
            listener_active: active_rx,
            _listener: listener,
            _status_publisher: publisher,
        });
    } else {
        // Lost a race: another caller installed a coordinator first. Drop ours.
        listener.abort();
        publisher.abort();
    }
}

async fn status_publisher_loop(app: AppHandle) {
    loop {
        publish_current_status_once(&app, "periodic").await;
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

pub(crate) async fn publish_current_status_once(app: &AppHandle, reason: &str) {
    let state = app.state::<AppState>();
    if let Err(error) = publish_current_status_for_state(&state).await {
        eprintln!("buzz-mesh: status report after {reason} failed: {error}");
    }
}

pub(crate) async fn publish_stopped_status_once(app: &AppHandle, reason: &str) {
    let state = app.state::<AppState>();
    if let Err(error) = publish_stopped_status_for_state(&state).await {
        eprintln!("buzz-mesh: stopped status report after {reason} failed: {error}");
    }
}

async fn publish_current_status_for_state(state: &AppState) -> Result<(), String> {
    let payload = {
        let runtime = state.mesh_llm_runtime.lock().await;
        match runtime.as_ref() {
            Some(runtime) => runtime
                .status_report_payload()
                .await
                .map_err(|error| error.to_string())?,
            None => return Ok(()),
        }
    };
    publish_status_report(state, payload).await
}

async fn publish_stopped_status_for_state(state: &AppState) -> Result<(), String> {
    publish_status_report(state, stopped_status_payload()).await
}

fn stopped_status_payload() -> serde_json::Value {
    serde_json::json!({
        "token": "",
        "hosted_models": [],
        "serving_models": [],
        "peers": [],
    })
}

/// The listener task body. Connects, authenticates as the Buzz identity,
/// subscribes `24622 #p=self`, and dials each paired call-me-now. Reconnects
/// with backoff on connection loss; flips `active` to `false` while down so
/// `start_client` won't publish during an outage.
async fn listener_loop(app: AppHandle, active: watch::Sender<bool>) {
    loop {
        if let Err(error) = listener_session(&app, &active).await {
            eprintln!("buzz-mesh: call-me-now listener session ended: {error}");
        }
        let _ = active.send(false);
        tokio::time::sleep(RETRY_BACKOFF).await;
    }
}

/// One authenticated listener session. Mirrors `commands::pairing`'s WS+NIP-42
/// preamble, then runs a long-lived REQ on `24622 #p=self`.
async fn listener_session(app: &AppHandle, active: &watch::Sender<bool>) -> Result<(), String> {
    let (relay_url, self_pk) = {
        let state = app.state::<AppState>();
        let url = crate::relay::relay_ws_url_with_override(&state);
        let pk = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            keys.public_key().to_hex()
        };
        (url, pk)
    };

    let (ws, _) = connect_async(&relay_url)
        .await
        .map_err(|e| format!("mesh listener WS connect failed: {e}"))?;
    let (mut write, mut read) = ws.split();

    authenticate(app, &relay_url, &mut read, &mut write).await?;

    let sub = json!([
        "REQ", "mesh-call-me-now",
        { "kinds": [KIND_MESH_CALL_ME_NOW], "#p": [self_pk], "limit": 0 }
    ]);
    write
        .send(Message::Text(sub.to_string().into()))
        .await
        .map_err(|e| format!("mesh listener subscribe failed: {e}"))?;

    // Subscription accepted — we can now receive our own paired 24622. Signal
    // before processing events so `start_client` may proceed.
    let _ = active.send(true);

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| format!("mesh listener WS read error: {e}"))?;
        let Message::Text(text) = msg else { continue };
        let Some(event) = parse_relay_event(text.as_str(), "mesh-call-me-now") else {
            continue;
        };
        if let Some(addr) = call_me_now_peer_addr(&event) {
            // Dial the peer the relay paired us with. Errors are per-attempt;
            // the initiator's retry loop and the peer's own dial cover loss.
            let state = app.state::<AppState>();
            if let Err(error) =
                crate::commands::ensure_client_node_for_model_dial_only(&state, &addr).await
            {
                eprintln!("buzz-mesh: call-me-now dial failed: {error}");
            }
        }
    }
    Ok(())
}

pub struct RelayMeshConnectRequest<'a> {
    pub target_pubkey: &'a str,
    pub peer_endpoint_addr: &'a str,
    pub self_endpoint_addr: &'a str,
    pub peer_endpoint_id: Option<&'a str>,
    pub self_endpoint_id: Option<&'a str>,
}

/// Publish a `kind:24621` connect-request and drive bounded publish+dial retry.
/// Blocks until the listener is active so we never request a connection we
/// cannot receive the pairing for. One `attempt_id` per call correlates the
/// retries in logs and tests.
pub async fn start_client(
    app: &AppHandle,
    request: RelayMeshConnectRequest<'_>,
) -> Result<String, String> {
    let attempt_id = uuid::Uuid::new_v4().to_string();
    {
        let state = app.state::<AppState>();
        let guard = state.mesh_coordinator.lock().await;
        let coordinator = guard
            .as_ref()
            .ok_or("mesh coordinator not started; cannot request connection")?;
        coordinator.await_listener(Duration::from_secs(10)).await?;
    }

    let deadline = tokio::time::Instant::now() + CALL_ME_NOW_TTL;
    let mut last_error = String::new();
    while tokio::time::Instant::now() < deadline {
        let state = app.state::<AppState>();
        match publish_connect_request(&state, &request, &attempt_id).await {
            Ok(()) => {
                // Dial in the same attempt: hole-punch needs both ends dialing.
                if let Err(error) = crate::commands::ensure_client_node_for_model_dial_only(
                    &state,
                    request.peer_endpoint_addr,
                )
                .await
                {
                    last_error = format!("dial after connect-request failed: {error}");
                } else {
                    return Ok(attempt_id);
                }
            }
            Err(error) => last_error = error,
        }
        tokio::time::sleep(RETRY_BACKOFF).await;
    }
    Err(format!(
        "mesh connect attempt {attempt_id} exhausted its window: {last_error}"
    ))
}

/// Build + sign + submit the kind:24621 connect-request as the Buzz identity.
async fn publish_connect_request(
    state: &AppState,
    request: &RelayMeshConnectRequest<'_>,
    attempt_id: &str,
) -> Result<(), String> {
    let builder = build_connect_request_event(request, attempt_id)?;
    crate::relay::submit_event(builder, state).await.map(|_| ())
}

fn build_connect_request_event(
    request: &RelayMeshConnectRequest<'_>,
    attempt_id: &str,
) -> Result<nostr::EventBuilder, String> {
    let mut content = json!({
        "v": 1,
        "self_endpoint_addr": request.self_endpoint_addr,
        "peer_endpoint_addr": request.peer_endpoint_addr,
        "attempt_id": attempt_id,
    });
    if let Some(endpoint_id) = request.self_endpoint_id {
        content["self_endpoint_id"] = serde_json::Value::String(endpoint_id.to_string());
    }
    if let Some(endpoint_id) = request.peer_endpoint_id {
        content["peer_endpoint_id"] = serde_json::Value::String(endpoint_id.to_string());
    }
    let target = nostr::PublicKey::from_hex(request.target_pubkey)
        .map_err(|e| format!("invalid target pubkey: {e}"))?;
    Ok(nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_MESH_CONNECT_REQUEST as u16),
        content.to_string(),
    )
    .tag(nostr::Tag::public_key(target)))
}

pub(crate) fn build_status_report_event(payload: serde_json::Value) -> nostr::EventBuilder {
    nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_MESH_STATUS_REPORT as u16),
        payload.to_string(),
    )
}

pub(crate) async fn publish_status_report(
    state: &AppState,
    payload: serde_json::Value,
) -> Result<(), String> {
    crate::relay::submit_event(build_status_report_event(payload), state)
        .await
        .map(|_| ())
}

/// Extract the peer endpoint addr from a paired call-me-now (24622) event,
/// dropping expired ones.
fn call_me_now_peer_addr(event: &nostr::Event) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(&event.content).ok()?;
    if payload.get("type")?.as_str()? != "buzz-iroh-call-me-now" {
        return None;
    }
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    if let Some(expires_at) = payload.get("expires_at").and_then(|v| v.as_u64()) {
        if expires_at < now {
            return None;
        }
    }
    payload
        .get("peer_endpoint_addr")?
        .as_str()
        .map(str::to_string)
}

/// NIP-42 AUTH as the Buzz identity. Generalized from `commands::pairing`'s
/// `handle_nip42_auth` — same flow, signs with `state.keys` instead of a
/// pairing session. Returns `Ok(())` when the relay does not challenge.
async fn authenticate<R, W>(
    app: &AppHandle,
    relay_url: &str,
    read: &mut R,
    write: &mut W,
) -> Result<(), String>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let challenge = match tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| "relay closed during mesh auth".to_string())?
                .map_err(|e| format!("WS error during mesh auth: {e}"))?;
            if let Message::Text(text) = msg {
                if let Some(challenge) = parse_auth_challenge(text.as_str()) {
                    return Ok::<String, String>(challenge);
                }
            }
        }
    })
    .await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(()), // no challenge: relay does not require AUTH
    };

    let relay_url_parsed =
        nostr::RelayUrl::parse(relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
    // Sign synchronously, drop the guard before awaiting (keep the future Send).
    let auth_json = {
        let state = app.state::<AppState>();
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        let event = nostr::EventBuilder::auth(challenge, relay_url_parsed)
            .sign_with_keys(&keys)
            .map_err(|e| format!("sign mesh auth event: {e}"))?;
        format!("[\"AUTH\",{}]", event.as_json())
    };
    write
        .send(Message::Text(auth_json.into()))
        .await
        .map_err(|e| format!("send mesh auth: {e}"))?;
    Ok(())
}

fn parse_auth_challenge(text: &str) -> Option<String> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;
    if arr.len() >= 2 && arr[0].as_str()? == "AUTH" {
        return arr[1].as_str().map(str::to_string);
    }
    None
}

fn parse_relay_event(text: &str, sub_id: &str) -> Option<nostr::Event> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;
    if arr.len() < 3 || arr[0].as_str()? != "EVENT" || arr[1].as_str()? != sub_id {
        return None;
    }
    serde_json::from_value(arr[2].clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_request_event_includes_optional_endpoint_ids() {
        let keys = nostr::Keys::generate();
        let request = RelayMeshConnectRequest {
            target_pubkey: &keys.public_key().to_hex(),
            peer_endpoint_addr: "peer-addr",
            self_endpoint_addr: "self-addr",
            peer_endpoint_id: Some("peer-id"),
            self_endpoint_id: Some("self-id"),
        };
        let event = build_connect_request_event(&request, "attempt-1")
            .expect("build event")
            .sign_with_keys(&keys)
            .expect("sign event");
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert_eq!(content["self_endpoint_id"], "self-id");
        assert_eq!(content["peer_endpoint_id"], "peer-id");
    }

    #[test]
    fn connect_request_event_omits_absent_endpoint_ids() {
        let keys = nostr::Keys::generate();
        let request = RelayMeshConnectRequest {
            target_pubkey: &keys.public_key().to_hex(),
            peer_endpoint_addr: "peer-addr",
            self_endpoint_addr: "self-addr",
            peer_endpoint_id: None,
            self_endpoint_id: None,
        };
        let event = build_connect_request_event(&request, "attempt-1")
            .expect("build event")
            .sign_with_keys(&keys)
            .expect("sign event");
        let content: serde_json::Value = serde_json::from_str(&event.content).unwrap();
        assert!(content.get("self_endpoint_id").is_none());
        assert!(content.get("peer_endpoint_id").is_none());
    }

    #[test]
    fn status_report_event_uses_kind_and_exact_content() {
        let keys = nostr::Keys::generate();
        let payload = json!({"v": 1, "models": ["demo"]});
        let event = build_status_report_event(payload.clone())
            .sign_with_keys(&keys)
            .expect("sign event");
        assert_eq!(
            event.kind,
            nostr::Kind::Custom(KIND_MESH_STATUS_REPORT as u16)
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&event.content).unwrap(),
            payload
        );
    }

    #[test]
    fn stopped_status_payload_withdraws_all_targets() {
        assert_eq!(
            stopped_status_payload(),
            json!({
                "token": "",
                "hosted_models": [],
                "serving_models": [],
                "peers": [],
            })
        );
    }

    #[test]
    fn call_me_now_peer_addr_extracts_unexpired() {
        let future = chrono::Utc::now().timestamp() as u64 + 30;
        let event = test_event(&json!({
            "type": "buzz-iroh-call-me-now",
            "peer_endpoint_addr": "node-abc",
            "attempt_id": "a1",
            "expires_at": future,
        }));
        assert_eq!(call_me_now_peer_addr(&event).as_deref(), Some("node-abc"));
    }

    #[test]
    fn call_me_now_peer_addr_drops_expired() {
        let event = test_event(&json!({
            "type": "buzz-iroh-call-me-now",
            "peer_endpoint_addr": "node-abc",
            "attempt_id": "a1",
            "expires_at": 1u64,
        }));
        assert_eq!(call_me_now_peer_addr(&event), None);
    }

    #[test]
    fn call_me_now_peer_addr_rejects_wrong_type() {
        let event = test_event(&json!({
            "type": "something-else",
            "peer_endpoint_addr": "node-abc",
        }));
        assert_eq!(call_me_now_peer_addr(&event), None);
    }

    #[test]
    fn parse_auth_challenge_reads_nip42() {
        assert_eq!(
            parse_auth_challenge(r#"["AUTH","chal-123"]"#).as_deref(),
            Some("chal-123")
        );
        assert_eq!(parse_auth_challenge(r#"["EVENT","x"]"#), None);
    }

    fn test_event(content: &serde_json::Value) -> nostr::Event {
        let keys = nostr::Keys::generate();
        nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_MESH_CALL_ME_NOW as u16),
            content.to_string(),
        )
        .sign_with_keys(&keys)
        .expect("sign test event")
    }
}
