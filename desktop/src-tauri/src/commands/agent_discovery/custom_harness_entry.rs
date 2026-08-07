//! Catalog-entry projection for a user-defined harness, split out of
//! `agent_discovery.rs` to respect the repo's file-size ratchet.

use crate::managed_agents::{
    custom_harnesses::HarnessDefinition, AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus,
    HarnessSource,
};

/// Build the catalog row returned to the UI after a custom harness is saved.
///
/// Custom harnesses declare no runtime capability metadata, so every env-var
/// target — including the structured thinking-config pair — is `None`; effort
/// for these entries can only come from the live ACP session.
pub(super) fn custom_harness_catalog_entry(
    definition: HarnessDefinition,
    availability: AcpAvailabilityStatus,
    command_opt: Option<String>,
    binary_path: Option<String>,
    default_args: Vec<String>,
) -> AcpRuntimeCatalogEntry {
    AcpRuntimeCatalogEntry {
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
        thinking_config_json_env_var: None,
        thinking_config_json_key: None,
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
    }
}
