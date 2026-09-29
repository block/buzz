use serde::Deserialize;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::ChannelInfo,
    nostr_convert,
    relay::{
        assert_expected_relay_scope, assert_expected_signer, parse_command_response,
        query_relay_at_with_keys, submit_event, submit_event_at_with_keys,
    },
};

#[derive(Deserialize)]
struct OpenDmAck {
    channel_id: String,
}

#[tauri::command]
pub async fn open_dm(
    pubkeys: Vec<String>,
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    open_dm_with_scope(
        pubkeys,
        expected_relay_url.as_deref(),
        expected_signer_pubkey.as_deref(),
        &state,
    )
    .await
}

pub(crate) async fn open_dm_with_scope(
    pubkeys: Vec<String>,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
    state: &AppState,
) -> Result<ChannelInfo, String> {
    // Resolve the relay AND the signing identity once for the open + metadata
    // read pair. Callers with a captured tenant scope (Projects agent sends)
    // pass `expected_relay_url` and `expected_signer_pubkey`; a mismatch on
    // either means the active community changed while their callback was
    // suspended. The relay check alone is not enough: relay and keys mutate
    // under separate locks during a workspace switch, so a switch landing
    // between the URL check and the key read would otherwise create the
    // tenant-A DM signed as tenant B's identity — fail closed instead, and
    // use this exact key snapshot for both the event signature and the
    // NIP-98 auth of every request in this command.
    let api_base_url = crate::relay::relay_api_base_url_with_override(state);
    assert_expected_relay_scope(expected_relay_url, &api_base_url)?;
    let keys = state.signing_keys()?;
    assert_expected_signer(expected_signer_pubkey, &keys.public_key().to_hex())?;

    // Submit a kind:41010 dm-open event; the relay replies with the channel id
    // in its OK message payload.
    let builder = events::build_dm_open(&pubkeys)?;
    let result = submit_event_at_with_keys(builder, state, &api_base_url, &keys).await?;
    let ack: OpenDmAck = parse_command_response(&result.message)?;

    // Re-fetch the channel metadata so the frontend gets the same `ChannelInfo`
    // shape as `get_channel_details` — through the same scope-checked base and
    // the same pinned identity.
    let metadata = query_relay_at_with_keys(
        state,
        &api_base_url,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [ack.channel_id],
            "limit": 1
        })],
        &keys,
        None,
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
