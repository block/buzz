//! Narrow, durable reply broker for managed-agent workers.
//!
//! The ACP harness owns the Nostr signing key.  Workers receive only a scoped
//! loopback capability which can create a kind-9 reply to an existing message;
//! they never receive the signing key or owner attestation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use buzz_cli::BuzzClient;
use buzz_sdk::{build_message, nip_oa::parse_auth_tag, ThreadRef};
use nostr::{Event, EventId, Keys};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_CONTENT_BYTES: usize = 64 * 1024;

/// Opaque loopback connection details passed to the worker MCP server.
#[derive(Clone, Debug)]
pub struct ReplyBrokerEndpoint {
    pub url: String,
    pub capability: String,
}

/// Keeps the broker alive for the lifetime of the harness.
pub struct ReplyBroker {
    endpoint: ReplyBrokerEndpoint,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ReplyBroker {
    pub fn endpoint(&self) -> &ReplyBrokerEndpoint {
        &self.endpoint
    }
}

impl Drop for ReplyBroker {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[derive(Clone)]
struct BrokerState {
    capability: String,
    client: Arc<BuzzClient>,
    ledger_dir: PathBuf,
    // A reply idempotency key must be serialized across the read → sign →
    // persist boundary.  One lock keeps that invariant straightforward; reply
    // publication is infrequent and a second request receives the exact first
    // event rather than a second freshly signed event.
    ledger_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ReplyRequest {
    capability: String,
    channel_id: String,
    reply_to: String,
    content: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LedgerRequest {
    channel_id: String,
    reply_to: String,
    content: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LedgerEntry {
    request: LedgerRequest,
    event: Event,
    event_id: String,
    state: LedgerState,
    run: ReplyRun,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LedgerState {
    Prepared,
    Sent,
    DeliveryUnknown,
}

/// Minimal durable continuation state for the broker-owned delivery phase.
/// Recovery remains deliberately disabled: an expired lease is evidence for an
/// operator, never authority for this broker to publish a different message.
#[derive(Debug, Deserialize, Serialize)]
struct ReplyRun {
    source_channel: String,
    source_event_id: String,
    objective: String,
    owner_pubkey: String,
    next_action: String,
    acceptance_evidence: Option<String>,
    state: ReplyRunState,
    current_thread_event: String,
    progress_lease_expires_at: i64,
    hard_deadline_at: i64,
    recovery_count: u8,
    terminal_reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReplyRunState {
    Owned,
    Complete,
    Blocked,
}

#[derive(Debug, Serialize)]
struct ReplyResponse {
    ok: bool,
    event_id: Option<String>,
    state: Option<LedgerState>,
    error: Option<String>,
}

impl ReplyResponse {
    fn success(event_id: String, state: LedgerState) -> Self {
        Self {
            ok: true,
            event_id: Some(event_id),
            state: Some(state),
            error: None,
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            event_id: None,
            state: None,
            error: Some(error.into()),
        }
    }
}

/// Start a broker bound only to loopback.  It accepts one JSON request per
/// connection and emits one JSON response, both newline-delimited.
pub fn start(
    relay_url: String,
    keys: Keys,
    auth_tag_json: Option<String>,
    state_dir: PathBuf,
) -> Result<ReplyBroker, String> {
    let auth_tag = auth_tag_json
        .as_deref()
        .map(parse_auth_tag)
        .transpose()
        .map_err(|e| format!("invalid broker auth tag: {e}"))?;
    let client = BuzzClient::new(relay_url, keys, auth_tag, auth_tag_json)
        .map_err(|e| format!("reply broker client setup failed: {e}"))?;
    let ledger_dir = state_dir.join("reply-ledger");
    create_private_dir(&ledger_dir)?;

    // A loopback listener is intentionally not a signing API.  The capability
    // authorizes only the constrained reply schema below; it cannot select an
    // event kind, signer, relay, auth tag, or arbitrary destination.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("reply broker bind failed: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("reply broker nonblocking setup failed: {e}"))?;
    let address = listener
        .local_addr()
        .map_err(|e| format!("reply broker address failed: {e}"))?;
    let listener = TcpListener::from_std(listener)
        .map_err(|e| format!("reply broker async setup failed: {e}"))?;
    let endpoint = ReplyBrokerEndpoint {
        url: format!("tcp://{address}"),
        capability: Uuid::new_v4().simple().to_string(),
    };
    let state = BrokerState {
        capability: endpoint.capability.clone(),
        client: Arc::new(client),
        ledger_dir,
        ledger_lock: Arc::new(Mutex::new(())),
    };
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        tokio::spawn(async move { serve_connection(stream, state).await; });
                    }
                    Err(error) => {
                        tracing::warn!("reply broker accept failed: {error}");
                        break;
                    }
                }
            }
        }
    });

    Ok(ReplyBroker {
        endpoint,
        shutdown: Some(shutdown_tx),
    })
}

pub fn state_dir_from_env(config_path: &Path) -> Result<PathBuf, String> {
    let dir = std::env::var_os("BUZZ_ACP_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("buzz-acp-state")
        });
    create_private_dir(&dir)?;
    Ok(dir)
}

async fn serve_connection(stream: TcpStream, state: BrokerState) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let response = match lines.next_line().await {
        Ok(Some(line)) if line.len() <= MAX_REQUEST_BYTES => match serde_json::from_str(&line) {
            Ok(request) => handle_request(&state, request).await,
            Err(error) => ReplyResponse::error(format!("invalid reply request: {error}")),
        },
        Ok(Some(_)) => ReplyResponse::error("reply request exceeds size limit"),
        Ok(None) => ReplyResponse::error("empty reply request"),
        Err(error) => ReplyResponse::error(format!("reply request read failed: {error}")),
    };
    if let Ok(payload) = serde_json::to_vec(&response) {
        let _ = write.write_all(&payload).await;
        let _ = write.write_all(b"\n").await;
    }
}

async fn handle_request(state: &BrokerState, request: ReplyRequest) -> ReplyResponse {
    if request.capability != state.capability {
        return ReplyResponse::error("reply capability rejected");
    }
    if request.content.is_empty() || request.content.len() > MAX_CONTENT_BYTES {
        return ReplyResponse::error("reply content must contain 1..=65536 bytes");
    }
    if request.content.contains('\0') {
        return ReplyResponse::error("reply content must not contain NUL bytes");
    }
    if contains_protected_secret_marker(&request.content) {
        return ReplyResponse::error(
            "reply content appears to contain a protected Buzz signing credential",
        );
    }
    if request.idempotency_key.is_empty() || request.idempotency_key.len() > 128 {
        return ReplyResponse::error("idempotency_key must contain 1..=128 bytes");
    }
    if !is_hex64(&request.reply_to) {
        return ReplyResponse::error("reply_to must be a 64-character hexadecimal event id");
    }
    let channel_id = match request.channel_id.parse::<Uuid>() {
        Ok(channel_id) => channel_id,
        Err(_) => return ReplyResponse::error("channel_id must be a UUID"),
    };

    let _guard = state.ledger_lock.lock().await;
    let ledger_path = ledger_path(&state.ledger_dir, &request.idempotency_key);
    let ledger_request = LedgerRequest {
        channel_id: request.channel_id.clone(),
        reply_to: request.reply_to.clone(),
        content: request.content.clone(),
        idempotency_key: request.idempotency_key.clone(),
    };

    let mut entry = match read_ledger(&ledger_path) {
        Ok(Some(entry)) => {
            if entry.request != ledger_request {
                return ReplyResponse::error(
                    "idempotency_key is already bound to a different reply",
                );
            }
            entry
        }
        Ok(None) => match prepare_entry(state, channel_id, &request, ledger_request).await {
            Ok(entry) => {
                if let Err(error) = write_ledger(&ledger_path, &entry) {
                    return ReplyResponse::error(error);
                }
                entry
            }
            Err(error) => return ReplyResponse::error(error),
        },
        Err(error) => return ReplyResponse::error(error),
    };

    if entry.state == LedgerState::Sent {
        return ReplyResponse::success(entry.event_id, LedgerState::Sent);
    }

    // A previous caller may have lost the response after the relay accepted
    // the event.  Reconcile by deterministic event id before any resend.
    match event_exists(&state.client, &entry.event_id).await {
        Ok(true) => {
            mark_sent(&mut entry);
            if let Err(error) = write_ledger(&ledger_path, &entry) {
                return ReplyResponse::error(error);
            }
            ReplyResponse::success(entry.event_id, LedgerState::Sent)
        }
        Ok(false) => match state.client.submit_event(entry.event.clone()).await {
            Ok(_) => {
                mark_sent(&mut entry);
                if let Err(error) = write_ledger(&ledger_path, &entry) {
                    return ReplyResponse::error(error);
                }
                ReplyResponse::success(entry.event_id, LedgerState::Sent)
            }
            Err(error) => {
                mark_delivery_unknown(&mut entry);
                let _ = write_ledger(&ledger_path, &entry);
                ReplyResponse::error(format!(
                    "reply delivery is unknown for {}: {error}",
                    entry.event_id
                ))
            }
        },
        Err(error) => {
            mark_delivery_unknown(&mut entry);
            let _ = write_ledger(&ledger_path, &entry);
            ReplyResponse::error(format!(
                "reply delivery reconciliation failed for {}: {error}",
                entry.event_id
            ))
        }
    }
}

async fn prepare_entry(
    state: &BrokerState,
    channel_id: Uuid,
    request: &ReplyRequest,
    ledger_request: LedgerRequest,
) -> Result<LedgerEntry, String> {
    let parent = fetch_parent(&state.client, &request.reply_to).await?;
    let parent_channel = channel_from_event(&parent)?;
    if parent_channel != channel_id {
        return Err("reply_to does not belong to channel_id".into());
    }
    let parent_id = EventId::from_hex(&request.reply_to)
        .map_err(|_| "reply_to must be a valid event id".to_string())?;
    let root_id = find_root_from_event(&parent)
        .and_then(|id| EventId::from_hex(&id).ok())
        .unwrap_or(parent_id);
    let thread = ThreadRef {
        root_event_id: root_id,
        parent_event_id: parent_id,
    };
    let builder = build_message(channel_id, &request.content, Some(&thread), &[], false, &[])
        .map_err(|e| format!("reply construction failed: {e}"))?;
    let event = state
        .client
        .sign_event(builder)
        .map_err(|e| format!("reply signing failed: {e}"))?;
    let now = chrono::Utc::now().timestamp();
    Ok(LedgerEntry {
        request: ledger_request,
        event_id: event.id.to_hex(),
        event,
        state: LedgerState::Prepared,
        run: ReplyRun {
            source_channel: request.channel_id.clone(),
            source_event_id: request.reply_to.clone(),
            objective: "publish a constrained reply".into(),
            owner_pubkey: state.client.keys().public_key().to_hex(),
            next_action: "reconcile or submit the exact persisted event".into(),
            acceptance_evidence: None,
            state: ReplyRunState::Owned,
            current_thread_event: request.reply_to.clone(),
            progress_lease_expires_at: now + 15 * 60,
            hard_deadline_at: now + 60 * 60,
            recovery_count: 0,
            terminal_reason: None,
        },
    })
}

fn mark_sent(entry: &mut LedgerEntry) {
    entry.state = LedgerState::Sent;
    entry.run.state = ReplyRunState::Complete;
    entry.run.next_action = "none".into();
    entry.run.acceptance_evidence = Some(entry.event_id.clone());
    entry.run.terminal_reason = Some("delivery_confirmed".into());
}

fn mark_delivery_unknown(entry: &mut LedgerEntry) {
    entry.state = LedgerState::DeliveryUnknown;
    entry.run.state = ReplyRunState::Blocked;
    entry.run.next_action = "query the persisted event id before any retry".into();
    entry.run.terminal_reason = Some("delivery_unknown".into());
}

async fn fetch_parent(client: &BuzzClient, event_id: &str) -> Result<serde_json::Value, String> {
    let raw = client
        .query(&serde_json::json!({ "ids": [event_id], "limit": 1 }))
        .await
        .map_err(|e| format!("parent lookup failed: {e}"))?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|e| format!("parent lookup JSON failed: {e}"))?
        .as_array()
        .and_then(|events| events.first())
        .cloned()
        .ok_or_else(|| "reply parent was not found".into())
}

async fn event_exists(client: &BuzzClient, event_id: &str) -> Result<bool, String> {
    let raw = client
        .query(&serde_json::json!({ "ids": [event_id], "limit": 1 }))
        .await
        .map_err(|e| format!("event lookup failed: {e}"))?;
    let events: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("event lookup JSON failed: {e}"))?;
    Ok(events.as_array().is_some_and(|events| !events.is_empty()))
}

fn channel_from_event(event: &serde_json::Value) -> Result<Uuid, String> {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .and_then(|tags| {
            tags.iter().find_map(|tag| {
                let parts = tag.as_array()?;
                (parts.first()?.as_str()? == "h")
                    .then(|| parts.get(1)?.as_str())
                    .flatten()
            })
        })
        .ok_or_else(|| "reply parent is missing a channel tag".to_string())?
        .parse::<Uuid>()
        .map_err(|_| "reply parent has an invalid channel tag".into())
}

fn find_root_from_event(event: &serde_json::Value) -> Option<String> {
    let tags = event.get("tags")?.as_array()?;
    let mut reply = None;
    for tag in tags {
        let parts = tag.as_array()?;
        if parts.len() >= 4 && parts.first()?.as_str()? == "e" && is_hex64(parts.get(1)?.as_str()?)
        {
            match parts.get(3)?.as_str()? {
                "root" => return Some(parts.get(1)?.as_str()?.to_string()),
                "reply" => reply = Some(parts.get(1)?.as_str()?.to_string()),
                _ => {}
            }
        }
    }
    reply
}

fn ledger_path(dir: &Path, idempotency_key: &str) -> PathBuf {
    let digest = Sha256::digest(idempotency_key.as_bytes());
    dir.join(format!("{}.json", hex::encode(digest)))
}

fn read_ledger(path: &Path) -> Result<Option<LedgerEntry>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("reply ledger is invalid: {e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("reply ledger read failed: {error}")),
    }
}

fn write_ledger(path: &Path, entry: &LedgerEntry) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(entry).map_err(|e| format!("reply ledger encode failed: {e}"))?;
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
    std::fs::write(&temporary, bytes).map_err(|e| format!("reply ledger write failed: {e}"))?;
    restrict_file(&temporary)?;
    std::fs::rename(&temporary, path).map_err(|e| format!("reply ledger commit failed: {e}"))
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("reply state directory failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("reply state directory permissions failed: {e}"))?;
    }
    Ok(())
}

fn restrict_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("reply ledger permissions failed: {e}"))?;
    }
    Ok(())
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn contains_protected_secret_marker(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("nsec1")
        || lower.contains("buzz_private_key")
        || lower.contains("nostr_private_key")
        || lower.contains("buzz_auth_tag")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_path_does_not_expose_idempotency_key() {
        let path = ledger_path(Path::new("/state"), "reply:private/input");
        assert_eq!(path.parent(), Some(Path::new("/state")));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".json"));
        assert!(!path.to_string_lossy().contains("private/input"));
    }

    #[test]
    fn root_parser_prefers_root_over_reply() {
        let event = serde_json::json!({
            "tags": [
                ["e", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "", "reply"],
                ["e", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "", "root"]
            ]
        });
        assert_eq!(
            find_root_from_event(&event).as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
    }

    #[test]
    fn protected_signing_markers_are_never_publishable() {
        assert!(contains_protected_secret_marker(
            "BUZZ_PRIVATE_KEY=redacted"
        ));
        assert!(contains_protected_secret_marker("nsec1example"));
        assert!(!contains_protected_secret_marker(
            "A normal delivery report."
        ));
    }
}
