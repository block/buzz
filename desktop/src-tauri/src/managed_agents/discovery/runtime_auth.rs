use std::collections::BTreeMap;

use crate::managed_agents::{
    config_bridge::codex_env_key_auth_satisfied, AcpAvailabilityStatus, AcpRuntimeCatalogEntry,
    AuthStatus,
};

use super::{AuthEvidenceStrategy, KnownAcpRuntime};

/// Return the auth state that can be established without invoking a CLI.
pub(super) fn initial_status(
    runtime: &KnownAcpRuntime,
    availability: &AcpAvailabilityStatus,
    effective_envs: &[BTreeMap<String, String>],
) -> AuthStatus {
    if *availability == AcpAvailabilityStatus::Available
        && auth_evidence_satisfied(runtime.auth_evidence, effective_envs)
    {
        AuthStatus::LoggedIn
    } else {
        AuthStatus::Unknown
    }
}

pub(crate) fn auth_evidence_satisfied(
    strategy: AuthEvidenceStrategy,
    effective_envs: &[BTreeMap<String, String>],
) -> bool {
    if effective_envs.is_empty() {
        auth_evidence_satisfied_for_env(strategy, &BTreeMap::new())
    } else {
        effective_envs
            .iter()
            .any(|env| auth_evidence_satisfied_for_env(strategy, env))
    }
}

fn auth_evidence_satisfied_for_env(
    strategy: AuthEvidenceStrategy,
    effective_env: &BTreeMap<String, String>,
) -> bool {
    match strategy {
        AuthEvidenceStrategy::None => false,
        AuthEvidenceStrategy::StaticEnvKeys(keys) => keys
            .iter()
            .any(|key| env_value_is_set_from(effective_env, key, std::env::var_os(key))),
        AuthEvidenceStrategy::CodexProviderEnvKey => codex_env_key_auth_satisfied(effective_env),
    }
}

fn env_value_is_set_from(
    effective_env: &BTreeMap<String, String>,
    key: &str,
    process_value: Option<std::ffi::OsString>,
) -> bool {
    match effective_env.get(key) {
        Some(value) => !value.trim().is_empty(),
        None => process_value.is_some_and(|value| !value.to_string_lossy().trim().is_empty()),
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

    #[test]
    fn static_env_key_uses_last_wins_semantics() {
        let env = BTreeMap::from([("TOKEN".to_string(), String::new())]);

        assert!(!env_value_is_set_from(
            &env,
            "TOKEN",
            Some("parent-secret".into())
        ));
    }

    #[test]
    fn static_env_key_rejects_whitespace_only_values() {
        let env = BTreeMap::from([("TOKEN".to_string(), " \t".to_string())]);

        assert!(!env_value_is_set_from(&env, "TOKEN", None));
    }
}
