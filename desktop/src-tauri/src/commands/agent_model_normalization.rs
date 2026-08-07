use std::collections::HashSet;

use crate::managed_agents::{AgentModelInfo, AgentModelsResponse};

fn effort_qualified_base_model_id(model_id: &str) -> Option<&str> {
    let (base_model_id, effort) = model_id.rsplit_once('[')?;
    let effort = effort.strip_suffix(']')?;
    (!base_model_id.is_empty() && !effort.is_empty()).then_some(base_model_id)
}

/// Normalize raw `buzz-acp models --json` output into a typed DTO for the frontend.
///
/// Merges models from both ACP paths (stable configOptions + unstable SessionModelState),
/// deduplicates equivalent entries, and returns a unified list.
///
/// Some agents expose a base model through stable config options while retaining
/// effort-qualified IDs such as `model[high]` in the legacy model state. Buzz's
/// model setting is currently one-dimensional for those agents, so the qualified
/// IDs are the actionable choices. Suppress the redundant base entry when those
/// richer variants are present.
pub(super) fn normalize_agent_models(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
) -> AgentModelsResponse {
    let agent_name = raw["agent"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let agent_version = raw["agent"]["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut models: Vec<AgentModelInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let effort_qualified_base_ids: HashSet<&str> = raw["unstable"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("modelId").and_then(|value| value.as_str()))
        .filter_map(effort_qualified_base_model_id)
        .collect();

    // 1. Stable configOptions (preferred). Only entries with category "model"
    //    are model options — the CLI pre-filters, but we're defensive here.
    if let Some(config_options) = raw["stable"]["configOptions"].as_array() {
        for opt in config_options {
            if opt.get("category").and_then(|c| c.as_str()) != Some("model") {
                continue;
            }
            if let Some(options) = opt.get("options").and_then(|v| v.as_array()) {
                for o in options {
                    if let Some(value) = o.get("value").and_then(|v| v.as_str()) {
                        if effort_qualified_base_ids.contains(value) {
                            continue;
                        }
                        if seen_ids.insert(value.to_string()) {
                            models.push(AgentModelInfo {
                                id: value.to_string(),
                                name: o
                                    .get("displayName")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                description: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. Unstable availableModels (fallback — skip duplicates from stable).
    let mut agent_default_model: Option<String> = None;
    if let Some(unstable) = raw.get("unstable") {
        agent_default_model = unstable["currentModelId"].as_str().map(str::to_string);
        if let Some(available) = unstable["availableModels"].as_array() {
            for m in available {
                if let Some(id) = m.get("modelId").and_then(|v| v.as_str()) {
                    if seen_ids.insert(id.to_string()) {
                        models.push(AgentModelInfo {
                            id: id.to_string(),
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        });
                    }
                }
            }
        }
    }

    let supports_switching = !models.is_empty();

    AgentModelsResponse {
        agent_name,
        agent_version,
        models,
        agent_default_model,
        selected_model: persisted_model,
        supports_switching,
    }
}
