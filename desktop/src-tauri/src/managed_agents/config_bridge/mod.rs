mod buzz_agent;
mod claude;
mod codex;
pub(crate) mod effort;
mod goose;
pub(crate) mod reader;
mod schema_walker;
pub(crate) mod types;

pub(crate) use types::*;

/// The legacy effort env key written by pre-migration saves.
///
/// Harnesses whose native `thinking_env_var` differs from this constant
/// (currently: Goose uses `GOOSE_THINKING_EFFORT`) need the alias resolver in
/// [`effort`] to translate old saves. buzz-agent's native key equals this
/// constant, so no aliasing applies there.
pub(crate) const LEGACY_THINKING_EFFORT_KEY: &str = "BUZZ_AGENT_THINKING_EFFORT";

/// Return all known native thinking-effort env keys across all runtimes.
///
/// Derived from `KNOWN_ACP_RUNTIMES::thinking_env_var` so that adding a new
/// runtime automatically participates in foreign-key suppression without a
/// separate constant to update.
pub(crate) fn all_known_effort_keys() -> impl Iterator<Item = &'static str> {
    crate::managed_agents::discovery::KNOWN_ACP_RUNTIMES
        .iter()
        .filter_map(|rt| rt.thinking_env_var)
}

/// Read the goose harness config file (`~/.config/goose/config.yaml`).
///
/// Used by readiness evaluation to silence requirements that are already
/// satisfied in the file config layer — the harness reads this file at startup
/// so env vars we would otherwise require are not needed from Buzz.
pub(crate) fn read_goose_file_config() -> Option<RuntimeFileConfig> {
    goose::read_config_file()
}
