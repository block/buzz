use serde::Deserialize;
use std::collections::BTreeMap;

use crate::managed_agents::agent_snapshot::AgentSnapshot;

/// User-reviewed definition values submitted from the imported-agent setup
/// dialog. Snapshot memory and identity material remain owned by the import
/// command; this only overrides editable definition/profile fields. Environment
/// values are returned separately because snapshots intentionally do not carry
/// machine-local credentials.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportSetup {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    pub respond_to: Option<String>,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    pub parallelism: Option<u32>,
}

pub(super) fn apply_import_setup(
    snapshot: &mut AgentSnapshot,
    setup: &AgentSnapshotImportSetup,
) -> BTreeMap<String, String> {
    snapshot.profile.display_name = setup.display_name.clone();
    match setup
        .avatar_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        Some(url) if url.starts_with("data:image/") => {
            snapshot.profile.avatar_data_url = Some(url.to_string());
            snapshot.profile.avatar_url = None;
        }
        Some(url) => {
            snapshot.profile.avatar_data_url = None;
            snapshot.profile.avatar_url = Some(url.to_string());
        }
        None => {
            snapshot.profile.avatar_data_url = None;
            snapshot.profile.avatar_url = None;
        }
    }
    snapshot.definition.system_prompt = Some(setup.system_prompt.clone());
    snapshot.definition.runtime = setup.runtime.clone();
    snapshot.definition.model = setup.model.clone();
    snapshot.definition.provider = setup.provider.clone();
    snapshot.definition.name_pool = setup.name_pool.clone();
    snapshot.definition.respond_to = setup.respond_to.clone();
    snapshot.definition.respond_to_allowlist = setup.respond_to_allowlist.clone();
    snapshot.definition.parallelism = setup.parallelism;
    setup.env_vars.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::agent_snapshot::{
        AgentSnapshotDefinition, AgentSnapshotMemory, AgentSnapshotProfile, FORMAT_DISCRIMINATOR,
        FORMAT_VERSION,
    };

    #[test]
    fn applies_reviewed_agent_definition_before_creation() {
        let mut snapshot = AgentSnapshot {
            format: FORMAT_DISCRIMINATOR.to_string(),
            version: FORMAT_VERSION,
            definition: AgentSnapshotDefinition {
                name: "Source Agent".to_string(),
                source_is_builtin: false,
                system_prompt: None,
                runtime: None,
                model: None,
                provider: None,
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                name_pool: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
            },
            profile: AgentSnapshotProfile {
                display_name: "Source Agent".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
                entries: vec![],
            },
        };
        let setup = AgentSnapshotImportSetup {
            display_name: "Reviewed Agent".to_string(),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            system_prompt: "Use the reviewed instructions.".to_string(),
            runtime: Some("claude-code".to_string()),
            model: Some("claude-sonnet-4-5".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: vec!["Reviewed Agent".to_string(), "Helper".to_string()],
            env_vars: BTreeMap::from([(
                "ANTHROPIC_API_KEY".to_string(),
                "reviewed-secret".to_string(),
            )]),
            respond_to: Some("anyone".to_string()),
            respond_to_allowlist: vec![],
            parallelism: Some(3),
        };

        let reviewed_env_vars = apply_import_setup(&mut snapshot, &setup);

        assert_eq!(snapshot.profile.display_name, "Reviewed Agent");
        assert_eq!(
            snapshot.profile.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
        assert!(snapshot.profile.avatar_data_url.is_none());
        assert_eq!(
            snapshot.definition.system_prompt.as_deref(),
            Some("Use the reviewed instructions.")
        );
        assert_eq!(snapshot.definition.runtime.as_deref(), Some("claude-code"));
        assert_eq!(
            snapshot.definition.model.as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(snapshot.definition.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            snapshot.definition.name_pool,
            vec!["Reviewed Agent".to_string(), "Helper".to_string()]
        );
        assert_eq!(snapshot.definition.respond_to.as_deref(), Some("anyone"));
        assert_eq!(snapshot.definition.parallelism, Some(3));
        assert_eq!(
            reviewed_env_vars
                .get("ANTHROPIC_API_KEY")
                .map(String::as_str),
            Some("reviewed-secret")
        );
    }
}
