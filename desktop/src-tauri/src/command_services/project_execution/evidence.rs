use serde_json::{json, Value};
use tauri::Manager;

use crate::command_services::trusted_lan::{load_optional, TrustedLanConfig};

const MAXIMUM_EVIDENCE_CHARACTERS: usize = 24_000;

fn bounded(value: Value) -> Value {
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
    if text.chars().count() <= MAXIMUM_EVIDENCE_CHARACTERS {
        value
    } else {
        Value::String(text.chars().take(MAXIMUM_EVIDENCE_CHARACTERS).collect())
    }
}

pub(crate) fn collect(app: &tauri::AppHandle, query: &str, planning_context: Value) -> Value {
    let config = app
        .path()
        .app_config_dir()
        .ok()
        .map(|path| path.join("trusted-lan-sources.json"))
        .and_then(|path| load_optional(&path).ok().flatten());
    collect_with_config(config.as_ref(), query, planning_context)
}

fn collect_with_config(
    config: Option<&TrustedLanConfig>,
    query: &str,
    planning_context: Value,
) -> Value {
    let Some(client) = config.and_then(|value| value.source_client().ok()) else {
        return json!({
            "planningContext": planning_context,
            "rag": null,
            "memory": null,
            "limitations": ["Trusted LAN RAG and Memory are not configured."]
        });
    };
    let rag = client.search_rag(query, &[]).ok().map(bounded);
    let memory = client.search_memory(query, 5).ok().map(bounded);
    let mut limitations = Vec::new();
    if rag.is_none() {
        limitations.push("RAG was unavailable; execution continued with other evidence.");
    }
    if memory.is_none() {
        limitations.push("Memory was unavailable; execution continued with other evidence.");
    }
    json!({
        "planningContext": planning_context,
        "rag": rag,
        "memory": memory,
        "limitations": limitations
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_source_config_is_visible_but_does_not_block_context() {
        let result = collect_with_config(None, "sailing", json!({"task": "Prepare"}));
        assert_eq!(result["planningContext"]["task"], "Prepare");
        assert!(result["limitations"][0]
            .as_str()
            .is_some_and(|value| value.contains("not configured")));
    }
}
