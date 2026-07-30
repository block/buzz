//! Relay-backed channel-state verification for adopted Slack conversations.

use uuid::Uuid;

use crate::client::BuzzClient;
use crate::error::CliError;

/// Read the relay-generated channel metadata after an unarchive request and
/// fail closed unless the channel is visibly active.
pub(super) async fn require_unarchived(
    client: &BuzzClient,
    channel_id: Uuid,
    slack_conversation_id: &str,
) -> Result<(), CliError> {
    let filter = serde_json::json!({
        "kinds": [39000],
        "#d": [channel_id.to_string()],
        "limit": 1,
    });
    let response = client.query(&filter).await.map_err(|error| {
        CliError::Other(format!(
            "could not verify conversation {slack_conversation_id} after its unarchive request: \
             {error}"
        ))
    })?;

    match metadata_archived_state(&response, channel_id)? {
        Some(false) => Ok(()),
        Some(true) => Err(CliError::Other(format!(
            "could not prepare conversation {slack_conversation_id} for history import: \
             Buzz channel {channel_id} remains archived after the unarchive request"
        ))),
        None => Err(CliError::Other(format!(
            "could not verify conversation {slack_conversation_id} after its unarchive request: \
             relay returned no current metadata for Buzz channel {channel_id}"
        ))),
    }
}

/// Return the archived state from the relay's current kind:39000 metadata.
///
/// Buzz encodes an active channel by omitting the `archived=true` tag.
pub(super) fn metadata_archived_state(
    response: &str,
    channel_id: Uuid,
) -> Result<Option<bool>, CliError> {
    let events: Vec<serde_json::Value> = serde_json::from_str(response).map_err(|error| {
        CliError::Other(format!(
            "could not parse channel metadata after unarchive request: {error}"
        ))
    })?;
    let expected_id = channel_id.to_string();

    Ok(events.iter().find_map(|event| {
        let tags = event.get("tags")?.as_array()?;
        let matches_channel = tags.iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.first().and_then(serde_json::Value::as_str) == Some("d")
                    && parts.get(1).and_then(serde_json::Value::as_str)
                        == Some(expected_id.as_str())
            })
        });
        if !matches_channel {
            return None;
        }
        let archived = tags.iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.first().and_then(serde_json::Value::as_str) == Some("archived")
                    && parts.get(1).and_then(serde_json::Value::as_str) == Some("true")
            })
        });
        Some(archived)
    }))
}
