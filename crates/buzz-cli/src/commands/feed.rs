use std::cmp::Reverse;
use std::io::Write;
use std::time::Duration;

use tokio::time::Instant;

use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};

use crate::client::{normalize_events, BuzzClient};
use crate::commands::event_output::format_events;
use crate::credentials::CapturingSigner;
use crate::error::CliError;

const VALID_FEED_TYPES: &[&str] = &["mentions", "needs_action", "activity", "agent_activity"];

/// How long a single `next_event` call waits before returning control to the
/// watch loop. Short enough that Ctrl-C and idle-timeout accounting stay
/// responsive, long enough not to spin.
const WATCH_POLL_SECS: u64 = 5;

/// Split and validate a `--types` value against [`VALID_FEED_TYPES`].
///
/// Only `feed get` accepts `--types`: the relay honors `feed_types` on its
/// HTTP query path but drops it from WebSocket `REQ` filters.
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
/// Shared by `feed get` and `feed watch`. `limit` is omitted for the streaming
/// case, where the relay should not cap the live subscription.
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

/// Build the `REQ` filter for `feed watch`.
///
/// Never carries `feed_types`: the relay parses WebSocket filters as plain
/// NIP-01 filters and silently drops that key, so sending it would look like
/// filtering while streaming everything.
fn build_watch_filter(
    my_pubkey: &str,
    since: Option<i64>,
    channel: Option<&str>,
) -> serde_json::Value {
    build_feed_filter(my_pubkey, since, None, None, channel)
}

/// What the watch loop should do with one relay message.
#[derive(Debug, PartialEq)]
enum WatchAction {
    /// Write this NDJSON line to stdout.
    Emit(String),
    /// Print this to stderr and keep streaming.
    Notice(String),
    /// Print this to stderr and stop: the relay closed our subscription.
    Closed(String),
    /// Answer a NIP-42 re-auth challenge.
    Reauth,
    /// Nothing to do.
    Ignore,
}

/// Decide how the watch loop handles one relay message. Pure, so the
/// routing can be tested without a live relay.
fn watch_action(msg: &RelayMessage, sub_id: &str) -> Result<WatchAction, CliError> {
    Ok(match msg {
        RelayMessage::Event {
            subscription_id,
            event,
        } if subscription_id == sub_id => WatchAction::Emit(
            serde_json::to_string(event).map_err(|e| CliError::Other(e.to_string()))?,
        ),
        RelayMessage::Closed {
            subscription_id,
            message,
        } if subscription_id == sub_id => {
            WatchAction::Closed(format!("relay closed subscription: {message}"))
        }
        RelayMessage::Notice { message } => WatchAction::Notice(format!("relay notice: {message}")),
        RelayMessage::Auth { .. } => WatchAction::Reauth,
        _ => WatchAction::Ignore,
    })
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
    let my_pk = client.public_key().to_hex();
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
    println!("{}", format_events(&normalized, format));
    Ok(())
}

/// Stream activity feed entries as NDJSON until Ctrl-C or idle timeout.
///
/// Opens its own authenticated WebSocket connection and issues a NIP-01 `REQ`
/// for events mentioning us. One JSON object is written per
/// line to stdout and flushed immediately; relay notices and closures go to
/// stderr so a `| jq` consumer only ever sees events.
pub async fn cmd_watch_feed(
    client: &BuzzClient,
    since: Option<i64>,
    channel: Option<&str>,
    idle_timeout: u64,
) -> Result<(), CliError> {
    let my_pk = client.public_key().to_hex();

    if let Some(c) = channel {
        crate::validate::validate_uuid(c)?;
    }

    let filter = build_watch_filter(&my_pk, since, channel);
    let sub_id = new_subscription_id();

    // NIP-42 signing goes through the credentials' signer (local keys or the
    // desktop provider), and the NIP-FI assertion rides on the upgrade request.
    let signer = CapturingSigner::new(client.credentials());
    let mut conn = NostrWsConnection::connect_authenticated_with_headers(
        client.ws_url(),
        &signer,
        client.auth_tag(),
        client.upgrade_headers().await?,
    )
    .await
    .map_err(|e| CliError::Other(e.to_string()))?;

    // Every exit path below — clean break, error, or Ctrl-C — must still close
    // the subscription and drop the socket, so the loop result is captured and
    // cleanup runs before it is returned.
    let result = watch_loop(&mut conn, client, &signer, &sub_id, &filter, idle_timeout).await;

    let _ = conn.send_raw(&serde_json::json!(["CLOSE", sub_id])).await;
    let _ = conn.disconnect().await;
    result
}

/// The `feed watch` receive loop, split out so [`cmd_watch_feed`] can run
/// subscription cleanup on every exit path including the error ones.
async fn watch_loop(
    conn: &mut NostrWsConnection,
    client: &BuzzClient,
    signer: &CapturingSigner,
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
                let msg = match msg {
                    Ok(m) => m,
                    // A poll that expired with no frame is the normal idle path.
                    // Anything else — a closed socket, a transport failure — is
                    // terminal and must surface rather than spin.
                    Err(WsClientError::Timeout) => continue,
                    Err(e) => return Err(CliError::Other(e.to_string())),
                };
                match watch_action(&msg, sub_id)? {
                    WatchAction::Emit(line) => {
                        last_event = Instant::now();
                        writeln!(stdout, "{line}").map_err(|e| CliError::Other(e.to_string()))?;
                        stdout.flush().map_err(|e| CliError::Other(e.to_string()))?;
                    }
                    WatchAction::Notice(text) => eprintln!("{text}"),
                    WatchAction::Closed(text) => {
                        eprintln!("{text}");
                        return Ok(());
                    }
                    WatchAction::Reauth => {
                        // Re-auth waits on the relay's OK, so it has to keep racing
                        // both Ctrl-C and the idle deadline. A relay that issues
                        // AUTH and withholds OK would otherwise swallow a SIGINT and
                        // stretch --idle-timeout by the auth budget.
                        match deadline {
                            Some(d) => tokio::select! {
                                biased;
                                _ = tokio::signal::ctrl_c() => return Ok(()),
                                _ = tokio::time::sleep_until(d) => return Ok(()),
                                r = conn.authenticate(signer, client.auth_tag()) => {
                                    r.map_err(|e| CliError::Other(e.to_string()))?
                                }
                            },
                            None => tokio::select! {
                                biased;
                                _ = tokio::signal::ctrl_c() => return Ok(()),
                                r = conn.authenticate(signer, client.auth_tag()) => {
                                    r.map_err(|e| CliError::Other(e.to_string()))?
                                }
                            },
                        }
                    }
                    WatchAction::Ignore => {}
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
            since,
            channel,
            idle_timeout,
        } => cmd_watch_feed(client, since, channel.as_deref(), idle_timeout).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_feed_filter, build_watch_filter, new_subscription_id, parse_feed_types, watch_action,
        WatchAction,
    };
    use buzz_ws_client::RelayMessage;
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
    fn the_watch_filter_never_carries_feed_types() {
        let bare = build_watch_filter(PUBKEY, None, None);
        assert_eq!(bare, json!({ "#p": [PUBKEY] }));

        let full = build_watch_filter(PUBKEY, Some(1_783_497_600), Some(UUID));
        assert!(full.get("feed_types").is_none());
        assert!(full.get("limit").is_none());
        assert_eq!(full["since"], json!(1_783_497_600));
        assert_eq!(full["#h"], json!([UUID]));
    }

    #[test]
    fn the_get_filter_still_carries_feed_types() {
        let f = build_feed_filter(PUBKEY, None, Some(20), Some(&["mentions"]), None);
        assert_eq!(f["feed_types"], json!(["mentions"]));
    }

    fn signed_event() -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::TextNote, "hello")
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap()
    }

    #[test]
    fn an_event_for_our_subscription_is_emitted_as_one_json_line() {
        let event = signed_event();
        let msg = RelayMessage::Event {
            subscription_id: "sub".into(),
            event: Box::new(event.clone()),
        };
        let WatchAction::Emit(line) = watch_action(&msg, "sub").unwrap() else {
            panic!("expected Emit");
        };
        assert!(!line.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["id"], json!(event.id.to_hex()));
        assert_eq!(parsed["content"], json!("hello"));
    }

    #[test]
    fn messages_for_another_subscription_are_ignored() {
        let event = RelayMessage::Event {
            subscription_id: "other".into(),
            event: Box::new(signed_event()),
        };
        assert_eq!(watch_action(&event, "sub").unwrap(), WatchAction::Ignore);
        let closed = RelayMessage::Closed {
            subscription_id: "other".into(),
            message: "bye".into(),
        };
        assert_eq!(watch_action(&closed, "sub").unwrap(), WatchAction::Ignore);
    }

    #[test]
    fn eose_notice_closed_and_auth_are_routed() {
        let eose = RelayMessage::Eose {
            subscription_id: "sub".into(),
        };
        assert_eq!(watch_action(&eose, "sub").unwrap(), WatchAction::Ignore);

        let notice = RelayMessage::Notice {
            message: "slow down".into(),
        };
        assert_eq!(
            watch_action(&notice, "sub").unwrap(),
            WatchAction::Notice("relay notice: slow down".into())
        );

        let closed = RelayMessage::Closed {
            subscription_id: "sub".into(),
            message: "restricted".into(),
        };
        assert_eq!(
            watch_action(&closed, "sub").unwrap(),
            WatchAction::Closed("relay closed subscription: restricted".into())
        );

        let auth = RelayMessage::Auth {
            challenge: "c".into(),
        };
        assert_eq!(watch_action(&auth, "sub").unwrap(), WatchAction::Reauth);
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
