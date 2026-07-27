use crate::managed_agents::{
    custom_harnesses::HarnessDefinition, AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AuthStatus,
    HarnessSource,
};

pub(crate) fn custom_runtime_catalog_entry(
    definition: HarnessDefinition,
    availability: AcpAvailabilityStatus,
    command: Option<String>,
    binary_path: Option<String>,
    default_args: Vec<String>,
) -> AcpRuntimeCatalogEntry {
    AcpRuntimeCatalogEntry {
        id: definition.id,
        display_label: definition.label.clone(),
        label: definition.label,
        sort_priority: 100,
        onboarding_visible: false,
        icon_url: String::new(),
        icon_scale: 1.0,
        // User-controlled custom avatar URLs never enter the catalog.
        avatar_url: String::new(),
        superseded_avatar_urls: Vec::new(),
        // Preserve the established custom-harness model surface.
        supports_buzz_model_config: true,
        availability,
        command,
        binary_path,
        default_args,
        mcp_command: None,
        model_env_var: None,
        provider_env_var: None,
        thinking_env_var: None,
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
    }
}
