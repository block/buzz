use serde::Serialize;
use std::collections::BTreeMap;

use super::{AcpAvailabilityStatus, AuthStatus, HarnessSource};
use crate::managed_agents::discovery::KnownAcpRuntime;

#[derive(Debug, Clone, Default, Serialize)]
pub struct AcpRuntimeCapabilityFacts {
    /// Buzz-owned static fact: this runtime exposes native ACP config read/write.
    pub supports_acp_native_config: bool,
    /// Buzz-owned static fact: this runtime handles ACP model switching natively.
    pub supports_acp_model_switching: bool,
    /// Buzz-owned static fact: this runtime receives Buzz MCP lifecycle hooks.
    pub mcp_hooks: bool,
}

impl From<&KnownAcpRuntime> for AcpRuntimeCapabilityFacts {
    fn from(runtime: &KnownAcpRuntime) -> Self {
        Self {
            supports_acp_native_config: runtime.supports_acp_native_config,
            supports_acp_model_switching: runtime.supports_acp_model_switching,
            mcp_hooks: runtime.mcp_hooks,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpRuntimeCatalogEntry {
    pub id: String,
    pub label: String,
    pub avatar_url: String,
    pub availability: AcpAvailabilityStatus,
    pub command: Option<String>,
    pub binary_path: Option<String>,
    pub default_args: Vec<String>,
    pub mcp_command: Option<String>,
    /// Environment variable used to apply the initial model, when supported.
    pub model_env_var: Option<String>,
    /// Environment variable used to apply the selected LLM provider, when supported.
    pub provider_env_var: Option<String>,
    /// Environment variable used to apply thinking effort, when supported.
    pub thinking_env_var: Option<String>,
    /// Runtime-provided canonical effort choices, preserved from the shared catalog.
    pub effort_canonical_values: Option<Vec<String>>,
    pub max_tokens_env_var: Option<String>,
    pub context_limit_env_var: Option<String>,
    pub max_rounds_env_var: Option<String>,
    pub install_hint: String,
    pub install_instructions_url: String,
    /// true when at least one automated install step is available
    pub can_auto_install: bool,
    /// true when this runtime depends on a separately installed vendor CLI.
    pub requires_external_cli: bool,
    pub underlying_cli_path: Option<String>,
    /// true when an npm adapter step is pending but Node.js / npm is absent.
    /// The UI hides the Install button and shows a Node.js install callout.
    pub node_required: bool,
    /// Login/authentication status for CLI-based runtimes.
    pub auth_status: AuthStatus,
    /// Hint for completing authentication, shown when `auth_status` is not `logged_in`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,
    /// Whether this entry came from the compiled-in catalog or a user-supplied
    /// JSON file in `custom_harnesses/`. The UI uses this to decide editability.
    pub source: HarnessSource,
    /// Definition-level environment variables for `source: custom` entries.
    ///
    /// Populated from `HarnessDefinition.env` so the edit form can read them
    /// back and the user doesn't silently lose env vars when saving. Always
    /// empty for `builtin` and `preset` entries (those env values come from the
    /// runtime metadata path, not user-editable JSON).
    ///
    /// Skipped in serialization when empty to keep the catalog payload compact.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub definition_env: BTreeMap<String, String>,
    /// Spawn-time parallelism cap; absent for uncapped harnesses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
    #[serde(flatten)]
    pub capabilities: AcpRuntimeCapabilityFacts,
}

impl AcpRuntimeCatalogEntry {
    /// Project a saved custom harness into its catalog row without claiming static capabilities.
    pub(crate) fn from_custom_harness(
        definition: crate::managed_agents::custom_harnesses::HarnessDefinition,
    ) -> Self {
        // Resolve availability for the returned catalog entry.
        let (availability, command_opt, binary_path) =
            match crate::managed_agents::find_command(&definition.command) {
                Some(path) => (
                    AcpAvailabilityStatus::Available,
                    Some(definition.command.clone()),
                    Some(path.display().to_string()),
                ),
                None => (AcpAvailabilityStatus::NotInstalled, None, None),
            };

        let default_args = crate::managed_agents::normalize_agent_args(
            &definition.command,
            definition.args.clone(),
        );

        Self {
            id: definition.id,
            label: definition.label,
            avatar_url: String::new(),
            availability,
            command: command_opt,
            binary_path,
            default_args,
            mcp_command: None,
            model_env_var: None,
            provider_env_var: None,
            thinking_env_var: None,
            effort_canonical_values: None,
            max_tokens_env_var: None,
            context_limit_env_var: None,
            max_rounds_env_var: None,
            install_hint: definition.install_hint,
            install_instructions_url: definition.install_instructions_url,
            can_auto_install: false,
            requires_external_cli: false,
            underlying_cli_path: None,
            node_required: false,
            auth_status: AuthStatus::NotApplicable,
            login_hint: None,
            source: HarnessSource::Custom,
            definition_env: definition.env,
            max_parallelism: crate::managed_agents::harness_max_parallelism(&definition.command),
            capabilities: Default::default(),
        }
    }
}
