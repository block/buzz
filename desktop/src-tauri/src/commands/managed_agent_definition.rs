//! Managed-agent definition validation at local mutation boundaries.

use crate::managed_agents::{CreateManagedAgentRequest, ManagedAgentRecord};

pub(super) fn validate_create_definition(
    name: &str,
    persona_id: Option<&str>,
    input: &CreateManagedAgentRequest,
) -> Result<(), String> {
    validate_definition_fields(name, persona_id, input.system_prompt.as_deref())
}

fn validate_definition_fields(
    name: &str,
    persona_id: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<(), String> {
    crate::managed_agents::validate_managed_agent_definition_text(name, persona_id, system_prompt)
        .map_err(|error| format!("Managed agent definition is unsafe: {error}"))
}

/// Apply definition-owned update fields, then validate the complete
/// prospective definition before the caller can persist it.
pub(super) fn apply_model_provider_prompt_update(
    record: &mut ManagedAgentRecord,
    model: Option<Option<String>>,
    provider: Option<Option<String>>,
    system_prompt: Option<Option<String>>,
) -> Result<(), String> {
    if record.persona_id.is_none() {
        if let Some(model_update) = model {
            record.model = model_update;
        }
        if let Some(provider_update) = provider {
            record.provider = provider_update;
        }
        if let Some(prompt_update) = system_prompt {
            record.system_prompt = prompt_update;
        }
    }

    validate_definition_fields(
        &record.name,
        record.persona_id.as_deref(),
        record.system_prompt.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standalone_record() -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": "standalone1",
            "name": "standalone-agent",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "safe prompt",
            "model": null,
            "provider": null,
            "env_vars": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("standalone agent record")
    }

    fn create_request(system_prompt: &str) -> CreateManagedAgentRequest {
        serde_json::from_value(serde_json::json!({
            "name": "Reviewer",
            "systemPrompt": system_prompt
        }))
        .expect("create request")
    }

    #[test]
    fn create_rejects_invisible_definition_less_name_or_prompt() {
        for (name, prompt, code) in [
            ("Review\u{200B}er", "Review code.", "U+200B"),
            ("Reviewer", "Review\u{202E} code.", "U+202E"),
        ] {
            let input = create_request(prompt);
            let error = validate_create_definition(name, None, &input)
                .expect_err("create must reject unsafe definition text");
            assert!(error.contains(code), "unexpected error: {error}");
        }
    }

    #[test]
    fn create_accepts_visible_multiline_definition_less_prompt() {
        let input = create_request("Review changes.\n\tCall out security risks.");
        validate_create_definition("Reviewer 🐝", None, &input)
            .expect("visible multiline instructions should remain valid");
    }

    #[test]
    fn update_rejects_invisible_definition_less_name_or_prompt() {
        let mut unsafe_prompt = standalone_record();
        let error = apply_model_provider_prompt_update(
            &mut unsafe_prompt,
            None,
            None,
            Some(Some("Review\u{200B} code.".to_string())),
        )
        .expect_err("definition-less prompt update must reject invisible text");
        assert!(error.contains("U+200B"), "unexpected error: {error}");

        let mut unsafe_name = standalone_record();
        unsafe_name.name = "Review\u{202E}er".to_string();
        let error = apply_model_provider_prompt_update(&mut unsafe_name, None, None, None)
            .expect_err("definition-less name update must reject formatting controls");
        assert!(error.contains("U+202E"), "unexpected error: {error}");
    }
}
