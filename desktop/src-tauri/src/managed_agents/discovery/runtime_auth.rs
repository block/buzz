use crate::managed_agents::{
    config_bridge::codex_env_key_auth_satisfied, AcpAvailabilityStatus, AcpRuntimeCatalogEntry,
    AuthStatus,
};

/// Return the auth state that can be established without invoking a CLI.
///
/// `codex login status` only checks the persisted credential store. A custom
/// provider can instead use the process env var named by its `env_key`.
pub(super) fn initial_status(runtime_id: &str, availability: &AcpAvailabilityStatus) -> AuthStatus {
    if runtime_id == "codex"
        && *availability == AcpAvailabilityStatus::Available
        && codex_env_key_auth_satisfied(&Default::default())
    {
        AuthStatus::LoggedIn
    } else {
        AuthStatus::Unknown
    }
}

pub(super) fn needs_probe(entry: &AcpRuntimeCatalogEntry) -> bool {
    needs_probe_status(&entry.availability, &entry.auth_status)
}

fn needs_probe_status(availability: &AcpAvailabilityStatus, auth_status: &AuthStatus) -> bool {
    *availability == AcpAvailabilityStatus::Available && *auth_status == AuthStatus::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_runtime_never_needs_auth_probe() {
        assert!(!needs_probe_status(
            &AcpAvailabilityStatus::NotInstalled,
            &AuthStatus::Unknown
        ));
    }

    #[test]
    fn preauthenticated_runtime_does_not_need_auth_probe() {
        assert!(!needs_probe_status(
            &AcpAvailabilityStatus::Available,
            &AuthStatus::LoggedIn
        ));
    }
}
