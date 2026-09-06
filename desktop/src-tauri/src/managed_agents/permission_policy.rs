//! Permission policy enum, source attribution, and the precedence resolver.
//!
//! `BUZZ_ACP_PERMISSION_POLICY` is in `RESERVED_ENV_KEYS` so users cannot
//! override it via the env-vars UI — a manual override would make the running
//! harness use a different policy than the saved/UI-visible setting.

use serde::{Deserialize, Serialize};

use super::types::{AgentDefinition, ManagedAgentRecord};

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
    /// Inherited from the linked definition's default policy.
    Definition,
    /// Inherited from the global agent config.
    GlobalDefault,
    /// Neither per-agent nor global is set; using the built-in desktop default.
    BuiltIn,
}

/// Resolve the effective permission policy for an agent.
///
/// Precedence (highest first):
/// 1. `record.permission_policy` — per-agent override.
/// 2. linked definition's `permission_policy` — definition default.
/// 3. `global.permission_policy` — fleet-wide default.
/// 4. [`PermissionPolicy::desktop_default`] — built-in.
///
/// The `definitions` slice is the same one every spawn/summary/deploy path
/// already loads. Tier 2 is a lookup by `record.persona_id`, so a linked
/// instance and the spawn-env it launches with resolve identically — a
/// definition-less or orphaned record simply skips the tier.
pub fn resolve_effective_permission_policy(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &super::global_config::GlobalAgentConfig,
) -> (PermissionPolicy, PermissionPolicySource) {
    if let Some(policy) = record.permission_policy {
        return (policy, PermissionPolicySource::Agent);
    }
    if let Some(policy) = record
        .persona_id
        .as_ref()
        .and_then(|pid| definitions.iter().find(|d| d.id == *pid))
        .and_then(|def| def.permission_policy)
    {
        return (policy, PermissionPolicySource::Definition);
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

/// Resolve the effective policy and inject `BUZZ_ACP_PERMISSION_POLICY` so the
/// running process and the UI-visible setting stay in sync. Returns the policy
/// so the caller can stamp it onto the spawn-config snapshot.
pub fn inject_spawn_permission_policy(
    command: &mut std::process::Command,
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &super::global_config::GlobalAgentConfig,
) -> PermissionPolicy {
    let (policy, _) = resolve_effective_permission_policy(record, definitions, global);
    command.env("BUZZ_ACP_PERMISSION_POLICY", policy.as_str());
    policy
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

    fn definition(id: &str, policy: Option<PermissionPolicy>) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            display_name: "Def".to_string(),
            avatar_url: None,
            description: None,
            system_prompt: String::new(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: Default::default(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            permission_policy: policy,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_per_agent_policy_beats_global_and_built_in() {
        let mut record = empty_record();
        record.permission_policy = Some(PermissionPolicy::Allow);
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Reject),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &[], &global);
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

        let (policy, source) = resolve_effective_permission_policy(&record, &[], &global);
        assert_eq!(policy, PermissionPolicy::Allow);
        assert_eq!(source, PermissionPolicySource::GlobalDefault);
    }

    #[test]
    fn test_built_in_used_when_neither_per_agent_nor_global_is_set() {
        let mut record = empty_record();
        record.permission_policy = None;
        let global = GlobalAgentConfig::default(); // permission_policy = None

        let (policy, source) = resolve_effective_permission_policy(&record, &[], &global);
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

        let (policy, source) = resolve_effective_permission_policy(&record, &[], &global);
        assert_eq!(policy, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::Agent);
    }

    #[test]
    fn test_definition_policy_beats_global_when_no_per_agent_override() {
        let mut record = empty_record();
        record.permission_policy = None;
        record.persona_id = Some("def-1".to_string());
        let defs = [definition("def-1", Some(PermissionPolicy::Reject))];
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Allow),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &defs, &global);
        assert_eq!(policy, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::Definition);
    }

    #[test]
    fn test_per_agent_override_beats_definition_default() {
        let mut record = empty_record();
        record.permission_policy = Some(PermissionPolicy::Allow);
        record.persona_id = Some("def-1".to_string());
        let defs = [definition("def-1", Some(PermissionPolicy::Reject))];
        let global = GlobalAgentConfig::default();

        let (policy, source) = resolve_effective_permission_policy(&record, &defs, &global);
        assert_eq!(policy, PermissionPolicy::Allow);
        assert_eq!(source, PermissionPolicySource::Agent);
    }

    #[test]
    fn test_definition_without_policy_falls_through_to_global() {
        let mut record = empty_record();
        record.permission_policy = None;
        record.persona_id = Some("def-1".to_string());
        // Linked definition carries no default — tier 2 is skipped.
        let defs = [definition("def-1", None)];
        let global = GlobalAgentConfig {
            permission_policy: Some(PermissionPolicy::Allow),
            ..Default::default()
        };

        let (policy, source) = resolve_effective_permission_policy(&record, &defs, &global);
        assert_eq!(policy, PermissionPolicy::Allow);
        assert_eq!(source, PermissionPolicySource::GlobalDefault);
    }

    #[test]
    fn test_orphaned_persona_id_skips_definition_tier() {
        let mut record = empty_record();
        record.permission_policy = None;
        record.persona_id = Some("missing".to_string());
        // The linked definition is gone; a stale slice with a different id
        // must not resolve tier 2 (no `find` match) — fall to built-in.
        let defs = [definition("def-1", Some(PermissionPolicy::Reject))];
        let global = GlobalAgentConfig::default();

        let (policy, source) = resolve_effective_permission_policy(&record, &defs, &global);
        assert_eq!(policy, PermissionPolicy::Ask);
        assert_eq!(source, PermissionPolicySource::BuiltIn);
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

        let (desired, source) =
            resolve_effective_permission_policy(&record, &[], &global_after_flip);
        assert_eq!(desired, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::GlobalDefault);
        assert_ne!(record.applied_permission_policy, Some(desired));
    }

    /// Paul's acceptance row for the definition tier: editing a definition's
    /// default policy MUST succeed while a linked instance is deployed — unlike
    /// a *per-instance* override, which `apply_permission_policy_update` rejects
    /// while deployed, a definition has no deploy receipt and its write path
    /// (`apply_persona_behavior`) carries no such guard. The edit then lights
    /// the deployed instance's drift row: with no per-instance override the
    /// recomputed *desired* policy resolves from the definition tier (now
    /// Reject) while the byte-stamped *applied* receipt stays Allow, so the two
    /// diverge exactly as the UI drift row keys on.
    #[test]
    fn test_definition_default_edit_while_deployed_succeeds_and_lights_drift() {
        use super::super::types::{apply_persona_behavior, PersonaBehaviorRequest};

        // A linked instance deployed remotely, launched under Allow (receipt),
        // with no per-instance override so it resolves through the definition.
        let mut record = empty_record();
        record.persona_id = Some("def-1".to_string());
        record.permission_policy = None;
        record.applied_permission_policy = Some(PermissionPolicy::Allow);

        // Edit the linked definition's default to Reject through the real write
        // path. There is no deployed-read-only guard here — a definition is
        // never itself deployed — so the edit succeeds unconditionally.
        let mut def = definition("def-1", Some(PermissionPolicy::Allow));
        apply_persona_behavior(
            &mut def,
            Some(PersonaBehaviorRequest {
                respond_to: None,
                respond_to_allowlist: Vec::new(),
                parallelism: None,
                permission_policy: Some(PermissionPolicy::Reject),
            }),
        )
        .expect("editing a definition default must succeed while deployed");
        assert_eq!(def.permission_policy, Some(PermissionPolicy::Reject));

        // The deployed instance now resolves desired=Reject from the definition
        // tier while applied stays Allow → drift.
        let global = GlobalAgentConfig::default();
        let (desired, source) = resolve_effective_permission_policy(&record, &[def], &global);
        assert_eq!(desired, PermissionPolicy::Reject);
        assert_eq!(source, PermissionPolicySource::Definition);
        assert_ne!(record.applied_permission_policy, Some(desired));
    }
}
