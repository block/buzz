use std::sync::Arc;
use std::time::Duration;

use buzz_core_pkg::kind::KIND_PAIRING;
use buzz_core_pkg::pairing::qr::encode_qr;
use buzz_core_pkg::pairing::session::PairingSession;
use buzz_core_pkg::pairing::types::{AbortReason, PayloadType};
use futures_util::{SinkExt, StreamExt};
use nostr::ToBech32;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
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

/// Managed Tauri state for an active pairing session.
pub struct PairingHandle {
    session: Arc<tokio::sync::Mutex<Option<PairingSession>>>,
    cancel: std::sync::Mutex<Option<CancellationToken>>,
    /// Send JSON-serialized events to the background WS task for relay publication.
    outbound_tx: std::sync::Mutex<Option<mpsc::Sender<String>>>,
    /// Pre-built payload string (contains nsec) to send after SAS confirmation.
    /// Wrapped in Zeroizing so the nsec is cleared from memory on drop.
    payload: std::sync::Mutex<Option<Zeroizing<String>>>,
}

impl PairingHandle {
    pub fn new() -> Self {
        Self {
            session: Arc::new(tokio::sync::Mutex::new(None)),
            cancel: std::sync::Mutex::new(None),
            outbound_tx: std::sync::Mutex::new(None),
            payload: std::sync::Mutex::new(None),
        }
    }

    fn clear(&self) {
        *self.cancel.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.outbound_tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.payload.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Start a NIP-AB pairing session as the source device.
///
/// Creates a `PairingSession`, connects to the relay, and returns the
/// `nostrpair://` QR URI for the frontend to display. The mobile peer will
/// receive the desktop's nsec (NIP-OA auth — no token minting needed).
#[tauri::command]
pub async fn start_pairing(
    app: AppHandle,
    state: State<'_, AppState>,
    pairing: State<'_, PairingHandle>,
) -> Result<String, String> {
    if let Some(token) = pairing.cancel.lock().map_err(|e| e.to_string())?.take() {
        token.cancel();
    }
    pairing.clear();

    let keys = state.signing_keys()?;
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|e| format!("encode nsec: {e}"))?;
    let pubkey_hex = keys.public_key().to_hex();

    let ws_url = relay_ws_url_with_override(&state);
    let http_url = relay_api_base_url_with_override(&state);

    // NIP-43 relays gate connections on membership, so an unpaired peer can't
    // reach the main relay yet — it must go through the /pair sidecar. Open
    // relays (no NIP-43) accept the peer directly. We key off the relay's
    // own NIP-11 declaration of NIP-43 support rather than `auth_required`,
    // which is also true for plain NIP-42 / NIP-OA relays where the main
    // relay is reachable.
    let qr_relay_url = match probe_pairing_relay(&ws_url).await {
        PairingRelay::Configured(url) => url,
        PairingRelay::LegacyPath => {
            let mut url =
                url::Url::parse(&ws_url).map_err(|e| format!("invalid relay URL: {e}"))?;
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&format!("{path}/pair"));
            url.to_string()
        }
        PairingRelay::MainRelay => ws_url.clone(),
    };

    let (session, qr_payload) = PairingSession::new_source(qr_relay_url);
    let qr_uri = encode_qr(&qr_payload);

    let payload_json = serde_json::json!({
        "relayUrl": http_url,
        "pubkey": pubkey_hex,
        "nsec": nsec,
    });

    {
        let mut s = pairing.session.lock().await;
        *s = Some(session);
    }
    *pairing.payload.lock().map_err(|e| e.to_string())? =
        Some(Zeroizing::new(payload_json.to_string()));

    let (outbound_tx, outbound_rx) = mpsc::channel::<String>(16);
    let cancel = CancellationToken::new();

    *pairing.outbound_tx.lock().map_err(|e| e.to_string())? = Some(outbound_tx);
    *pairing.cancel.lock().map_err(|e| e.to_string())? = Some(cancel.clone());

    let session_arc = Arc::clone(&pairing.session);
    tauri::async_runtime::spawn(pairing_ws_task(
        ws_url,
        session_arc,
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
        .map_err(|_| "failed to send sas-confirm")?;

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

    Ok(())
}

/// Cancel the active pairing session.
#[tauri::command]
pub async fn cancel_pairing(pairing: State<'_, PairingHandle>) -> Result<(), String> {
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

    if let Some(token) = pairing.cancel.lock().map_err(|e| e.to_string())?.take() {
        token.cancel();
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
    cancel: CancellationToken,
    mut outbound_rx: mpsc::Receiver<String>,
    app: AppHandle,
) {
    if let Err(e) =
        pairing_ws_task_inner(&relay_url, &session, &cancel, &mut outbound_rx, &app).await
    {
        let _ = app.emit("pairing-error", PairingErrorPayload { message: e });
    }
    let mut s = session.lock().await;
    *s = None;
}

async fn pairing_ws_task_inner(
    relay_url: &str,
    session: &Arc<tokio::sync::Mutex<Option<PairingSession>>>,
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
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = &mut hard_timeout => {
                let _ = app.emit("pairing-error", PairingErrorPayload {
                    message: "Session timed out".into(),
                });
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
                    let mut guard = session.lock().await;
                    let Some(s) = guard.as_mut() else { break };

                    if let Ok(reason) = s.handle_abort(&event) {
                        let _ = app.emit("pairing-aborted", PairingAbortedPayload {
                            reason: format!("{reason:?}"),
                        });
                        break;
                    }

                    if let Ok(sas) = s.handle_offer(&event) {
                        let _ = app.emit("pairing-sas-received", PairingSasPayload { sas });
                        continue;
                    }

                    match s.handle_complete(&event) {
                        Ok(()) => {
                            let _ = app.emit("pairing-complete", serde_json::json!({}));
                            break;
                        }
                        Err(ref e) if format!("{e}").contains("success=false") => {
                            let _ = app.emit("pairing-error", PairingErrorPayload {
                                message: "Mobile device reported failure importing credentials".into(),
                            });
                            break;
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    Ok(())
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

#[cfg(test)]
mod pairing_relay_tests {
    use super::{pairing_relay_from_nip11, probe_pairing_relay, PairingRelay};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn live_nip11_probe_discovers_configured_pairing_relay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test NIP-11 server");
        let addr = listener.local_addr().expect("test server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept NIP-11 request");
            let mut request = vec![0; 2048];
            let bytes_read = stream.read(&mut request).await.expect("read request");
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            assert!(request.starts_with("GET / HTTP/1.1"));
            assert!(request
                .to_ascii_lowercase()
                .contains("accept: application/nostr+json"));

            let body = r#"{"pairing_relay_url":"ws://127.0.0.1:5000"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/nostr+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        assert_eq!(
            probe_pairing_relay(&format!("ws://{addr}")).await,
            PairingRelay::Configured("ws://127.0.0.1:5000".to_string())
        );
        server.await.expect("NIP-11 server task");
    }

    #[test]
    fn configured_pairing_relay_takes_precedence_over_legacy_path() {
        let document = serde_json::json!({
            "pairing_relay_url": "wss://pairing.buzz.xyz",
            "supported_nips": [43]
        });

        assert_eq!(
            pairing_relay_from_nip11(&document),
            PairingRelay::Configured("wss://pairing.buzz.xyz".to_string())
        );
    }

    #[test]
    fn invalid_pairing_relay_url_falls_back_to_legacy_path() {
        let document = serde_json::json!({
            "pairing_relay_url": "https://pairing.buzz.xyz",
            "supported_nips": [43]
        });

        assert_eq!(
            pairing_relay_from_nip11(&document),
            PairingRelay::LegacyPath
        );
    }

    #[test]
    fn document_without_pairing_configuration_uses_main_relay() {
        let document = serde_json::json!({ "supported_nips": [1, 11] });

        assert_eq!(pairing_relay_from_nip11(&document), PairingRelay::MainRelay);
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
