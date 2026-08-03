//! Parsing for the raw `session/new` payload a running harness reports via the
//! `session_config_captured` observer frame.
//!
//! Lives beside the `Acp*` types it produces rather than in the command layer:
//! these are pure shape-normalizers over untrusted JSON, with no Tauri or app
//! state involvement, and every adapter disagrees slightly about key names
//! (`id` vs `configId`, `value` vs `currentValue`, object vs array `models`).

use super::types::{AcpConfigOptionEntry, AcpConfigOptionValue, AcpModelEntry};

pub(crate) fn parse_config_options(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionEntry> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|opt| {
            let config_id = opt
                .get("id")
                .or_else(|| opt.get("configId"))?
                .as_str()?
                .to_string();
            Some(AcpConfigOptionEntry {
                config_id,
                category: opt
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                display_name: opt
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                current_value: opt
                    .get("value")
                    .or_else(|| opt.get("currentValue"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                options: parse_option_values(opt.get("options")),
            })
        })
        .collect()
}

fn parse_option_values(raw: Option<&serde_json::Value>) -> Vec<AcpConfigOptionValue> {
    let arr = match raw.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|o| {
            let value = o.get("value").and_then(|v| v.as_str())?.to_string();
            Some(AcpConfigOptionValue {
                value,
                display_name: o
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

pub(crate) fn parse_modes(
    config_options: &[AcpConfigOptionEntry],
    raw: Option<&serde_json::Value>,
) -> Vec<String> {
    if let Some(arr) = raw.and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|m| m.as_str().map(str::to_string))
            .collect();
    }
    // Fall back: extract mode options from configOptions with category "mode".
    config_options
        .iter()
        .filter(|o| o.category.as_deref() == Some("mode"))
        .flat_map(|o| o.options.iter().map(|v| v.value.clone()))
        .collect()
}

pub(crate) fn parse_models(
    raw: Option<&serde_json::Value>,
) -> (Vec<AcpModelEntry>, Option<String>) {
    let raw = match raw {
        Some(v) => v,
        None => return (Vec::new(), None),
    };

    // Object shape: { currentModelId, availableModels: [...] }
    if let Some(obj) = raw.as_object() {
        let current_model = obj
            .get("currentModelId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let models = obj
            .get("availableModels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let model_id = m
                            .get("modelId")
                            .or_else(|| m.get("id"))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        Some(AcpModelEntry {
                            model_id,
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        return (models, current_model);
    }

    // Array shape: [{ modelId, isCurrent, ... }]
    let arr = match raw.as_array() {
        Some(a) => a,
        None => return (Vec::new(), None),
    };
    let mut current_model = None;
    let models = arr
        .iter()
        .filter_map(|m| {
            let model_id = m
                .get("modelId")
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())?
                .to_string();
            if m.get("isCurrent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                current_model = Some(model_id.clone());
            }
            Some(AcpModelEntry {
                model_id,
                name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                description: m
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect();
    (models, current_model)
}
