use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

const MAX_REPAIR_PAGE_LIMIT: u32 = 500;
const CHANNEL_REPAIR_KINDS: [u32; 15] = [
    5, 7, 9, 9005, 40001, 40002, 40003, 40008, 40099, 45001, 45003, 48100, 48101, 48102, 48103,
];

fn build_channel_reconnect_repair_filter(
    channel_id: &str,
    since: u64,
    limit: u32,
    until: Option<u64>,
    before_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    uuid::Uuid::parse_str(channel_id).map_err(|_| "invalid channel id".to_string())?;
    if limit == 0 || limit > MAX_REPAIR_PAGE_LIMIT {
        return Err(format!(
            "limit must be between 1 and {MAX_REPAIR_PAGE_LIMIT}"
        ));
    }
    if before_id.is_some() && until.is_none() {
        return Err("before_id requires until".to_string());
    }
    if let Some(event_id) = before_id {
        if event_id.len() != 64 || !event_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("before_id must be a 64-character hex event id".to_string());
        }
    }

    let mut filter = serde_json::Map::new();
    filter.insert("#h".to_string(), serde_json::json!([channel_id]));
    filter.insert("kinds".to_string(), serde_json::json!(CHANNEL_REPAIR_KINDS));
    filter.insert("since".to_string(), serde_json::json!(since));
    filter.insert("limit".to_string(), serde_json::json!(limit));
    if let Some(value) = until {
        filter.insert("until".to_string(), serde_json::json!(value));
    }
    if let Some(value) = before_id {
        filter.insert("before_id".to_string(), serde_json::json!(value));
    }
    Ok(serde_json::Value::Object(filter))
}

/// Fetch one lossless keyset page for reconnect repair using a fixed channel-event filter.
#[tauri::command]
pub async fn get_channel_reconnect_repair(
    channel_id: String,
    since: u64,
    limit: u32,
    until: Option<u64>,
    before_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let filter = build_channel_reconnect_repair_filter(
        &channel_id,
        since,
        limit,
        until,
        before_id.as_deref(),
    )?;
    Ok(query_relay(&state, &[filter])
        .await?
        .iter()
        .filter_map(|event| serde_json::to_value(event).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_filter_is_fixed_and_keyset_scoped() {
        let id = "ab".repeat(32);
        let filter = build_channel_reconnect_repair_filter(
            "270f6caf-0feb-4055-93f3-cdbeb567ff28",
            100,
            500,
            Some(200),
            Some(&id),
        )
        .expect("valid filter");
        assert_eq!(
            filter["#h"],
            serde_json::json!(["270f6caf-0feb-4055-93f3-cdbeb567ff28"])
        );
        assert_eq!(filter["kinds"], serde_json::json!(CHANNEL_REPAIR_KINDS));
        assert_eq!(filter["since"], 100);
        assert_eq!(filter["limit"], 500);
        assert_eq!(filter["until"], 200);
        assert_eq!(filter["before_id"], id);
        assert!(filter.get("top_level").is_none());
        assert!(filter.get("include_summaries").is_none());
        assert!(filter.get("include_aux").is_none());
    }

    #[test]
    fn repair_filter_rejects_renderer_escape_hatches() {
        assert!(build_channel_reconnect_repair_filter("not-a-channel", 0, 1, None, None).is_err());
        assert!(build_channel_reconnect_repair_filter(
            "270f6caf-0feb-4055-93f3-cdbeb567ff28",
            0,
            0,
            None,
            None
        )
        .is_err());
        assert!(build_channel_reconnect_repair_filter(
            "270f6caf-0feb-4055-93f3-cdbeb567ff28",
            0,
            1,
            None,
            Some("bad")
        )
        .is_err());
    }
}
