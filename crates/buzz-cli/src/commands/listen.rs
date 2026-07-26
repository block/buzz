//! `buzz listen` — persistent WebSocket stream of channel events (#2663 Option B).
//!
//! Prints one normalized JSON object per inbound EVENT matching the subscription
//! filters (newline-delimited JSON on stdout). Uses the same NIP-42 / NIP-OA
//! path as `publish_ephemeral_event`.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use crate::client::{normalize_events, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_uuid;

/// Default kinds for channel traffic (matches `messages get` defaults).
const DEFAULT_KINDS: &[u32] = &[9, 40002, 40008, 45001, 45003];

fn parse_kinds(raw: Option<&str>) -> Result<Vec<u32>, CliError> {
    match raw {
        None => Ok(DEFAULT_KINDS.to_vec()),
        Some(s) if s.trim().is_empty() => Ok(DEFAULT_KINDS.to_vec()),
        Some(s) => {
            let mut out = Vec::new();
            for part in s.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let k: u32 = part
                    .parse()
                    .map_err(|_| CliError::Usage(format!("invalid kind in --kinds: {part}")))?;
                out.push(k);
            }
            if out.is_empty() {
                return Err(CliError::Usage("--kinds must list at least one kind".into()));
            }
            Ok(out)
        }
    }
}

/// Build subscription filter list for REQ.
pub(crate) fn build_listen_filters(
    channels: &[String],
    mentions_of_me: bool,
    my_pubkey_hex: &str,
    kinds: &[u32],
) -> Result<Vec<serde_json::Value>, CliError> {
    if channels.is_empty() && !mentions_of_me {
        return Err(CliError::Usage(
            "buzz listen requires --channel <UUID> and/or --mentions-of-me".into(),
        ));
    }
    for ch in channels {
        validate_uuid(ch)?;
    }

    let mut filters = Vec::new();
    if !channels.is_empty() {
        filters.push(json!({
            "kinds": kinds,
            "#h": channels,
        }));
    }
    if mentions_of_me {
        let mut f = json!({
            "kinds": kinds,
            "#p": [my_pubkey_hex],
        });
        if !channels.is_empty() {
            f["#h"] = json!(channels);
        }
        filters.push(f);
    }
    Ok(filters)
}

/// Optional POST of each event JSON body to `url` (Option A, best-effort).
async fn post_webhook(client: &reqwest::Client, url: &str, body: &str) {
    if let Err(e) = client
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
    {
        eprintln!(
            "{}",
            json!({"error":"webhook","message": format!("{e}")})
        );
    }
}

fn http_to_ws(http_url: &str) -> String {
    http_url
        .replace("https://", "wss://")
        .replace("http://", "ws://")
}

/// Run the listen loop until Ctrl-C or fatal error.
pub async fn cmd_listen(
    client: &BuzzClient,
    channels: Vec<String>,
    mentions_of_me: bool,
    kinds_raw: Option<String>,
    webhook: Option<String>,
    reconnect: bool,
) -> Result<(), CliError> {
    let kinds = parse_kinds(kinds_raw.as_deref())?;
    let my_pk = client.keys().public_key().to_hex();
    let filters = build_listen_filters(&channels, mentions_of_me, &my_pk, &kinds)?;
    let ws_url = http_to_ws(client.relay_url());
    let http = reqwest::Client::new();
    let running = Arc::new(AtomicBool::new(true));

    // Background Ctrl-C watcher (unix + windows via tokio::signal).
    {
        let r = running.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            r.store(false, Ordering::SeqCst);
        });
    }

    let mut backoff_ms: u64 = 500;
    const MAX_BACKOFF_MS: u64 = 30_000;

    while running.load(Ordering::SeqCst) {
        match listen_session(
            client,
            &ws_url,
            &filters,
            webhook.as_deref(),
            &http,
            running.clone(),
        )
        .await
        {
            Ok(()) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                if !reconnect {
                    break;
                }
            }
            Err(e) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                if !reconnect {
                    return Err(e);
                }
                eprintln!(
                    "{}",
                    json!({
                        "error": "listen_reconnect",
                        "message": e.to_string(),
                        "backoff_ms": backoff_ms,
                    })
                );
            }
        }
        if !reconnect || !running.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms.saturating_mul(2)).min(MAX_BACKOFF_MS);
    }
    Ok(())
}

async fn listen_session(
    client: &BuzzClient,
    ws_url: &str,
    filters: &[serde_json::Value],
    webhook: Option<&str>,
    http: &reqwest::Client,
    running: Arc<AtomicBool>,
) -> Result<(), CliError> {
    use buzz_ws_client::{NostrWsConnection, RelayMessage};

    let mut conn = NostrWsConnection::connect_authenticated(
        ws_url,
        client.keys(),
        client.auth_tag(),
    )
    .await
    .map_err(|e| CliError::Other(format!("websocket: {e}")))?;

    let sub_id = format!("buzz-listen-{}", &Uuid::new_v4().to_string()[..8]);
    let mut req = vec![json!("REQ"), json!(sub_id)];
    for f in filters {
        req.push(f.clone());
    }
    conn.send_raw(&json!(req))
        .await
        .map_err(|e| CliError::Other(format!("websocket send: {e}")))?;

    while running.load(Ordering::SeqCst) {
        // Short poll so Ctrl-C can exit promptly.
        let msg = match conn.next_event(Duration::from_millis(500)).await {
            Ok(m) => m,
            Err(buzz_ws_client::WsClientError::Timeout) => continue,
            Err(e) => return Err(CliError::Other(format!("websocket: {e}"))),
        };

        match msg {
            RelayMessage::Event { event, .. } => {
                let raw = serde_json::to_value(event.as_ref())
                    .map_err(|e| CliError::Other(format!("event serialize: {e}")))?;
                let line = normalize_events(std::slice::from_ref(&raw));
                // normalize_events returns a JSON array; unwrap first element for NDJSON stream.
                let arr: Vec<serde_json::Value> =
                    serde_json::from_str(&line).unwrap_or_default();
                let body = arr.first().cloned().unwrap_or(raw);
                let text = serde_json::to_string(&body).unwrap_or_default();
                println!("{text}");
                let _ = std::io::stdout().flush();
                if let Some(url) = webhook {
                    post_webhook(http, url, &text).await;
                }
            }
            RelayMessage::Closed { message, .. } => {
                return Err(CliError::Other(format!("subscription closed: {message}")));
            }
            RelayMessage::Notice { message } => {
                eprintln!("{}", json!({"notice": message}));
            }
            RelayMessage::Eose { .. }
            | RelayMessage::Ok(_)
            | RelayMessage::Auth { .. }
            | RelayMessage::Count { .. } => {}
        }
    }

    let _ = conn.send_raw(&json!(["CLOSE", sub_id])).await;
    let _ = conn.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_channel_or_mentions() {
        let err = build_listen_filters(&[], false, &"a".repeat(64), DEFAULT_KINDS).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn channel_filter_uses_h_tag() {
        let ch = "11111111-1111-1111-1111-111111111111".to_string();
        let f = build_listen_filters(&[ch.clone()], false, &"a".repeat(64), DEFAULT_KINDS).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["#h"][0], ch);
    }

    #[test]
    fn mentions_filter_uses_p_tag() {
        let pk = "b".repeat(64);
        let f = build_listen_filters(&[], true, &pk, DEFAULT_KINDS).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["#p"][0], pk);
    }

    #[test]
    fn both_channel_and_mentions_two_filters() {
        let ch = "11111111-1111-1111-1111-111111111111".to_string();
        let pk = "c".repeat(64);
        let f = build_listen_filters(&[ch], true, &pk, DEFAULT_KINDS).unwrap();
        assert_eq!(f.len(), 2);
    }
}
