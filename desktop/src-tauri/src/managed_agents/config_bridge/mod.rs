mod buzz_agent;
mod claude;
mod codex;
mod goose;
mod pi;
pub(crate) mod reader;
mod schema_walker;
pub(crate) mod types;

pub(crate) use types::*;

/// Read the goose harness config file (`~/.config/goose/config.yaml`).
///
/// Used by readiness evaluation to silence requirements that are already
/// satisfied in the file config layer — the harness reads this file at startup
/// so env vars we would otherwise require are not needed from Buzz.
pub(crate) fn read_goose_file_config() -> Option<RuntimeFileConfig> {
    goose::read_config_file()
}

/// Ensure the Buzz nest workdir has a `.pi/mcp.json` registering
/// `buzz-dev-mcp` for pi's MCP extension. Called from the spawn path when
/// launching a pi agent. See `pi::ensure_workdir_mcp_json`.
pub(crate) fn ensure_pi_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> {
    pi::ensure_workdir_mcp_json(workdir)
}
