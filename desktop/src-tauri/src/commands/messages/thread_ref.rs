use nostr::EventId;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay_at, query_relay_at_with_keys},
};

/// Fetch a parent event and extract the thread root from its NIP-10 e-tags.
///
/// Reads through the explicit `api_base_url` the calling command resolved —
/// never re-resolving the workspace override — so a mid-command community
/// switch cannot split one logical send across two relays. Callers that
/// pinned a signer snapshot pass it as `keys` so this read's NIP-98 auth is
/// minted by the same identity that signs the eventual event; `None`
/// preserves the active-identity read for unpinned callers.
pub(super) async fn resolve_thread_ref(
    parent_event_id: &str,
    state: &AppState,
    api_base_url: &str,
    keys: Option<&nostr::Keys>,
) -> Result<events::ThreadRef, String> {
    let parent_eid =
        EventId::from_hex(parent_event_id).map_err(|e| format!("invalid parent event ID: {e}"))?;

    let filters = [serde_json::json!({
        "ids": [parent_event_id],
        "kinds": [9, 40002, 45001, 45003, buzz_core_pkg::kind::KIND_HUDDLE_STARTED],
        "limit": 1
    })];
    let evs = match keys {
        Some(keys) => query_relay_at_with_keys(state, api_base_url, &filters, keys, None).await?,
        None => query_relay_at(state, api_base_url, &filters).await?,
    };

    let parent = evs
        .first()
        .ok_or_else(|| "parent event not found".to_string())?;

    // Walk tags looking for NIP-10 root/reply markers.
    let (mut root, mut reply) = (None, None);
    for tag in parent.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 4 && s[0] == "e" {
            match s[3].as_str() {
                "root" => root = Some(s[1].clone()),
                "reply" => reply = Some(s[1].clone()),
                _ => {}
            }
        }
    }
    let root_hex = root.or(reply);

    let root_eid = match root_hex {
        Some(hex) if hex != parent_event_id => {
            EventId::from_hex(&hex).map_err(|e| format!("invalid root event ID: {e}"))?
        }
        _ => parent_eid,
    };

    Ok(events::ThreadRef {
        root_event_id: root_eid,
        parent_event_id: parent_eid,
    })
}
