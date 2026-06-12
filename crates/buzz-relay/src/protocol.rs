//! NIP-01 client/relay message parsing and formatting.

use nostr::{Event, Filter};
use serde_json::Value;

use crate::error::{RelayError, Result};

/// NIP-11 advertised limit: subscription IDs longer than this are rejected.
const MAX_SUB_ID_LENGTH: usize = 256;

/// NIP-11 advertised limit: REQ messages with more filters than this are rejected.
const MAX_FILTERS_PER_REQ: usize = 10;

/// A message sent by a NIP-01 client to the relay.
#[derive(Debug, Clone)]
pub enum ClientMessage {
    /// An EVENT message submitting a signed Nostr event.
    Event(Event),
    /// A REQ message opening a subscription with one or more filters.
    Req {
        /// The client-assigned subscription identifier.
        sub_id: String,
        /// The filters that determine which events are delivered.
        filters: Vec<Filter>,
    },
    /// A CLOSE message cancelling an active subscription.
    Close(String),
    /// A COUNT message requesting aggregate counts (NIP-45).
    Count {
        /// The client-assigned subscription identifier.
        sub_id: String,
        /// The filters to count against.
        filters: Vec<Filter>,
    },
    /// An AUTH message responding to a NIP-42 challenge.
    Auth(Event),
}

impl ClientMessage {
    /// Parse a raw JSON WebSocket frame into a [`ClientMessage`].
    pub fn parse(raw: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|e| RelayError::InvalidMessage(format!("JSON parse error: {e}")))?;

        let arr = value
            .as_array()
            .ok_or_else(|| RelayError::InvalidMessage("expected JSON array".to_string()))?;

        if arr.is_empty() {
            return Err(RelayError::InvalidMessage("empty array".to_string()));
        }

        let msg_type = arr[0].as_str().ok_or_else(|| {
            RelayError::InvalidMessage("first element must be a string".to_string())
        })?;

        match msg_type {
            "EVENT" => {
                if arr.len() < 2 {
                    return Err(RelayError::InvalidMessage(
                        "EVENT requires event object".to_string(),
                    ));
                }
                let event: Event = serde_json::from_value(arr[1].clone())
                    .map_err(|e| RelayError::InvalidMessage(format!("invalid event: {e}")))?;
                Ok(ClientMessage::Event(event))
            }
            "REQ" => {
                if arr.len() < 2 {
                    return Err(RelayError::InvalidMessage(
                        "REQ requires sub_id".to_string(),
                    ));
                }
                let sub_id = arr[1]
                    .as_str()
                    .ok_or_else(|| {
                        RelayError::InvalidMessage("REQ sub_id must be a string".to_string())
                    })?
                    .to_string();
                if sub_id.is_empty() {
                    return Err(RelayError::InvalidMessage(
                        "REQ sub_id must not be empty".to_string(),
                    ));
                }
                // Enforce NIP-11 advertised max_subid_length: 256
                if sub_id.len() > MAX_SUB_ID_LENGTH {
                    return Err(RelayError::InvalidMessage(format!(
                        "REQ sub_id exceeds maximum length of {MAX_SUB_ID_LENGTH} bytes"
                    )));
                }
                let filter_values = &arr[2..];
                // Enforce NIP-11 advertised max_filters: 10
                if filter_values.len() > MAX_FILTERS_PER_REQ {
                    return Err(RelayError::InvalidMessage(format!(
                        "REQ contains {} filters, maximum is {MAX_FILTERS_PER_REQ}",
                        filter_values.len()
                    )));
                }
                let filters: Vec<Filter> = filter_values
                    .iter()
                    .map(|v| {
                        serde_json::from_value(v.clone())
                            .map_err(|e| RelayError::InvalidMessage(format!("invalid filter: {e}")))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(ClientMessage::Req { sub_id, filters })
            }
            "COUNT" => {
                if arr.len() < 2 {
                    return Err(RelayError::InvalidMessage(
                        "COUNT requires sub_id".to_string(),
                    ));
                }
                let sub_id = arr[1]
                    .as_str()
                    .ok_or_else(|| {
                        RelayError::InvalidMessage("COUNT sub_id must be a string".to_string())
                    })?
                    .to_string();
                if sub_id.is_empty() {
                    return Err(RelayError::InvalidMessage(
                        "COUNT sub_id must not be empty".to_string(),
                    ));
                }
                if sub_id.len() > MAX_SUB_ID_LENGTH {
                    return Err(RelayError::InvalidMessage(format!(
                        "COUNT sub_id exceeds maximum length of {MAX_SUB_ID_LENGTH} bytes"
                    )));
                }
                let filter_values = &arr[2..];
                if filter_values.len() > MAX_FILTERS_PER_REQ {
                    return Err(RelayError::InvalidMessage(format!(
                        "COUNT contains {} filters, maximum is {MAX_FILTERS_PER_REQ}",
                        filter_values.len()
                    )));
                }
                let filters: Vec<Filter> = filter_values
                    .iter()
                    .map(|v| {
                        serde_json::from_value(v.clone())
                            .map_err(|e| RelayError::InvalidMessage(format!("invalid filter: {e}")))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(ClientMessage::Count { sub_id, filters })
            }
            "CLOSE" => {
                if arr.len() < 2 {
                    return Err(RelayError::InvalidMessage(
                        "CLOSE requires sub_id".to_string(),
                    ));
                }
                let sub_id = arr[1]
                    .as_str()
                    .ok_or_else(|| {
                        RelayError::InvalidMessage("CLOSE sub_id must be a string".to_string())
                    })?
                    .to_string();
                Ok(ClientMessage::Close(sub_id))
            }
            "AUTH" => {
                if arr.len() < 2 {
                    return Err(RelayError::InvalidMessage(
                        "AUTH requires event object".to_string(),
                    ));
                }
                let event: Event = serde_json::from_value(arr[1].clone())
                    .map_err(|e| RelayError::InvalidMessage(format!("invalid auth event: {e}")))?;
                Ok(ClientMessage::Auth(event))
            }
            other => Err(RelayError::InvalidMessage(format!(
                "unknown message type: {other}"
            ))),
        }
    }
}

/// Helpers for formatting NIP-01 relay-to-client messages as JSON strings.
pub struct RelayMessage;

impl RelayMessage {
    /// Format an AUTH challenge message.
    pub fn auth_challenge(challenge: &str) -> String {
        serde_json::json!(["AUTH", challenge]).to_string()
    }

    /// Format an EVENT message delivering an event to a subscriber.
    pub fn event(sub_id: &str, event: &Event) -> String {
        let event_json = serde_json::to_value(event)
            .expect("SAFETY: nostr::Event serialization is infallible for well-formed events");
        serde_json::json!(["EVENT", sub_id, event_json]).to_string()
    }

    /// Format a NOTICE message with a human-readable string.
    pub fn notice(message: &str) -> String {
        serde_json::json!(["NOTICE", message]).to_string()
    }

    /// Format an EOSE (End of Stored Events) message for a subscription.
    pub fn eose(sub_id: &str) -> String {
        serde_json::json!(["EOSE", sub_id]).to_string()
    }

    /// Format an OK message acknowledging an EVENT submission.
    pub fn ok(event_id: &str, accepted: bool, message: &str) -> String {
        serde_json::json!(["OK", event_id, accepted, message]).to_string()
    }

    /// Format a CLOSED message indicating a subscription was terminated by the relay.
    pub fn closed(sub_id: &str, message: &str) -> String {
        serde_json::json!(["CLOSED", sub_id, message]).to_string()
    }

    /// Format a COUNT response (NIP-45).
    pub fn count(sub_id: &str, count: u64) -> String {
        serde_json::json!(["COUNT", sub_id, {"count": count}]).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::test_helpers::make_event;
    use nostr::{EventBuilder, Keys, Kind};

    fn make_auth_event(keys: &Keys, challenge: &str, relay: &str) -> Event {
        let url: nostr::RelayUrl = relay.parse().expect("url");
        EventBuilder::auth(challenge, url)
            .sign_with_keys(keys)
            .expect("sign")
    }

    // ── ClientMessage parsing — table-driven ─────────────────────────────

    // Type alias to avoid clippy::type_complexity warning on the test case table.
    // The tuple holds: raw JSON string + a boxed checker closure.
    type ParseCase<'a> = (&'a str, Box<dyn Fn(ClientMessage)>);

    #[test]
    fn parse_valid_messages() {
        let keys = Keys::generate();
        let event = make_event(Kind::TextNote);
        let auth_event = make_auth_event(&keys, "challenge", "wss://relay.example.com");
        let filter = Filter::new().kind(Kind::TextNote);

        let cases: &[ParseCase<'_>] = &[
            (
                &serde_json::json!(["EVENT", serde_json::to_value(&event).unwrap()]).to_string(),
                Box::new(move |m| match m {
                    ClientMessage::Event(e) => assert_eq!(e.id, event.id),
                    _ => panic!("expected Event"),
                }),
            ),
            (
                &serde_json::json!(["REQ", "sub1", serde_json::to_value(&filter).unwrap()])
                    .to_string(),
                Box::new(|m| match m {
                    ClientMessage::Req { sub_id, filters } => {
                        assert_eq!(sub_id, "sub1");
                        assert_eq!(filters.len(), 1);
                    }
                    _ => panic!("expected Req"),
                }),
            ),
            (
                r#"["CLOSE", "sub1"]"#,
                Box::new(|m| match m {
                    ClientMessage::Close(id) => assert_eq!(id, "sub1"),
                    _ => panic!("expected Close"),
                }),
            ),
            (
                &serde_json::json!(["AUTH", serde_json::to_value(&auth_event).unwrap()])
                    .to_string(),
                Box::new(move |m| match m {
                    ClientMessage::Auth(e) => assert_eq!(e.id, auth_event.id),
                    _ => panic!("expected Auth"),
                }),
            ),
        ];

        for (raw, check) in cases {
            let msg = ClientMessage::parse(raw).expect("parse");
            check(msg);
        }
    }

    #[test]
    fn parse_req_multiple_filters() {
        let f1 = Filter::new().kind(Kind::TextNote);
        let f2 = Filter::new().kind(Kind::Metadata);
        let raw = serde_json::json!([
            "REQ",
            "sub2",
            serde_json::to_value(&f1).unwrap(),
            serde_json::to_value(&f2).unwrap()
        ])
        .to_string();
        match ClientMessage::parse(&raw).unwrap() {
            ClientMessage::Req { sub_id, filters } => {
                assert_eq!(sub_id, "sub2");
                assert_eq!(filters.len(), 2);
            }
            _ => panic!("expected Req"),
        }
    }

    #[test]
    fn parse_invalid_messages() {
        let cases = [
            ("not json", "JSON"),
            ("[]", "empty"),
            (r#"["UNKNOWN", "data"]"#, "unknown"),
            (r#"["EVENT"]"#, "EVENT requires"),
            (r#"["REQ"]"#, "REQ requires"),
            (r#"["REQ", ""]"#, "must not be empty"),
        ];

        for (raw, hint) in cases {
            let err = ClientMessage::parse(raw).unwrap_err();
            assert!(
                matches!(err, RelayError::InvalidMessage(_)),
                "expected InvalidMessage for {raw:?}, got {err:?}"
            );
            let _ = hint; // used for readability only
        }
    }

    #[test]
    fn parse_req_sub_id_too_long_is_rejected() {
        let long_id = "x".repeat(MAX_SUB_ID_LENGTH + 1);
        let raw = serde_json::json!(["REQ", long_id]).to_string();
        let err = ClientMessage::parse(&raw).unwrap_err();
        assert!(
            matches!(err, RelayError::InvalidMessage(_)),
            "expected InvalidMessage for oversized sub_id, got {err:?}"
        );
    }

    #[test]
    fn parse_req_too_many_filters_is_rejected() {
        let filter = Filter::new().kind(Kind::TextNote);
        let filter_val = serde_json::to_value(&filter).unwrap();
        let mut arr: Vec<serde_json::Value> = vec![
            serde_json::Value::String("REQ".to_string()),
            serde_json::Value::String("sub3".to_string()),
        ];
        for _ in 0..=MAX_FILTERS_PER_REQ {
            arr.push(filter_val.clone());
        }
        let raw = serde_json::Value::Array(arr).to_string();
        let err = ClientMessage::parse(&raw).unwrap_err();
        assert!(
            matches!(err, RelayError::InvalidMessage(_)),
            "expected InvalidMessage for too many filters, got {err:?}"
        );
    }

    #[test]
    fn parse_req_exactly_max_filters_is_accepted() {
        let filter = Filter::new().kind(Kind::TextNote);
        let filter_val = serde_json::to_value(&filter).unwrap();
        let mut arr: Vec<serde_json::Value> = vec![
            serde_json::Value::String("REQ".to_string()),
            serde_json::Value::String("sub4".to_string()),
        ];
        for _ in 0..MAX_FILTERS_PER_REQ {
            arr.push(filter_val.clone());
        }
        let raw = serde_json::Value::Array(arr).to_string();
        assert!(
            ClientMessage::parse(&raw).is_ok(),
            "exactly {MAX_FILTERS_PER_REQ} filters should be accepted"
        );
    }

    // ── RelayMessage formatting — table-driven ────────────────────────────

    // Type alias to avoid clippy::type_complexity warning on the format test table.
    type FormatCase<'a> = (&'a str, Box<dyn Fn()>);

    #[test]
    fn format_relay_messages() {
        let event = make_event(Kind::TextNote);

        let cases: &[FormatCase<'_>] = &[
            (
                "auth_challenge",
                Box::new(|| {
                    let msg = RelayMessage::auth_challenge("abc123");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[0], "AUTH");
                    assert_eq!(v[1], "abc123");
                }),
            ),
            (
                "event",
                Box::new({
                    let event = event.clone();
                    move || {
                        let msg = RelayMessage::event("sub1", &event);
                        let v: Value = serde_json::from_str(&msg).unwrap();
                        assert_eq!(v[0], "EVENT");
                        assert_eq!(v[1], "sub1");
                        assert_eq!(v[2]["id"], event.id.to_hex());
                    }
                }),
            ),
            (
                "notice",
                Box::new(|| {
                    let msg = RelayMessage::notice("hello");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[0], "NOTICE");
                    assert_eq!(v[1], "hello");
                }),
            ),
            (
                "eose",
                Box::new(|| {
                    let msg = RelayMessage::eose("sub1");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[0], "EOSE");
                    assert_eq!(v[1], "sub1");
                }),
            ),
            (
                "ok_accepted",
                Box::new(|| {
                    let msg = RelayMessage::ok("eid", true, "");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[0], "OK");
                    assert_eq!(v[2], true);
                    assert_eq!(v[3], "");
                }),
            ),
            (
                "ok_rejected",
                Box::new(|| {
                    let msg = RelayMessage::ok("eid", false, "auth-required");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[2], false);
                    assert_eq!(v[3], "auth-required");
                }),
            ),
            (
                "closed",
                Box::new(|| {
                    let msg = RelayMessage::closed("sub1", "auth-required: not authenticated");
                    let v: Value = serde_json::from_str(&msg).unwrap();
                    assert_eq!(v[0], "CLOSED");
                    assert_eq!(v[1], "sub1");
                    assert_eq!(v[2], "auth-required: not authenticated");
                }),
            ),
        ];

        for (name, check) in cases {
            let _ = name;
            check();
        }
    }
}
