use std::collections::HashMap;

use nostr::{Event, EventId, Kind};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_hex64;

fn verify_reaction_events(
    raw_events: Vec<serde_json::Value>,
    target_event_id: &str,
) -> Result<Vec<Event>, CliError> {
    let mut events = Vec::with_capacity(raw_events.len());
    for (index, raw_event) in raw_events.into_iter().enumerate() {
        let event: Event = serde_json::from_value(raw_event).map_err(|e| {
            CliError::Other(format!(
                "malformed reactions query response: event {index} is not a complete Nostr event: {e}"
            ))
        })?;
        event.verify().map_err(|e| {
            CliError::Other(format!(
                "malformed reactions query response: event {index} failed cryptographic verification: {e}"
            ))
        })?;
        if event.kind != Kind::Reaction {
            return Err(CliError::Other(format!(
                "malformed reactions query response: event {index} has kind {}, expected 7",
                event.kind.as_u16()
            )));
        }
        let targets_requested_event = event.tags.iter().any(|tag| {
            let tag = tag.as_slice();
            tag.first().map(String::as_str) == Some("e")
                && tag.get(1).map(String::as_str) == Some(target_event_id)
        });
        if !targets_requested_event {
            return Err(CliError::Other(format!(
                "malformed reactions query response: event {index} does not target {target_event_id}"
            )));
        }
        events.push(event);
    }

    Ok(events)
}

fn render_reactions(mut events: Vec<Event>, include_events: bool) -> Result<String, CliError> {
    if include_events {
        events.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        return serde_json::to_string(&serde_json::json!({ "events": events }))
            .map_err(|e| CliError::Other(format!("failed to serialize reaction events: {e}")));
    }

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for event in &events {
        let emoji = if event.content.is_empty() {
            "+"
        } else {
            &event.content
        };
        groups
            .entry(emoji.to_string())
            .or_default()
            .push(event.pubkey.to_hex());
    }

    let mut reactions: Vec<serde_json::Value> = groups
        .into_iter()
        .map(|(emoji, pubkeys)| {
            serde_json::json!({
                "emoji": emoji,
                "count": pubkeys.len(),
                "pubkeys": pubkeys,
            })
        })
        .collect();
    reactions.sort_by(|a, b| {
        a.get("emoji")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("emoji").and_then(|v| v.as_str()).unwrap_or(""))
    });

    serde_json::to_string(&serde_json::json!({ "reactions": reactions }))
        .map_err(|e| CliError::Other(format!("failed to serialize reactions: {e}")))
}

async fn fetch_reaction_events(
    client: &BuzzClient,
    target_event_id: &str,
) -> Result<Vec<Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [7],
        "#e": [target_event_id]
    });
    let raw_events = client.query_all(filter).await?;
    let target_event_id = target_event_id.to_string();
    tokio::task::spawn_blocking(move || verify_reaction_events(raw_events, &target_event_id))
        .await
        .map_err(|e| CliError::Other(format!("reaction verification task failed: {e}")))?
}

pub async fn cmd_add_reaction(
    client: &BuzzClient,
    event_id: &str,
    emoji: &str,
    emoji_url: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let target_eid =
        EventId::parse(event_id).map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))?;

    let builder = if let Some(url) = emoji_url {
        buzz_sdk::build_custom_emoji_reaction(target_eid, emoji, url)
            .map_err(|e| CliError::Other(format!("build_custom_emoji_reaction failed: {e}")))?
    } else {
        buzz_sdk::build_reaction(target_eid, emoji)
            .map_err(|e| CliError::Other(format!("build_reaction failed: {e}")))?
    };

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_remove_reaction(
    client: &BuzzClient,
    event_id: &str,
    emoji: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let keys = client.keys();

    // Find our reaction event by querying kind:7 reactions on this event from us
    let my_pk = keys.public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [7],
        "#e": [event_id],
        "authors": [my_pk]
    });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse reactions query: {e}")))?;
    let arr = events
        .as_array()
        .ok_or_else(|| CliError::Other("reactions query response is not an array".into()))?;

    // Find the reaction event matching the emoji
    let reaction_event_id = arr
        .iter()
        .find(|ev| ev.get("content").and_then(|c| c.as_str()) == Some(emoji))
        .and_then(|ev| ev.get("id").and_then(|id| id.as_str()))
        .ok_or_else(|| {
            CliError::Other(format!(
                "no reaction with emoji '{emoji}' found for your pubkey on event {event_id}"
            ))
        })?;

    let reaction_eid = EventId::parse(reaction_event_id)
        .map_err(|e| CliError::Other(format!("invalid reaction event ID: {e}")))?;

    let builder = buzz_sdk::build_remove_reaction(reaction_eid)
        .map_err(|e| CliError::Other(format!("build_remove_reaction failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_get_reactions(
    client: &BuzzClient,
    event_id: &str,
    include_events: bool,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let target_event_id = EventId::parse(event_id)
        .map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))?
        .to_hex();
    let events = fetch_reaction_events(client, &target_event_id).await?;
    println!("{}", render_reactions(events, include_events)?);
    Ok(())
}

pub async fn dispatch(cmd: crate::ReactionsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::ReactionsCmd;
    match cmd {
        ReactionsCmd::Add {
            event,
            emoji,
            emoji_url,
        } => cmd_add_reaction(client, &event, &emoji, emoji_url.as_deref()).await,
        ReactionsCmd::Remove { event, emoji } => cmd_remove_reaction(client, &event, &emoji).await,
        ReactionsCmd::Get { event, events } => cmd_get_reactions(client, &event, events).await,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{
        body::Bytes,
        extract::State,
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::post,
        Json, Router,
    };
    use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};
    use tokio::net::TcpListener;

    use super::{fetch_reaction_events, render_reactions, verify_reaction_events};
    use crate::client::BuzzClient;

    const TARGET_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn target_id() -> EventId {
        EventId::from_hex(TARGET_ID).unwrap()
    }

    fn signed_reaction(keys: &Keys, created_at: u64, content: &str) -> Event {
        buzz_sdk::build_reaction(target_id(), content)
            .unwrap()
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn signed_custom_reaction(keys: &Keys, created_at: u64) -> Event {
        buzz_sdk::build_custom_emoji_reaction(target_id(), "fire", "https://example.com/fire.png")
            .unwrap()
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn verify(events: &[Event]) -> Result<Vec<Event>, crate::error::CliError> {
        verify_reaction_events(
            events
                .iter()
                .map(|event| serde_json::to_value(event).unwrap())
                .collect(),
            TARGET_ID,
        )
    }

    #[test]
    fn events_output_preserves_exact_full_event_fields_and_orders_oldest_first() {
        let first_keys = Keys::generate();
        let second_keys = Keys::generate();
        let first = signed_reaction(&first_keys, 1_750_000_000, "+");
        let second = signed_custom_reaction(&second_keys, 1_750_000_001);
        let verified = verify(&[second.clone(), first.clone()]).unwrap();

        let output: serde_json::Value =
            serde_json::from_str(&render_reactions(verified, true).unwrap()).unwrap();

        assert_eq!(
            output,
            serde_json::json!({
                "events": [
                    serde_json::to_value(&first).unwrap(),
                    serde_json::to_value(&second).unwrap()
                ]
            })
        );
        assert_eq!(output["events"][0]["id"], first.id.to_hex());
        assert_eq!(output["events"][0]["created_at"], 1_750_000_000u64);
        assert_eq!(output["events"][0]["sig"], first.sig.to_string());
        assert_eq!(
            output["events"][1]["tags"][1],
            serde_json::json!(["emoji", "fire", "https://example.com/fire.png"])
        );
    }

    #[test]
    fn events_output_breaks_timestamp_ties_by_exact_event_id() {
        let keys = Keys::generate();
        let mut events = vec![
            signed_reaction(&keys, 1_750_000_000, "👎"),
            signed_reaction(&keys, 1_750_000_000, "👍"),
        ];
        events.sort_by_key(|event| event.id);
        let first_id = events[0].id.to_hex();
        let second_id = events[1].id.to_hex();

        let output: serde_json::Value =
            serde_json::from_str(&render_reactions(verify(&events).unwrap(), true).unwrap())
                .unwrap();

        assert_eq!(output["events"][0]["id"], first_id);
        assert_eq!(output["events"][1]["id"], second_id);
    }

    #[test]
    fn aggregate_output_remains_compatible_by_default() {
        let first_keys = Keys::generate();
        let second_keys = Keys::generate();
        let events = verify(&[
            signed_reaction(&first_keys, 1, "👍"),
            signed_reaction(&second_keys, 2, "👍"),
        ])
        .unwrap();

        assert_eq!(
            render_reactions(events, false).unwrap(),
            format!(
                r#"{{"reactions":[{{"count":2,"emoji":"👍","pubkeys":["{}","{}"]}}]}}"#,
                first_keys.public_key().to_hex(),
                second_keys.public_key().to_hex()
            )
        );
    }

    #[test]
    fn forged_event_id_is_rejected_before_output() {
        let event = signed_reaction(&Keys::generate(), 1, "+");
        let mut forged = serde_json::to_value(event).unwrap();
        forged["id"] = serde_json::json!("0".repeat(64));

        assert!(verify_reaction_events(vec![forged], TARGET_ID).is_err());
    }

    #[test]
    fn tampered_content_is_rejected_before_output() {
        let event = signed_reaction(&Keys::generate(), 1, "+");
        let mut tampered = serde_json::to_value(event).unwrap();
        tampered["content"] = serde_json::json!("approve");

        assert!(verify_reaction_events(vec![tampered], TARGET_ID).is_err());
    }

    #[test]
    fn bad_schnorr_signature_is_rejected_before_output() {
        let event = signed_reaction(&Keys::generate(), 1, "+");
        let mut tampered = serde_json::to_value(event).unwrap();
        tampered["sig"] = serde_json::json!("0".repeat(128));

        assert!(verify_reaction_events(vec![tampered], TARGET_ID).is_err());
    }

    #[test]
    fn incomplete_wrong_kind_and_wrong_target_rows_are_rejected() {
        let keys = Keys::generate();
        let valid = signed_reaction(&keys, 1, "+");
        let mut incomplete = serde_json::to_value(&valid).unwrap();
        incomplete.as_object_mut().unwrap().remove("sig");
        let wrong_kind = EventBuilder::new(Kind::TextNote, "+")
            .tags([Tag::event(target_id())])
            .custom_created_at(Timestamp::from(1))
            .sign_with_keys(&keys)
            .unwrap();
        let wrong_target =
            buzz_sdk::build_reaction(EventId::from_hex(&"b".repeat(64)).unwrap(), "+")
                .unwrap()
                .custom_created_at(Timestamp::from(1))
                .sign_with_keys(&keys)
                .unwrap();

        for raw in [
            incomplete,
            serde_json::to_value(wrong_kind).unwrap(),
            serde_json::to_value(wrong_target).unwrap(),
        ] {
            assert!(verify_reaction_events(vec![raw], TARGET_ID).is_err());
        }
    }

    #[derive(Clone)]
    struct PagingState {
        first_page_event: serde_json::Value,
        second_page_event: serde_json::Value,
        expected_until: u64,
        expected_before_id: String,
        calls: Arc<AtomicUsize>,
        requested_limit: Arc<AtomicUsize>,
    }

    async fn paging_handler(State(state): State<PagingState>, body: Bytes) -> Response {
        let filters: Vec<serde_json::Value> = match serde_json::from_slice(&body) {
            Ok(filters) => filters,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": error.to_string()})),
                )
                    .into_response();
            }
        };
        let filter = &filters[0];
        let limit = filter["limit"].as_u64().unwrap() as usize;
        state.requested_limit.store(limit, Ordering::SeqCst);
        let call = state.calls.fetch_add(1, Ordering::SeqCst);

        if call == 0 {
            return Json(serde_json::Value::Array(vec![
                state.first_page_event.clone();
                limit
            ]))
            .into_response();
        }

        if filter["until"].as_u64() != Some(state.expected_until)
            || filter["before_id"].as_str() != Some(&state.expected_before_id)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "incorrect composite cursor"})),
            )
                .into_response();
        }

        Json(serde_json::Value::Array(vec![state.second_page_event])).into_response()
    }

    async fn paged_client(
        first_page_event: &Event,
        second_page_event: &Event,
    ) -> (BuzzClient, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let requested_limit = Arc::new(AtomicUsize::new(0));
        let state = PagingState {
            first_page_event: serde_json::to_value(first_page_event).unwrap(),
            second_page_event: serde_json::to_value(second_page_event).unwrap(),
            expected_until: first_page_event.created_at.as_secs(),
            expected_before_id: first_page_event.id.to_hex(),
            calls: calls.clone(),
            requested_limit: requested_limit.clone(),
        };
        let app = Router::new()
            .route("/query", post(paging_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            BuzzClient::new(format!("http://{addr}"), Keys::generate(), None, None).unwrap();
        (client, calls, requested_limit)
    }

    #[tokio::test]
    async fn pagination_finds_first_winner_beyond_more_than_one_hundred_reactions() {
        let keys = Keys::generate();
        let first_page = signed_reaction(&keys, 200, "reject");
        let globally_oldest = signed_reaction(&keys, 100, "approve");
        let (client, calls, requested_limit) = paged_client(&first_page, &globally_oldest).await;

        let events = fetch_reaction_events(&client, TARGET_ID).await.unwrap();
        let page_size = requested_limit.load(Ordering::SeqCst);
        assert!(page_size > 100);
        assert_eq!(events.len(), page_size + 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let output: serde_json::Value =
            serde_json::from_str(&render_reactions(events, true).unwrap()).unwrap();
        assert_eq!(output["events"][0]["id"], globally_oldest.id.to_hex());
        assert_eq!(output["events"][0]["content"], "approve");
    }

    #[tokio::test]
    async fn pagination_retains_timestamp_ties_across_the_page_boundary() {
        let keys = Keys::generate();
        let mut tied = vec![
            signed_reaction(&keys, 300, "first"),
            signed_reaction(&keys, 300, "second"),
        ];
        tied.sort_by_key(|event| event.id);
        let first_page = tied.remove(0);
        let second_page = tied.remove(0);
        let (client, calls, requested_limit) = paged_client(&first_page, &second_page).await;

        let events = fetch_reaction_events(&client, TARGET_ID).await.unwrap();
        let page_size = requested_limit.load(Ordering::SeqCst);
        assert_eq!(events.len(), page_size + 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let output: serde_json::Value =
            serde_json::from_str(&render_reactions(events, true).unwrap()).unwrap();
        assert_eq!(output["events"][0]["id"], first_page.id.to_hex());
        assert_eq!(output["events"][page_size]["id"], second_page.id.to_hex());
    }
}
