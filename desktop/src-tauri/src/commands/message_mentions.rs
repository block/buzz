use nostr::Keys;

use crate::{app_state::AppState, nostr_convert, relay::query_relay};

pub(super) fn resolve(
    content: &str,
    existing_mentions: &[String],
    fresh_member_pubkeys: &[String],
    sender_pubkey: &str,
) -> Result<Vec<String>, String> {
    buzz_sdk_pkg::mentions::resolve_all_mention_pubkeys(
        content,
        existing_mentions,
        fresh_member_pubkeys,
        sender_pubkey,
    )
    .map(|resolved| resolved.unwrap_or_else(|| existing_mentions.to_vec()))
    .map_err(|error| error.to_string())
}

async fn from_relay(
    state: &AppState,
    channel_id: &str,
    content: &str,
    existing_mentions: Option<Vec<String>>,
    sender_pubkey: &str,
) -> Result<Vec<String>, String> {
    let existing_mentions = existing_mentions.unwrap_or_default();
    if !buzz_sdk_pkg::mentions::contains_all_mention(content) {
        return Ok(existing_mentions);
    }

    let membership_events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
            "limit": 1,
        })],
    )
    .await?;
    let membership = membership_events
        .first()
        .map(nostr_convert::channel_members_from_event)
        .transpose()?
        .ok_or_else(|| "channel members not found for @all mention".to_string())?;
    let member_pubkeys: Vec<String> = membership
        .members
        .into_iter()
        .map(|member| member.pubkey)
        .collect();

    resolve(content, &existing_mentions, &member_pubkeys, sender_pubkey)
}

pub(super) async fn human(
    state: &AppState,
    channel_id: &str,
    content: &str,
    existing_mentions: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let sender_pubkey = {
        state
            .keys
            .lock()
            .map_err(|error| error.to_string())?
            .public_key()
            .to_hex()
    };
    from_relay(
        state,
        channel_id,
        content,
        existing_mentions,
        &sender_pubkey,
    )
    .await
}

pub(super) async fn agent(
    state: &AppState,
    channel_id: &str,
    content: &str,
    existing_mentions: Option<Vec<String>>,
    keys: &Keys,
) -> Result<Vec<String>, String> {
    from_relay(
        state,
        channel_id,
        content,
        existing_mentions,
        &keys.public_key().to_hex(),
    )
    .await
}
