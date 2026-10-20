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

/// Drops signature material (`sig`, and the NIP-OA `auth` conditions and
/// signature while keeping its owner pubkey) and folds the NIP-10 thread
/// markers into `reply_to`, plus `thread_root` for nested replies; every other
/// tag (channel, mentions, edit targets, diff provenance, ...) is kept verbatim.
///
/// Other agent read surfaces trim the same fields; change them together:
/// buzz-app's MCP (`src/buzz-mcp/tools.ts`, `Tools.messages`) and kgoose's
/// hosted read tools (squareup/cash-server `BuzzExtensionClient.messageJson`).
fn agent_event(event: &Value) -> Value {
    let raw_tags = event.get("tags").unwrap_or(&Value::Null);
    let thread = thread_markers(raw_tags).resolve();
    let kept: Vec<Value> = raw_tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            let name = tag.get(0).and_then(Value::as_str);
            let marker = tag.get(3).and_then(Value::as_str);
            match (name, marker) {
                (Some("auth"), _) => Some(serde_json::json!([
                    "auth",
                    tag.get(1).cloned().unwrap_or_default()
                ])),
                (Some("e"), Some("root" | "reply")) if thread.is_some() => None,
                _ => Some(tag.clone()),
            }
        })
        .collect();
    let mut projected = serde_json::json!({
        "id": field(event, "id"),
        "pubkey": field(event, "pubkey"),
        "kind": field(event, "kind"),
        "created_at": field(event, "created_at"),
        "content": field(event, "content"),
        "tags": kept,
    });
    if let Some((root, parent)) = thread {
        if root != parent {
            projected["thread_root"] = Value::String(root);
        }
        projected["reply_to"] = Value::String(parent);
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHANNEL: &str = "6f1c7c1e-1d5b-4c55-9d43-0c1f0b6a2f10";

    fn auth_tag() -> Value {
        json!(["auth", "9".repeat(64), "kind=9", "8".repeat(128)])
    }

    fn signed_reply() -> Value {
        json!({
            "id": "a".repeat(64),
            "pubkey": "b".repeat(64),
            "kind": 9,
            "content": "reply content",
            "created_at": 1_787_754_972_u64,
            "tags": [
                ["h", CHANNEL],
                ["e", "c".repeat(64), "", "root"],
                ["e", "d".repeat(64), "", "reply"],
                ["p", "e".repeat(64)],
                auth_tag(),
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
    fn agent_format_drops_signature_material_but_keeps_the_owner_and_thread() {
        assert_eq!(
            render(json!([signed_reply()]), &OutputFormat::Agent)[0],
            json!({
                "id": "a".repeat(64),
                "pubkey": "b".repeat(64),
                "kind": 9,
                "created_at": 1_787_754_972_u64,
                "content": "reply content",
                "thread_root": "c".repeat(64),
                "reply_to": "d".repeat(64),
                "tags": [["h", CHANNEL], ["p", "e".repeat(64)], ["auth", "9".repeat(64)]],
            })
        );
    }

    #[test]
    fn agent_format_omits_thread_root_for_a_direct_reply() {
        let mut direct = signed_reply();
        direct["tags"] = json!([["h", CHANNEL], ["e", "c".repeat(64), "", "reply"]]);
        let output = &render(json!([direct]), &OutputFormat::Agent)[0];
        assert_eq!(output["reply_to"], "c".repeat(64));
        assert!(output.get("thread_root").is_none());
        assert_eq!(output["tags"], json!([["h", CHANNEL]]));
    }

    #[test]
    fn agent_format_keeps_semantic_tags_such_as_edit_targets_and_diff_provenance() {
        let edit = json!({
            "id": "a".repeat(64),
            "pubkey": "b".repeat(64),
            "kind": 40003,
            "content": "edited",
            "created_at": 1_787_754_972_u64,
            "tags": [["h", CHANNEL], ["e", "c".repeat(64)], auth_tag()],
            "sig": "f".repeat(128),
        });
        let diff = json!({
            "id": "d".repeat(64),
            "pubkey": "b".repeat(64),
            "kind": 40008,
            "content": "diff --git a/x b/x",
            "created_at": 1_787_754_972_u64,
            "tags": [["h", CHANNEL], ["repo", "https://example.com/r.git"], ["commit", "abc123"]],
            "sig": "f".repeat(128),
        });

        let output = render(json!([edit, diff]), &OutputFormat::Agent);

        assert_eq!(
            output[0]["tags"],
            json!([
                ["h", CHANNEL],
                ["e", "c".repeat(64)],
                ["auth", "9".repeat(64)]
            ])
        );
        assert!(output[0].get("reply_to").is_none());
        assert_eq!(
            output[1]["tags"],
            json!([
                ["h", CHANNEL],
                ["repo", "https://example.com/r.git"],
                ["commit", "abc123"]
            ])
        );
    }

    #[test]
    fn json_format_is_passthrough() {
        let normalized = json!([signed_reply()]).to_string();
        assert_eq!(format_events(&normalized, &OutputFormat::Json), normalized);
    }
}
