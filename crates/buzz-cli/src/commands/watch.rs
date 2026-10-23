//! A finite-lifetime streaming primitive. Supervisors own reconnect and replay policy.
use std::io::Write;
use std::time::Duration;

use buzz_ws_client::RelayMessage;
use nostr::{Event, PublicKey};
use serde_json::json;

use crate::{client::BuzzClient, error::CliError, validate::validate_uuid};

const SUB: &str = "buzz-messages-watch";
const KINDS: [u16; 3] = [9, 45001, 45003];
// Fail at the requested cap rather than silently treating truncated history as complete.
const HISTORY_LIMIT: usize = 1000;

fn matches(event: &Event, channel: &str, authors: &[PublicKey], since: u64) -> bool {
    KINDS.contains(&event.kind.as_u16())
        && authors.contains(&event.pubkey)
        && event.created_at.as_secs() >= since
        && event.tags.iter().any(|t| {
            let t = t.as_slice();
            t.len() >= 2 && t[0] == "h" && t[1] == channel
        })
}

pub(crate) async fn run(
    client: &BuzzClient,
    channel: &str,
    authors: &[String],
    since: u64,
) -> Result<(), CliError> {
    validate_uuid(channel)?;
    let authors: Vec<PublicKey> = authors
        .iter()
        .map(|a| PublicKey::parse(a).map_err(|e| CliError::Usage(format!("invalid author: {e}"))))
        .collect::<Result<_, _>>()?;
    let mut connection = client.connect_stream().await?;
    connection
        .send_raw(&json!(["REQ", SUB, {
            "#h": [channel], "authors": authors.iter().map(PublicKey::to_hex).collect::<Vec<_>>(),
            "kinds": KINDS, "since": since, "limit": HISTORY_LIMIT,
        }]))
        .await
        .map_err(|e| CliError::Other(format!("stream subscription: {e}")))?;
    let mut stored = Vec::new();
    let mut live = false;
    let mut stored_bytes = 0;
    let mut out = std::io::stdout();
    loop {
        // A silent or half-open socket must not strand a supervisor indefinitely.
        let msg = connection
            .next_event(Duration::from_secs(90))
            .await
            .map_err(|e| CliError::Other(format!("stream ended: {e}; reconnect with --since")))?;
        match msg {
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == SUB => {
                event
                    .verify()
                    .map_err(|e| CliError::Other(format!("invalid signed event: {e}")))?;
                if !matches(&event, channel, &authors, since) {
                    return Err(CliError::Other(
                        "relay returned an event outside the subscription".into(),
                    ));
                }
                if event.content.len() > 1_048_576 {
                    return Err(CliError::Other("watch message exceeds 1 MiB".into()));
                }
                if live {
                    emit(&mut out, &json!({"type": "event", "event": event}))?;
                } else {
                    stored_bytes += serde_json::to_vec(&event)
                        .map_err(|e| CliError::Other(e.to_string()))?
                        .len();
                    if stored_bytes > 8 * 1_048_576 {
                        return Err(CliError::Other(
                            "watch history exceeds 8 MiB; narrow --since".into(),
                        ));
                    }
                    stored.push(event);
                    if stored.len() >= HISTORY_LIMIT {
                        return Err(CliError::Other(
                            "watch history reached 1000 events; narrow --since before retrying"
                                .into(),
                        ));
                    }
                }
            }
            RelayMessage::Eose { subscription_id } if subscription_id == SUB => {
                if live {
                    return Err(CliError::Other("duplicate EOSE".into()));
                }
                stored.sort_by_key(|e| (e.created_at, e.id));
                for event in stored.drain(..) {
                    emit(&mut out, &json!({"type": "event", "event": event}))?;
                }
                live = true;
                emit(&mut out, &json!({"type": "ready"}))?;
            }
            RelayMessage::Closed { message, .. } | RelayMessage::Notice { message } => {
                return Err(CliError::Other(format!("relay stopped watch: {message}")));
            }
            RelayMessage::Auth { .. } => {
                return Err(CliError::Auth(
                    "relay requested reauthentication; reconnect".into(),
                ));
            }
            _ => {
                return Err(CliError::Other(
                    "unexpected message on watch subscription".into(),
                ))
            }
        }
    }
}

fn emit(out: &mut impl Write, value: &serde_json::Value) -> Result<(), CliError> {
    writeln!(out, "{value}")
        .and_then(|_| out.flush())
        .map_err(|e| CliError::Other(format!("watch output: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    #[test]
    fn accepts_only_signed_scope_not_neighbor_channel_author_kind_or_time() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "report")
            .tags([Tag::parse(["h", "task"]).unwrap()])
            .custom_created_at(Timestamp::from(100))
            .sign_with_keys(&keys)
            .unwrap();
        assert!(matches(&event, "task", &[keys.public_key()], 100));
        assert!(!matches(&event, "other", &[keys.public_key()], 100));
        assert!(!matches(
            &event,
            "task",
            &[Keys::generate().public_key()],
            100
        ));
        assert!(!matches(&event, "task", &[keys.public_key()], 101));
        let mut other = event.clone();
        other.kind = Kind::Custom(20001);
        assert!(!matches(&other, "task", &[keys.public_key()], 100));
        other = event;
        other.content = "tampered".into();
        assert!(other.verify().is_err());
    }
}
