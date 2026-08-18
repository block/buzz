use tauri::{AppHandle, Manager};

use crate::command_services::trusted_lan::{
    load_optional as load_optional_trusted_lan_config, ModelRoutingPreference,
};
use crate::managed_agents::GlobalAgentConfig;
const ENV_KEYS: &[&str] = &[
    "COMMAND_ADVISER_PERSONA_ID",
    "COMMAND_ADVISER_MEMORY_URL",
    "COMMAND_ADVISER_RAG_URL",
    "COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT",
    "COMMAND_ADVISER_WORLD_MONITOR_USAGE_PATH",
    "COMMAND_ADVISER_WORLD_MONITOR_OAUTH_PATH",
    "BUZZ_ACP_EXPERIENCE_OUTBOX",
];

pub(crate) fn is_command_adviser_persona(persona_id: &str) -> bool {
    matches!(
        persona_id,
        "builtin:command-chief-of-staff"
            | "builtin:command-operations"
            | "builtin:command-intelligence"
            | "builtin:command-logistics"
            | "builtin:command-navigation"
            | "builtin:command-daily-routine"
            | "builtin:command-reporting"
            | "builtin:command-plans"
    )
}

pub(crate) fn qualified_local_model(
    _persona_id: Option<&str>,
    runtime: Option<&crate::managed_agents::KnownAcpRuntime>,
) -> Option<&'static str> {
    runtime
        .is_some_and(|runtime| runtime.id == "buzz-lmstudio-agent")
        .then_some(crate::commands::QUALIFIED_INSTANCE_ID)
}

pub(crate) fn should_publish_agent_output(
    _persona_id: Option<&str>,
    runtime: Option<&crate::managed_agents::KnownAcpRuntime>,
) -> bool {
    runtime.is_some_and(|runtime| runtime.id == "buzz-lmstudio-agent")
}

pub(crate) fn routed_global_agent_config_for_app(
    app: &AppHandle,
    _persona_id: Option<&str>,
    global: &GlobalAgentConfig,
) -> GlobalAgentConfig {
    let preference = app
        .path()
        .app_config_dir()
        .ok()
        .and_then(|directory| {
            load_optional_trusted_lan_config(&directory.join("trusted-lan-sources.json"))
                .ok()
                .flatten()
        })
        .map(|config| config.routing_preference());
    if !matches!(
        preference,
        Some(ModelRoutingPreference::LocalFirst | ModelRoutingPreference::LocalOnly)
    ) {
        return global.clone();
    }
    let mut routed = global.clone();
    routed.preferred_runtime = Some("buzz-lmstudio-agent".to_string());
    routed.model = Some(crate::commands::QUALIFIED_INSTANCE_ID.to_string());
    routed.provider = Some("lmstudio-native".to_string());
    routed
}

pub(super) fn should_enable_mcp_hooks(
    runtime_supports_hooks: bool,
    persona_id: Option<&str>,
) -> bool {
    runtime_supports_hooks && !persona_id.is_some_and(is_command_adviser_persona)
}

pub(super) fn apply_source_env(
    command: &mut std::process::Command,
    app: &AppHandle,
    persona_id: Option<&str>,
    agent_pubkey: &str,
) {
    for key in ENV_KEYS {
        command.env_remove(key);
    }
    let Some(persona_id) = persona_id.filter(|id| is_command_adviser_persona(id)) else {
        return;
    };
    command.env("COMMAND_ADVISER_PERSONA_ID", persona_id);
    let config_dir = match app.path().app_config_dir() {
        Ok(path) => path,
        Err(_) => return,
    };
    let config_path = config_dir.join("trusted-lan-sources.json");
    command.env(
        "BUZZ_ACP_EXPERIENCE_OUTBOX",
        config_dir
            .join("experience-outbox")
            .join(format!("{agent_pubkey}.sqlite3")),
    );
    if let Ok(Some(config)) = crate::command_services::trusted_lan::load_optional(&config_path) {
        command.env("COMMAND_ADVISER_MEMORY_URL", config.memory_url().as_str());
        command.env("COMMAND_ADVISER_RAG_URL", config.rag_url().as_str());
        command.env(
            "COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT",
            config.world_monitor().endpoint(),
        );
    } else {
        command.env(
            "COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT",
            buzz_command_sources_pkg::DEFAULT_WORLD_MONITOR_ENDPOINT,
        );
    }
    command.env(
        "COMMAND_ADVISER_WORLD_MONITOR_USAGE_PATH",
        config_dir.join("world-monitor-usage.json"),
    );
    if persona_id == "builtin:command-intelligence" {
        command.env(
            "COMMAND_ADVISER_WORLD_MONITOR_OAUTH_PATH",
            config_dir.join(buzz_command_sources_pkg::WORLD_MONITOR_OAUTH_FILENAME),
        );
    }
}
