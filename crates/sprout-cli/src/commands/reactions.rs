use nostr::EventId;

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

pub async fn cmd_add_reaction(
    client: &SproutClient,
    event_id: &str,
    emoji: &str,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let target_eid =
        EventId::parse(event_id).map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))?;

    let builder = sprout_sdk::build_reaction(target_eid, emoji)
        .map_err(|e| CliError::Other(format!("build_reaction failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_remove_reaction(
    client: &SproutClient,
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

    let builder = sprout_sdk::build_remove_reaction(reaction_eid)
        .map_err(|e| CliError::Other(format!("build_remove_reaction failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_reactions(client: &SproutClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    // Query kind:7 reactions referencing this event
    let filter = serde_json::json!({
        "kinds": [7],
        "#e": [event_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}
