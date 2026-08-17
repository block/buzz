use tauri::AppHandle;

use super::{
    default_agent_command, known_acp_runtime_exact, load_global_agent_config, AgentDefinition,
    ManagedAgentRecord,
};

/// Resolve an agent command using instance, record, persona, global, then
/// built-in precedence.
pub fn record_agent_command_with_preferred_runtime(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    preferred_runtime: Option<&str>,
) -> String {
    // The Command Console route is an operational mode, not a weak default.
    // It must replace stale per-agent pins created before the routing toggle
    // governed managed adviser conversations.
    if record
        .persona_id
        .as_deref()
        .is_some_and(super::runtime::is_command_adviser_persona)
    {
        if let Some(command) = preferred_runtime
            .map(str::trim)
            .filter(|runtime| !runtime.is_empty())
            .and_then(known_acp_runtime_exact)
            .and_then(|runtime| runtime.commands.first().copied())
        {
            return command.to_string();
        }
        if let Some(command) = preferred_runtime
            .map(str::trim)
            .filter(|runtime| !runtime.is_empty())
            .and_then(|runtime| {
                super::custom_harnesses::lookup_loaded_harness_by_id(runtime)
                    .map(|definition| definition.command.clone())
            })
        {
            return command;
        }
    }
    if let Some(pin) = record
        .agent_command_override
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
    {
        return pin.to_string();
    }
    if let Some(command) = record
        .runtime
        .as_deref()
        .and_then(known_acp_runtime_exact)
        .and_then(|runtime| runtime.commands.first().copied())
    {
        return command.to_string();
    }
    if let Some(command) = record.runtime.as_deref().and_then(|runtime| {
        super::custom_harnesses::lookup_loaded_harness_by_id(runtime)
            .map(|definition| definition.command.clone())
    }) {
        return command;
    }
    if let Some(command) = record
        .persona_id
        .as_deref()
        .and_then(|id| personas.iter().find(|persona| persona.id == id))
        .and_then(|persona| persona.runtime.as_deref())
        .and_then(known_acp_runtime_exact)
        .and_then(|runtime| runtime.commands.first().copied())
    {
        return command.to_string();
    }
    if let Some(command) = record
        .persona_id
        .as_deref()
        .and_then(|id| personas.iter().find(|persona| persona.id == id))
        .and_then(|persona| persona.runtime.as_deref())
        .and_then(|runtime| {
            super::custom_harnesses::lookup_loaded_harness_by_id(runtime)
                .map(|definition| definition.command.clone())
        })
    {
        return command;
    }
    if let Some(command) = preferred_runtime
        .map(str::trim)
        .filter(|runtime| !runtime.is_empty())
        .and_then(|runtime| {
            super::custom_harnesses::lookup_loaded_harness_by_id(runtime)
                .map(|definition| definition.command.clone())
        })
    {
        return command;
    }
    preferred_runtime
        .map(str::trim)
        .filter(|runtime| !runtime.is_empty())
        .and_then(known_acp_runtime_exact)
        .and_then(|runtime| runtime.commands.first().copied())
        .map(str::to_string)
        .unwrap_or_else(default_agent_command)
}

/// Resolve the effective command using the application's persisted default.
pub fn record_agent_command_for_app(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> String {
    let global = load_global_agent_config(app).unwrap_or_default();
    let global = super::runtime::routed_global_agent_config_for_app(
        app,
        record.persona_id.as_deref(),
        &global,
    );
    record_agent_command_with_preferred_runtime(
        record,
        personas,
        global.preferred_runtime.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{AgentDefinition, GlobalAgentConfig};
    use std::collections::BTreeMap;

    fn command_persona(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            display_name: "Navigation Adviser".to_string(),
            avatar_url: None,
            system_prompt: "Provide navigation advice.".to_string(),
            runtime: None,
            model: Some("google/gemma-4-26b-a4b".to_string()),
            provider: None,
            name_pool: Vec::new(),
            is_builtin: true,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-07-28T00:00:00Z".to_string(),
            updated_at: "2026-07-28T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn global_preference_wins_before_buzz_fallback() {
        let record = serde_json::from_value(serde_json::json!({
            "pubkey": "a".repeat(64),
            "name": "test",
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "created_at": "2026-07-28T00:00:00Z",
            "updated_at": "2026-07-28T00:00:00Z"
        }))
        .expect("minimal managed agent record");

        assert_eq!(
            record_agent_command_with_preferred_runtime(&record, &[], Some("codex")),
            "codex-acp"
        );
    }

    #[test]
    fn local_preference_routes_command_adviser_away_from_cloud_pin() {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "a".repeat(64),
            "name": "Navigation Adviser",
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "created_at": "2026-07-28T00:00:00Z",
            "updated_at": "2026-07-28T00:00:00Z"
        }))
        .expect("minimal managed agent record");
        record.persona_id = Some("builtin:command-navigation".to_string());
        record.agent_command_override = Some("codex-acp".to_string());

        assert_eq!(
            record_agent_command_with_preferred_runtime(&record, &[], Some("buzz-lmstudio-agent")),
            "buzz-lmstudio-agent"
        );
    }

    #[test]
    fn cloud_preference_routes_command_adviser_away_from_local_pin() {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "a".repeat(64),
            "name": "Navigation Adviser",
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "created_at": "2026-07-28T00:00:00Z",
            "updated_at": "2026-07-28T00:00:00Z"
        }))
        .expect("minimal managed agent record");
        record.persona_id = Some("builtin:command-navigation".to_string());
        record.runtime = Some("buzz-lmstudio-agent".to_string());

        assert_eq!(
            record_agent_command_with_preferred_runtime(&record, &[], Some("codex")),
            "codex-acp"
        );
    }

    #[test]
    fn local_command_adviser_descriptor_uses_qualified_model() {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "a".repeat(64),
            "name": "Navigation Adviser",
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "created_at": "2026-07-28T00:00:00Z",
            "updated_at": "2026-07-28T00:00:00Z"
        }))
        .expect("minimal managed agent record");
        record.persona_id = Some("builtin:command-navigation".to_string());
        record.agent_command_override = Some("codex-acp".to_string());
        let global = GlobalAgentConfig {
            preferred_runtime: Some("buzz-lmstudio-agent".to_string()),
            model: Some("gemma4-26b-official".to_string()),
            ..GlobalAgentConfig::default()
        };

        let personas = [command_persona("builtin:command-navigation")];
        let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
            &record, &personas, &global,
        )
        .expect("local Command Adviser descriptor");

        assert_eq!(descriptor.command, "buzz-lmstudio-agent");
        assert_eq!(
            descriptor.env.get("LM_STUDIO_MODEL").map(String::as_str),
            Some("gemma4-26b-official")
        );
    }
}
