//! Tauri commands for NIP-17 encrypted messaging (serverless private channels
//! and DMs). See `crate::encrypted` for the gift-wrap scheme.

use nostr::{Event, JsonUtil};
use tauri::State;

use crate::app_state::AppState;

/// A decrypted gift wrap, shaped like a `RelayEvent` so the frontend can feed
/// it straight into the existing message pipeline.
#[derive(serde::Serialize)]
pub struct DecryptedEvent {
    /// Inner rumor id (stable logical message id).
    pub id: String,
    /// Real author (recovered + verified from the seal).
    pub pubkey: String,
    pub created_at: i64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    /// Rumors carry no signature; always empty.
    pub sig: String,
    /// The channel id from the rumor's `h` tag, if present (convenience for
    /// routing on the frontend without re-scanning tags).
    pub channel_id: Option<String>,
}

/// Decrypt a `kind:1059` gift wrap addressed to us and return the inner message
/// rumor as a `RelayEvent`-shaped object. Errors if the wrap isn't addressed to
/// our identity or can't be decrypted.
#[tauri::command]
pub async fn decrypt_gift_wrap(
    event_json: String,
    state: State<'_, AppState>,
) -> Result<DecryptedEvent, String> {
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };

    let wrap = Event::from_json(&event_json).map_err(|e| format!("invalid event JSON: {e}"))?;
    let unwrapped = crate::encrypted::unwrap_gift(&keys, &wrap).await?;

    let channel_id = unwrapped.channel_id();
    let rumor = &unwrapped.rumor;
    let tags: Vec<Vec<String>> = rumor.tags.iter().map(|t| t.as_slice().to_vec()).collect();

    Ok(DecryptedEvent {
        id: rumor.id.map(|id| id.to_hex()).unwrap_or_default(),
        pubkey: unwrapped.sender.to_hex(),
        created_at: rumor.created_at.as_secs() as i64,
        kind: rumor.kind.as_u16(),
        tags,
        content: rumor.content.clone(),
        sig: String::new(),
        channel_id,
    })
}
