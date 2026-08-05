use super::{known_acp_runtime, preset_mcp_command};

/// Return the bundled MCP command declared by a builtin or static preset.
pub(crate) fn mcp_command_for_runtime(command: &str) -> Option<&'static str> {
    known_acp_runtime(command)
        .and_then(|runtime| runtime.mcp_command)
        .or_else(|| preset_mcp_command(command))
}

#[test]
fn hermes_preset_supplies_buzz_mcp_to_managed_agent_spawns() {
    assert_eq!(
        mcp_command_for_runtime("/Users/example/.local/bin/hermes-acp"),
        Some("buzz-dev-mcp")
    );
}
