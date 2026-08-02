use std::path::Path;

use crate::managed_agents::AcpAvailabilityStatus;

/// Plans adapter installation without spawning npm.
pub(super) fn plan_adapter_install<'c>(
    runtime_id: &str,
    adapter_path: Option<&Path>,
    adapter_install_commands: &'c [&'c str],
    adapter_probe_path: Option<&str>,
) -> Option<Vec<&'c str>> {
    match adapter_path {
        Some(_) if runtime_id != "codex" => None,
        Some(path)
            if !crate::managed_agents::codex_adapter_is_outdated_with_path(
                path,
                adapter_probe_path,
            ) =>
        {
            None
        }
        Some(_) => Some(vec![
            "npm uninstall -g @zed-industries/codex-acp",
            "npm install -g @agentclientprotocol/codex-acp",
        ]),
        None => Some(adapter_install_commands.to_vec()),
    }
}

pub(super) fn should_install_cli(
    cli_found: bool,
    availability: Option<AcpAvailabilityStatus>,
) -> bool {
    !cli_found || availability == Some(AcpAvailabilityStatus::CliOutdated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incompatible_cli_requires_update() {
        assert!(should_install_cli(
            true,
            Some(AcpAvailabilityStatus::CliOutdated),
        ));
        assert!(should_install_cli(
            false,
            Some(AcpAvailabilityStatus::NotInstalled),
        ));
        assert!(!should_install_cli(
            true,
            Some(AcpAvailabilityStatus::Available),
        ));
        assert!(!should_install_cli(
            true,
            Some(AcpAvailabilityStatus::CompatibilityUnknown),
        ));
    }
}
