use nostr::{EventBuilder, EventId, Kind, Tag};
use tauri::State;

use crate::{app_state::AppState, relay::submit_event};

/// Delete a message as a relay-level moderator, owner, or admin (kind 9005).
///
/// Unlike `delete_message` (kind 5, author-only), this sends a Buzz-native
/// moderator delete event validated by the relay against the actor's relay
/// role via `authorize_moderation_action`.
#[tauri::command]
pub async fn moderator_delete_message(
    channel_id: String,
    event_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let target_eid = EventId::from_hex(&event_id).map_err(|e| format!("invalid event ID: {e}"))?;
    let tags = vec![
        Tag::parse(vec!["h", &channel_uuid.to_string()]).map_err(|e| format!("tag error: {e}"))?,
        Tag::parse(vec!["e", &target_eid.to_hex()]).map_err(|e| format!("tag error: {e}"))?,
    ];
    let builder = EventBuilder::new(Kind::Custom(9005), "").tags(tags);
    submit_event(builder, &state).await?;
    Ok(())
}
