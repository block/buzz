use std::collections::HashMap;

use nostr::{EventId, PublicKey, Tag};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_hex64;

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
    // NIP-25: lets the target's author find reactions with a `#p` filter.
    // An unscoped read can miss a target the sender may react to (system
    // channels), so the relay, not this lookup, decides whether it exists.
    let builder = match fetch_target_author(client, event_id).await? {
        Some(target_author) => builder.tag(Tag::public_key(target_author)),
        None => builder,
    };

    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn fetch_target_author(
    client: &BuzzClient,
    event_id: &str,
) -> Result<Option<PublicKey>, CliError> {
    // Strong: a target just written may not have reached the read replica yet.
    let filter = serde_json::json!({ "ids": [event_id], "limit": 1, "consistency": "strong" });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
    let Some(author) = events
        .as_array()
        .and_then(|events| events.first())
        .and_then(|event| event.get("pubkey"))
        .and_then(|pubkey| pubkey.as_str())
    else {
        return Ok(None);
    };
    PublicKey::from_hex(author)
        .map(Some)
        .map_err(|e| CliError::Other(format!("event {event_id} has an invalid pubkey: {e}")))
}

pub async fn cmd_remove_reaction(
    client: &BuzzClient,
    event_id: &str,
    emoji: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    // Find our reaction event by querying kind:7 reactions on this event from us
    let my_pk = client.public_key().to_hex();
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

    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_get_reactions(client: &BuzzClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let filter = serde_json::json!({
        "kinds": [7],
        "#e": [event_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for e in &events {
        let emoji = e
            .get("content")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("+")
            .to_string();
        let pubkey = e
            .get("pubkey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        groups.entry(emoji).or_default().push(pubkey);
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

    let output = serde_json::json!({ "reactions": reactions });
    println!("{}", serde_json::to_string(&output).unwrap_or_default());
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
        ReactionsCmd::Get { event } => cmd_get_reactions(client, &event).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{cmd_add_reaction, BuzzClient, CliError};
    use crate::credentials::Credentials;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::Router;
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const TARGET_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TARGET_AUTHOR: &str = "35c18ae273fccfaf80d629e20e7f8721b90499379addff533054acc2504c12b4";

    type Submitted = Arc<Mutex<Option<serde_json::Value>>>;
    type Reply = (StatusCode, String);

    const ACCEPTED: &str = r#"{"event_id":"fake","accepted":true}"#;

    #[derive(Clone)]
    struct FakeRelay {
        query: Reply,
        events: Reply,
        submitted: Submitted,
        queried: Submitted,
    }

    async fn fake_relay(query_body: String) -> (String, Submitted, Submitted) {
        fake_relay_replying(
            (StatusCode::OK, query_body),
            (StatusCode::OK, ACCEPTED.into()),
        )
        .await
    }

    async fn fake_relay_replying(query: Reply, events: Reply) -> (String, Submitted, Submitted) {
        let relay = FakeRelay {
            query,
            events,
            submitted: Arc::new(Mutex::new(None)),
            queried: Arc::new(Mutex::new(None)),
        };
        let (submitted, queried) = (relay.submitted.clone(), relay.queried.clone());
        let app = Router::new()
            .route(
                "/query",
                post(
                    |State(relay): State<FakeRelay>, request: Bytes| async move {
                        *relay.queried.lock().unwrap() = serde_json::from_slice(&request).ok();
                        let (status, body) = relay.query;
                        (status, [("content-type", "application/json")], body)
                    },
                ),
            )
            .route(
                "/events",
                post(|State(relay): State<FakeRelay>, body: Bytes| async move {
                    *relay.submitted.lock().unwrap() = serde_json::from_slice(&body).ok();
                    let (status, body) = relay.events;
                    (status, [("content-type", "application/json")], body)
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), submitted, queried)
    }

    fn target_response() -> String {
        serde_json::json!([{ "id": TARGET_ID, "pubkey": TARGET_AUTHOR, "kind": 9, "tags": [] }])
            .to_string()
    }

    fn submitted_tags(submitted: &Submitted) -> Vec<Vec<String>> {
        let event = submitted.lock().unwrap().clone().expect("event submitted");
        serde_json::from_value(event["tags"].clone()).unwrap()
    }

    #[tokio::test]
    async fn add_reaction_tags_target_author() {
        let (url, submitted, _) = fake_relay(target_response()).await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        cmd_add_reaction(&client, TARGET_ID, "👍", None)
            .await
            .unwrap();

        let tags = submitted_tags(&submitted);
        assert!(tags.contains(&vec!["e".to_string(), TARGET_ID.to_string()]));
        assert!(
            tags.contains(&vec!["p".to_string(), TARGET_AUTHOR.to_string()]),
            "reaction must carry a p tag naming the target author, got {tags:?}"
        );
    }

    #[tokio::test]
    async fn add_custom_emoji_reaction_tags_target_author() {
        let (url, submitted, _) = fake_relay(target_response()).await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        cmd_add_reaction(
            &client,
            TARGET_ID,
            "wave",
            Some("https://cdn.example.com/wave.png"),
        )
        .await
        .unwrap();

        let tags = submitted_tags(&submitted);
        assert!(tags.contains(&vec!["p".to_string(), TARGET_AUTHOR.to_string()]));
        assert!(tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("emoji")));
    }

    #[tokio::test]
    async fn add_reaction_to_unreadable_target_submits_without_p_tag() {
        let (url, submitted, _) = fake_relay("[]".to_string()).await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        cmd_add_reaction(&client, TARGET_ID, "👍", None)
            .await
            .unwrap();

        let tags = submitted_tags(&submitted);
        assert!(tags.contains(&vec!["e".to_string(), TARGET_ID.to_string()]));
        assert!(
            !tags
                .iter()
                .any(|tag| tag.first().map(String::as_str) == Some("p")),
            "an unreadable target has no known author to tag, got {tags:?}"
        );
    }

    #[tokio::test]
    async fn add_reaction_shows_the_relay_rejection_verbatim() {
        let rejection = r#"{"error":"invalid: reaction target event not found"}"#;
        let (url, _, _) = fake_relay_replying(
            (StatusCode::OK, "[]".into()),
            (StatusCode::BAD_REQUEST, rejection.into()),
        )
        .await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        let error = cmd_add_reaction(&client, TARGET_ID, "👍", None)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid: reaction target event not found"),
            "the relay's rejection must reach the user as written, got {error}"
        );
    }

    #[tokio::test]
    async fn add_reaction_fails_without_submitting_when_target_lookup_errors() {
        let (url, submitted, _) = fake_relay_replying(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":"boom"}"#.into(),
            ),
            (StatusCode::OK, ACCEPTED.into()),
        )
        .await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        let result = cmd_add_reaction(&client, TARGET_ID, "👍", None).await;

        assert!(matches!(result, Err(CliError::Relay { status: 500, .. })));
        assert!(submitted.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn add_reaction_reads_target_from_the_writer() {
        let (url, _, queried) = fake_relay(target_response()).await;
        let client = BuzzClient::new(url, Credentials::from_keys(Keys::generate())).unwrap();

        cmd_add_reaction(&client, TARGET_ID, "👍", None)
            .await
            .unwrap();

        let query = queried.lock().unwrap().clone().expect("target queried");
        let filter = query
            .as_array()
            .and_then(|filters| filters.first())
            .unwrap_or(&query);
        assert_eq!(
            filter.get("consistency").and_then(|value| value.as_str()),
            Some("strong"),
            "the target lookup gates a write, so replica lag must not hide the target: {query}"
        );
    }
}
