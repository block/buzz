use std::process::Command;

use crate::managed_agents::KnownAcpRuntime;

pub(super) fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

/// Honor the desktop caller's lazy request only when the runtime catalog says
/// deferring the ACP subprocess is appropriate.
pub(super) fn should_defer_agent_start(
    requested_lazy: bool,
    runtime: Option<&KnownAcpRuntime>,
) -> bool {
    requested_lazy
        && runtime
            .map(|runtime| runtime.defer_agent_start_until_work)
            .unwrap_or(true)
}

/// Resolve the idle timeout that desktop should write into the harness
/// environment. Explicit record values win; an inherited process value remains
/// untouched; otherwise the runtime catalog may provide a safer default.
pub(super) fn effective_idle_timeout(
    configured: Option<u64>,
    inherited_is_set: bool,
    runtime: Option<&KnownAcpRuntime>,
) -> Option<u64> {
    configured.or_else(|| {
        (!inherited_is_set)
            .then(|| runtime.and_then(|runtime| runtime.default_idle_timeout_secs))
            .flatten()
    })
}

/// Apply launch-only runtime environment policy after all user environment
/// layers have been merged.
pub(super) fn apply_runtime_env_policy(command: &mut Command, runtime: Option<&KnownAcpRuntime>) {
    let Some(runtime) = runtime else {
        return;
    };
    for key in runtime.scrub_env_vars {
        command.env_remove(key);
    }
    for (key, value) in runtime.enforced_env {
        command.env(key, value);
    }
}

/// Resolve the generic harness bootstrap model for a known runtime.
///
/// Unknown/custom runtimes preserve the historical behavior because Buzz
/// cannot infer their ACP capabilities. Known runtime policy comes only from
/// `KnownAcpRuntime`, so launch code never needs a runtime-ID branch.
pub(super) fn harness_model_for_runtime<'a>(
    runtime: Option<&KnownAcpRuntime>,
    effective_model: Option<&'a str>,
) -> Option<&'a str> {
    if runtime.is_some_and(|runtime| !runtime.accepts_harness_model) {
        None
    } else {
        effective_model
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_runtime_env_policy, effective_idle_timeout, harness_model_for_runtime,
        should_defer_agent_start,
    };
    use crate::managed_agents::known_acp_runtime;

    #[test]
    fn devin_policy_enforces_safe_permissions_and_stored_login_identity() {
        let mut command = std::process::Command::new("buzz-acp");
        command.env("BUZZ_ACP_PERMISSION_MODE", "bypassPermissions");
        command.env("BUZZ_ACP_AUTO_APPROVE_PERMISSIONS", "true");
        command.env("BUZZ_ACP_INTERACTIVE_PERMISSIONS", "false");
        command.env("WINDSURF_API_KEY", "sentinel");

        apply_runtime_env_policy(&mut command, known_acp_runtime("devin"));

        assert!(command.get_envs().any(|(key, value)| {
            key == "BUZZ_ACP_PERMISSION_MODE" && value.is_some_and(|value| value == "default")
        }));
        assert!(command.get_envs().any(|(key, value)| {
            key == "BUZZ_ACP_AUTO_APPROVE_PERMISSIONS"
                && value.is_some_and(|value| value == "false")
        }));
        assert!(command.get_envs().any(|(key, value)| {
            key == "BUZZ_ACP_INTERACTIVE_PERMISSIONS" && value.is_some_and(|value| value == "true")
        }));
        assert!(command.get_envs().any(|(key, value)| {
            key == "BUZZ_ACP_SELF_PUBLISH_COMPLETION_GRACE"
                && value.is_some_and(|value| value == "30")
        }));
        assert!(command
            .get_envs()
            .any(|(key, value)| { key == "WINDSURF_API_KEY" && value.is_none() }));
    }

    #[test]
    fn existing_runtime_policy_remains_unchanged() {
        let mut command = std::process::Command::new("buzz-acp");
        command.env("GOOSE_MODE", "custom");

        apply_runtime_env_policy(&mut command, known_acp_runtime("goose"));

        assert!(command.get_envs().any(|(key, value)| {
            key == "GOOSE_MODE" && value.is_some_and(|value| value == "custom")
        }));
    }

    #[test]
    fn devin_owns_its_model_selection_without_changing_existing_runtime_bootstrap() {
        let requested = Some("swe-1-7-lightning");
        let devin = known_acp_runtime("devin").expect("Devin must remain cataloged");
        assert_eq!(harness_model_for_runtime(Some(devin), requested), None);

        for runtime_id in ["goose", "claude", "codex", "buzz-agent"] {
            let runtime =
                known_acp_runtime(runtime_id).expect("existing runtime must remain cataloged");
            assert_eq!(
                harness_model_for_runtime(Some(runtime), requested),
                requested,
                "{runtime_id} bootstrap behavior must remain unchanged"
            );
        }

        assert_eq!(
            harness_model_for_runtime(None, requested),
            requested,
            "custom runtimes preserve historical bootstrap behavior"
        );
    }

    #[test]
    fn devin_starts_eagerly_without_changing_existing_runtime_startup() {
        assert!(!should_defer_agent_start(true, known_acp_runtime("devin")));
        for runtime_id in ["goose", "claude", "codex", "buzz-agent"] {
            assert!(
                should_defer_agent_start(true, known_acp_runtime(runtime_id)),
                "{runtime_id} must retain lazy startup"
            );
        }
        assert!(should_defer_agent_start(true, None));
        assert!(!should_defer_agent_start(false, known_acp_runtime("goose")));
    }

    #[test]
    fn devin_idle_default_preserves_all_override_layers() {
        let devin = known_acp_runtime("devin");
        assert_eq!(effective_idle_timeout(None, false, devin), Some(120));
        assert_eq!(effective_idle_timeout(Some(45), false, devin), Some(45));
        assert_eq!(effective_idle_timeout(None, true, devin), None);

        for runtime_id in ["goose", "claude", "codex", "buzz-agent"] {
            assert_eq!(
                effective_idle_timeout(None, false, known_acp_runtime(runtime_id)),
                None,
                "{runtime_id} must retain the harness idle default"
            );
        }
        assert_eq!(effective_idle_timeout(None, false, None), None);
    }
}
