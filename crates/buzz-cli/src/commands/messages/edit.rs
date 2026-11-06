//! Resolve replacement attachment state before publishing an edit.
use super::{channel_id_from_event, fetch_event};
use crate::{
    client::BuzzClient,
    error::CliError,
    validate::{parse_event_id, validate_content_size, validate_hex64},
};
use nostr::{Event, Timestamp};

// Exhaustion must abort, never fall back to the original's attachments.
const EDIT_HISTORY_BOUND: u32 = 10_000;

pub(super) async fn edit_message(
    client: &BuzzClient,
    event_id: &str,
    content: &str,
    clear_attachments: bool,
) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    validate_content_size(content)?;
    let target = parse_event_id(event_id)?;
    let raw = fetch_event(client, &target.to_hex()).await?;
    let channel = channel_id_from_event(event_id, &raw)?;
    let original: Event = serde_json::from_value(raw)
        .map_err(|e| CliError::Other(format!("invalid edit target: {e}")))?;
    if original.id != target || !matches!(original.kind.as_u16(), 9 | 40002) {
        return Err(CliError::Usage(
            "edit requires the original message's event ID (kind 9 or 40002)".into(),
        ));
    }
    // Query live edits in this channel. An exact #e filter would miss mixed-case
    // hex targets that the relay accepts, and author filtering would miss edits
    // made by the agent's human owner. query_all_bounded fails on truncation.
    let history = client
        .query_all_bounded(
            serde_json::json!({
                "kinds": [40003], "#h": [channel.to_string()],
            }),
            EDIT_HISTORY_BOUND,
        )
        .await?;
    let edits: Vec<Event> = history
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|e| CliError::Other(format!("invalid edit history: {e}")))
        })
        .collect::<Result<_, _>>()?;
    let effective = latest_edit(&original, &edits);
    let media = if clear_attachments {
        Vec::new()
    } else {
        effective
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some("imeta"))
            .map(|t| t.as_slice().to_vec())
            .collect()
    };
    let now = Timestamp::now().as_secs();
    // Make successive edits effective even when issued in the same second.
    // Refuse an unexpectedly future timeline instead of extending it forever.
    let head = effective.created_at.as_secs();
    if head > now.saturating_add(60) {
        return Err(CliError::Other(
            "latest edit is too far in the future; retry when the clock catches up".into(),
        ));
    }
    let created_at = now.max(head + 1);
    let builder = buzz_sdk::build_edit_with_media(channel, target, content, &media)
        .map_err(|e| CliError::Other(format!("build_edit failed: {e}")))?
        .custom_created_at(Timestamp::from(created_at));
    let event = client.sign_event(builder).await?;
    let resp = client.submit_event(event).await?;
    println!("{}", crate::client::normalize_write_response(&resp));
    Ok(())
}

fn latest_edit<'a>(original: &'a Event, edits: &'a [Event]) -> &'a Event {
    let channel = |event: &Event| {
        event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(String::as_str) == Some("h"))
            .and_then(|t| t.content())
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
    };
    edits
        .iter()
        .filter(|edit| {
            edit.kind.as_u16() == 40003
                && channel(edit) == channel(original)
                && buzz_core::media::first_edit_target(edit).as_deref()
                    == Some(original.id.as_bytes().as_slice())
        })
        .max_by_key(|edit| (edit.created_at, edit.id))
        .unwrap_or(original)
}

#[cfg(test)]
mod tests;
