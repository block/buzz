use super::error;
use crate::types::{AgentError, ProviderStop};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Chat {
    message: Value,
    calls: BTreeMap<usize, Value>,
    usage: Option<Value>,
    finish_reason: Option<Value>,
    done: bool,
}

impl Chat {
    pub(super) fn is_done(&self) -> bool {
        self.done
    }
    pub(super) fn event(&mut self, data: &str) -> Result<Option<String>, AgentError> {
        if self.done {
            return Err(error("event after chat completion"));
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(None);
        }
        let v: Value = serde_json::from_str(data).map_err(error)?;
        if let Some(err) = v.get("error") {
            return Err(error(err));
        }
        if let Some(usage) = v.get("usage").filter(|v| !v.is_null()) {
            self.usage = Some(usage.clone());
        }
        let Some(choices) = v.get("choices").and_then(Value::as_array) else {
            return Err(error("chat event missing choices"));
        };
        let Some(choice) = choices
            .iter()
            .find(|v| v.get("index").and_then(Value::as_u64).unwrap_or(0) == 0)
        else {
            return Ok(None); // A trailing usage-only chunk has no choices.
        };
        if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
            self.finish_reason = Some(reason.clone());
        }
        if choice.get("finish_reason").and_then(Value::as_str) == Some("error") {
            return Err(error(choice.get("error").unwrap_or(choice)));
        }
        let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
            return Ok(None);
        };
        let text = delta
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if self.message.is_null() {
            self.message = json!({"role":"assistant"});
        }
        for (key, value) in delta {
            if key == "tool_calls" {
                for call in value.as_array().into_iter().flatten() {
                    let index = call
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| error("tool call missing index"))?;
                    if index >= crate::config::MAX_TOOL_CALLS_PER_TURN {
                        return Err(error("too many streamed tool calls"));
                    }
                    merge(
                        self.calls.entry(index).or_insert_with(|| json!({})),
                        call,
                        "",
                    );
                }
            } else {
                merge(&mut self.message[key], value, key);
            }
        }
        Ok(text)
    }

    pub(super) fn finish(mut self) -> Result<Value, AgentError> {
        if !self.done || self.finish_reason.is_none() {
            return Err(error("chat response ended before terminal event"));
        }
        let stop = super::super::map_stop(self.finish_reason.as_ref().and_then(Value::as_str));
        if !self.calls.is_empty() && matches!(stop, ProviderStop::Refusal | ProviderStop::Other) {
            return Err(error("tool calls in an unsuccessful chat response"));
        }
        if !self.calls.is_empty() {
            self.message["tool_calls"] = Value::Array(
                self.calls
                    .into_values()
                    .map(|mut call| {
                        // `index` routes stream fragments; it is not part of an
                        // assistant tool call and must not become replay metadata.
                        if let Some(call) = call.as_object_mut() {
                            call.remove("index");
                        }
                        call
                    })
                    .collect(),
            );
        }
        Ok(
            json!({"choices":[{"message":self.message,"finish_reason":self.finish_reason}],"usage":self.usage}),
        )
    }
}

/// Append textual deltas while retaining opaque provider metadata for replay.
fn merge(out: &mut Value, delta: &Value, field: &str) {
    if delta.is_null() {
        return;
    }
    match (out, delta) {
        (Value::Object(out), Value::Object(delta)) => {
            for (key, value) in delta {
                merge(out.entry(key).or_insert(Value::Null), value, key);
            }
        }
        (Value::String(out), Value::String(delta))
            if matches!(
                field,
                "content"
                    | "reasoning"
                    | "reasoning_content"
                    | "arguments"
                    | "text"
                    | "summary"
                    | "data"
                    | "signature"
                    | "name"
            ) =>
        {
            out.push_str(delta)
        }
        (Value::Array(out), Value::Array(delta))
            if matches!(field, "reasoning_details" | "content") =>
        {
            for item in delta {
                let index = item.get("index").and_then(Value::as_u64);
                let id = item.get("id").and_then(Value::as_str);
                if let Some(existing) = out.iter_mut().find(|v| {
                    index.is_some_and(|i| v.get("index").and_then(Value::as_u64) == Some(i))
                        || id.is_some_and(|id| v.get("id").and_then(Value::as_str) == Some(id))
                }) {
                    merge(existing, item, "");
                } else {
                    out.push(item.clone());
                }
            }
        }
        (out, delta) => *out = delta.clone(),
    }
}
