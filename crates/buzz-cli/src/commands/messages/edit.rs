//! Resolve replacement attachment state before publishing an edit.
use super::{channel_id_from_event, fetch_event};
use crate::{
    client::BuzzClient,
    error::CliError,
    validate::{parse_event_id, validate_content_size, validate_hex64},
};
use nostr::{Event, Timestamp};

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
    if original.id != target {
        return Err(CliError::Other(
            "relay returned the wrong edit target".into(),
        ));
    }
    // Keep this boundary aligned with media_reference_query and the relay lookup.
    // Other channel-tagged events retain the historical plain-edit behavior.
    if !buzz_core::media::EDIT_GOVERNED_KINDS.contains(&original.kind.as_u16()) {
        if clear_attachments {
            return Err(CliError::Usage(format!(
                "attachments on kind {} are not edit-governed",
                original.kind.as_u16()
            )));
        }
        let builder = buzz_sdk::build_edit(channel, target, content)
            .map_err(|e| CliError::Other(format!("build_edit failed: {e}")))?;
        let event = client.sign_event(builder).await?;
        let resp = client.submit_event(event).await?;
        println!("{}", crate::client::normalize_write_response(&resp));
        return Ok(());
    }
    let raw = client
        .query(&serde_json::json!({
            "kinds": buzz_core::media::EDIT_GOVERNED_KINDS, "ids": [target.to_hex()], "#h": [channel.to_string()],
            "effective_edit": 1, "consistency": "strong",
        }))
        .await?;
    let snapshot: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid effective edit response: {e}")))?;
    if snapshot.get("effective_edit") != Some(&serde_json::json!(1))
        || snapshot.get("edit").is_none()
    {
        return Err(CliError::Other(
            "relay does not support effective_edit v1; upgrade the relay before editing".into(),
        ));
    }
    let original: Event = serde_json::from_value(snapshot["original"].clone())
        .map_err(|e| CliError::Other(format!("invalid effective original: {e}")))?;
    if original.id != target
        || !buzz_core::media::EDIT_GOVERNED_KINDS.contains(&original.kind.as_u16())
        || channel_id_from_event(event_id, &snapshot["original"])? != channel
    {
        return Err(CliError::Other(
            "effective edit response has the wrong original".into(),
        ));
    }
    let edits: Vec<Event> = if snapshot["edit"].is_null() {
        vec![]
    } else {
        vec![serde_json::from_value(snapshot["edit"].clone())
            .map_err(|e| CliError::Other(format!("invalid effective edit: {e}")))?]
    };
    let effective = latest_edit(&original, &edits);
    if !edits.is_empty() && effective.id == original.id {
        return Err(CliError::Other(
            "effective edit response has the wrong replacement".into(),
        ));
    }
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
