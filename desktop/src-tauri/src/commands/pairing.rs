use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use buzz_core_pkg::kind::KIND_PAIRING;
use buzz_core_pkg::pairing::qr::encode_qr;
use buzz_core_pkg::pairing::session::PairingSession;
use buzz_core_pkg::pairing::types::{AbortReason, PayloadType};
use futures_util::{SinkExt, StreamExt};
use nostr::ToBech32;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::app_state::AppState;
use crate::relay::{relay_api_base_url_with_override, relay_ws_url_with_override};

#[derive(Serialize, Clone)]
struct PairingSasPayload {
    sas: String,
}

#[derive(Serialize, Clone)]
struct PairingAbortedPayload {
    reason: String,
}

#[derive(Serialize, Clone)]
struct PairingErrorPayload {
    message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PairingMode {
    SendIdentity,
    RecoverIdentity,
}

#[derive(Clone)]
struct PairingTaskContext {
    mode: PairingMode,
    generation: Arc<AtomicU64>,
    generation_fence: Arc<std::sync::Mutex<()>>,
    task_generation: u64,
}

/// Managed Tauri state for an active pairing session.
pub struct PairingHandle {
    session: Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    generation: Arc<AtomicU64>,
    /// Linearizes cancellation/replacement against recovered identity commits.
    generation_fence: Arc<std::sync::Mutex<()>>,
    /// Serializes session setup so an older start cannot resume after relay
    /// discovery and overwrite a newer session's shared state.
    start_lock: tokio::sync::Mutex<()>,
    cancel: std::sync::Mutex<Option<CancellationToken>>,
    /// Send JSON-serialized events to the background WS task for relay publication.
    outbound_tx: std::sync::Mutex<Option<mpsc::Sender<String>>>,
    /// Pre-built payload string (contains nsec) to send after SAS confirmation.
    /// Wrapped in Zeroizing so the nsec is cleared from memory on drop.
    payload: std::sync::Mutex<Option<Zeroizing<String>>>,
    mode: Arc<std::sync::Mutex<PairingMode>>,
}

impl PairingHandle {
    pub fn new() -> Self {
        Self {
            session: Arc::new(tokio::sync::Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            generation_fence: Arc::new(std::sync::Mutex::new(())),
            start_lock: tokio::sync::Mutex::new(()),
            cancel: std::sync::Mutex::new(None),
            outbound_tx: std::sync::Mutex::new(None),
            payload: std::sync::Mutex::new(None),
            mode: Arc::new(std::sync::Mutex::new(PairingMode::SendIdentity)),
        }
    }

    fn clear(&self) {
        *self.cancel.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.outbound_tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.payload.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Start a NIP-AB pairing session that sends this desktop identity to mobile.
#[tauri::command]
pub async fn start_pairing(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing: State<'_, PairingHandle>,
) -> Result<String, String> {
    start_pairing_session(app, state, pairing, PairingMode::SendIdentity).await
}

/// Start a recovery session. The fresh desktop shows the QR and receives the
/// full identity from an already-authorized phone after both users approve SAS.
#[tauri::command]
pub async fn start_identity_recovery_pairing(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing: State<'_, PairingHandle>,
) -> Result<String, String> {
    start_pairing_session(app, state, pairing, PairingMode::RecoverIdentity).await
}

async fn start_pairing_session(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing: State<'_, PairingHandle>,
    mode: PairingMode,
) -> Result<String, String> {
    let _start_guard = pairing.start_lock.lock().await;
    let task_generation =
        invalidate_pairing_generation(&pairing.generation, &pairing.generation_fence)?;
    if let Some(token) = pairing.cancel.lock().map_err(|e| e.to_string())?.take() {
        token.cancel();
    }
    pairing.clear();
    {
        let mut session = pairing.session.lock().await;
        *session = None;
    }
    *pairing.mode.lock().map_err(|e| e.to_string())? = mode;
    *pairing.payload.lock().map_err(|e| e.to_string())? = None;

    let ws_url = relay_ws_url_with_override(&state);
    let http_url = relay_api_base_url_with_override(&state);
    let pairing_relay_url = resolve_pairing_relay_url(&ws_url, probe_pairing_relay(&ws_url).await)?;
    let (session, qr_payload) = PairingSession::new_source(pairing_relay_url.clone());
    let mut qr_uri = encode_qr(&qr_payload);
    if mode == PairingMode::RecoverIdentity {
        qr_uri.push_str("&mode=recover");
    }

    if mode == PairingMode::SendIdentity {
        let keys = state.signing_keys()?;
        let nsec = keys
            .secret_key()
            .to_bech32()
            .map_err(|e| format!("encode nsec: {e}"))?;
        let payload_json = serde_json::json!({
            "relayUrl": http_url,
            "pubkey": keys.public_key().to_hex(),
            "nsec": nsec,
        });
        *pairing.payload.lock().map_err(|e| e.to_string())? =
            Some(Zeroizing::new(payload_json.to_string()));
    }

    {
        let mut active = pairing.session.lock().await;
        *active = Some(session);
    }

    let (outbound_tx, outbound_rx) = mpsc::channel::<String>(16);
    let cancel = CancellationToken::new();
    *pairing.outbound_tx.lock().map_err(|e| e.to_string())? = Some(outbound_tx);
    *pairing.cancel.lock().map_err(|e| e.to_string())? = Some(cancel.clone());

    tauri::async_runtime::spawn(pairing_ws_task(
        pairing_relay_url,
        Arc::clone(&pairing.session),
        PairingTaskContext {
            mode,
            generation: Arc::clone(&pairing.generation),
            generation_fence: Arc::clone(&pairing.generation_fence),
            task_generation,
        },
        cancel,
        outbound_rx,
        app,
    ));

    Ok(qr_uri)
}

/// User confirmed the SAS codes match. Sends sas-confirm + payload.
#[tauri::command]
pub async fn confirm_pairing_sas(pairing: State<'_, PairingHandle>) -> Result<(), String> {
    let tx = pairing
        .outbound_tx
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or("no active pairing session")?;

    let sas_confirm_json = {
        let mut guard = pairing.session.lock().await;
        let session = guard.as_mut().ok_or("no active pairing session")?;
        let event = session.confirm_sas().map_err(|e| e.to_string())?;
        event_to_relay_json(&event)
    };

    tx.send(sas_confirm_json)
        .await
        .map_err(|_| "Pairing code expired. Create a new code and try again.")?;

    let mode = *pairing.mode.lock().map_err(|e| e.to_string())?;
    if mode == PairingMode::SendIdentity {
        let payload = pairing
            .payload
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or("no payload prepared")?;

        let payload_json = {
            let mut guard = pairing.session.lock().await;
            let session = guard.as_mut().ok_or("no active pairing session")?;
            let event = session
                .send_payload(PayloadType::Custom, payload)
                .map_err(|e| e.to_string())?;
            event_to_relay_json(&event)
        };

        tx.send(payload_json)
            .await
            .map_err(|_| "failed to send payload")?;
    }

    Ok(())
}

/// Cancel the active pairing session.
#[tauri::command]
pub async fn cancel_pairing(pairing: State<'_, PairingHandle>) -> Result<(), String> {
    // Invalidate the task before waiting for its session lock. Recovery may be
    // blocked on identity persistence after releasing this lock, and must see
    // cancellation before crossing the durable commit boundary.
    invalidate_pairing_generation(&pairing.generation, &pairing.generation_fence)?;
    if let Some(token) = pairing.cancel.lock().map_err(|e| e.to_string())?.take() {
        token.cancel();
    }

    let abort_json = {
        let mut guard = pairing.session.lock().await;
        if let Some(session) = guard.as_mut() {
            session
                .abort(AbortReason::UserDenied)
                .ok()
                .flatten()
                .map(|e| event_to_relay_json(&e))
        } else {
            None
        }
    };

    if let Some(json) = abort_json {
        let tx = pairing
            .outbound_tx
            .lock()
            .map_err(|e| e.to_string())?
            .clone();
        if let Some(tx) = tx {
            let _ = tx.send(json).await;
        }
    }

    pairing.clear();

    {
        let mut s = pairing.session.lock().await;
        *s = None;
    }

    Ok(())
}

async fn pairing_ws_task(
    relay_url: String,
    session: Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    context: PairingTaskContext,
    cancel: CancellationToken,
    mut outbound_rx: mpsc::Receiver<String>,
    app: AppHandle,
) {
    if let Err(e) = pairing_ws_task_inner(
        &relay_url,
        &session,
        &context,
        &cancel,
        &mut outbound_rx,
        &app,
    )
    .await
    {
        if pairing_task_is_current(&context.generation, context.task_generation) {
            let _ = app.emit("pairing-error", PairingErrorPayload { message: e });
        }
    }
    clear_pairing_session_if_current(&session, &context.generation, context.task_generation).await;
}

async fn pairing_ws_task_inner(
    relay_url: &str,
    session: &Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    context: &PairingTaskContext,
    cancel: &CancellationToken,
    outbound_rx: &mut mpsc::Receiver<String>,
    app: &AppHandle,
) -> Result<(), String> {
    let (ws, _) = connect_async(relay_url)
        .await
        .map_err(|e| format!("WebSocket connection failed: {e}"))?;
    let (mut write, mut read) = ws.split();

    handle_nip42_auth(&mut read, &mut write, session, relay_url).await?;

    let our_pk = {
        let guard = session.lock().await;
        guard.as_ref().ok_or("session gone")?.pubkey().to_hex()
    };
    let sub_msg = serde_json::json!([
        "REQ", "pair",
        { "kinds": [KIND_PAIRING], "#p": [our_pk] }
    ]);
    write
        .send(Message::Text(sub_msg.to_string().into()))
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;

    wait_for_eose(&mut read, "pair", Duration::from_secs(10)).await?;

    let hard_timeout = tokio::time::sleep(Duration::from_secs(130));
    tokio::pin!(hard_timeout);

    loop {
        if !pairing_task_is_current(&context.generation, context.task_generation) {
            break;
        }

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = &mut hard_timeout => {
                if pairing_task_is_current(&context.generation, context.task_generation) {
                    let _ = app.emit("pairing-error", PairingErrorPayload {
                        message: "Session timed out".into(),
                    });
                }
                break;
            }
            Some(json_msg) = outbound_rx.recv() => {
                if let Err(e) = write.send(Message::Text(json_msg.into())).await {
                    return Err(format!("publish failed: {e}"));
                }
            }
            msg = read.next() => {
                let Some(msg) = msg else {
                    return Err("relay connection closed".into());
                };
                let msg = msg.map_err(|e| format!("WS read error: {e}"))?;
                let Message::Text(text) = msg else { continue };

                if let Some(event) = parse_relay_event(text.as_str(), "pair") {
                    if !pairing_task_is_current(&context.generation, context.task_generation) {
                        break;
                    }

                    let mut guard = session.lock().await;
                    let Some(s) = guard.as_mut() else { break };

                    if let Ok(reason) = s.handle_abort(&event) {
                        if pairing_task_is_current(&context.generation, context.task_generation) {
                            let _ = app.emit("pairing-aborted", PairingAbortedPayload {
                                reason: format!("{reason:?}"),
                            });
                        }
                        break;
                    }

                    if let Ok(sas) = s.handle_offer(&event) {
                        if pairing_task_is_current(&context.generation, context.task_generation) {
                            let _ = app.emit("pairing-sas-received", PairingSasPayload { sas });
                        }
                        continue;
                    }

                    if context.mode == PairingMode::RecoverIdentity {
                        if let Ok((payload_type, payload)) = s.handle_return_payload(&event) {
                            if let Err(message) = validate_recovery_payload_type(payload_type) {
                                let complete = s
                                    .send_source_complete(false)
                                    .map_err(|e| e.to_string())?;
                                write
                                    .send(Message::Text(event_to_relay_json(&complete).into()))
                                    .await
                                    .map_err(|e| format!("publish complete failed: {e}"))?;
                                if pairing_task_is_current(
                                    &context.generation,
                                    context.task_generation,
                                ) {
                                    let _ = app.emit(
                                        "pairing-error",
                                        PairingErrorPayload { message },
                                    );
                                }
                                break;
                            }

                            let payload = payload;
                            drop(guard);

                            let imported = import_recovered_identity(
                                app,
                                payload,
                                &context.generation,
                                &context.generation_fence,
                                context.task_generation,
                            )
                            .await;
                            let success = imported.is_ok();
                            let complete = {
                                let mut guard = session.lock().await;
                                if !pairing_task_is_current(
                                    &context.generation,
                                    context.task_generation,
                                ) {
                                    break;
                                }
                                let Some(s) = guard.as_mut() else { break };
                                s.send_source_complete(success)
                                    .map_err(|e| e.to_string())?
                            };
                            let completion_result = write
                                .send(Message::Text(event_to_relay_json(&complete).into()))
                                .await
                                .map_err(|e| format!("publish complete failed: {e}"));
                            finish_recovery(imported, completion_result, context, app)?;
                            break;
                        }
                    } else {
                        match s.handle_complete(&event) {
                            Ok(()) => {
                                if pairing_task_is_current(&context.generation, context.task_generation) {
                                    let _ = app.emit("pairing-complete", serde_json::json!({}));
                                }
                                break;
                            }
                            Err(ref e) if format!("{e}").contains("success=false") => {
                                if pairing_task_is_current(&context.generation, context.task_generation) {
                                    let _ = app.emit("pairing-error", PairingErrorPayload {
                                        message: "Mobile device reported failure importing credentials".into(),
                                    });
                                }
                                break;
                            }
                            Err(_) => {}
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn import_recovered_identity(
    app: &AppHandle,
    nsec: Zeroizing<String>,
    generation: &Arc<AtomicU64>,
    generation_fence: &Arc<std::sync::Mutex<()>>,
    task_generation: u64,
) -> Result<(), String> {
    let app = app.clone();
    let generation = Arc::clone(generation);
    let generation_fence = Arc::clone(generation_fence);
    tokio::task::spawn_blocking(move || {
        let keys = nostr::Keys::parse(nsec.trim())
            .map_err(|e| format!("Phone sent an invalid identity: {e}"))?;
        let state = app.state::<AppState>();
        let _mutation_guard = state.identity_mutation.lock().map_err(|e| e.to_string())?;
        commit_recovery_if_current(&generation, &generation_fence, task_generation, || {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
            let key_path = data_dir.join("identity.key");
            crate::commands::identity::commit_imported_identity(&state, &data_dir, keys, |keys| {
                let store =
                    crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
                crate::app_state::persist_imported_identity(store, keys, &key_path, &data_dir)
            })?;
            Ok(())
        })
    })
    .await
    .map_err(|e| format!("identity recovery task failed: {e}"))?
}

fn ensure_pairing_task_is_current(
    generation: &AtomicU64,
    task_generation: u64,
) -> Result<(), String> {
    if pairing_task_is_current(generation, task_generation) {
        Ok(())
    } else {
        Err("Pairing session was superseded or cancelled".into())
    }
}

fn invalidate_pairing_generation(
    generation: &AtomicU64,
    generation_fence: &std::sync::Mutex<()>,
) -> Result<u64, String> {
    let _fence = generation_fence.lock().map_err(|e| e.to_string())?;
    Ok(generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1))
}

fn commit_recovery_if_current<T>(
    generation: &AtomicU64,
    generation_fence: &std::sync::Mutex<()>,
    task_generation: u64,
    commit: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _fence = generation_fence.lock().map_err(|e| e.to_string())?;
    ensure_pairing_task_is_current(generation, task_generation)?;
    commit()
}

fn recovery_result_after_completion(
    imported: Result<(), String>,
    _completion_result: Result<(), String>,
) -> Result<(), String> {
    // Once the identity is durable, notifying the peer cannot roll it back.
    imported
}

fn finish_recovery(
    imported: Result<(), String>,
    completion_result: Result<(), String>,
    context: &PairingTaskContext,
    app: &AppHandle,
) -> Result<(), String> {
    if !pairing_task_is_current(&context.generation, context.task_generation) {
        return Ok(());
    }

    match recovery_result_after_completion(imported, completion_result) {
        Ok(()) => {
            let _ = app.emit("pairing-complete", serde_json::json!({}));
        }
        Err(message) => {
            let _ = app.emit("pairing-error", PairingErrorPayload { message });
        }
    }
    Ok(())
}

fn pairing_task_is_current(generation: &AtomicU64, task_generation: u64) -> bool {
    generation.load(Ordering::SeqCst) == task_generation
}

fn validate_recovery_payload_type(payload_type: PayloadType) -> Result<(), String> {
    if payload_type == PayloadType::Nsec {
        Ok(())
    } else {
        Err("Mobile device sent an unsupported recovery payload".into())
    }
}

async fn clear_pairing_session_if_current(
    session: &Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    generation: &AtomicU64,
    task_generation: u64,
) {
    let mut session = session.lock().await;
    if pairing_task_is_current(generation, task_generation) {
        *session = None;
    }
}

async fn handle_nip42_auth<R, W>(
    read: &mut R,
    write: &mut W,
    session: &Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    relay_url: &str,
) -> Result<(), String>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let auth_result = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| "relay closed during auth".to_string())?
                .map_err(|e| format!("WS error during auth: {e}"))?;
            if let Message::Text(text) = msg {
                if let Some(challenge) = parse_auth_challenge(text.as_str()) {
                    return Ok(challenge);
                }
            }
        }
    })
    .await;

    let challenge: String = match auth_result {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(()),
    };

    let relay_url_parsed =
        nostr::RelayUrl::parse(relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
    let auth_json = {
        let guard = session.lock().await;
        let s = guard.as_ref().ok_or("session gone during auth")?;
        let auth_event = s
            .sign_event(nostr::EventBuilder::auth(challenge, relay_url_parsed))
            .map_err(|e| format!("sign auth event: {e}"))?;
        format!("[\"AUTH\",{}]", nostr::JsonUtil::as_json(&auth_event))
    };

    write
        .send(Message::Text(auth_json.into()))
        .await
        .map_err(|e| format!("send auth: {e}"))?;

    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| "relay closed during auth OK".to_string())?
                .map_err(|e| format!("WS error during auth OK: {e}"))?;
            if let Message::Text(text) = msg {
                if text.contains("\"OK\"") || text.contains("[\"OK\"") {
                    return Ok::<(), String>(());
                }
            }
        }
    })
    .await;

    Ok(())
}

/// Serialize a nostr 0.36 Event to `["EVENT", <event>]` JSON string.
fn event_to_relay_json(event: &nostr::Event) -> String {
    format!("[\"EVENT\",{}]", nostr::JsonUtil::as_json(event))
}

/// Parse a relay EVENT message into a nostr 0.36 Event (buzz-core compatible).
fn parse_relay_event(text: &str, sub_id: &str) -> Option<nostr::Event> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;
    if arr.len() < 3 {
        return None;
    }
    if arr[0].as_str()? != "EVENT" {
        return None;
    }
    if arr[1].as_str()? != sub_id {
        return None;
    }
    serde_json::from_value(arr[2].clone()).ok()
}

/// Pairing route discovered from the main relay's NIP-11 document.
#[derive(Debug, PartialEq, Eq)]
enum PairingRelay {
    Configured(String),
    LegacyPath,
    MainRelay,
}

/// Prefer the relay-advertised dedicated pairing URL. The legacy `/pair`
/// convention remains as a compatibility fallback for NIP-43 relays that do
/// not advertise the extension yet.
async fn probe_pairing_relay(relay_url: &str) -> PairingRelay {
    let http_url = if let Some(rest) = relay_url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = relay_url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        return PairingRelay::MainRelay;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let resp = match client
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return PairingRelay::MainRelay,
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(value) => value,
        Err(_) => return PairingRelay::MainRelay,
    };

    pairing_relay_from_nip11(&json)
}

fn resolve_pairing_relay_url(
    main_relay_url: &str,
    pairing_relay: PairingRelay,
) -> Result<String, String> {
    match pairing_relay {
        PairingRelay::Configured(url) => Ok(url),
        PairingRelay::LegacyPath => {
            let mut url =
                url::Url::parse(main_relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&format!("{path}/pair"));
            Ok(url.to_string())
        }
        PairingRelay::MainRelay => Ok(main_relay_url.to_string()),
    }
}

fn pairing_relay_from_nip11(json: &serde_json::Value) -> PairingRelay {
    if let Some(value) = json
        .get("pairing_relay_url")
        .and_then(|value| value.as_str())
    {
        if let Ok(url) = url::Url::parse(value) {
            if matches!(url.scheme(), "ws" | "wss") && url.host_str().is_some() {
                return PairingRelay::Configured(value.to_string());
            }
        }
    }

    if json
        .get("supported_nips")
        .and_then(|value| value.as_array())
        .is_some_and(|nips| nips.iter().any(|nip| nip.as_u64() == Some(43)))
    {
        PairingRelay::LegacyPath
    } else {
        PairingRelay::MainRelay
    }
}

fn parse_auth_challenge(text: &str) -> Option<String> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;
    if arr.len() >= 2 && arr[0].as_str()? == "AUTH" {
        return arr[1].as_str().map(|s| s.to_string());
    }
    None
}

async fn wait_for_eose<S>(read: &mut S, sub_id: &str, dur: Duration) -> Result<(), String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    tokio::time::timeout(dur, async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| "relay closed waiting for EOSE".to_string())?
                .map_err(|e| format!("WS error waiting for EOSE: {e}"))?;
            if let Message::Text(text) = msg {
                if let Ok(arr) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                    if let Some(arr) = arr.as_array() {
                        if arr.len() >= 2
                            && arr[0].as_str() == Some("EOSE")
                            && arr[1].as_str() == Some(sub_id)
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for EOSE".to_string())?
}

#[cfg(test)]
#[path = "pairing_generation_tests.rs"]
mod pairing_generation_tests;

#[cfg(test)]
#[path = "pairing_relay_tests.rs"]
mod pairing_relay_tests;
