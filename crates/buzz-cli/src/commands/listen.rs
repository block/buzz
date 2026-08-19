//! Persistent external-agent event stream.
//!
//! `buzz listen` prints one newline-delimited JSON record per matching relay
//! event. With `--envelope v1`, lifecycle records use the same stdout stream.
//! Human diagnostics remain on stderr.

use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use crate::client::{extract_d_tag, BuzzClient};
use crate::error::CliError;
use crate::validate::parse_uuid;

/// Default kinds for channel traffic. Matches `messages get`.
const DEFAULT_KINDS: &[u32] = &[9, 40002, 40008, 45001, 45003];
const MAX_LISTEN_CHANNELS: usize = 1024;
const LISTEN_REPLAY_LIMIT: u64 = 1_000;

pub(crate) fn parse_kinds(raw: Option<&str>) -> Result<Vec<u32>, CliError> {
    match raw {
        None => Ok(DEFAULT_KINDS.to_vec()),
        Some(s) if s.trim().is_empty() => Ok(DEFAULT_KINDS.to_vec()),
        Some(s) => {
            let mut kinds = Vec::new();
            for part in s.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let kind = part
                    .parse::<u32>()
                    .map_err(|_| CliError::Usage(format!("invalid kind in --kinds: {part}")))?;
                kinds.push(kind);
            }
            if kinds.is_empty() {
                return Err(CliError::Usage(
                    "--kinds must list at least one kind".into(),
                ));
            }
            Ok(kinds)
        }
    }
}

/// Build REQ filters for `buzz listen`.
///
/// Each channel gets its own filter and relay subscription. Buzz deliberately
/// excludes logically global subscriptions from live channel fan-out, so a
/// multi-value `#h` filter cannot implement live multi-channel listening.
pub(crate) fn build_listen_filters(
    channels: &[String],
    mentions_of_me: bool,
    my_pubkey_hex: &str,
    kinds: &[u32],
    since: Option<u64>,
) -> Result<Vec<serde_json::Value>, CliError> {
    if channels.is_empty() {
        return Err(CliError::Usage(
            "buzz listen requires at least one resolved channel".into(),
        ));
    }
    let mut unique_channels = Vec::new();
    let mut seen = HashSet::new();
    for channel in channels {
        let channel = parse_uuid(channel)?.to_string();
        if seen.insert(channel.clone()) {
            unique_channels.push(channel);
        }
    }
    if unique_channels.len() > MAX_LISTEN_CHANNELS {
        return Err(CliError::Usage(format!(
            "buzz listen supports at most {MAX_LISTEN_CHANNELS} channels"
        )));
    }

    Ok(unique_channels
        .into_iter()
        .map(|channel| {
            let mut filter = json!({
                "kinds": kinds,
                "#h": [channel],
            });
            if mentions_of_me {
                filter["#p"] = json!([my_pubkey_hex]);
            }
            if let Some(since) = since {
                filter["since"] = json!(since);
                filter["limit"] = json!(LISTEN_REPLAY_LIMIT);
            }
            filter
        })
        .collect())
}

fn parse_count_response(raw: &str) -> Result<u64, CliError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("count response is not JSON: {e} ({raw})")))?;
    value
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::Other(format!("count response did not include count: {raw}")))
}

async fn guard_bounded_catchup(
    client: &BuzzClient,
    filters: &[serde_json::Value],
    since: Option<u64>,
) -> Result<(), CliError> {
    if since.is_none() {
        return Ok(());
    }

    for filter in filters {
        let count = parse_count_response(&client.count(filter).await?)?;
        if count > LISTEN_REPLAY_LIMIT {
            return Err(CliError::Usage(format!(
                "buzz listen refuses this replay window because one subscription has {count} \
                 historical events after --since, exceeding the bounded catch-up limit of \
                 {LISTEN_REPLAY_LIMIT}; restart with a newer --since or narrower filters"
            )));
        }
    }
    Ok(())
}

fn channel_ids_from_metadata(events: &[serde_json::Value]) -> Vec<String> {
    let mut channels: Vec<String> = events
        .iter()
        .filter_map(|event| parse_uuid(&extract_d_tag(event)).ok())
        .map(|channel| channel.to_string())
        .collect();
    channels.sort_unstable();
    channels.dedup();
    channels
}

async fn resolve_listen_channels(
    client: &BuzzClient,
    channels: Vec<String>,
    mentions_of_me: bool,
) -> Result<Vec<String>, CliError> {
    if !channels.is_empty() {
        return Ok(channels);
    }
    if !mentions_of_me {
        return Err(CliError::Usage(
            "buzz listen requires --channel <UUID> and/or --mentions-of-me".into(),
        ));
    }

    let metadata = client.query_all(json!({"kinds": [39000]})).await?;
    let channels = channel_ids_from_metadata(&metadata);
    if channels.is_empty() {
        return Err(CliError::NotFound(
            "no visible channels available for --mentions-of-me".into(),
        ));
    }
    Ok(channels)
}

fn http_to_ws(http_url: &str) -> String {
    http_url
        .replace("https://", "wss://")
        .replace("http://", "ws://")
}

fn lifecycle_record(state: &str, message: Option<&str>) -> serde_json::Value {
    let mut record = json!({
        "schema_version": 1,
        "type": "lifecycle",
        "state": state,
    });
    if let Some(message) = message {
        record["message"] = json!(message);
    }
    record
}

pub(crate) fn event_record(
    event: serde_json::Value,
    envelope: crate::ListenEnvelope,
) -> serde_json::Value {
    match envelope {
        crate::ListenEnvelope::Flat => event,
        crate::ListenEnvelope::V1 => json!({
            "schema_version": 1,
            "type": "event",
            "event": event,
        }),
    }
}

fn tag_values<'a>(event: &'a serde_json::Value, tag_name: &str) -> Vec<&'a str> {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_array)
        .filter(|tag| tag.first().and_then(serde_json::Value::as_str) == Some(tag_name))
        .filter_map(|tag| tag.get(1).and_then(serde_json::Value::as_str))
        .collect()
}

fn filter_values<'a>(filter: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
    filter
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect()
}

fn event_matches_filter(
    event: &serde_json::Value,
    filter: &serde_json::Value,
) -> Result<(), CliError> {
    let event_id = event
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<unknown>");
    let kind = event
        .get("kind")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::Other(format!("event {event_id} has no numeric kind")))?;
    let kinds = filter
        .get("kinds")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::Other("listen filter has no kinds".into()))?;
    if !kinds
        .iter()
        .any(|candidate| candidate.as_u64() == Some(kind))
    {
        return Err(CliError::Other(format!(
            "relay delivered event {event_id} with kind {kind}, which does not match the \
             subscription filter"
        )));
    }

    if let Some(since) = filter.get("since").and_then(serde_json::Value::as_u64) {
        let created_at = event
            .get("created_at")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| CliError::Other(format!("event {event_id} has no created_at")))?;
        if created_at < since {
            return Err(CliError::Other(format!(
                "relay delivered event {event_id} with created_at {created_at}, before \
                 requested --since {since}"
            )));
        }
    }

    for (filter_key, tag_name) in [("#h", "h"), ("#p", "p")] {
        let required = filter_values(filter, filter_key);
        if required.is_empty() {
            continue;
        }
        let actual = tag_values(event, tag_name);
        if !required.iter().any(|required_value| {
            actual
                .iter()
                .any(|actual_value| actual_value == required_value)
        }) {
            return Err(CliError::Other(format!(
                "relay delivered event {event_id} without required {filter_key} match"
            )));
        }
    }

    Ok(())
}

async fn verified_event_json(event: &nostr::Event) -> Result<serde_json::Value, CliError> {
    let event_for_verification = event.clone();
    tokio::task::spawn_blocking(move || buzz_core::verify_event(&event_for_verification))
        .await
        .map_err(|e| CliError::Other(format!("event verification task failed: {e}")))?
        .map_err(|e| CliError::Other(format!("relay delivered unverifiable event: {e}")))?;
    serde_json::to_value(event).map_err(|e| CliError::Other(format!("event serialize: {e}")))
}

fn write_stdout_record(record: &serde_json::Value) -> Result<(), CliError> {
    let line = serde_json::to_string(record)
        .map_err(|e| CliError::Other(format!("serialize listen record: {e}")))?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}").map_err(|e| CliError::Other(format!("stdout write: {e}")))?;
    stdout
        .flush()
        .map_err(|e| CliError::Other(format!("stdout flush: {e}")))?;
    Ok(())
}

fn write_lifecycle(
    envelope: crate::ListenEnvelope,
    state: &str,
    message: Option<&str>,
) -> Result<(), CliError> {
    if matches!(envelope, crate::ListenEnvelope::V1) {
        write_stdout_record(&lifecycle_record(state, message))?;
    }
    Ok(())
}

fn spawn_shutdown_watcher(running: Arc<AtomicBool>) {
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        running.store(false, Ordering::SeqCst);
    });
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let interrupt = signal(SignalKind::interrupt());
    let terminate = signal(SignalKind::terminate());

    match (interrupt, terminate) {
        (Ok(mut interrupt), Ok(mut terminate)) => {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        }
        _ => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn map_ws_error(context: &str, error: buzz_ws_client::WsClientError) -> CliError {
    use buzz_ws_client::WsClientError;

    let detail = error.to_string();
    match error {
        WsClientError::WebSocket(_) | WsClientError::Timeout | WsClientError::ConnectionClosed => {
            CliError::Transport(format!("{context}: {detail}"))
        }
        WsClientError::AuthFailed(_) | WsClientError::NoAuthChallenge => {
            CliError::Auth(format!("{context}: {detail}"))
        }
        WsClientError::Url(_) => CliError::Usage(format!("{context}: {detail}")),
        WsClientError::EventBuilder(_) => CliError::Key(format!("{context}: {detail}")),
        WsClientError::Json(_)
        | WsClientError::UnexpectedMessage(_)
        | WsClientError::EventRejected(_) => {
            CliError::Other(format!("{context}: protocol error: {detail}"))
        }
    }
}

fn relay_closed_error(message: &str) -> CliError {
    if message.starts_with("auth-required:") || message.starts_with("restricted:") {
        CliError::Auth(format!("subscription closed: {message}"))
    } else {
        CliError::Other(format!("subscription closed: {message}"))
    }
}

async fn sleep_with_shutdown(duration: Duration, running: &AtomicBool) {
    let deadline = tokio::time::Instant::now() + duration;
    while running.load(Ordering::SeqCst) {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            break;
        };
        tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
    }
}

/// Run the listen loop until shutdown or fatal error.
pub async fn cmd_listen(
    client: &BuzzClient,
    channels: Vec<String>,
    mentions_of_me: bool,
    kinds_raw: Option<String>,
    since: Option<u64>,
    envelope: crate::ListenEnvelope,
    reconnect: bool,
) -> Result<(), CliError> {
    let kinds = parse_kinds(kinds_raw.as_deref())?;
    let my_pubkey = client.keys().public_key().to_hex();
    let channels = resolve_listen_channels(client, channels, mentions_of_me).await?;
    let filters = build_listen_filters(&channels, mentions_of_me, &my_pubkey, &kinds, since)?;
    guard_bounded_catchup(client, &filters, since).await?;
    let ws_url = http_to_ws(client.relay_url());
    let running = Arc::new(AtomicBool::new(true));
    spawn_shutdown_watcher(running.clone());

    let mut backoff_ms = 500_u64;
    const MAX_BACKOFF_MS: u64 = 30_000;

    while running.load(Ordering::SeqCst) {
        match listen_session(client, &ws_url, &filters, envelope, running.clone()).await {
            Ok(()) => {
                if !running.load(Ordering::SeqCst) || !reconnect {
                    break;
                }
            }
            Err(error) => {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                if !reconnect || !crate::error::is_retryable_error(&error) {
                    let _ = write_lifecycle(envelope, "fatal", Some(&error.to_string()));
                    return Err(error);
                }
                eprintln!(
                    "{}",
                    json!({
                        "error": "listen_reconnect",
                        "message": error.to_string(),
                        "backoff_ms": backoff_ms,
                    })
                );
            }
        }

        if !reconnect || !running.load(Ordering::SeqCst) {
            break;
        }
        sleep_with_shutdown(Duration::from_millis(backoff_ms), running.as_ref()).await;
        backoff_ms = backoff_ms.saturating_mul(2).min(MAX_BACKOFF_MS);
    }

    Ok(())
}

async fn listen_session(
    client: &BuzzClient,
    ws_url: &str,
    filters: &[serde_json::Value],
    envelope: crate::ListenEnvelope,
    running: Arc<AtomicBool>,
) -> Result<(), CliError> {
    use buzz_ws_client::{NostrWsConnection, RelayMessage};

    let mut conn =
        NostrWsConnection::connect_authenticated(ws_url, client.keys(), client.auth_tag())
            .await
            .map_err(|error| map_ws_error("websocket connect", error))?;

    write_lifecycle(envelope, "connected", None)?;

    let subscriptions: Vec<(String, serde_json::Value)> = filters
        .iter()
        .map(|filter| {
            (
                format!("buzz-listen-{}", &Uuid::new_v4().to_string()[..8]),
                filter.clone(),
            )
        })
        .collect();
    let subscription_ids: HashSet<String> = subscriptions
        .iter()
        .map(|(sub_id, _)| sub_id.clone())
        .collect();
    let subscription_filters: std::collections::HashMap<String, serde_json::Value> =
        subscriptions.iter().cloned().collect();
    let mut awaiting_eose = subscription_ids.clone();
    let mut eose_emitted = false;

    for (sub_id, filter) in &subscriptions {
        conn.send_raw(&json!(["REQ", sub_id, filter]))
            .await
            .map_err(|error| map_ws_error("websocket subscribe", error))?;
    }

    while running.load(Ordering::SeqCst) {
        let msg = match conn.next_event(Duration::from_millis(500)).await {
            Ok(msg) => msg,
            Err(buzz_ws_client::WsClientError::Timeout) => continue,
            Err(error) => return Err(map_ws_error("websocket receive", error)),
        };

        match msg {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                if !subscription_ids.contains(&subscription_id) {
                    return Err(CliError::Other(format!(
                        "websocket receive: protocol error: unknown subscription {subscription_id}"
                    )));
                }
                let event_json = verified_event_json(event.as_ref()).await?;
                let filter = subscription_filters.get(&subscription_id).ok_or_else(|| {
                    CliError::Other(format!(
                        "websocket receive: protocol error: missing filter for subscription \
                         {subscription_id}"
                    ))
                })?;
                event_matches_filter(&event_json, filter)?;
                write_stdout_record(&event_record(event_json, envelope))?;
            }
            RelayMessage::Eose { subscription_id } => {
                if awaiting_eose.remove(&subscription_id)
                    && awaiting_eose.is_empty()
                    && !eose_emitted
                {
                    write_lifecycle(envelope, "eose", None)?;
                    eose_emitted = true;
                }
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                if !subscription_ids.contains(&subscription_id) {
                    return Err(CliError::Other(format!(
                        "websocket receive: protocol error: unknown subscription {subscription_id}"
                    )));
                }
                write_lifecycle(envelope, "closed", Some(&message))?;
                return Err(relay_closed_error(&message));
            }
            RelayMessage::Notice { message } => {
                eprintln!("{}", json!({"notice": message}));
            }
            RelayMessage::Ok(_) | RelayMessage::Auth { .. } | RelayMessage::Count { .. } => {}
        }
    }

    for sub_id in subscription_ids {
        let _ = conn.send_raw(&json!(["CLOSE", sub_id])).await;
    }
    let _ = conn.disconnect().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_channel_or_mentions() {
        let err =
            build_listen_filters(&[], false, &"a".repeat(64), DEFAULT_KINDS, None).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn channel_filter_uses_h_tag() {
        let channel = "11111111-1111-1111-1111-111111111111".to_string();
        let filters = build_listen_filters(
            std::slice::from_ref(&channel),
            false,
            &"a".repeat(64),
            DEFAULT_KINDS,
            None,
        )
        .unwrap();

        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0]["#h"][0], channel);
        assert!(filters[0].get("#p").is_none());
    }

    #[test]
    fn mentions_filter_uses_p_tag() {
        let pubkey = "b".repeat(64);
        let channel = "11111111-1111-1111-1111-111111111111".to_string();
        let filters = build_listen_filters(
            std::slice::from_ref(&channel),
            true,
            &pubkey,
            DEFAULT_KINDS,
            None,
        )
        .unwrap();

        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0]["#p"][0], pubkey);
        assert_eq!(filters[0]["#h"][0], channel);
    }

    #[test]
    fn channel_and_mentions_are_single_and_filter() {
        let channel = "11111111-1111-1111-1111-111111111111".to_string();
        let pubkey = "c".repeat(64);
        let filters = build_listen_filters(
            std::slice::from_ref(&channel),
            true,
            &pubkey,
            DEFAULT_KINDS,
            None,
        )
        .unwrap();

        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0]["#h"][0], channel);
        assert_eq!(filters[0]["#p"][0], pubkey);
    }

    #[test]
    fn multiple_channels_get_independent_filters() {
        let channels = vec![
            "11111111-1111-1111-1111-111111111111".to_string(),
            "22222222-2222-2222-2222-222222222222".to_string(),
        ];
        let filters =
            build_listen_filters(&channels, true, &"d".repeat(64), DEFAULT_KINDS, None).unwrap();

        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0]["#h"], json!([channels[0]]));
        assert_eq!(filters[1]["#h"], json!([channels[1]]));
        assert_eq!(filters[0]["#p"], json!(["d".repeat(64)]));
        assert_eq!(filters[1]["#p"], json!(["d".repeat(64)]));
    }

    #[test]
    fn duplicate_channels_are_canonicalized_and_deduplicated() {
        let filters = build_listen_filters(
            &[
                "11111111-1111-1111-1111-111111111111".to_string(),
                "11111111111111111111111111111111".to_string(),
            ],
            false,
            &"a".repeat(64),
            DEFAULT_KINDS,
            None,
        )
        .unwrap();

        assert_eq!(filters.len(), 1);
        assert_eq!(
            filters[0]["#h"],
            json!(["11111111-1111-1111-1111-111111111111"])
        );
    }

    #[test]
    fn metadata_channel_ids_are_valid_sorted_and_unique() {
        let events = json!([
            {"tags": [["d", "22222222-2222-2222-2222-222222222222"]]},
            {"tags": [["d", "not-a-channel"]]},
            {"tags": [["d", "11111111111111111111111111111111"]]},
            {"tags": [["d", "22222222-2222-2222-2222-222222222222"]]}
        ]);

        assert_eq!(
            channel_ids_from_metadata(events.as_array().unwrap()),
            vec![
                "11111111-1111-1111-1111-111111111111",
                "22222222-2222-2222-2222-222222222222",
            ]
        );
    }

    #[test]
    fn websocket_disconnect_is_retryable_network_error() {
        let error = map_ws_error(
            "websocket receive",
            buzz_ws_client::WsClientError::ConnectionClosed,
        );

        assert!(matches!(error, CliError::Transport(_)));
        assert!(crate::error::is_retryable_error(&error));
        assert_eq!(crate::error::exit_code(&error), 2);
    }

    #[test]
    fn restricted_subscription_close_is_auth_error() {
        let error = relay_closed_error("restricted: not a channel member");

        assert!(matches!(error, CliError::Auth(_)));
        assert_eq!(crate::error::exit_code(&error), 3);
    }

    #[test]
    fn since_is_added_to_filter() {
        let filters = build_listen_filters(
            &["11111111-1111-1111-1111-111111111111".to_string()],
            false,
            &"a".repeat(64),
            DEFAULT_KINDS,
            Some(1785100000),
        )
        .unwrap();

        assert_eq!(filters[0]["since"], 1785100000_u64);
        assert_eq!(filters[0]["limit"], LISTEN_REPLAY_LIMIT);
    }

    #[test]
    fn parse_kinds_defaults_and_parses_csv() {
        assert_eq!(parse_kinds(None).unwrap(), DEFAULT_KINDS);
        assert_eq!(parse_kinds(Some("9, 40002")).unwrap(), vec![9, 40002]);
    }

    #[test]
    fn parse_kinds_rejects_invalid_value() {
        let err = parse_kinds(Some("9,nope")).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn v1_event_record_wraps_flat_event() {
        let event = json!({
            "id": "event",
            "pubkey": "author",
            "kind": 40002,
            "content": "hello",
            "created_at": 1785100000_u64,
            "tags": [["h", "channel"]],
        });

        let record = event_record(event.clone(), crate::ListenEnvelope::V1);

        assert_eq!(record["schema_version"], 1);
        assert_eq!(record["type"], "event");
        assert_eq!(record["event"], event);
    }

    #[test]
    fn flat_event_record_is_unchanged() {
        let event = json!({"id": "event"});
        assert_eq!(
            event_record(event.clone(), crate::ListenEnvelope::Flat),
            event
        );
    }

    #[test]
    fn lifecycle_record_uses_v1_shape() {
        let record = lifecycle_record("eose", None);

        assert_eq!(
            record,
            json!({
                "schema_version": 1,
                "type": "lifecycle",
                "state": "eose",
            })
        );
    }

    #[test]
    fn parse_count_response_reads_count_field() {
        assert_eq!(parse_count_response(r#"{"count":42}"#).unwrap(), 42);
        assert!(parse_count_response(r#"{"total":42}"#).is_err());
    }

    #[test]
    fn event_filter_validation_rejects_out_of_scope_event() {
        let event = json!({
            "id": "event",
            "pubkey": "author",
            "kind": 40002,
            "content": "hello",
            "created_at": 1785100000_u64,
            "tags": [["h", "other"], ["p", "agent"]],
            "sig": "sig",
        });
        let filter = json!({
            "kinds": [40002],
            "#h": ["channel"],
            "#p": ["agent"],
            "since": 1785000000_u64,
        });

        assert!(event_matches_filter(&event, &filter).is_err());
    }

    #[test]
    fn event_filter_validation_accepts_matching_event() {
        let event = json!({
            "id": "event",
            "pubkey": "author",
            "kind": 40002,
            "content": "hello",
            "created_at": 1785100000_u64,
            "tags": [["h", "channel"], ["p", "agent"]],
            "sig": "sig",
        });
        let filter = json!({
            "kinds": [9, 40002],
            "#h": ["channel"],
            "#p": ["agent"],
            "since": 1785000000_u64,
        });

        event_matches_filter(&event, &filter).unwrap();
    }

    #[tokio::test]
    async fn verified_event_json_preserves_signature() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(40002), "hello")
            .tags([])
            .sign_with_keys(&keys)
            .unwrap();

        let json = verified_event_json(&event).await.unwrap();

        assert_eq!(json["sig"], event.sig.to_string());
    }

    #[tokio::test]
    async fn verified_event_json_rejects_invalid_signature() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(40002), "hello")
            .tags([])
            .sign_with_keys(&keys)
            .unwrap();
        let mut raw = serde_json::to_value(&event).unwrap();
        raw["sig"] = json!("0".repeat(128));
        let tampered: nostr::Event = serde_json::from_value(raw).unwrap();

        assert!(verified_event_json(&tampered).await.is_err());
    }

    #[tokio::test]
    async fn verified_event_json_rejects_forged_author() {
        let keys = nostr::Keys::generate();
        let other_keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(40002), "hello")
            .tags([])
            .sign_with_keys(&keys)
            .unwrap();
        let mut raw = serde_json::to_value(&event).unwrap();
        raw["pubkey"] = json!(other_keys.public_key().to_hex());
        let tampered: nostr::Event = serde_json::from_value(raw).unwrap();

        assert!(verified_event_json(&tampered).await.is_err());
    }
}
