//! Optional Shepherd execution-evidence adapter.
//!
//! Shepherd remains an external execution producer. This module accepts its
//! flat JSON trace export and converts it into a small Buzz-owned envelope.
//! Raw effect payloads are deliberately not retained because they may contain
//! prompts, tool results, or file content.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct ShepherdExport {
    total_effects: usize,
    #[serde(default)]
    effect_types: Vec<String>,
    timeline: Vec<Value>,
}

/// A redacted Shepherd boundary event safe to join to Buzz's evidence plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShepherdEvidenceEvent {
    pub sequence: u64,
    pub scope_id: Option<String>,
    pub effect_type: String,
    pub phase: Option<String>,
    pub binding: Option<String>,
    pub path: Option<String>,
    pub operation_id: Option<String>,
    pub payload_sha256: String,
}

/// Buzz-owned representation of one imported Shepherd trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShepherdEvidenceEnvelope {
    pub schema: &'static str,
    pub source: &'static str,
    pub source_run_ref: Option<String>,
    pub coverage: &'static str,
    pub total_effects: usize,
    pub effect_types: Vec<String>,
    pub events: Vec<ShepherdEvidenceEvent>,
}

fn optional_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
    })
}

fn sequence(value: &Value, fallback: usize) -> Result<u64, String> {
    value
        .get("_sequence")
        .or_else(|| value.get("sequence"))
        .and_then(Value::as_u64)
        .or_else(|| u64::try_from(fallback).ok())
        .ok_or_else(|| "Shepherd trace sequence exceeds the supported range".to_string())
}

fn normalize_event(value: &Value, fallback: usize) -> Result<ShepherdEvidenceEvent, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("Shepherd timeline entry {fallback} must be an object"))?;
    let effect_type = optional_string(value, &["effect_type", "effectType", "kind"])
        .ok_or_else(|| format!("Shepherd timeline entry {fallback} is missing effect_type"))?;
    let canonical = serde_json::to_vec(object)
        .map_err(|error| format!("failed to hash Shepherd timeline entry {fallback}: {error}"))?;
    Ok(ShepherdEvidenceEvent {
        sequence: sequence(value, fallback)?,
        scope_id: optional_string(value, &["_scope_id", "scope_id", "scopeId"]),
        effect_type,
        phase: optional_string(value, &["phase"]),
        binding: optional_string(value, &["binding"]),
        path: optional_string(value, &["path"]),
        operation_id: optional_string(value, &["operation_id", "operationId"]),
        payload_sha256: hex::encode(Sha256::digest(canonical)),
    })
}

/// Convert a Shepherd flat JSON export into redacted Buzz evidence.
pub fn normalize_shepherd_export(
    export_json: &str,
    source_run_ref: Option<String>,
) -> Result<ShepherdEvidenceEnvelope, String> {
    if export_json.len() > MAX_EXPORT_BYTES {
        return Err("Shepherd trace export exceeds the 16 MiB import limit".to_string());
    }
    let export: ShepherdExport = serde_json::from_str(export_json)
        .map_err(|error| format!("invalid Shepherd trace export: {error}"))?;
    if export.total_effects != export.timeline.len() {
        return Err(format!(
            "Shepherd trace total_effects mismatch: declared {}, found {}",
            export.total_effects,
            export.timeline.len()
        ));
    }

    let events = export
        .timeline
        .iter()
        .enumerate()
        .map(|(index, value)| normalize_event(value, index + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen_sequences = BTreeSet::new();
    if events
        .iter()
        .any(|event| !seen_sequences.insert(event.sequence))
    {
        return Err("Shepherd trace contains duplicate event sequences".to_string());
    }

    let observed_types = events
        .iter()
        .map(|event| event.effect_type.clone())
        .collect::<BTreeSet<_>>();
    let declared_types = export.effect_types.into_iter().collect::<BTreeSet<_>>();
    if !declared_types.is_empty() && declared_types != observed_types {
        return Err("Shepherd trace effect_types do not match timeline events".to_string());
    }

    Ok(ShepherdEvidenceEnvelope {
        schema: "buzz.external-execution-evidence.v1",
        source: "shepherd",
        source_run_ref: source_run_ref.filter(|value| !value.trim().is_empty()),
        coverage: "boundary-effects-only",
        total_effects: events.len(),
        effect_types: observed_types.into_iter().collect(),
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_redacts_a_shepherd_export() {
        let input = serde_json::json!({
            "total_effects": 2,
            "effect_types": ["FilePatch", "task_started"],
            "timeline": [
                {"_sequence": 1, "_scope_id": "scope-1", "effect_type": "task_started", "prompt": "secret"},
                {"_sequence": 2, "_scope_id": "scope-1", "effect_type": "FilePatch", "phase": "proposed", "binding": "repo", "path": "src/lib.rs", "content": "private"}
            ]
        });
        let result = normalize_shepherd_export(&input.to_string(), Some("run-7".into()))
            .expect("normalize Shepherd export");

        assert_eq!(result.schema, "buzz.external-execution-evidence.v1");
        assert_eq!(result.source_run_ref.as_deref(), Some("run-7"));
        assert_eq!(result.effect_types, vec!["FilePatch", "task_started"]);
        assert_eq!(result.events[1].path.as_deref(), Some("src/lib.rs"));
        let serialized = serde_json::to_string(&result).expect("serialize result");
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("private"));
    }

    #[test]
    fn rejects_inconsistent_or_duplicate_traces() {
        let mismatch = r#"{"total_effects":2,"timeline":[]}"#;
        assert!(normalize_shepherd_export(mismatch, None).is_err());

        let duplicate = r#"{"total_effects":2,"timeline":[{"_sequence":1,"effect_type":"a"},{"_sequence":1,"effect_type":"b"}]}"#;
        assert!(normalize_shepherd_export(duplicate, None).is_err());
    }
}
