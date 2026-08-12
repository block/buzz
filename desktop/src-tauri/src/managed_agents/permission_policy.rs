//! Permission policy enum, source attribution, and the precedence resolver.
//!
//! `BUZZ_ACP_PERMISSION_POLICY` is in `RESERVED_ENV_KEYS` so users cannot
//! override it via the env-vars UI — a manual override would make the running
//! harness use a different policy than the saved/UI-visible setting.

use serde::{Deserialize, Serialize};

use super::types::ManagedAgentRecord;

/// How the agent answers `session/request_permission` requests.
///
/// - `Ask`    — show an Allow/Deny card; auto-deny after 300 s (desktop default).
/// - `Allow`  — auto-select the unique `allow_once` option; explicit opt-in.
/// - `Reject` — deny immediately; headless/CLI default.
///
/// Wire format is lowercase to match the harness CLI vocabulary and the
/// `BUZZ_ACP_PERMISSION_POLICY` env var the harness reads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionPolicy {
    Ask,
    Allow,
    Reject,
}

impl PermissionPolicy {
    /// The env-var wire string consumed by the harness
    /// (`BUZZ_ACP_PERMISSION_POLICY`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Allow => "allow",
            Self::Reject => "reject",
        }
    }

    /// The built-in desktop default: show the Allow/Deny card.
    ///
    /// Headless / bare-CLI callers use `Reject` — they never have a UI to
    /// answer a card. The desktop injects the resolved effective policy so
    /// headless sessions spawned by the desktop still pick up the user's
    /// choice.
    pub fn desktop_default() -> Self {
        Self::Ask
    }
}

/// Where the effective [`PermissionPolicy`] came from. Serialized as a
/// `snake_case` string for TypeScript's exhaustive-switch pattern.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicySource {
    /// Set explicitly on this agent record.
    Agent,
    /// Inherited from the global agent config.
    GlobalDefault,
    /// Neither per-agent nor global is set; using the built-in desktop default.
    BuiltIn,
}

/// Resolve the effective permission policy for an agent.
///
/// Precedence (highest first):
/// 1. `record.permission_policy` — per-agent override.
/// 2. `global.permission_policy` — fleet-wide default.
/// 3. [`PermissionPolicy::desktop_default`] — built-in.
pub fn resolve_effective_permission_policy(
    record: &ManagedAgentRecord,
    global: &super::global_config::GlobalAgentConfig,
) -> (PermissionPolicy, PermissionPolicySource) {
    if let Some(policy) = record.permission_policy {
        return (policy, PermissionPolicySource::Agent);
    }
    if let Some(policy) = global.permission_policy {
        return (policy, PermissionPolicySource::GlobalDefault);
    }
    (
        PermissionPolicy::desktop_default(),
        PermissionPolicySource::BuiltIn,
    )
}

/// Apply a permission-policy update from an agent-update request.
///
/// Returns `Ok(())` when the field was updated (or there was nothing to do).
/// Returns `Err(message)` when the update is rejected because the agent is
/// deployed remotely and its policy is therefore read-only.
///
/// `update` is the two-layer optional: `None` = don't touch, `Some(None)` =
/// clear the per-agent override, `Some(Some(policy))` = set the override.
pub fn apply_permission_policy_update(
    record: &mut ManagedAgentRecord,
    update: Option<Option<PermissionPolicy>>,
) -> Result<(), String> {
    let Some(policy) = update else { return Ok(()) };
    if matches!(record.backend, super::BackendKind::Provider { .. })
        && record.backend_agent_id.is_some()
    {
        return Err("permission_policy is read-only while the agent is deployed remotely; shut down and redeploy to change it".to_string());
    }
    record.permission_policy = policy;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::global_config::GlobalAgentConfig;

    fn empty_record() -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": "abcd1234",
            "name": "test",
            "display_name": "Test",
            "private_key_nsec": "nsec1fake",
            "relay_url": "wss://relay.example",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 300,
            "idle_timeout_seconds": 900,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .expect("minimal ManagedAgentRecord")
    }

    #[test]
    fn test_per_agent_policy_beats_global_and_built_in() {
        let mut record = empty_record();
        record.permission_policy = Some(PermissionPolicy::Allow);
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Reject),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &global);
        assert_eq!(policy, PermissionPolicy::Allow);
        assert_eq!(source, PermissionPolicySource::Agent);
    }

    #[test]
    fn test_global_policy_beats_built_in_when_no_per_agent() {
        let mut record = empty_record();
        record.permission_policy = None;
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Allow),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &global);
        assert_eq!(policy, PermissionPolicy::Allow);
        assert_eq!(source, PermissionPolicySource::GlobalDefault);
    }

    #[test]
    fn test_built_in_used_when_neither_per_agent_nor_global_is_set() {
        let mut record = empty_record();
        record.permission_policy = None;
        let global = GlobalAgentConfig::default(); // permission_policy = None

        let (policy, source) = resolve_effective_permission_policy(&record, &global);
        assert_eq!(policy, PermissionPolicy::Ask); // desktop_default
        assert_eq!(source, PermissionPolicySource::BuiltIn);
    }

    #[test]
    fn test_per_agent_reject_beats_global_allow() {
        let mut record = empty_record();
        record.permission_policy = Some(PermissionPolicy::Reject);
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Allow),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &global);
        assert_eq!(policy, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::Agent);
    }

    /// Desired-vs-applied drift at the resolver level (Wes's regression, resolver
    /// half): after a post-deploy global flip to Reject, the recomputed *desired*
    /// policy is Reject while the persisted *applied* receipt stays Allow, so the
    /// two diverge and the UI can flag drift. The production stamp/receipt half —
    /// that `applied` is written from the byte-identical sent value and survives a
    /// failed redeploy — is pinned by the discriminating transition tests in
    /// `commands/agents_deploy.rs`.
    #[test]
    fn test_applied_policy_survives_global_flip_deploy_allow_global_flips_to_reject() {
        let mut record = empty_record();
        record.permission_policy = None;
        record.applied_permission_policy = Some(PermissionPolicy::Allow);

        let global_after_flip = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Reject),
            ..Default::default()
        };

        let (desired, source) = resolve_effective_permission_policy(&record, &global_after_flip);
        assert_eq!(desired, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::GlobalDefault);
        assert_ne!(record.applied_permission_policy, Some(desired));
    }
}
