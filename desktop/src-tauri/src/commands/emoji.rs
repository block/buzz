use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

/// Kind of the per-emoji catalog entry (`KIND_EMOJI_ENTRY` in `buzz-core`).
const KIND_EMOJI_ENTRY: u32 = 30180;
const MAX_EMOJI_SEARCH_RESULTS: u32 = 50;

fn build_emoji_entry_search_filter(query: &str, limit: Option<u32>) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_EMOJI_ENTRY],
        "search": query.trim(),
        // `search_mode` is a bridge-only extension; the WebSocket path cannot carry it.
        "search_mode": "prefix",
        "limit": limit.unwrap_or(MAX_EMOJI_SEARCH_RESULTS).min(MAX_EMOJI_SEARCH_RESULTS),
    })
}

/// Prefix-search the relay's emoji catalog (kind 30180) over `POST /query`.
///
/// A blank query is an error rather than an empty result so callers cannot
/// mistake "not searched" for "no matches".
#[tauri::command]
pub async fn search_emoji_entries(
    query: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    if query.trim().is_empty() {
        return Err("emoji search query is empty".to_string());
    }
    let filter = build_emoji_entry_search_filter(&query, limit);
    let events = query_relay(&state, &[filter]).await?;
    Ok(events
        .iter()
        .filter_map(|event| serde_json::to_value(event).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_filter_is_prefix_mode_on_the_entry_kind_and_bounded() {
        let filter = build_emoji_entry_search_filter("  parr ", Some(500));
        assert_eq!(filter["kinds"], serde_json::json!([30180]));
        assert_eq!(filter["search"], serde_json::json!("parr"));
        assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
        assert_eq!(filter["limit"], serde_json::json!(50));
        assert_eq!(
            build_emoji_entry_search_filter("parr", None)["limit"],
            serde_json::json!(50)
        );
        assert_eq!(
            build_emoji_entry_search_filter("parr", Some(20))["limit"],
            serde_json::json!(20)
        );
    }
}
