use serde::Deserialize;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::ChannelInfo,
    nostr_convert,
    relay::{parse_command_response, query_relay, submit_event},
};

#[derive(Deserialize)]
struct OpenDmAck {
    channel_id: String,
}

/// Stable namespace bytes for serverless DM channel ids (random, fixed).
const DM_NAMESPACE_BYTES: [u8; 16] = [
    0x6f, 0x1d, 0x2c, 0x3b, 0x4a, 0x59, 0x4e, 0x87, 0x9b, 0x0c, 0x1d, 0x2e, 0x3f, 0x4a, 0x5b, 0x6c,
];

/// Derive a deterministic DM channel id from the sorted participant set so all
/// participants converge on the same channel without a relay assigning one.
/// Participants must already be lowercased, sorted, and de-duplicated.
fn derive_dm_channel_id(participants: &[String]) -> String {
    let namespace = uuid::Uuid::from_bytes(DM_NAMESPACE_BYTES);
    let joined = participants.join(",");
    uuid::Uuid::new_v5(&namespace, joined.as_bytes()).to_string()
}

#[tauri::command]
pub async fn open_dm(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    let channel_id = if state.is_serverless() {
        // No relay to assign a channel id. Derive a deterministic id from the
        // sorted participant set (including self) so both sides converge on the
        // same DM channel, then publish 39000 + 39002 directly. Return
        // ChannelInfo from the locally signed event rather than re-querying the
        // relay (which races against propagation lag).
        let (my_pubkey, keys) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            (keys.public_key().to_hex(), keys.clone())
        };
        let mut participants: Vec<String> = pubkeys
            .iter()
            .map(|p| p.to_ascii_lowercase())
            .chain(std::iter::once(my_pubkey))
            .collect();
        participants.sort();
        participants.dedup();

        let channel_id = derive_dm_channel_id(&participants);

        let meta = events::build_channel_metadata_serverless(
            &channel_id,
            "Direct message",
            "private",
            "dm",
            None,
            &participants,
        )?;
        let meta_event = meta
            .clone()
            .sign_with_keys(&keys)
            .map_err(|e| format!("failed to sign DM metadata: {e}"))?;
        submit_event(meta, &state).await?;
        let members = events::build_channel_members_serverless(&channel_id, &participants)?;
        submit_event(members, &state).await?;

        return nostr_convert::channel_info_from_event(&meta_event, None, Some(true));
    } else {
        // Submit a kind:41010 dm-open event; the relay replies with the channel
        // id in its OK message payload.
        let builder = events::build_dm_open(&pubkeys)?;
        let result = submit_event(builder, &state).await?;
        let ack: OpenDmAck = parse_command_response(&result.message)?;
        ack.channel_id
    };

    // Re-fetch the channel metadata so the frontend gets the same `ChannelInfo`
    // shape as `get_channel_details`.
    let metadata = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    metadata
        .first()
        .map(|ev| nostr_convert::channel_info_from_event(ev, None, None))
        .transpose()?
        .ok_or_else(|| "DM channel created but metadata not yet available".to_string())
}

#[tauri::command]
pub async fn hide_dm(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let builder = events::build_dm_hide(&channel_id)?;
    submit_event(builder, &state).await?;
    Ok(())
}
