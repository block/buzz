//! Guarded read-only relay transport for sticker-pack imports.
//!
//! Sticker pack links carry attacker-controlled `relay=` hints, so this module
//! owns the socket instead of `NostrWsConnection`: the hint host is resolved
//! first, every resolved address is checked against
//! [`buzz_core_pkg::network::is_private_ip`], and the WebSocket handshake runs
//! over a TCP stream already connected to that vetted address. Handing a URL to
//! a connector that re-resolves it would reopen the DNS-rebinding window
//! between the check and the connect.

use std::net::SocketAddr;
use std::time::Duration;

use buzz_ws_client::{parse_relay_message, RelayMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Instant};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

/// Upper bound on events collected from one relay for a single subscription.
/// The REQ asks for a handful of revisions; a hostile relay must not be able
/// to stream indefinitely inside the fetch budget.
const MAX_RELAY_EVENTS: usize = 16;

/// Frame/message ceiling for imported pack events. Sticker packs are small
/// JSON documents, so anything larger is a resource-exhaustion attempt.
const MAX_RELAY_MESSAGE_BYTES: usize = 512 * 1024;

/// A relay hint that passed scheme, credential, and port validation.
///
/// Construction does not perform DNS resolution; [`fetch_events`] resolves and
/// screens the addresses immediately before connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelayHint {
    /// The hint URL as given, used for the handshake `Host` header and SNI.
    pub(crate) url: String,
    host: String,
    port: u16,
    /// Set only for the plaintext local-development exception, where the
    /// private-address screen would otherwise reject loopback by definition.
    allow_private: bool,
}

fn is_local_dev_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Validate an untrusted `relay=` hint from a sticker pack link.
///
/// Accepts `wss://` only. Plaintext `ws://` is a local-development exception:
/// it is honored for loopback hosts in debug builds (`just relay` serves
/// `ws://localhost:3000`) and rejected outright in release builds.
pub(crate) fn validate_relay_hint(raw: &str) -> Result<RelayHint, String> {
    let parsed = url::Url::parse(raw).map_err(|_| format!("Invalid relay hint: {raw}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("Relay hint has no host: {raw}"))?
        .to_string();
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("Relay hints must not carry credentials: {raw}"));
    }
    let allow_private = match parsed.scheme() {
        "wss" => false,
        "ws" if cfg!(debug_assertions) && is_local_dev_host(&host) => true,
        "ws" => {
            return Err(format!(
                "Relay hints must use wss:// (plaintext ws:// is local-development only): {raw}"
            ))
        }
        _ => return Err(format!("Relay hints must be ws(s) URLs: {raw}")),
    };
    let port = parsed
        .port()
        .unwrap_or(if allow_private { 80 } else { 443 });
    Ok(RelayHint {
        url: raw.to_string(),
        host,
        port,
        allow_private,
    })
}

/// Resolve the hint host and return the address to connect to, rejecting the
/// hint when resolution is empty or *any* answer is private/reserved. Checking
/// every answer stops a multi-record response from smuggling one private
/// address past a first-record-only check.
async fn pin_relay_address(hint: &RelayHint) -> Result<SocketAddr, String> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((hint.host.as_str(), hint.port))
        .await
        .map_err(|_| format!("Could not resolve relay hint: {}", hint.url))?
        .collect();
    let Some(first) = addresses.first().copied() else {
        return Err(format!("Could not resolve relay hint: {}", hint.url));
    };
    if !hint.allow_private
        && addresses
            .iter()
            .any(|address| buzz_core_pkg::network::is_private_ip(&address.ip()))
    {
        return Err(format!(
            "Relay hint resolves to a private or reserved address: {}",
            hint.url
        ));
    }
    Ok(first)
}

/// Run one unauthenticated REQ against `hint` and collect the events it
/// returns for `subscription_id`.
///
/// `budget` bounds the entire operation — DNS, TCP connect, TLS handshake,
/// request send, and the read loop — so a slow or hostile hint cannot stall
/// the caller for longer than one budget. Events are returned unverified;
/// callers must check signatures before trusting them.
pub(crate) async fn fetch_events(
    hint: &RelayHint,
    request: &serde_json::Value,
    subscription_id: &str,
    budget: Duration,
) -> Result<Vec<nostr::Event>, String> {
    let deadline = Instant::now() + budget;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| "relay fetch timed out".to_string())
    };

    let address = timeout(remaining()?, pin_relay_address(hint))
        .await
        .map_err(|_| "relay fetch timed out".to_string())??;
    let stream = timeout(remaining()?, TcpStream::connect(address))
        .await
        .map_err(|_| "relay connect timed out".to_string())?
        .map_err(|error| format!("Could not reach relay hint: {error}"))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_RELAY_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_RELAY_MESSAGE_BYTES));
    let (mut ws, _response) = timeout(
        remaining()?,
        tokio_tungstenite::client_async_tls_with_config(
            hint.url.as_str(),
            stream,
            Some(config),
            None,
        ),
    )
    .await
    .map_err(|_| "relay connect timed out".to_string())?
    .map_err(|error| format!("Could not open relay hint: {error}"))?;

    timeout(
        remaining()?,
        ws.send(Message::Text(request.to_string().into())),
    )
    .await
    .map_err(|_| "relay request timed out".to_string())?
    .map_err(|error| format!("Could not query relay hint: {error}"))?;

    let mut events = Vec::new();
    while events.len() < MAX_RELAY_EVENTS {
        let Ok(left) = remaining() else { break };
        let Ok(Some(Ok(message))) = timeout(left, ws.next()).await else {
            break;
        };
        let Message::Text(text) = message else {
            // Ping/Pong are answered by the protocol layer; Binary and Close
            // carry nothing this read loop needs.
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        match parse_relay_message(&text) {
            Ok(RelayMessage::Event {
                subscription_id: id,
                event,
            }) if id == subscription_id => events.push(*event),
            Ok(RelayMessage::Eose {
                subscription_id: id,
            })
            | Ok(RelayMessage::Closed {
                subscription_id: id,
                ..
            }) if id == subscription_id => break,
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_wss_hint() {
        let hint = validate_relay_hint("wss://relay.damus.io").expect("valid hint");
        assert_eq!(hint.host, "relay.damus.io");
        assert_eq!(hint.port, 443);
        assert!(!hint.allow_private);
    }

    #[test]
    fn rejects_non_ws_schemes_and_credentials() {
        assert!(validate_relay_hint("https://evil.example").is_err());
        assert!(validate_relay_hint("wss://user:pass@relay.example").is_err());
        assert!(validate_relay_hint("wss://").is_err());
        assert!(validate_relay_hint("not a url").is_err());
    }

    #[test]
    fn plaintext_ws_is_limited_to_local_development() {
        // Non-loopback plaintext is rejected in every build profile.
        assert!(validate_relay_hint("ws://relay.example").is_err());
        let loopback = validate_relay_hint("ws://127.0.0.1:3000");
        if cfg!(debug_assertions) {
            let hint = loopback.expect("loopback dev hint");
            assert!(hint.allow_private);
            assert_eq!(hint.port, 3000);
        } else {
            assert!(loopback.is_err());
        }
    }

    #[tokio::test]
    async fn private_addresses_are_rejected_before_connecting() {
        // Literal hosts resolve without a network round trip, so this asserts
        // the screen itself rather than DNS behavior.
        let hint = RelayHint {
            url: "wss://internal.example".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
            allow_private: false,
        };
        let error = pin_relay_address(&hint).await.expect_err("blocked");
        assert!(error.contains("private or reserved"), "{error}");

        let link_local = RelayHint {
            host: "169.254.169.254".to_string(),
            ..hint.clone()
        };
        assert!(pin_relay_address(&link_local).await.is_err());
    }

    #[tokio::test]
    async fn local_development_hint_may_resolve_to_loopback() {
        let hint = RelayHint {
            url: "ws://127.0.0.1:3000".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
            allow_private: true,
        };
        let address = pin_relay_address(&hint).await.expect("loopback allowed");
        assert!(address.ip().is_loopback());
    }

    #[tokio::test]
    async fn fetch_stops_at_the_budget() {
        // 198.51.100.0/24 (TEST-NET-2) is unroutable, so the connect stalls
        // until the budget expires instead of erroring instantly.
        let hint = RelayHint {
            url: "wss://blackhole.example".to_string(),
            host: "198.51.100.1".to_string(),
            port: 443,
            allow_private: false,
        };
        let started = Instant::now();
        let result = fetch_events(
            &hint,
            &serde_json::json!(["REQ", "s", {}]),
            "s",
            Duration::from_millis(200),
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
