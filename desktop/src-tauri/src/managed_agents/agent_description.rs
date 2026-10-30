//! Effective public agent description — the Rust twin of
//! `desktop/src/features/agents/lib/agentDescription.ts`.
//!
//! The desktop publishes an agent's effective description as the `about`
//! field of its kind:0 profile event. Only the owner-authored
//! `AgentDefinition.description` publishes; a blank description publishes an
//! empty `about`, exactly as before the field existed.

use super::{AgentDefinition, ManagedAgentRecord};

/// The description to publish for an agent: the authored `description`,
/// trimmed, when non-empty; otherwise `None`.
///
/// TS twin: `effectiveAgentDescription` in `lib/agentDescription.ts`.
pub(crate) fn effective_agent_description(description: Option<&str>) -> Option<String> {
    let authored = description.map(str::trim).unwrap_or("");
    if authored.is_empty() {
        return None;
    }
    Some(authored.to_string())
}

/// Effective description for a managed-agent record's kind:0 profile.
///
/// A persona-linked instance publishes its linked definition's authored
/// description — the definition is the authority for identity metadata,
/// matching how the card face resolves it. A missing linked definition yields
/// no description rather than reviving a stale instance copy. Only a
/// definition-less instance falls back to its own record field.
pub(crate) fn record_effective_description(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> Option<String> {
    if let Some(persona_id) = record.persona_id.as_deref() {
        return personas
            .iter()
            .find(|persona| persona.id == persona_id)
            .and_then(|persona| effective_agent_description(persona.description.as_deref()));
    }
    effective_agent_description(record.description.as_deref())
}

// Tests mirror `lib/agentDescription.test.mjs` case-for-case so the Rust
// publish path and the TS display path cannot drift silently.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_description_wins() {
        assert_eq!(
            effective_agent_description(Some("Reviews desktop PRs.")).as_deref(),
            Some("Reviews desktop PRs.")
        );
    }

    #[test]
    fn authored_description_is_trimmed() {
        assert_eq!(
            effective_agent_description(Some("  Reviews desktop PRs.  ")).as_deref(),
            Some("Reviews desktop PRs.")
        );
    }

    #[test]
    fn blank_and_none_descriptions_yield_none() {
        assert_eq!(effective_agent_description(None), None);
        assert_eq!(effective_agent_description(Some("")), None);
        assert_eq!(effective_agent_description(Some("   ")), None);
    }

    fn record_with(description: Option<&str>, persona_id: Option<&str>) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "abcd1234",
                "name": "test-agent",
                "private_key_nsec": "nsec1fake",
                "relay_url": "wss://localhost:3000",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("sample record");
        record.description = description.map(str::to_string);
        record.persona_id = persona_id.map(str::to_string);
        record
    }

    fn persona_with(id: &str, description: Option<&str>) -> AgentDefinition {
        let mut persona: AgentDefinition = serde_json::from_str(
            r#"{
                "id": "placeholder",
                "display_name": "Helper",
                "system_prompt": "You help.",
                "is_builtin": false,
                "is_active": true,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-02T00:00:00Z"
            }"#,
        )
        .expect("sample persona");
        persona.id = id.to_string();
        persona.description = description.map(str::to_string);
        persona
    }

    #[test]
    fn linked_record_publishes_the_definition_description() {
        let record = record_with(Some("record-level"), Some("p1"));
        let personas = vec![persona_with("p1", Some("Definition description."))];
        assert_eq!(
            record_effective_description(&record, &personas).as_deref(),
            Some("Definition description.")
        );
    }

    #[test]
    fn linked_record_with_blank_definition_description_publishes_none() {
        let record = record_with(Some("record-level"), Some("p1"));
        let personas = vec![persona_with("p1", None)];
        assert_eq!(record_effective_description(&record, &personas), None);
    }

    #[test]
    fn definition_less_record_falls_back_to_its_own_description() {
        let record = record_with(Some("Record description."), None);
        assert_eq!(
            record_effective_description(&record, &[]).as_deref(),
            Some("Record description.")
        );
    }

    #[test]
    fn dangling_persona_link_does_not_revive_a_stale_record_description() {
        let record = record_with(Some("Stale imported description."), Some("missing"));
        assert_eq!(record_effective_description(&record, &[]), None);
    }

    #[test]
    fn no_description_anywhere_yields_none() {
        let record = record_with(None, None);
        assert_eq!(record_effective_description(&record, &[]), None);
    }
}
