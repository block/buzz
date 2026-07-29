use tauri::State;

use crate::{app_state::AppState, models::SearchResponse, nostr_convert, relay::query_relay};

fn build_search_messages_filter(
    q: &str,
    cap: u32,
    channel_id: Option<&str>,
    authors: Option<&[String]>,
    since: Option<i64>,
    until: Option<i64>,
) -> serde_json::Value {
    let mut filter = serde_json::Map::new();
    filter.insert(
        "kinds".to_string(),
        serde_json::json!([
            9,
            40002,
            buzz_core_pkg::kind::KIND_STREAM_MESSAGE_FORWARD,
            45001,
            45003
        ]),
    );
    filter.insert("search".to_string(), serde_json::json!(q.trim()));
    // The desktop topbar is a typeahead surface. This bridge-only extension is
    // consumed before nostr::Filter parsing on the relay, so general WS/NIP-50
    // search remains word/lexeme-based.
    filter.insert("search_mode".to_string(), serde_json::json!("prefix"));
    filter.insert("limit".to_string(), serde_json::json!(cap));
    if let Some(cid) = channel_id {
        filter.insert("#h".to_string(), serde_json::json!([cid]));
    }
    // Optional operators from the desktop search parser (#2853). The relay
    // already maps authors/since/until onto FTS; search remains never the
    // access boundary (hits are refetched and re-authorized).
    if let Some(authors) = authors {
        let cleaned: Vec<&str> = authors
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .collect();
        if !cleaned.is_empty() {
            filter.insert("authors".to_string(), serde_json::json!(cleaned));
        }
    }
    if let Some(since) = since {
        filter.insert("since".to_string(), serde_json::json!(since));
    }
    if let Some(until) = until {
        filter.insert("until".to_string(), serde_json::json!(until));
    }
    serde_json::Value::Object(filter)
}

#[tauri::command]
pub async fn search_messages(
    q: String,
    limit: Option<u32>,
    channel_id: Option<String>,
    authors: Option<Vec<String>>,
    since: Option<i64>,
    until: Option<i64>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let cap = limit.unwrap_or(20).min(100);
    let filter = build_search_messages_filter(
        &q,
        cap,
        channel_id.as_deref(),
        authors.as_deref(),
        since,
        until,
    );

    let events = query_relay(&state, &[filter]).await?;
    Ok(nostr_convert::search_response_from_events(&events))
}

#[cfg(test)]
#[path = "messages_search_tests.rs"]
mod tests;
