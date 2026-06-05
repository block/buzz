//! Serverless-mode relay transport over a plain WebSocket.
//!
//! In serverless mode the desktop app talks to a generic public Nostr relay
//! that has no Sprout HTTP bridge (`/query`, `/events`), no Postgres, and no
//! NIP-98 auth. This module provides the WebSocket equivalents:
//!
//! - [`query_relay_ws`] — one-shot `REQ` / collect until `EOSE` / `CLOSE`.
//! - [`submit_event_ws`] — `EVENT` publish, wait for `OK`.
//!
//! Both perform NIP-42 AUTH only if the relay challenges (most public relays
//! do not). The signing key is the user's identity key from [`AppState`].
//!
//! These mirror the HTTP helpers in `relay.rs` so the rest of the codebase
//! (channels, DMs, agents) is transport-agnostic: it calls `query_relay` /
//! `submit_event`, which dispatch here when `state.is_serverless()`.

use futures_util::StreamExt;
use nostr::EventBuilder;

use crate::app_state::AppState;
use crate::relay::SubmitEventResponse;

/// Execute one or more filters as a single `REQ` and collect matching events
/// until the relay sends `EOSE`. Mirrors `relay::query_relay` but over a plain
/// WebSocket against a generic relay.
/// Max extra time to wait for the remaining (slower) relays once at least one
/// relay has returned its results. Bounds read tail-latency: a slow-but-alive
/// relay can no longer drag every query out to the full per-relay
/// `QUERY_TIMEOUT` (6s) via `join_all`. After this grace window we merge
/// whatever arrived. Dead relays are already skipped via the pool's failure
/// cooldown, so this only affects relays that connect but answer slowly.
const QUERY_MERGE_GRACE: std::time::Duration = std::time::Duration::from_millis(800);

/// Query a set of relays concurrently and merge results, deduplicating events
/// by id. Succeeds if any relay responds; errors only if all fail.
///
/// Returns as soon as the first relay responds plus a short grace window for
/// stragglers ([`QUERY_MERGE_GRACE`]) — so one slow relay does not hold up the
/// whole read. The per-relay futures still run to their own `QUERY_TIMEOUT`;
/// we just stop *waiting* on them once we have a result and the grace elapses.
pub async fn query_relay_ws(
    state: &AppState,
    relay_urls: &[String],
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let mut futures: futures_util::stream::FuturesUnordered<_> = relay_urls
        .iter()
        .map(|url| query_relay_ws_one(state, url, filters))
        .collect();

    let mut by_id: std::collections::HashMap<String, nostr::Event> =
        std::collections::HashMap::new();
    let mut last_err = None;
    let mut any_ok = false;

    // Phase 1: wait for the first relay to come back (success or error).
    while let Some(r) = futures.next().await {
        match r {
            Ok(events) => {
                any_ok = true;
                for ev in events {
                    by_id.entry(ev.id.to_hex()).or_insert(ev);
                }
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }

    // Phase 2: give the remaining relays a short grace window to contribute,
    // then merge whatever has arrived and stop waiting on slow stragglers.
    if any_ok {
        let grace = tokio::time::sleep(QUERY_MERGE_GRACE);
        tokio::pin!(grace);
        loop {
            tokio::select! {
                maybe = futures.next() => match maybe {
                    Some(Ok(events)) => {
                        for ev in events {
                            by_id.entry(ev.id.to_hex()).or_insert(ev);
                        }
                    }
                    Some(Err(e)) => last_err = Some(e),
                    None => break, // all relays done
                },
                _ = &mut grace => break, // grace elapsed — merge what we have
            }
        }
    } else {
        // No relay succeeded yet; drain the rest (all likely failing fast).
        while let Some(r) = futures.next().await {
            match r {
                Ok(events) => {
                    any_ok = true;
                    for ev in events {
                        by_id.entry(ev.id.to_hex()).or_insert(ev);
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
    }

    if !any_ok {
        return Err(last_err.unwrap_or_else(|| "all relays failed".to_string()));
    }
    Ok(by_id.into_values().collect())
}

async fn query_relay_ws_one(
    state: &AppState,
    relay_url: &str,
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    state.relay_pool.query(relay_url, &keys, filters).await
}

/// Publish a signed event over a plain WebSocket and wait for the relay's
/// `OK` acknowledgement. Mirrors `relay::submit_event`.
/// Sign once, then publish to all relays. Succeeds if any relay accepts.
pub async fn submit_event_ws(
    builder: EventBuilder,
    state: &AppState,
    relay_urls: &[String],
) -> Result<SubmitEventResponse, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;

    // Publish to all relays concurrently and return as soon as ONE accepts —
    // don't block on slow/unreachable relays (a stalled public relay must not
    // freeze the UI). The same event to N relays is idempotent (dedup by id),
    // so the first acceptance is authoritative. Public relays rate-limit bursts
    // per connection; with multiple relays another usually accepts. If all
    // relays rate-limit, retry once after a short backoff.
    for attempt in 0..2 {
        let mut futures: futures_util::stream::FuturesUnordered<_> = relay_urls
            .iter()
            .map(|url| submit_event_ws_one(state, &event, &keys, url))
            .collect();

        let mut last_err = None;
        let mut all_rate_limited = true;
        while let Some(r) = futures.next().await {
            match r {
                Ok(resp) if resp.accepted => return Ok(resp), // first acceptance wins
                Ok(resp) => {
                    if !resp.message.contains("rate-limit") {
                        all_rate_limited = false;
                    }
                    last_err = Some(format!("relay rejected event: {}", resp.message));
                }
                Err(e) => {
                    all_rate_limited = false;
                    last_err = Some(e);
                }
            }
        }

        if attempt == 0 && all_rate_limited && !relay_urls.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            continue;
        }
        return Err(last_err.unwrap_or_else(|| "all relays failed".to_string()));
    }
    unreachable!("loop returns on both attempts")
}

async fn submit_event_ws_one(
    state: &AppState,
    event: &nostr::Event,
    keys: &nostr::Keys,
    relay_url: &str,
) -> Result<SubmitEventResponse, String> {
    state.relay_pool.publish(relay_url, keys, event).await
}

/// Publish an already-signed event over a plain WebSocket and wait for `OK`.
///
/// Unlike [`submit_event_ws`], this takes a pre-signed event and the keys that
/// signed it (used to answer a NIP-42 AUTH challenge). Used for serverless
/// agent-profile sync, where the event is signed by the agent's keys rather
/// than the user's identity key.
pub async fn publish_signed_event_ws(
    state: &AppState,
    event: &nostr::Event,
    keys: &nostr::Keys,
    relay_urls: &[String],
) -> Result<(), String> {
    let mut last_err = None;
    for url in relay_urls {
        match state.relay_pool.publish(url, keys, event).await {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "all relays failed".to_string()))
}

/// Build a NIP-42 `["AUTH", <event>]` message string.
pub(crate) fn build_auth_message(
    keys: &nostr::Keys,
    relay_url: &str,
    challenge: &str,
) -> Result<String, String> {
    let url = nostr::RelayUrl::parse(relay_url).map_err(|e| format!("invalid relay URL: {e}"))?;
    let event = EventBuilder::auth(challenge.to_string(), url)
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign auth event: {e}"))?;
    Ok(serde_json::json!(["AUTH", event]).to_string())
}
