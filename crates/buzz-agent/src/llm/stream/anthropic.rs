use super::error;
use crate::types::{AgentError, ProviderStop};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Anthropic {
    message: Value,
    blocks: BTreeMap<u64, Value>,
    inputs: BTreeMap<u64, String>,
    done: bool,
}

impl Anthropic {
    pub(super) fn is_done(&self) -> bool {
        self.done
    }
    pub(super) fn event(&mut self, data: &str) -> Result<Option<String>, AgentError> {
        let v: Value = serde_json::from_str(data).map_err(error)?;
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if !self.message.is_null() {
                    return Err(error("duplicate message start"));
                }
                self.message = v
                    .get("message")
                    .filter(|v| v.is_object())
                    .cloned()
                    .ok_or_else(|| error("missing message"))?;
                if self
                    .message
                    .get("usage")
                    .is_some_and(|v| !v.is_object() && !v.is_null())
                {
                    return Err(error("invalid initial usage"));
                }
            }
            Some("content_block_start") => {
                if !self.message.is_object() {
                    return Err(error("block before message start"));
                }
                let i = index(&v)?;
                let block = v
                    .get("content_block")
                    .filter(|v| v.is_object())
                    .cloned()
                    .ok_or_else(|| error("missing content block"))?;
                if self.blocks.contains_key(&i) {
                    return Err(error("duplicate content block"));
                }
                let text = block.get("text").and_then(Value::as_str).map(str::to_owned);
                self.blocks.insert(i, block);
                return Ok(text);
            }
            Some("content_block_delta") => {
                let i = index(&v)?;
                let delta = v.get("delta").ok_or_else(|| error("missing block delta"))?;
                let block = self
                    .blocks
                    .get_mut(&i)
                    .ok_or_else(|| error("delta before block start"))?;
                let field = match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => "text",
                    Some("thinking_delta") => "thinking",
                    Some("signature_delta") => "signature",
                    Some("input_json_delta") => {
                        self.inputs.entry(i).or_default().push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .ok_or_else(|| error("missing partial JSON"))?,
                        );
                        return Ok(None);
                    }
                    Some("citations_delta") => {
                        if block.get("citations").is_none() {
                            block["citations"] = json!([]);
                        }
                        if let Some(citations) = block["citations"].as_array_mut() {
                            citations.push(delta["citation"].clone());
                        }
                        return Ok(None);
                    }
                    _ => return Ok(None),
                };
                let part = delta
                    .get(field)
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("missing delta text"))?;
                if block.get(field).is_none() {
                    block[field] = Value::String(String::new());
                }
                block[field]
                    .as_str()
                    .ok_or_else(|| error("invalid block text"))?;
                if let Some(Value::String(text)) = block.get_mut(field) {
                    text.push_str(part);
                }
                if field == "text" {
                    return Ok(Some(part.to_owned()));
                }
            }
            Some("message_delta") => {
                if !self.message.is_object() {
                    return Err(error("delta before message start"));
                }
                if let Some(delta) = v.get("delta").and_then(Value::as_object) {
                    for (k, value) in delta {
                        self.message[k] = value.clone();
                    }
                }
                if self
                    .message
                    .get("usage")
                    .is_some_and(|v| !v.is_object() && !v.is_null())
                {
                    return Err(error("invalid updated usage"));
                }
                if let Some(usage) = v.get("usage").and_then(Value::as_object) {
                    for (k, value) in usage {
                        self.message["usage"][k] = value.clone();
                    }
                }
            }
            Some("message_stop") => self.done = true,
            Some("error") => return Err(error(v.get("error").unwrap_or(&v))),
            _ => {}
        }
        Ok(None)
    }

    pub(super) fn finish(mut self) -> Result<Value, AgentError> {
        if !self.done
            || self
                .message
                .get("stop_reason")
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(error("Anthropic response ended before terminal event"));
        }
        let stop = super::super::map_stop(self.message["stop_reason"].as_str());
        if matches!(stop, ProviderStop::Refusal | ProviderStop::Other)
            && self
                .blocks
                .values()
                .any(|block| block["type"] == "tool_use")
        {
            return Err(error("tool calls in an unsuccessful Anthropic response"));
        }
        if stop != ProviderStop::MaxTokens {
            for (index, input) in self.inputs {
                let block = self
                    .blocks
                    .get_mut(&index)
                    .ok_or_else(|| error("missing tool block"))?;
                block["input"] = serde_json::from_str(&input).map_err(error)?;
            }
        }
        self.message["content"] = Value::Array(self.blocks.into_values().collect());
        Ok(self.message)
    }
}

fn index(v: &Value) -> Result<u64, AgentError> {
    v.get("index")
        .and_then(Value::as_u64)
        .ok_or_else(|| error("block missing index"))
}
