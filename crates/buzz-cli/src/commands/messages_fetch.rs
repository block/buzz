//! Shared signed-event retrieval for message commands.

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Fetch a message event while preserving its signed relay representation.
pub(super) async fn fetch_event(
    client: &BuzzClient,
    event_id: &str,
) -> Result<serde_json::Value, CliError> {
    let filter = serde_json::json!({
        "ids": [event_id],
        "kinds": [9, 40002, 40003, 40008, 45001, 45003],
        "limit": 1
    });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("failed to parse query response: {error}")))?;
    events
        .as_array()
        .and_then(|events| events.first())
        .cloned()
        .ok_or_else(|| CliError::NotFound(format!("event {event_id} not found")))
}

/// Print a signed message event without model-oriented normalization.
pub(super) async fn cmd_get_raw_event(client: &BuzzClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let event = fetch_event(client, event_id).await?;
    let json = serde_json::to_string(&event)
        .map_err(|error| CliError::Other(format!("failed to serialize raw event: {error}")))?;
    println!("{json}");
    Ok(())
}
