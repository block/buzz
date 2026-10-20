//! Establishes agent-authenticated publishers for locally synthesized speech.

use std::sync::Arc;

use super::{relay_api, tts};
use crate::app_state::AppState;

pub(super) async fn ensure(
    app: &tauri::AppHandle,
    state: &AppState,
    pipeline: &tts::TtsPipeline,
    speaker_pubkey: &str,
) -> Result<bool, String> {
    if pipeline.has_audio_publisher(speaker_pubkey) {
        return Ok(true);
    }

    let app_for_load = app.clone();
    let speaker_for_load = speaker_pubkey.to_ascii_lowercase();
    let record = tokio::task::spawn_blocking(move || {
        crate::managed_agents::load_managed_agents(&app_for_load).map(|agents| {
            agents.into_iter().find(|agent| {
                agent.pubkey.eq_ignore_ascii_case(&speaker_for_load)
                    && !agent.private_key_nsec.trim().is_empty()
            })
        })
    })
    .await
    .map_err(|error| format!("managed-agent identity task failed: {error}"))??;
    let Some(record) = record else {
        return Ok(false);
    };

    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("managed-agent identity is unavailable: {error}"))?;
    if !keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(speaker_pubkey)
    {
        return Err("managed-agent identity does not match the Huddle speaker".to_string());
    }
    let (ephemeral_channel_id, parent_channel_id, local_tts_publishers) = {
        let huddle = state.huddle()?;
        (
            huddle
                .ephemeral_channel_id
                .clone()
                .ok_or("active Huddle has no backing channel")?,
            huddle.parent_channel_id.clone(),
            Arc::clone(&huddle.local_tts_publishers),
        )
    };
    let has_bot_membership =
        relay_api::fetch_channel_members_with_roles(&ephemeral_channel_id, state)
            .await?
            .into_iter()
            .any(|(pubkey, role)| {
                pubkey.eq_ignore_ascii_case(speaker_pubkey) && role.as_deref() == Some("bot")
            });
    if !has_bot_membership {
        return Err("agent is not an active bot member of the Huddle".to_string());
    }
    let publisher = relay_api::connect_tts_audio_publisher(
        &ephemeral_channel_id,
        parent_channel_id.as_deref(),
        state,
        &keys,
        record.auth_tag.as_deref(),
        local_tts_publishers,
    )
    .await?;
    pipeline.register_audio_publisher(speaker_pubkey, publisher);
    Ok(true)
}
