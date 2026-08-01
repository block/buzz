//! Exact-event relay reads with mandatory local NIP-01 verification.

use std::time::Duration;

use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use nostr::{Event, EventId, Keys, Tag};
use serde::Deserialize;

use crate::error::CliError;
use crate::validate::validate_hex64;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_SUBSCRIPTION_ID: &str = "buzz-get-verified-0";
const AUTHENTICATED_SUBSCRIPTION_ID: &str = "buzz-get-verified-1";

fn transport_error(error: WsClientError) -> CliError {
    match error {
        WsClientError::Url(message) => CliError::Usage(format!("invalid relay URL: {message}")),
        WsClientError::AuthFailed(message) => CliError::Auth(message),
        WsClientError::AmbiguousAuthChallenge => CliError::Ambiguous(error.to_string()),
        WsClientError::WebSocket(_)
        | WsClientError::Timeout
        | WsClientError::ConnectionClosed
        | WsClientError::AuthTransportPoisoned => CliError::Transport(error.to_string()),
        WsClientError::Json(_)
        | WsClientError::InvalidEvent { .. }
        | WsClientError::EventBuilder(_)
        | WsClientError::UnexpectedMessage(_)
        | WsClientError::EventRejected(_)
        | WsClientError::NoAuthChallenge
        | WsClientError::AuthChallengeTooLarge { .. }
        | WsClientError::AuthFrameTooLarge { .. }
        | WsClientError::ReflectedAuthMaterial => CliError::RelayProtocol(error.to_string()),
    }
}

fn exact_event_id(value: &str) -> Result<EventId, CliError> {
    validate_hex64(value)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_digit() || matches!(character, 'a'..='f'))
    {
        return Err(CliError::Usage(
            "event ID must be canonical lowercase hex".into(),
        ));
    }
    EventId::parse(value).map_err(|error| CliError::Usage(format!("invalid event ID: {error}")))
}

fn exact_relay_url(value: &str) -> Result<String, CliError> {
    if value.trim() != value {
        return Err(CliError::Usage(
            "relay URL must not contain leading or trailing whitespace".into(),
        ));
    }
    let url = url::Url::parse(value)
        .map_err(|error| CliError::Usage(format!("invalid relay URL: {error}")))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(CliError::Usage("relay URL scheme must be ws or wss".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CliError::Usage(
            "relay URL must not contain credentials".into(),
        ));
    }
    if url.fragment().is_some() {
        return Err(CliError::Usage(
            "relay URL must not contain a fragment".into(),
        ));
    }
    if url.host().is_none() {
        return Err(CliError::Usage("relay URL must contain a host".into()));
    }
    Ok(value.to_string())
}

fn request(subscription_id: &str, event_id: &EventId) -> serde_json::Value {
    serde_json::json!([
        "REQ",
        subscription_id,
        {
            "ids": [event_id.to_hex()],
            "limit": 2
        }
    ])
}

// NIP-42 documents `auth-required` as a machine-readable CLOSED prefix. Match
// only that exact token or its colon-delimited form; human prose and lookalike
// strings must never authorize signing with ambient credentials.
fn is_auth_required_reason(message: &str) -> bool {
    message == "auth-required" || message.starts_with("auth-required:")
}

#[derive(Debug)]
struct ReceivedEvent {
    event: Event,
    raw_json: Box<str>,
}

struct CollectedEvents {
    events: Vec<ReceivedEvent>,
    private_auth_started: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEventEnvelope {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u16,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

fn is_canonical_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_raw_event(requested_id: &EventId, received: &ReceivedEvent) -> Result<(), CliError> {
    let raw: RawEventEnvelope = serde_json::from_str(&received.raw_json).map_err(|error| {
        CliError::RelayMismatch(format!(
            "relay returned a malformed signed-event envelope: {error}"
        ))
    })?;

    if !is_canonical_hex(&raw.id, 64) {
        return Err(CliError::RelayMismatch(
            "relay returned a non-canonical event id".into(),
        ));
    }
    if raw.id != requested_id.to_hex() {
        return Err(CliError::RelayMismatch(format!(
            "requested event {} but raw relay payload declared {}",
            requested_id.to_hex(),
            raw.id
        )));
    }
    if !is_canonical_hex(&raw.pubkey, 64) {
        return Err(CliError::RelayMismatch(
            "relay returned a non-canonical event pubkey".into(),
        ));
    }
    if !is_canonical_hex(&raw.sig, 128) {
        return Err(CliError::SignatureInvalid(
            "relay returned a malformed Schnorr signature".into(),
        ));
    }

    let typed_tags: Vec<Vec<String>> = received
        .event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    if raw.id != received.event.id.to_hex()
        || raw.pubkey != received.event.pubkey.to_hex()
        || raw.created_at != received.event.created_at.as_secs()
        || raw.kind != received.event.kind.as_u16()
        || raw.tags != typed_tags
        || raw.content != received.event.content
        || raw.sig != received.event.sig.to_string()
    {
        return Err(CliError::RelayMismatch(
            "raw signed-event fields did not match their typed representation".into(),
        ));
    }

    Ok(())
}

fn malformed_event_error(requested_id: &EventId, raw_event_json: &str, message: &str) -> CliError {
    let value = match serde_json::from_str::<serde_json::Value>(raw_event_json) {
        Ok(serde_json::Value::Object(value)) => value,
        _ => {
            return CliError::RelayMismatch(format!(
                "relay returned a malformed signed-event envelope: {message}"
            ));
        }
    };

    match value.get("id") {
        Some(serde_json::Value::String(id))
            if is_canonical_hex(id, 64) && id == &requested_id.to_hex() => {}
        Some(serde_json::Value::String(id)) if is_canonical_hex(id, 64) => {
            return CliError::RelayMismatch(format!(
                "requested event {} but raw relay payload declared {id}",
                requested_id.to_hex()
            ));
        }
        _ => {
            return CliError::RelayMismatch(
                "relay returned a malformed or non-canonical event id".into(),
            );
        }
    }

    if !matches!(value.get("sig"), Some(serde_json::Value::String(sig)) if is_canonical_hex(sig, 128))
    {
        return CliError::SignatureInvalid("relay returned a malformed Schnorr signature".into());
    }

    CliError::RelayMismatch(format!(
        "relay returned a malformed signed-event envelope: {message}"
    ))
}

fn receive_error(error: WsClientError, requested_id: &EventId, authenticated: bool) -> CliError {
    if authenticated {
        return match error {
            WsClientError::WebSocket(_)
            | WsClientError::Timeout
            | WsClientError::ConnectionClosed
            | WsClientError::AuthTransportPoisoned => CliError::Transport(
                "authenticated relay connection failed during exact-event fetch".into(),
            ),
            WsClientError::AmbiguousAuthChallenge => {
                CliError::Ambiguous("relay sent another authentication challenge".into())
            }
            WsClientError::AuthFailed(_) => {
                CliError::Auth("relay rejected NIP-42 authentication".into())
            }
            _ => CliError::RelayProtocol(
                "relay sent an invalid response after authentication".into(),
            ),
        };
    }

    match error {
        WsClientError::InvalidEvent {
            raw_event_json,
            message,
        } => malformed_event_error(requested_id, &raw_event_json, &message),
        other => transport_error(other),
    }
}

// Once private AUTH may have crossed the wire, no relay-derived value may be
// rendered into CLI output. This static boundary covers typed
// fields, serde diagnostics, WebSocket errors, and validation failures without
// attempting to recognize transformed or split secrets.
fn private_auth_error(error: CliError) -> CliError {
    match error {
        CliError::Usage(_) => CliError::Usage(
            "exact-event fetch could not continue after private authentication".into(),
        ),
        CliError::Relay { .. } | CliError::RelayProtocol(_) => CliError::RelayProtocol(
            "relay sent an invalid response after private authentication".into(),
        ),
        CliError::Network(_) | CliError::Transport(_) => {
            CliError::Transport("relay connection failed after private authentication".into())
        }
        CliError::Auth(_) | CliError::Key(_) => {
            CliError::Auth("private relay authentication failed".into())
        }
        CliError::Conflict(_) => {
            CliError::Conflict("relay reported a conflict after private authentication".into())
        }
        CliError::NotFound(_) => {
            CliError::NotFound("event not found after private authentication".into())
        }
        CliError::Ambiguous(_) => CliError::Ambiguous(
            "relay returned an ambiguous result after private authentication".into(),
        ),
        CliError::RelayMismatch(_) => CliError::RelayMismatch(
            "relay response did not match the exact-event request after private authentication"
                .into(),
        ),
        CliError::IdMismatch(_) => CliError::IdMismatch(
            "relay event failed local ID verification after private authentication".into(),
        ),
        CliError::SignatureInvalid(_) => CliError::SignatureInvalid(
            "relay event failed local signature verification after private authentication".into(),
        ),
        CliError::DeliveryUnknown(_) => CliError::DeliveryUnknown(
            "relay delivery state is unknown after private authentication".into(),
        ),
        CliError::Other(_) => {
            CliError::Other("exact-event fetch failed after private authentication".into())
        }
    }
}

fn verify_exact_result(
    requested_id: &EventId,
    relay: &str,
    mut events: Vec<ReceivedEvent>,
) -> Result<ReceivedEvent, CliError> {
    if events.is_empty() {
        return Err(CliError::NotFound(format!(
            "event {} not found at {relay}",
            requested_id.to_hex()
        )));
    }
    if events.len() != 1 {
        return Err(CliError::Ambiguous(format!(
            "relay returned {} events for exact event ID {}",
            events.len(),
            requested_id.to_hex()
        )));
    }

    let received = events.remove(0);
    validate_raw_event(requested_id, &received)?;
    if received.event.id != *requested_id {
        return Err(CliError::RelayMismatch(format!(
            "requested event {} but relay returned {}",
            requested_id.to_hex(),
            received.event.id.to_hex()
        )));
    }
    if !received.event.verify_id() {
        return Err(CliError::IdMismatch(format!(
            "event {} does not match its local NIP-01 ID recomputation",
            received.event.id.to_hex()
        )));
    }
    if !received.event.verify_signature() {
        return Err(CliError::SignatureInvalid(format!(
            "event {} has an invalid Schnorr signature",
            received.event.id.to_hex()
        )));
    }

    Ok(received)
}

async fn collect_exact_event(
    relay: &str,
    event_id: &EventId,
    keys: Option<&Keys>,
    auth_tag: Option<&Tag>,
) -> Result<CollectedEvents, CliError> {
    let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
    let mut private_auth_started = false;
    let result = tokio::time::timeout_at(
        deadline,
        collect_exact_event_inner(relay, event_id, keys, auth_tag, &mut private_auth_started),
    )
    .await
    .map_err(|_| {
        CliError::Transport(format!(
            "exact-event fetch timed out after {} seconds for {relay}",
            FETCH_TIMEOUT.as_secs()
        ))
    })
    .and_then(|result| result);

    let events = result.map_err(|error| {
        if private_auth_started {
            private_auth_error(error)
        } else {
            error
        }
    })?;
    Ok(CollectedEvents {
        events,
        private_auth_started,
    })
}

async fn collect_exact_event_inner(
    relay: &str,
    event_id: &EventId,
    keys: Option<&Keys>,
    auth_tag: Option<&Tag>,
    private_auth_started: &mut bool,
) -> Result<Vec<ReceivedEvent>, CliError> {
    let mut connection = NostrWsConnection::connect(relay)
        .await
        .map_err(transport_error)?;
    let mut subscription_id = INITIAL_SUBSCRIPTION_ID;
    let mut retired_subscription_id = None;
    let mut authenticated = false;
    let mut auth_challenge_received = false;
    let mut authentication_required = false;
    let mut events = Vec::new();

    let result = async {
        connection
            .send_raw(&request(subscription_id, event_id))
            .await
            .map_err(transport_error)?;

        loop {
            match connection
                .next_event(FETCH_TIMEOUT)
                .await
                .map_err(|error| receive_error(error, event_id, authenticated))?
            {
                RelayMessage::Event {
                    subscription_id: received,
                    event,
                    raw_event_json,
                } if received == subscription_id && !authentication_required => {
                    if !events.is_empty() {
                        return Err(CliError::Ambiguous(format!(
                            "relay returned more than one event for exact event ID {}",
                            event_id.to_hex()
                        )));
                    }
                    events.push(ReceivedEvent {
                        event: *event,
                        raw_json: raw_event_json,
                    });
                }
                RelayMessage::Eose {
                    subscription_id: received,
                } if received == subscription_id && !authentication_required => break,
                RelayMessage::Event {
                    subscription_id: received,
                    ..
                }
                | RelayMessage::Eose {
                    subscription_id: received,
                } if received == subscription_id && authentication_required => {}
                RelayMessage::Closed {
                    subscription_id: received,
                    message,
                } if received == subscription_id => {
                    if authenticated {
                        return Err(CliError::RelayProtocol(
                            "relay closed authenticated exact-event subscription".into(),
                        ));
                    }
                    if is_auth_required_reason(&message) && keys.is_none() {
                        return Err(CliError::Auth(
                            "relay requires NIP-42 authentication; set BUZZ_PRIVATE_KEY or pass --private-key"
                                .into(),
                        ));
                    }
                    if is_auth_required_reason(&message) && !authenticated {
                        authentication_required = true;
                    } else {
                        return Err(CliError::RelayProtocol(format!(
                            "relay closed exact-event subscription: {message}"
                        )));
                    }
                }
                RelayMessage::Event {
                    subscription_id: received,
                    ..
                }
                | RelayMessage::Eose {
                    subscription_id: received,
                }
                | RelayMessage::Closed {
                    subscription_id: received,
                    ..
                } if retired_subscription_id == Some(received.as_str()) => {}
                RelayMessage::Event {
                    subscription_id: received,
                    ..
                }
                | RelayMessage::Eose {
                    subscription_id: received,
                }
                | RelayMessage::Closed {
                    subscription_id: received,
                    ..
                } => {
                    let message = if authenticated {
                        "relay responded for an unexpected authenticated subscription".into()
                    } else {
                        format!(
                            "relay responded for subscription {received}, expected {subscription_id}"
                        )
                    };
                    return Err(CliError::RelayMismatch(message));
                }
                // NIP-42 relays may advertise a challenge even when the requested read
                // is public. Keep waiting for EOSE or an explicit auth-required CLOSED
                // instead of treating the challenge itself as denial.
                RelayMessage::Auth { .. } if !authenticated => {
                    auth_challenge_received = true;
                }
                RelayMessage::Auth { .. } => {
                    return Err(CliError::RelayProtocol(
                        "relay sent a second authentication challenge".into(),
                    ));
                }
                RelayMessage::Notice { .. } => {}
                RelayMessage::Ok(_) | RelayMessage::Count { .. } => {
                    return Err(CliError::RelayProtocol(
                        "relay sent an unexpected message during exact-event fetch".into(),
                    ));
                }
            }

            if authentication_required && auth_challenge_received && !authenticated {
                let keys = keys.ok_or_else(|| {
                    CliError::Auth("NIP-42 authentication key is unavailable".into())
                })?;
                retired_subscription_id = Some(subscription_id);
                // Start conservatively before the cancellable await, then use
                // the connection's precise write state on any normal return.
                // A relay can reflect the private event before sending OK.
                *private_auth_started = true;
                let authentication = connection.authenticate(keys, auth_tag).await;
                *private_auth_started = connection.private_auth_started();
                authentication.map_err(transport_error)?;
                authenticated = true;
                authentication_required = false;
                auth_challenge_received = false;
                subscription_id = AUTHENTICATED_SUBSCRIPTION_ID;
                events.clear();
                connection
                    .send_raw(&request(subscription_id, event_id))
                    .await
                    .map_err(transport_error)?;
            }
        }

        Ok(events)
    }
    .await;

    let _ = connection
        .send_raw(&serde_json::json!(["CLOSE", subscription_id]))
        .await;
    let _ = connection.disconnect().await;
    result
}

/// Fetch exactly one event from one relay and emit it only after local verification.
pub async fn cmd_get_verified(
    relay: &str,
    event_id: &str,
    keys: Option<&Keys>,
    auth_tag: Option<&Tag>,
) -> Result<(), CliError> {
    let relay = exact_relay_url(relay)?;
    let event_id = exact_event_id(event_id)?;
    let collected = collect_exact_event(&relay, &event_id, keys, auth_tag).await?;
    let event = verify_exact_result(&event_id, &relay, collected.events).map_err(|error| {
        if collected.private_auth_started {
            private_auth_error(error)
        } else {
            error
        }
    })?;
    println!("{}", event.raw_json);
    Ok(())
}

pub async fn dispatch(
    cmd: &crate::EventsCmd,
    keys: Option<&Keys>,
    auth_tag: Option<&Tag>,
) -> Result<(), CliError> {
    match cmd {
        crate::EventsCmd::GetVerified { relay, event } => {
            cmd_get_verified(relay, event, keys, auth_tag).await
        }
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, JsonUtil, Kind};

    use super::*;

    fn signed_event(content: &str) -> Event {
        EventBuilder::new(Kind::TextNote, content)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    fn mutate(event: &Event, field: &str, value: serde_json::Value) -> Event {
        let mut json = serde_json::to_value(event).unwrap();
        json[field] = value;
        Event::from_json(json.to_string()).unwrap()
    }

    fn received(event: Event) -> ReceivedEvent {
        let raw_json = event.as_json().into_boxed_str();
        ReceivedEvent { event, raw_json }
    }

    #[test]
    fn valid_exact_event_is_returned() {
        let event = signed_event("valid");
        let result = verify_exact_result(
            &event.id,
            "wss://relay.example",
            vec![received(event.clone())],
        );
        assert_eq!(result.unwrap().event, event);
    }

    #[test]
    fn zero_results_are_not_found() {
        let id = signed_event("missing").id;
        let error = verify_exact_result(&id, "wss://relay.example", vec![]).unwrap_err();
        assert!(matches!(error, CliError::NotFound(_)));
    }

    #[test]
    fn multiple_results_are_ambiguous() {
        let event = signed_event("duplicate");
        let event_id = event.id;
        let error = verify_exact_result(
            &event_id,
            "wss://relay.example",
            vec![received(event.clone()), received(event)],
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Ambiguous(_)));
    }

    #[test]
    fn returned_wrong_event_is_relay_mismatch() {
        let requested = signed_event("requested");
        let returned = signed_event("returned");
        let error = verify_exact_result(
            &requested.id,
            "wss://relay.example",
            vec![received(returned)],
        )
        .unwrap_err();
        assert!(matches!(error, CliError::RelayMismatch(_)));
    }

    #[test]
    fn mutated_content_is_id_mismatch() {
        let valid = signed_event("original");
        let mutated = mutate(&valid, "content", serde_json::json!("mutated"));
        let error = verify_exact_result(&valid.id, "wss://relay.example", vec![received(mutated)])
            .unwrap_err();
        assert!(matches!(error, CliError::IdMismatch(_)));
    }

    #[test]
    fn invalid_signature_is_distinct_from_id_mismatch() {
        let valid = signed_event("original");
        let mutated = mutate(&valid, "sig", serde_json::json!("0".repeat(128)));
        let error = verify_exact_result(&valid.id, "wss://relay.example", vec![received(mutated)])
            .unwrap_err();
        assert!(matches!(error, CliError::SignatureInvalid(_)));
    }

    #[test]
    fn request_is_exact_id_and_cardinality_bounded() {
        let event = signed_event("filter");
        assert_eq!(
            request("subscription", &event.id),
            serde_json::json!([
                "REQ",
                "subscription",
                {"ids": [event.id.to_hex()], "limit": 2}
            ])
        );
    }

    #[test]
    fn auth_required_reason_accepts_only_documented_machine_token() {
        for accepted in ["auth-required", "auth-required:", "auth-required: policy"] {
            assert!(is_auth_required_reason(accepted), "rejected {accepted:?}");
        }
        for rejected in [
            "not-auth-required",
            "restricted: not-auth-required",
            "restricted auth-required response",
            "auth-required-suffix",
            "AUTH-REQUIRED",
        ] {
            assert!(!is_auth_required_reason(rejected), "accepted {rejected:?}");
        }
    }

    #[test]
    fn relay_must_be_explicit_websocket_url() {
        assert!(exact_relay_url("wss://relay.example").is_ok());
        assert!(exact_relay_url("ws://127.0.0.1:3000").is_ok());
        assert!(exact_relay_url("https://relay.example").is_err());
        assert!(exact_relay_url("relay.example").is_err());
        assert!(exact_relay_url(" wss://relay.example").is_err());
        assert!(exact_relay_url("wss://relay.example ").is_err());
    }

    #[test]
    fn relay_connection_host_is_not_rewritten() {
        assert_eq!(
            exact_relay_url("wss://localhost:443/community").unwrap(),
            "wss://localhost:443/community"
        );
    }

    #[test]
    fn event_id_must_be_canonical_lowercase_hex() {
        let lowercase = "abcdef0123456789".repeat(4);
        assert!(exact_event_id(&lowercase).is_ok());
        assert!(exact_event_id(&lowercase.to_ascii_uppercase()).is_err());
    }

    #[test]
    fn raw_event_requires_exact_signed_field_set() {
        let event = signed_event("extra field");
        let mut raw = serde_json::to_value(&event).unwrap();
        raw["unexpected"] = serde_json::json!(true);
        let received = ReceivedEvent {
            event: event.clone(),
            raw_json: raw.to_string().into_boxed_str(),
        };

        let error = validate_raw_event(&event.id, &received).unwrap_err();
        assert!(matches!(error, CliError::RelayMismatch(_)));
    }

    #[test]
    fn malformed_raw_signature_has_signature_category() {
        let event = signed_event("malformed signature");
        let mut raw = serde_json::to_value(&event).unwrap();
        raw["sig"] = serde_json::json!("ABC");
        let received = ReceivedEvent {
            event: event.clone(),
            raw_json: raw.to_string().into_boxed_str(),
        };

        let error = validate_raw_event(&event.id, &received).unwrap_err();
        assert!(matches!(error, CliError::SignatureInvalid(_)));
    }
}
