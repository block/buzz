mod activity;
pub(crate) mod calendar;
mod liveness;
mod sanitize;
pub(crate) mod storage;
pub(crate) mod types;
mod value_inbox;
mod worker;

use std::sync::OnceLock;

use nostr::Keys;

use crate::{app_state::AppState, relay::query_relay_at_with_keys};

static OPERATIONS_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub(crate) fn operations_lock() -> &'static tokio::sync::Mutex<()> {
    OPERATIONS_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(crate) async fn channel_member_pubkeys(
    state: &AppState,
    api_base_url: &str,
    owner_keys: &Keys,
    channel_id: &str,
) -> Result<Vec<String>, String> {
    let events = query_relay_at_with_keys(
        state,
        api_base_url,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
            "limit": 1
        })],
        owner_keys,
        None,
    )
    .await?;
    let response = events
        .first()
        .map(crate::nostr_convert::channel_members_from_event)
        .transpose()?
        .ok_or_else(|| "channel members not found".to_string())?;
    Ok(response
        .members
        .into_iter()
        .map(|member| member.pubkey)
        .collect())
}

pub(crate) use worker::spawn;
