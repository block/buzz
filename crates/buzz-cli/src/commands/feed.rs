use std::cmp::Reverse;
use std::io::Write;
use std::time::Duration;

use tokio::time::Instant;

use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};

use crate::client::{normalize_events, BuzzClient};
use crate::error::CliError;

const VALID_FEED_TYPES: &[&str] = &["mentions", "needs_action", "activity", "agent_activity"];

/// How long a single `next_event` call waits before returning control to the
/// watch loop. Short enough that Ctrl-C and idle-timeout accounting stay
/// responsive, long enough not to spin.
const WATCH_POLL_SECS: u64 = 5;

/// Split and validate a `--types` value against [`VALID_FEED_TYPES`].
///
/// Shared by `feed get` and `feed watch` so both reject the same bad input
/// with the same message.
fn parse_feed_types(types_str: &str) -> Result<Vec<&str>, CliError> {
    let type_list: Vec<&str> = types_str.split(',').map(str::trim).collect();
    for t in &type_list {
        if !VALID_FEED_TYPES.contains(t) {
            return Err(CliError::Usage(format!(
                "invalid feed type {t:?} — must be one of: {}",
                VALID_FEED_TYPES.join(", ")
            )));
        }
    }
    Ok(type_list)
}

/// Build the relay filter for the activity feed.
///
/// Shared by `feed get` and `feed watch` so a stream and a one-shot read
/// select exactly the same events. `limit` is omitted for the streaming case,
/// where the relay should not cap the live subscription.
fn build_feed_filter(
    my_pubkey: &str,
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<&[&str]>,
    channel: Option<&str>,
) -> serde_json::Value {
    let mut filter = serde_json::json!({ "#p": [my_pubkey] });
    if let Some(l) = limit {
        filter["limit"] = serde_json::json!(l);
    }
    if let Some(s) = since {
        filter["since"] = serde_json::json!(s);
    }
    if let Some(t) = types {
        filter["feed_types"] = serde_json::json!(t);
    }
    if let Some(c) = channel {
        filter["#h"] = serde_json::json!([c]);
    }
    filter
}

/// Generate a NIP-01 subscription id (1–64 chars).
fn new_subscription_id() -> String {
    format!("buzz-feed-watch-{:016x}", rand::random::<u64>())
}

/// Get activity feed — query events mentioning our pubkey (via p-tag).
pub async fn cmd_get_feed(
    client: &BuzzClient,
    since: Option<i64>,
    limit: Option<u32>,
    types: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(20).min(50);

    let parsed_types = match types {
        Some(t) => Some(parse_feed_types(t)?),
        None => None,
    };
    let filter = build_feed_filter(&my_pk, since, Some(limit), parsed_types.as_deref(), None);

    let resp = client.query(&filter).await?;
    let mut events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    events.sort_by_key(|e| Reverse(e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0)));
    let normalized = normalize_events(&events);
    let output = match format {
        crate::OutputFormat::Compact => {
            let evts: Vec<serde_json::Value> =
                serde_json::from_str(&normalized).unwrap_or_default();
            let compact: Vec<serde_json::Value> = evts
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.get("id").cloned().unwrap_or_default(),
                        "content": e.get("content").cloned().unwrap_or_default(),
                        "created_at": e.get("created_at").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => normalized,
    };
    println!("{output}");
    Ok(())
}

/// Stream activity feed entries as NDJSON until Ctrl-C or idle timeout.
///
/// Opens its own authenticated WebSocket connection and issues a NIP-01 `REQ`
/// with the same filter [`cmd_get_feed`] builds. One JSON object is written per
/// line to stdout and flushed immediately; relay notices and closures go to
/// stderr so a `| jq` consumer only ever sees events.
pub async fn cmd_watch_feed(
    client: &BuzzClient,
    types: Option<&str>,
    since: Option<i64>,
    channel: Option<&str>,
    idle_timeout: u64,
) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();

    let parsed_types = match types {
        Some(t) => Some(parse_feed_types(t)?),
        None => None,
    };
    if let Some(c) = channel {
        crate::validate::validate_uuid(c)?;
    }

    let filter = build_feed_filter(&my_pk, since, None, parsed_types.as_deref(), channel);
    let sub_id = new_subscription_id();

    let mut conn = NostrWsConnection::connect_authenticated(
        &client.ws_url(),
        client.keys(),
        client.auth_tag(),
    )
    .await
    .map_err(|e| CliError::Other(e.to_string()))?;

    // Every exit path below — clean break, error, or Ctrl-C — must still close
    // the subscription and drop the socket, so the loop result is captured and
    // cleanup runs before it is returned.
    let result = watch_loop(&mut conn, client, &sub_id, &filter, idle_timeout).await;

    let _ = conn.send_raw(&serde_json::json!(["CLOSE", sub_id])).await;
    let _ = conn.disconnect().await;
    result
}

/// The `feed watch` receive loop, split out so [`cmd_watch_feed`] can run
/// subscription cleanup on every exit path including the error ones.
async fn watch_loop(
    conn: &mut NostrWsConnection,
    client: &BuzzClient,
    sub_id: &str,
    filter: &serde_json::Value,
    idle_timeout: u64,
) -> Result<(), CliError> {
    conn.send_raw(&serde_json::json!(["REQ", sub_id, filter]))
        .await
        .map_err(|e| CliError::Other(e.to_string()))?;

    let poll = Duration::from_secs(WATCH_POLL_SECS);
    let idle_limit = (idle_timeout > 0).then(|| Duration::from_secs(idle_timeout));
    // Measured from the last event actually delivered, not from the last poll.
    // The deadline is its own `select!` branch rather than a post-poll check:
    // `next_event` answers a relay Ping internally and restarts its own timeout,
    // so a chatty relay could otherwise keep it pending past the deadline.
    let mut last_event = Instant::now();
    let mut stdout = std::io::stdout();

    loop {
        // Copied out before the future is built so the Event arm can still
        // reset `last_event` without holding a borrow across the select.
        let deadline = idle_limit.map(|limit| last_event + limit);
        let idle_expired = async move {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = idle_expired => return Ok(()),
            msg = conn.next_event(poll) => {
                match msg {
                    Ok(RelayMessage::Event { event, .. }) => {
                        last_event = Instant::now();
                        let line = serde_json::to_string(&event)
                            .map_err(|e| CliError::Other(e.to_string()))?;
                        writeln!(stdout, "{line}").map_err(|e| CliError::Other(e.to_string()))?;
                        stdout.flush().map_err(|e| CliError::Other(e.to_string()))?;
                    }
                    Ok(RelayMessage::Eose { .. }) => {}
                    Ok(RelayMessage::Closed { message, .. }) => {
                        eprintln!("relay closed subscription: {message}");
                        return Ok(());
                    }
                    Ok(RelayMessage::Notice { message }) => eprintln!("relay notice: {message}"),
                    Ok(RelayMessage::Auth { .. }) => {
                        // Re-auth waits on the relay's OK, so it has to keep racing
                        // both Ctrl-C and the idle deadline. A relay that issues
                        // AUTH and withholds OK would otherwise swallow a SIGINT and
                        // stretch --idle-timeout by the auth budget.
                        match deadline {
                            Some(d) => tokio::select! {
                                biased;
                                _ = tokio::signal::ctrl_c() => return Ok(()),
                                _ = tokio::time::sleep_until(d) => return Ok(()),
                                r = conn.authenticate(client.keys(), client.auth_tag()) => {
                                    r.map_err(|e| CliError::Other(e.to_string()))?
                                }
                            },
                            None => tokio::select! {
                                biased;
                                _ = tokio::signal::ctrl_c() => return Ok(()),
                                r = conn.authenticate(client.keys(), client.auth_tag()) => {
                                    r.map_err(|e| CliError::Other(e.to_string()))?
                                }
                            },
                        }
                    }
                    Ok(RelayMessage::Ok(_)) | Ok(RelayMessage::Count { .. }) => {}
                    // A poll that expired with no frame is the normal idle path.
                    // Anything else — a closed socket, a transport failure — is
                    // terminal and must surface rather than spin.
                    Err(WsClientError::Timeout) => {}
                    Err(e) => return Err(CliError::Other(e.to_string())),
                }
            }
        }
    }
}

pub async fn dispatch(
    cmd: crate::FeedCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::FeedCmd;
    match cmd {
        FeedCmd::Get {
            since,
            limit,
            types,
        } => cmd_get_feed(client, since, limit, types.as_deref(), format).await,
        FeedCmd::Watch {
            types,
            since,
            channel,
            idle_timeout,
        } => {
            cmd_watch_feed(
                client,
                types.as_deref(),
                since,
                channel.as_deref(),
                idle_timeout,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_feed_filter, new_subscription_id, parse_feed_types};
    use serde_json::json;

    const PUBKEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const UUID: &str = "0b7f9c2e-3d4a-4b1c-8e5f-6a7b8c9d0e1f";

    #[test]
    fn every_valid_feed_type_is_accepted() {
        for t in ["mentions", "needs_action", "activity", "agent_activity"] {
            assert!(parse_feed_types(t).is_ok(), "{t} should be valid");
        }
        assert_eq!(
            parse_feed_types("mentions,activity").unwrap(),
            vec!["mentions", "activity"]
        );
    }

    #[test]
    fn an_unknown_feed_type_is_rejected() {
        let err = parse_feed_types("mentions,bogus").unwrap_err();
        assert!(
            err.to_string().contains("invalid feed type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            parse_feed_types(" mentions , activity ").unwrap(),
            vec!["mentions", "activity"]
        );
    }

    #[test]
    fn the_no_flags_filter_matches_what_feed_get_sent_before() {
        let f = build_feed_filter(PUBKEY, None, Some(20), None, None);
        assert_eq!(f, json!({ "#p": [PUBKEY], "limit": 20 }));
    }

    #[test]
    fn optional_fields_appear_only_when_supplied() {
        let bare = build_feed_filter(PUBKEY, None, None, None, None);
        assert!(bare.get("since").is_none());
        assert!(bare.get("#h").is_none());
        assert!(bare.get("feed_types").is_none());
        assert!(bare.get("limit").is_none());

        let full = build_feed_filter(
            PUBKEY,
            Some(1_783_497_600),
            Some(10),
            Some(&["mentions"]),
            Some(UUID),
        );
        assert_eq!(full["since"], json!(1_783_497_600));
        assert_eq!(full["limit"], json!(10));
        assert_eq!(full["feed_types"], json!(["mentions"]));
        assert_eq!(full["#h"], json!([UUID]));
        assert_eq!(full["#p"], json!([PUBKEY]));
    }

    #[test]
    fn subscription_ids_are_within_the_nip01_length_bound() {
        let id = new_subscription_id();
        assert!(
            (1..=64).contains(&id.len()),
            "subscription id length {} out of NIP-01 range",
            id.len()
        );
        assert_ne!(id, new_subscription_id(), "ids should not repeat");
    }
}
