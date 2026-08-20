use std::collections::BTreeMap;

pub(crate) const MIGRATED_OPENAI_PROVIDER_ENV: &str = "OPENAI_COMPAT_MIGRATED_PROVIDER";

pub(crate) fn project_buzz_agent_definition(
    definition: &mut crate::managed_agents::AgentDefinition,
) {
    let is_buzz_agent = definition
        .runtime
        .as_deref()
        .and_then(crate::managed_agents::known_acp_runtime_exact)
        .is_some_and(|runtime| runtime.id == "buzz-agent");
    if !is_buzz_agent {
        return;
    }
    if let Some(provider) = definition.env_vars.get(MIGRATED_OPENAI_PROVIDER_ENV) {
        definition.provider = Some(provider.clone());
    }
}

pub(crate) fn effective_runtime_provider(
    command: &str,
    configured_provider: Option<String>,
    env: &BTreeMap<String, String>,
) -> Option<String> {
    if crate::managed_agents::known_acp_runtime(command)
        .is_some_and(|runtime| runtime.id == "buzz-agent")
    {
        env.get("BUZZ_AGENT_PROVIDER")
            .cloned()
            .or(configured_provider)
    } else {
        configured_provider
    }
}

pub(crate) fn canonicalize_openai_provider_env(
    env: &mut BTreeMap<String, String>,
    provider: Option<&str>,
) -> Option<String> {
    let migrated_provider = env.remove(MIGRATED_OPENAI_PROVIDER_ENV);
    let provider = migrated_provider
        .as_deref()
        .map(str::trim)
        .filter(|value| matches!(*value, "openai" | "openai-compat"))
        .map(str::to_string)
        .or_else(|| provider.map(str::trim).map(str::to_ascii_lowercase));
    match provider.as_deref() {
        Some("openai") => {
            env.remove("OPENAI_COMPAT_BASE_URL");
            env.remove("OPENAI_COMPAT_API_KEY");
        }
        Some("openai-compat") => {
            env.remove("OPENAI_API_KEY");
            env.entry("OPENAI_COMPAT_API_KEY".to_string()).or_default();
        }
        _ => {}
    }
    if let (Some(value), Some(slot)) = (provider.as_ref(), env.get_mut("BUZZ_AGENT_PROVIDER")) {
        *slot = value.clone();
    }
    provider
}
