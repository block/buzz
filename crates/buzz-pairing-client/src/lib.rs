#![deny(unsafe_code)]

//! UI-independent NIP-AB source-side pairing orchestration.
//!
//! This crate owns the ephemeral WebSocket connection and protocol session.
//! Callers provide the active Buzz identity and receive typed events suitable
//! for a desktop, terminal, or other UI adapter.

use std::fmt;
use std::time::Duration;

use buzz_core::kind::KIND_PAIRING;
use buzz_core::pairing::qr::encode_qr;
use buzz_core::pairing::{AbortReason, PairingError, PairingSession, PayloadType, SessionState};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, RelayUrl, ToBech32};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use zeroize::Zeroizing;

const AUTH_CHALLENGE_GRACE: Duration = Duration::from_secs(3);
const AUTH_OK_TIMEOUT: Duration = Duration::from_secs(5);
const EOSE_TIMEOUT: Duration = Duration::from_secs(10);
const SESSION_TIMEOUT: Duration = Duration::from_secs(120);
const CONTROL_CAPACITY: usize = 4;
const PAIRING_SUBSCRIPTION_ID: &str = "pair";

/// Inputs frozen when a source pairing session starts.
pub struct PairingSourceConfig {
    /// HTTP URL stored by mobile as the paired Buzz community.
    pub relay_http_url: String,
    /// Active Nostr private key, in nsec or hex form.
    private_key: Zeroizing<String>,
}

impl PairingSourceConfig {
    /// Freeze the relay and identity used by one pairing session.
    pub fn new(relay_http_url: String, private_key: String) -> Self {
        Self {
            relay_http_url,
            private_key: Zeroizing::new(private_key),
        }
    }
}

impl fmt::Debug for PairingSourceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingSourceConfig")
            .field("relay_http_url", &self.relay_http_url)
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

/// Short-lived `nostrpair://` URI, zeroized when dropped.
#[derive(Eq, PartialEq)]
pub struct PairingUri(Zeroizing<String>);

impl PairingUri {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// Borrow the URI for QR rendering or explicit clipboard transfer.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for PairingUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairingUri([REDACTED])")
    }
}

/// User actions accepted by an active source pairing session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingCommand {
    /// The user verified that both devices display the same SAS code.
    ConfirmSas,
    /// Abort the session because the user closed or denied it.
    Cancel,
}

/// Observable source-side pairing state changes.
#[derive(Debug, Eq, PartialEq)]
pub enum PairingEvent {
    /// The relay subscription is registered and the URI is ready to scan.
    Ready { uri: PairingUri },
    /// Mobile sent a valid offer and both devices can compare this SAS.
    SasReceived { code: String },
    /// Mobile imported and validated the transferred credentials.
    Complete,
    /// The peer or local user aborted the session.
    Aborted { reason: String },
    /// Pairing stopped because of a protocol, relay, or configuration error.
    Failed { message: String },
}

/// Errors returned before or while controlling a pairing task.
#[derive(Debug, Error)]
pub enum PairingClientError {
    /// No pairing task is active.
    #[error("no active pairing session")]
    NotActive,
    /// The active task is no longer accepting commands.
    #[error("pairing session is no longer available")]
    Closed,
}

/// Single-session source pairing runtime.
///
/// Starting a new session cancels and joins the previous session first.
pub struct PairingSourceRuntime {
    control_tx: Option<mpsc::Sender<PairingCommand>>,
    task: Option<JoinHandle<()>>,
}

impl Default for PairingSourceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingSourceRuntime {
    /// Construct an idle runtime.
    pub fn new() -> Self {
        Self {
            control_tx: None,
            task: None,
        }
    }

    /// Start a source session and forward its state changes to `event_tx`.
    pub async fn start(
        &mut self,
        config: PairingSourceConfig,
        event_tx: mpsc::UnboundedSender<PairingEvent>,
    ) {
        self.stop().await;
        let (control_tx, control_rx) = mpsc::channel(CONTROL_CAPACITY);
        self.control_tx = Some(control_tx);
        self.task = Some(tokio::spawn(async move {
            if let Err(error) = run_source(config, control_rx, &event_tx).await {
                let _ = event_tx.send(PairingEvent::Failed {
                    message: error.to_string(),
                });
            }
        }));
    }

    /// Send a user action to the active pairing task.
    pub fn send(&self, command: PairingCommand) -> Result<(), PairingClientError> {
        let tx = self
            .control_tx
            .as_ref()
            .ok_or(PairingClientError::NotActive)?;
        tx.try_send(command).map_err(|_| PairingClientError::Closed)
    }

    /// Best-effort cancel and join the active pairing task.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.control_tx.take() {
            let _ = tx.send(PairingCommand::Cancel).await;
        }
        if let Some(task) = self.task.take() {
            let mut task = task;
            if tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }

    /// Whether this runtime currently owns a task.
    pub fn is_active(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

impl Drop for PairingSourceRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Debug, Error)]
enum RunError {
    #[error("invalid private key: {0}")]
    PrivateKey(String),
    #[error("encode nsec: {0}")]
    EncodeNsec(String),
    #[error("invalid relay URL: {0}")]
    RelayUrl(String),
    #[error("WebSocket connection failed: {0}")]
    WebSocketConnect(String),
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("pairing protocol error: {0}")]
    Protocol(#[from] PairingError),
    #[error("pairing session timed out")]
    Timeout,
    #[error("pairing relay closed the connection")]
    ConnectionClosed,
    #[error("pairing relay subscription was closed: {0}")]
    SubscriptionClosed(String),
    #[error("mobile device reported failure importing credentials")]
    ImportFailed,
}

async fn run_source(
    config: PairingSourceConfig,
    mut control_rx: mpsc::Receiver<PairingCommand>,
    event_tx: &mpsc::UnboundedSender<PairingEvent>,
) -> Result<(), RunError> {
    let keys = Keys::parse(config.private_key.as_str())
        .map_err(|error| RunError::PrivateKey(error.to_string()))?;
    let nsec = Zeroizing::new(
        keys.secret_key()
            .to_bech32()
            .map_err(|error| RunError::EncodeNsec(error.to_string()))?,
    );
    let relay_http_url = normalize_http_url(&config.relay_http_url)?;
    let relay_ws_url = http_to_ws_url(&relay_http_url)?;
    let pairing_relay_url = discover_pairing_relay(&relay_http_url, &relay_ws_url).await;

    let (mut session, qr_payload) = PairingSession::new_source(pairing_relay_url.clone());
    let qr_uri = encode_qr(&qr_payload);
    let payload = Zeroizing::new(
        serde_json::json!({
            "relayUrl": relay_http_url,
            "pubkey": keys.public_key().to_hex(),
            "nsec": &*nsec,
        })
        .to_string(),
    );

    let (ws, _) = tokio_tungstenite::connect_async(&pairing_relay_url)
        .await
        .map_err(|error| RunError::WebSocketConnect(error.to_string()))?;
    let (mut write, mut read) = ws.split();
    handle_optional_nip42_auth(&mut read, &mut write, &session, &pairing_relay_url).await?;

    let subscription = serde_json::json!([
        "REQ",
        PAIRING_SUBSCRIPTION_ID,
        {
            "kinds": [KIND_PAIRING],
            "#p": [session.pubkey().to_hex()],
        }
    ]);
    write
        .send(Message::Text(subscription.to_string().into()))
        .await
        .map_err(|error| RunError::WebSocket(error.to_string()))?;
    wait_for_eose(&mut read).await?;

    let _ = event_tx.send(PairingEvent::Ready {
        uri: PairingUri::new(qr_uri),
    });
    let timeout = tokio::time::sleep(SESSION_TIMEOUT);
    tokio::pin!(timeout);
    let mut payload = Some(payload);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                publish_abort(&mut write, &mut session, AbortReason::Timeout).await;
                return Err(RunError::Timeout);
            }
            command = control_rx.recv() => {
                match command {
                    Some(PairingCommand::Cancel) | None => {
                        publish_abort(&mut write, &mut session, AbortReason::UserDenied).await;
                        let _ = event_tx.send(PairingEvent::Aborted {
                            reason: "user denied".to_string(),
                        });
                        return Ok(());
                    }
                    Some(PairingCommand::ConfirmSas) => {
                        if session.state() != SessionState::Confirming {
                            continue;
                        }
                        let confirm = session.confirm_sas()?;
                        publish_event(&mut write, &confirm).await?;
                        if let Some(payload) = payload.take() {
                            let event = session.send_payload(PayloadType::Custom, payload)?;
                            publish_event(&mut write, &event).await?;
                        }
                    }
                }
            }
            message = read.next() => {
                let message = message.ok_or(RunError::ConnectionClosed)?
                    .map_err(|error| RunError::WebSocket(error.to_string()))?;
                let Message::Text(text) = message else {
                    continue;
                };
                let Some(event) = parse_relay_event(text.as_str()) else {
                    if let Some(reason) = parse_closed_message(text.as_str()) {
                        return Err(RunError::SubscriptionClosed(reason));
                    }
                    continue;
                };

                if let Ok(reason) = session.handle_abort(&event) {
                    let _ = event_tx.send(PairingEvent::Aborted {
                        reason: format!("{reason:?}"),
                    });
                    return Ok(());
                }
                if let Ok(code) = session.handle_offer(&event) {
                    let _ = event_tx.send(PairingEvent::SasReceived { code });
                    continue;
                }
                match session.handle_complete(&event) {
                    Ok(()) => {
                        let _ = event_tx.send(PairingEvent::Complete);
                        return Ok(());
                    }
                    Err(PairingError::UnexpectedMessage { got, .. })
                        if got == "complete(success=false)" =>
                    {
                        return Err(RunError::ImportFailed);
                    }
                    Err(_) => {}
                }
            }
        }
    }
}

async fn publish_abort<S>(write: &mut S, session: &mut PairingSession, reason: AbortReason)
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if let Ok(Some(event)) = session.abort(reason) {
        let _ = publish_event(write, &event).await;
    }
}

async fn publish_event<S>(write: &mut S, event: &Event) -> Result<(), RunError>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let message = format!("[\"EVENT\",{}]", JsonUtil::as_json(event));
    write
        .send(Message::Text(message.into()))
        .await
        .map_err(|error| RunError::WebSocket(error.to_string()))
}

async fn handle_optional_nip42_auth<R, W>(
    read: &mut R,
    write: &mut W,
    session: &PairingSession,
    relay_url: &str,
) -> Result<(), RunError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let challenge = match tokio::time::timeout(AUTH_CHALLENGE_GRACE, async {
        loop {
            let message = read.next().await.ok_or(RunError::ConnectionClosed)?;
            let message = message.map_err(|error| RunError::WebSocket(error.to_string()))?;
            if let Message::Text(text) = message {
                if let Some(challenge) = parse_auth_challenge(text.as_str()) {
                    return Ok::<String, RunError>(challenge);
                }
            }
        }
    })
    .await
    {
        Ok(result) => result?,
        Err(_) => return Ok(()),
    };

    let relay_url =
        RelayUrl::parse(relay_url).map_err(|error| RunError::RelayUrl(error.to_string()))?;
    let auth_event = session.sign_event(EventBuilder::auth(challenge, relay_url))?;
    let event_id = auth_event.id.to_hex();
    let message = format!("[\"AUTH\",{}]", JsonUtil::as_json(&auth_event));
    write
        .send(Message::Text(message.into()))
        .await
        .map_err(|error| RunError::WebSocket(error.to_string()))?;

    tokio::time::timeout(AUTH_OK_TIMEOUT, async {
        loop {
            let message = read.next().await.ok_or(RunError::ConnectionClosed)?;
            let message = message.map_err(|error| RunError::WebSocket(error.to_string()))?;
            if let Message::Text(text) = message {
                if let Some((ok_event_id, accepted, reason)) = parse_ok(text.as_str()) {
                    if ok_event_id == event_id {
                        return if accepted {
                            Ok(())
                        } else {
                            Err(RunError::WebSocket(format!(
                                "pairing relay rejected authentication: {reason}"
                            )))
                        };
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| RunError::WebSocket("pairing relay did not confirm authentication".into()))?
}

async fn wait_for_eose<R>(read: &mut R) -> Result<(), RunError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    tokio::time::timeout(EOSE_TIMEOUT, async {
        loop {
            let message = read.next().await.ok_or(RunError::ConnectionClosed)?;
            let message = message.map_err(|error| RunError::WebSocket(error.to_string()))?;
            if let Message::Text(text) = message {
                if is_eose(text.as_str()) {
                    return Ok(());
                }
                if let Some(reason) = parse_closed_message(text.as_str()) {
                    return Err(RunError::SubscriptionClosed(reason));
                }
            }
        }
    })
    .await
    .map_err(|_| RunError::WebSocket("timeout waiting for pairing subscription".into()))?
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PairingRelay {
    Configured(String),
    LegacyPath,
    MainRelay,
}

async fn discover_pairing_relay(http_url: &str, ws_url: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let document = match client
        .get(http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    {
        Ok(response) => response.json::<Value>().await.ok(),
        Err(_) => None,
    };
    resolve_pairing_relay_url(ws_url, pairing_relay_from_nip11(document.as_ref()))
        .unwrap_or_else(|_| ws_url.to_string())
}

fn pairing_relay_from_nip11(document: Option<&Value>) -> PairingRelay {
    let Some(document) = document else {
        return PairingRelay::MainRelay;
    };
    if let Some(value) = document.get("pairing_relay_url").and_then(Value::as_str) {
        if let Ok(url) = url::Url::parse(value) {
            if matches!(url.scheme(), "ws" | "wss") && url.host_str().is_some() {
                return PairingRelay::Configured(value.to_string());
            }
        }
    }
    if document
        .get("supported_nips")
        .and_then(Value::as_array)
        .is_some_and(|nips| nips.iter().any(|nip| nip.as_u64() == Some(43)))
    {
        PairingRelay::LegacyPath
    } else {
        PairingRelay::MainRelay
    }
}

fn resolve_pairing_relay_url(
    main_relay_url: &str,
    pairing_relay: PairingRelay,
) -> Result<String, RunError> {
    match pairing_relay {
        PairingRelay::Configured(url) => Ok(url),
        PairingRelay::LegacyPath => {
            let mut url = url::Url::parse(main_relay_url)
                .map_err(|error| RunError::RelayUrl(error.to_string()))?;
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&format!("{path}/pair"));
            Ok(url.to_string())
        }
        PairingRelay::MainRelay => Ok(main_relay_url.to_string()),
    }
}

fn normalize_http_url(value: &str) -> Result<String, RunError> {
    let value = value.trim().trim_end_matches('/');
    let value = if let Some(rest) = value.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if let Some(rest) = value.strip_prefix("wss://") {
        format!("https://{rest}")
    } else {
        value.to_string()
    };
    let parsed = url::Url::parse(&value).map_err(|error| RunError::RelayUrl(error.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(RunError::RelayUrl(
            "relay must be an HTTP or HTTPS URL with a host".to_string(),
        ));
    }
    Ok(value)
}

fn http_to_ws_url(value: &str) -> Result<String, RunError> {
    let mut url = url::Url::parse(value).map_err(|error| RunError::RelayUrl(error.to_string()))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => {
            return Err(RunError::RelayUrl(
                "relay must use HTTP or HTTPS".to_string(),
            ));
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| RunError::RelayUrl("could not convert relay URL".to_string()))?;
    Ok(url.to_string())
}

fn parse_relay_event(text: &str) -> Option<Event> {
    let array = serde_json::from_str::<Value>(text).ok()?;
    let array = array.as_array()?;
    if array.first()?.as_str()? != "EVENT" || array.get(1)?.as_str()? != PAIRING_SUBSCRIPTION_ID {
        return None;
    }
    serde_json::from_value(array.get(2)?.clone()).ok()
}

fn parse_auth_challenge(text: &str) -> Option<String> {
    let array = serde_json::from_str::<Value>(text).ok()?;
    let array = array.as_array()?;
    (array.first()?.as_str()? == "AUTH")
        .then(|| array.get(1)?.as_str().map(ToString::to_string))
        .flatten()
}

fn parse_ok(text: &str) -> Option<(String, bool, String)> {
    let array = serde_json::from_str::<Value>(text).ok()?;
    let array = array.as_array()?;
    if array.first()?.as_str()? != "OK" {
        return None;
    }
    Some((
        array.get(1)?.as_str()?.to_string(),
        array.get(2)?.as_bool()?,
        array
            .get(3)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ))
}

fn is_eose(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .is_some_and(|array| {
            array.first().and_then(Value::as_str) == Some("EOSE")
                && array.get(1).and_then(Value::as_str) == Some(PAIRING_SUBSCRIPTION_ID)
        })
}

fn parse_closed_message(text: &str) -> Option<String> {
    let array = serde_json::from_str::<Value>(text).ok()?;
    let array = array.as_array()?;
    if array.first()?.as_str()? != "CLOSED" || array.get(1)?.as_str()? != PAIRING_SUBSCRIPTION_ID {
        return None;
    }
    Some(
        array
            .get(2)
            .and_then(Value::as_str)
            .unwrap_or("unknown reason")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_pairing_relay_takes_precedence() {
        let document = serde_json::json!({
            "pairing_relay_url": "wss://pairing.buzz.xyz",
            "supported_nips": [43]
        });
        assert_eq!(
            pairing_relay_from_nip11(Some(&document)),
            PairingRelay::Configured("wss://pairing.buzz.xyz".to_string())
        );
    }

    #[test]
    fn invalid_configured_url_falls_back_to_legacy_path() {
        let document = serde_json::json!({
            "pairing_relay_url": "https://pairing.buzz.xyz",
            "supported_nips": [43]
        });
        assert_eq!(
            pairing_relay_from_nip11(Some(&document)),
            PairingRelay::LegacyPath
        );
    }

    #[test]
    fn open_relay_uses_main_socket() {
        let document = serde_json::json!({"supported_nips": [1, 11, 42]});
        assert_eq!(
            pairing_relay_from_nip11(Some(&document)),
            PairingRelay::MainRelay
        );
    }

    #[test]
    fn legacy_path_preserves_existing_relay_path() {
        assert_eq!(
            resolve_pairing_relay_url("wss://relay.example/community", PairingRelay::LegacyPath)
                .expect("valid URL"),
            "wss://relay.example/community/pair"
        );
    }

    #[test]
    fn relay_url_normalization_preserves_mobile_http_payload() {
        assert_eq!(
            normalize_http_url("wss://relay.example/community/").expect("valid URL"),
            "https://relay.example/community"
        );
        assert_eq!(
            http_to_ws_url("https://relay.example/community").expect("valid URL"),
            "wss://relay.example/community"
        );
    }

    #[test]
    fn parses_only_the_pairing_subscription() {
        assert!(!is_eose(r#"["EOSE","other"]"#));
        assert!(is_eose(r#"["EOSE","pair"]"#));
        assert_eq!(
            parse_closed_message(r#"["CLOSED","pair","rate-limited"]"#).as_deref(),
            Some("rate-limited")
        );
    }

    #[test]
    fn secret_debug_values_are_redacted() {
        let config = PairingSourceConfig::new(
            "https://relay.example".to_string(),
            "nsec1secret".to_string(),
        );
        assert!(!format!("{config:?}").contains("nsec1secret"));

        let event = PairingEvent::Ready {
            uri: PairingUri::new("nostrpair://secret".to_string()),
        };
        assert!(!format!("{event:?}").contains("nostrpair://secret"));
    }
}
