use super::types::RuntimeFileConfig;

/// Buzz-agent has no config file — returns an empty config.
/// All config comes from env vars (tier 2a) set at spawn time.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    None
}
