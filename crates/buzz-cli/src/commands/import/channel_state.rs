//! Relay-backed channel-state verification for adopted Slack conversations.

use uuid::Uuid;

use crate::client::BuzzClient;
use crate::error::CliError;

/// Whether the relay currently reports the channel as archived. Fails when
/// the relay holds no metadata the caller can read for it, so a mistyped or
/// invisible channel is never mistaken for an active one.
pub(super) async fn is_archived(
    client: &BuzzClient,
    channel_id: Uuid,
    slack_conversation_id: &str,
) -> Result<bool, CliError> {
    let filter = serde_json::json!({
        "kinds": [39000],
        "#d": [channel_id.to_string()],
        "limit": 1,
    });
    let response = client.query(&filter).await.map_err(|error| {
        CliError::Other(format!(
            "could not read the state of Buzz channel {channel_id} for conversation \
             {slack_conversation_id}: {error}"
        ))
    })?;
    metadata_archived_state(&response, channel_id)?.ok_or_else(|| {
        CliError::Other(format!(
            "relay returned no current metadata for Buzz channel {channel_id} \
             (conversation {slack_conversation_id})"
        ))
    })
}

/// Read the relay-generated channel metadata after an unarchive request and
/// fail closed unless the channel is visibly active.
pub(super) async fn require_unarchived(
    client: &BuzzClient,
    channel_id: Uuid,
    slack_conversation_id: &str,
) -> Result<(), CliError> {
    if is_archived(client, channel_id, slack_conversation_id).await? {
        return Err(CliError::Other(format!(
            "could not prepare conversation {slack_conversation_id} for history import: \
             Buzz channel {channel_id} remains archived after the unarchive request"
        )));
    }
    Ok(())
}

/// Return the archived state from the relay's current kind:39000 metadata.
///
/// Buzz encodes an active channel by omitting the `archived=true` tag.
pub(super) fn metadata_archived_state(
    response: &str,
    channel_id: Uuid,
) -> Result<Option<bool>, CliError> {
    let events: Vec<serde_json::Value> = serde_json::from_str(response)
        .map_err(|error| CliError::Other(format!("could not parse channel metadata: {error}")))?;
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
