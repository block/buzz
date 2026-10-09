use buzz_core::nip10::ThreadMarkers;
use serde_json::Value;

use crate::OutputFormat;

/// Parse NIP-10 `root`/`reply` markers from a JSON tag array via
/// [`buzz_core::nip10`], the rule shared with relay ingest and ACP.
pub(crate) fn thread_markers(tags: &Value) -> ThreadMarkers {
    let parts: Vec<Vec<&str>> = tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            tag.as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap_or("")).collect())
        })
        .collect();
    buzz_core::nip10::parse_thread_markers_from_parts(parts.iter().map(Vec::as_slice))
}

/// Render normalized event JSON (from `normalize_events`) in the requested format.
pub(crate) fn format_events(normalized: &str, format: &OutputFormat) -> String {
    let project: fn(&Value) -> Value = match format {
        OutputFormat::Json => return normalized.to_string(),
        OutputFormat::Compact => compact_event,
        OutputFormat::Agent => agent_event,
    };
    let events: Vec<Value> = serde_json::from_str(normalized).unwrap_or_default();
    let projected: Vec<Value> = events.iter().map(project).collect();
    serde_json::to_string(&projected).unwrap_or_default()
}

fn field(event: &Value, key: &str) -> Value {
    event.get(key).cloned().unwrap_or_default()
}

fn compact_event(event: &Value) -> Value {
    serde_json::json!({
        "id": field(event, "id"),
        "content": field(event, "content"),
        "created_at": field(event, "created_at"),
    })
}

fn agent_event(event: &Value) -> Value {
    let mut projected = serde_json::json!({
        "id": field(event, "id"),
        "pubkey": field(event, "pubkey"),
        "kind": field(event, "kind"),
        "created_at": field(event, "created_at"),
        "content": field(event, "content"),
    });
    let tags = event.get("tags").unwrap_or(&Value::Null);
    if let Some((_, parent)) = thread_markers(tags).resolve() {
        projected["reply_to"] = Value::String(parent);
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn signed_reply() -> Value {
        json!({
            "id": "a".repeat(64),
            "pubkey": "b".repeat(64),
            "kind": 9,
            "content": "reply content",
            "created_at": 1_787_754_972_u64,
            "tags": [
                ["h", "6f1c7c1e-1d5b-4c55-9d43-0c1f0b6a2f10"],
                ["e", "c".repeat(64), "", "root"],
                ["e", "d".repeat(64), "", "reply"],
                ["p", "e".repeat(64)],
            ],
            "sig": "f".repeat(128),
        })
    }

    fn render(events: Value, format: &OutputFormat) -> Vec<Value> {
        serde_json::from_str(&format_events(&events.to_string(), format)).unwrap()
    }

    #[test]
    fn compact_event_format_remains_the_three_key_contract() {
        assert_eq!(
            render(json!([signed_reply()]), &OutputFormat::Compact)[0],
            json!({
                "id": "a".repeat(64),
                "content": "reply content",
                "created_at": 1_787_754_972_u64,
            })
        );
    }

    #[test]
    fn agent_format_keeps_author_and_reply_target_but_drops_signature_and_tags() {
        assert_eq!(
            render(json!([signed_reply()]), &OutputFormat::Agent)[0],
            json!({
                "id": "a".repeat(64),
                "pubkey": "b".repeat(64),
                "kind": 9,
                "created_at": 1_787_754_972_u64,
                "content": "reply content",
                "reply_to": "d".repeat(64),
            })
        );
    }

    #[test]
    fn agent_format_omits_reply_to_for_top_level_messages() {
        let mut top_level = signed_reply();
        top_level["tags"] = json!([["h", "6f1c7c1e-1d5b-4c55-9d43-0c1f0b6a2f10"]]);
        assert!(render(json!([top_level]), &OutputFormat::Agent)[0]
            .get("reply_to")
            .is_none());
    }

    #[test]
    fn json_format_is_passthrough() {
        let normalized = json!([signed_reply()]).to_string();
        assert_eq!(format_events(&normalized, &OutputFormat::Json), normalized);
    }
}
