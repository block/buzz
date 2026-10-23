//! Unit tests for harness provider-inventory normalization.
//!
//! These bind the production parse path (`normalize_agent_providers`), so
//! removing a guard here (e.g. the blank-id filter) fails the suite.

use super::normalize_agent_providers;
use serde_json::json;

#[test]
fn maps_goose_entries_into_provider_rows() {
    let raw = json!({
        "agent": { "name": "goose", "version": "1.51.0" },
        "providers": [
            {
                "id": "aws_bedrock",
                "name": "Amazon Bedrock",
                "configured": true,
                "defaultModel": "global.anthropic.claude-sonnet-5",
                "acp": false
            },
            {
                "id": "ollama",
                "name": "Ollama",
                "configured": false,
                "defaultModel": "qwen3",
                "acp": false
            }
        ]
    });

    let response = normalize_agent_providers(&raw);

    assert_eq!(response.agent_name, "goose");
    assert_eq!(response.agent_version, "1.51.0");
    assert_eq!(response.providers.len(), 2);
    let first = &response.providers[0];
    assert_eq!(first.id, "aws_bedrock");
    assert_eq!(first.name.as_deref(), Some("Amazon Bedrock"));
    assert!(first.configured);
    assert_eq!(
        first.default_model.as_deref(),
        Some("global.anthropic.claude-sonnet-5")
    );
    assert!(!first.acp);
    assert!(!response.providers[1].configured);
}

#[test]
fn drops_entries_without_a_usable_id() {
    // A provider row is only selectable by id; a blank/missing id would render
    // as a nameless option that persists an empty GOOSE_PROVIDER.
    let raw = json!({
        "agent": { "name": "goose", "version": "1.51.0" },
        "providers": [
            { "name": "No id" },
            { "id": "   ", "name": "Whitespace id" },
            { "id": "ollama", "name": "Ollama", "acp": true }
        ]
    });

    let response = normalize_agent_providers(&raw);

    assert_eq!(response.providers.len(), 1);
    assert_eq!(response.providers[0].id, "ollama");
    assert!(response.providers[0].acp);
}

#[test]
fn tolerates_a_harness_without_an_inventory() {
    // Non-goose adapters answer no inventory; the response must still be a
    // valid shape so the caller can fall back to the built-in catalog.
    let raw = json!({
        "agent": { "name": "claude-agent-acp", "version": "0.1.0" }
    });

    let response = normalize_agent_providers(&raw);

    assert_eq!(response.agent_name, "claude-agent-acp");
    assert!(response.providers.is_empty());
}
