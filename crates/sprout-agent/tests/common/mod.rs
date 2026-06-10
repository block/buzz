//! Shared helpers for the `sprout-agent` integration tests. A `common/mod.rs`
//! module (rather than a top-level `common.rs`) keeps Cargo from treating this
//! file as its own test binary.

use serde_json::{json, Value};

/// Convert a canned OpenAI Chat Completions response into the SSE delta events
/// a streaming consumer would receive. Emits a content delta (when present),
/// two deltas per tool call (id+name, then arguments), and a final
/// finish-reason chunk. Usage is carried from the original response when
/// present, otherwise a default is supplied.
pub fn openai_to_sse_events(response: &Value) -> Vec<String> {
    let mut events = Vec::new();
    let choice = &response["choices"][0];
    let msg = &choice["message"];

    if let Some(content) = msg.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            events.push(
                json!({
                    "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": null}]
                })
                .to_string(),
            );
        }
    }

    if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
        for (i, tc) in tcs.iter().enumerate() {
            let id = tc.get("id").and_then(Value::as_str).unwrap_or("");
            let name = tc["function"]
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = tc["function"]
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            events.push(json!({
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{"index": i, "id": id, "function": {"name": name, "arguments": ""}}]
                }, "finish_reason": null}]
            }).to_string());
            events.push(
                json!({
                    "choices": [{"index": 0, "delta": {
                        "tool_calls": [{"index": i, "function": {"arguments": args}}]
                    }, "finish_reason": null}]
                })
                .to_string(),
            );
        }
    }

    let finish = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let mut final_event = json!({
        "choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
    });
    if let Some(usage) = response.get("usage") {
        final_event["usage"] = usage.clone();
    } else {
        final_event["usage"] = json!({"prompt_tokens": 10, "completion_tokens": 5});
    }
    events.push(final_event.to_string());

    events
}
