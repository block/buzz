use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

pub(crate) const ACP_SESSION_POLICY_ENV_VAR: &str = "BUZZ_ACP_SESSION_POLICY";

/// Desktop experiment state that influences managed-agent lifecycle behavior.
pub struct ManagedAgentExperimentState {
    pub(crate) profile_reconcile_enabled: AtomicBool,
    pub(crate) thread_scoped_acp_sessions_enabled: AtomicBool,
}

impl Default for ManagedAgentExperimentState {
    fn default() -> Self {
        Self {
            profile_reconcile_enabled: AtomicBool::new(true),
            thread_scoped_acp_sessions_enabled: AtomicBool::new(false),
        }
    }
}

impl AppState {
    pub(crate) fn managed_agent_profile_reconcile_enabled(&self) -> &AtomicBool {
        &self.managed_agent_experiments.profile_reconcile_enabled
    }

    pub(crate) fn thread_scoped_acp_sessions_enabled(&self) -> &AtomicBool {
        &self
            .managed_agent_experiments
            .thread_scoped_acp_sessions_enabled
    }
}

/// Desktop-owned ACP session policy applied to every managed-agent launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcpSessionPolicy {
    Channel,
    Thread,
}

impl AcpSessionPolicy {
    pub(crate) fn from_thread_scoped_enabled(enabled: bool) -> Self {
        if enabled {
            Self::Thread
        } else {
            Self::Channel
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Thread => "thread",
        }
    }
}

/// Resolve the persisted experiment state at the shared launch boundary.
pub(crate) fn acp_session_policy(state: &AppState) -> AcpSessionPolicy {
    AcpSessionPolicy::from_thread_scoped_enabled(
        state
            .thread_scoped_acp_sessions_enabled()
            .load(Ordering::Acquire),
    )
}

pub(crate) fn apply_acp_session_policy_env(
    command: &mut std::process::Command,
    policy: AcpSessionPolicy,
) {
    command.env(ACP_SESSION_POLICY_ENV_VAR, policy.as_str());
}

/// Resolve the effective policy, apply it to `command`, and return it so the
/// caller can stamp the same value onto the spawn snapshot (env and badge can
/// never disagree about what the child launched with).
pub(crate) fn apply_app_acp_session_policy_env(
    app: &AppHandle,
    command: &mut std::process::Command,
) -> AcpSessionPolicy {
    let policy = acp_session_policy(app.state::<AppState>().inner());
    apply_acp_session_policy_env(command, policy);
    policy
}

pub(crate) fn insert_acp_session_policy_env(
    policy_env: &mut BTreeMap<String, String>,
    policy: AcpSessionPolicy,
) {
    policy_env.insert(
        ACP_SESSION_POLICY_ENV_VAR.to_string(),
        policy.as_str().to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_policy(command: &std::process::Command) -> Option<&str> {
        command
            .get_envs()
            .find(|(key, _)| *key == ACP_SESSION_POLICY_ENV_VAR)
            .and_then(|(_, value)| value)
            .and_then(std::ffi::OsStr::to_str)
    }

    #[test]
    fn absent_or_disabled_experiment_selects_channel_policy() {
        assert_eq!(
            AcpSessionPolicy::from_thread_scoped_enabled(false),
            AcpSessionPolicy::Channel
        );
        assert_eq!(AcpSessionPolicy::Channel.as_str(), "channel");
    }

    #[test]
    fn enabled_experiment_selects_thread_policy() {
        assert_eq!(
            AcpSessionPolicy::from_thread_scoped_enabled(true),
            AcpSessionPolicy::Thread
        );
        assert_eq!(AcpSessionPolicy::Thread.as_str(), "thread");
    }

    #[test]
    fn local_launch_env_receives_the_selected_policy() {
        let mut command = std::process::Command::new("true");
        command.env(ACP_SESSION_POLICY_ENV_VAR, "ambient");

        apply_acp_session_policy_env(&mut command, AcpSessionPolicy::Thread);

        assert_eq!(command_policy(&command), Some("thread"));
    }
}
