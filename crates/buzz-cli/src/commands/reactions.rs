use std::collections::HashMap;

use nostr::EventId;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_hex64, validate_hex64};

/// The kind:7 filter both `reactions get` and `reactions remove` query with,
/// optionally narrowed to one author.
///
/// `event_id` must already be lowercased (`parse_hex64`): `#e` is a generic
/// tag filter compared as a raw string against a lowercase tag, so an
/// uppercase id returns nothing at all.
fn reactions_filter(event_id: &str, author: Option<&str>) -> serde_json::Value {
    let mut filter = serde_json::json!({
        "kinds": [7],
        "#e": [event_id]
    });
    if let Some(author) = author {
        filter["authors"] = serde_json::json!([author]);
    }
    filter
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
    // The id reaches a `#e` generic tag filter below, which is matched as a
    // raw string against a lowercase tag.
    let event_id = &parse_hex64(event_id)?;
    let keys = client.keys();

    // Find our reaction event by querying kind:7 reactions on this event from us
    let my_pk = keys.public_key().to_hex();
    let filter = reactions_filter(event_id, Some(&my_pk));
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

pub async fn cmd_get_reactions(client: &BuzzClient, event_id: &str) -> Result<(), CliError> {
    // As in `cmd_remove_reaction`: `#e` is compared as a raw string.
    let event_id = &parse_hex64(event_id)?;
    let filter = reactions_filter(event_id, None);
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
mod reactions_filter_tests {
    use super::reactions_filter;
    use crate::validate::parse_hex64;

    #[test]
    fn the_e_tag_filter_carries_the_normalized_id() {
        // An uppercase id here returns no reactions at all: `get` prints an
        // empty list and `remove` reports the reaction as missing.
        let upper = "ABCDEF0123456789".repeat(4);
        let filter = reactions_filter(&parse_hex64(&upper).unwrap(), None);
        assert_eq!(filter["#e"][0], upper.to_lowercase().as_str());
        assert_eq!(filter["kinds"][0], 7);
        assert!(filter.get("authors").is_none());
    }

    #[test]
    fn an_author_narrows_the_same_filter() {
        let id = "a".repeat(64);
        let author = "b".repeat(64);
        let filter = reactions_filter(&id, Some(&author));
        assert_eq!(filter["#e"][0], id.as_str());
        assert_eq!(filter["authors"][0], author.as_str());
    }
}
