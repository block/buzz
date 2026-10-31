use std::{collections::BTreeMap, sync::atomic::AtomicBool};

use serde::{Deserialize, Deserializer, Serialize};

use super::{AgentDefinition, ManagedAgentRecord};
use crate::app_state::AppState;

pub(crate) const ACP_SESSION_POLICY_ENV_VAR: &str = "BUZZ_ACP_SESSION_POLICY";

/// Desktop experiment state that influences managed-agent lifecycle behavior.
pub struct ManagedAgentExperimentState {
    pub(crate) profile_reconcile_enabled: AtomicBool,
}

impl Default for ManagedAgentExperimentState {
    fn default() -> Self {
        Self {
            profile_reconcile_enabled: AtomicBool::new(true),
        }
    }
}

impl AppState {
    pub(crate) fn managed_agent_profile_reconcile_enabled(&self) -> &AtomicBool {
        &self.managed_agent_experiments.profile_reconcile_enabled
    }
}

/// Defines whether one ACP conversation is shared by a channel or isolated per thread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpSessionPolicy {
    /// Share one ACP conversation across all threads in a channel.
    #[default]
    Channel,
    /// Keep a separate ACP conversation for each channel thread.
    Thread,
}

impl<'de> Deserialize<'de> for AcpSessionPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value.as_str() {
            Some("thread") => Self::Thread,
            _ => Self::Channel,
        })
    }
}

impl AcpSessionPolicy {
    pub(crate) const fn is_channel(&self) -> bool {
        matches!(self, Self::Channel)
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Thread => "thread",
        }
    }
}

pub(crate) fn apply_acp_session_policy_env(
    command: &mut std::process::Command,
    policy: AcpSessionPolicy,
) {
    command.env(ACP_SESSION_POLICY_ENV_VAR, policy.as_str());
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

/// Resolve the policy that a launch would use. Linked instances inherit the
/// current definition, so an edit takes effect on restart without rewriting
/// already-deployed records. Orphaned and definition-less instances retain
/// the last policy stored on their record.
pub(crate) fn effective_acp_session_policy(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
) -> AcpSessionPolicy {
    record
        .persona_id
        .as_deref()
        .and_then(|id| definitions.iter().find(|definition| definition.id == id))
        .map(|definition| definition.session_policy)
        .unwrap_or(record.session_policy)
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
    fn default_policy_is_channel() {
        assert_eq!(AcpSessionPolicy::default(), AcpSessionPolicy::Channel);
        assert_eq!(AcpSessionPolicy::Channel.as_str(), "channel");
    }

    #[test]
    fn policies_serialize_for_storage_and_ipc() {
        assert_eq!(
            serde_json::to_string(&AcpSessionPolicy::Thread).unwrap_or_default(),
            "\"thread\""
        );
        assert_eq!(AcpSessionPolicy::Thread.as_str(), "thread");
    }

    #[test]
    fn unknown_or_null_policy_degrades_to_channel() {
        for value in [serde_json::json!("conversation"), serde_json::Value::Null] {
            assert_eq!(
                serde_json::from_value::<AcpSessionPolicy>(value)
                    .unwrap_or_else(|error| panic!("policy should degrade gracefully: {error}")),
                AcpSessionPolicy::Channel
            );
        }
    }

    #[test]
    fn local_launch_env_receives_the_selected_policy() {
        let mut command = std::process::Command::new("true");
        command.env(ACP_SESSION_POLICY_ENV_VAR, "ambient");

        apply_acp_session_policy_env(&mut command, AcpSessionPolicy::Thread);

        assert_eq!(command_policy(&command), Some("thread"));
    }
}
