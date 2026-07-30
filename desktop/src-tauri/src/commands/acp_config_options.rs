//! Reading ACP `configOptions` entries across spec revisions.
//!
//! The current ACP spec spells a config option's identity and label `id` and
//! `name`. Earlier adapters shipped `configId` and `displayName` (claude-agent-acp
//! still does), and both spellings are in the wild, so every reader has to accept
//! either. The lookups live here rather than being re-derived per call site: a
//! reader that checks only one spelling silently drops the field, which is how a
//! spec-current adapter such as letta-acp ends up rendering raw model ids — or,
//! when the id itself is missed, no models at all.

use serde_json::Value;

/// A config option's id, under either spelling.
pub(crate) fn option_id(option: &Value) -> Option<&str> {
    option
        .get("id")
        .or_else(|| option.get("configId"))
        .and_then(Value::as_str)
}

/// The human-readable label of a config option or of one of its option values,
/// under either spelling.
pub(crate) fn option_label(option: &Value) -> Option<&str> {
    option
        .get("displayName")
        .or_else(|| option.get("name"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_current_spec_spelling() {
        let option = json!({ "id": "model", "name": "Model" });
        assert_eq!(option_id(&option), Some("model"));
        assert_eq!(option_label(&option), Some("Model"));
    }

    #[test]
    fn reads_the_legacy_spelling() {
        let option = json!({ "configId": "model", "displayName": "Model" });
        assert_eq!(option_id(&option), Some("model"));
        assert_eq!(option_label(&option), Some("Model"));
    }

    #[test]
    fn prefers_the_legacy_label_when_an_adapter_sends_both() {
        // Adapters that emit both spell the richer label `displayName`; `name`
        // is then usually the bare id, so the legacy key wins for labels.
        let option = json!({ "id": "model", "displayName": "Model", "name": "model" });
        assert_eq!(option_label(&option), Some("Model"));
    }

    #[test]
    fn missing_and_non_string_fields_are_none() {
        assert_eq!(option_id(&json!({})), None);
        assert_eq!(option_label(&json!({})), None);
        assert_eq!(option_id(&json!({ "id": 7 })), None);
        assert_eq!(option_label(&json!({ "name": ["Model"] })), None);
    }
}
