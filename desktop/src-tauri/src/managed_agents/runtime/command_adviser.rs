use tauri::{AppHandle, Manager};
const ENV_KEYS: &[&str] = &[
    "COMMAND_ADVISER_PERSONA_ID",
    "COMMAND_ADVISER_RAG_URL",
    "COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT",
    "COMMAND_ADVISER_WORLD_MONITOR_USAGE_PATH",
    "COMMAND_ADVISER_WORLD_MONITOR_OAUTH_PATH",
];

pub(super) fn is_command_adviser_persona(persona_id: &str) -> bool {
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
    if let Ok(Some(config)) = crate::command_services::trusted_lan::load_optional(&config_path) {
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
