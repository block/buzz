use super::error;
use crate::types::AgentError;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Responses {
    items: BTreeMap<u64, Value>,
    response: Option<Value>,
}

impl Responses {
    pub(super) fn is_done(&self) -> bool {
        self.response.is_some()
    }
    pub(super) fn event(&mut self, data: &str) -> Result<Option<String>, AgentError> {
        let v: Value = serde_json::from_str(data).map_err(error)?;
        match v.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                Ok(v.get("delta").and_then(Value::as_str).map(str::to_owned))
            }
            Some("response.output_item.done") => {
                let index = v
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| error("output item missing index"))?;
                let item = v
                    .get("item")
                    .ok_or_else(|| error("output item missing item"))?;
                self.items.insert(index, item.clone());
                Ok(None)
            }
            Some("response.completed" | "response.incomplete") => {
                let mut response = v
                    .get("response")
                    .cloned()
                    .ok_or_else(|| error("missing terminal response"))?;
                if !matches!(
                    (
                        v.get("type").and_then(Value::as_str),
                        response.get("status").and_then(Value::as_str)
                    ),
                    (Some("response.completed"), Some("completed"))
                        | (Some("response.incomplete"), Some("incomplete"))
                ) {
                    return Err(error("invalid terminal response status"));
                }
                if response.get("output").is_none() {
                    response["output"] =
                        Value::Array(core::mem::take(&mut self.items).into_values().collect());
                }
                let truncated = response["status"] == "incomplete"
                    && response["incomplete_details"]["reason"] == "max_output_tokens";
                for item in response
                    .get("output")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if item["type"] == "function_call"
                        && !truncated
                        && (response["status"] != "completed"
                            || item.get("status").is_some_and(|s| s != "completed"))
                    {
                        return Err(error("unfinished tool call in terminal response"));
                    }
                }
                self.response = Some(response);
                Ok(None)
            }
            Some("error" | "response.failed") => Err(error(v.get("error").unwrap_or(&v))),
            _ => Ok(None),
        }
    }

    pub(super) fn finish(self) -> Result<Value, AgentError> {
        self.response
            .ok_or_else(|| error("response ended before terminal event"))
    }
}
