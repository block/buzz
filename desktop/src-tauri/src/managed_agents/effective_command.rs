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
    record_agent_command_with_preferred_runtime(
        record,
        personas,
        global.preferred_runtime.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
