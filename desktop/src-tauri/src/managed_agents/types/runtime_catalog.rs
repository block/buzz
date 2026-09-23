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
