/// Returns the (key, value) env var pairs that should be forwarded to the
/// agent process for model and provider selection.
///
/// Model injection is unconditional — even agents that support ACP model
/// switching need the initial bootstrap value. Provider injection is skipped
/// when `provider_locked` is true (e.g. Claude runtimes that only work with
/// Anthropic).
pub(crate) fn runtime_metadata_env_vars<'a>(
    model_env_var: Option<&'a str>,
    provider_env_var: Option<&'a str>,
    provider_locked: bool,
    effective_model: Option<&'a str>,
    effective_provider: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut vars = Vec::new();
    if let (Some(env_key), Some(model)) = (model_env_var, effective_model) {
        vars.push((env_key, model));
    }
    if !provider_locked {
        if let (Some(env_key), Some(provider)) = (provider_env_var, effective_provider) {
            vars.push((env_key, provider));
        }
    }
    vars
}

/// Resolve the effective (prompt, model, provider) triple for a persona-linked agent.
///
/// Given a persona_id, finds the persona in the list and returns its system_prompt,
/// model, and provider as the authoritative values. When the persona leaves `model`
/// or `provider` blank (None or whitespace-only), falls back to the record's own
/// field using the same precedence rule as `persona_snapshot_with_agent_config_fallback`
/// so the display surface matches spawn behavior. Falls back to the record's own
/// prompt/model/provider when no persona is linked or found.
///
/// Used by `agent_config.rs` to inject persona defaults into the config surface
/// before running the reader, so BuzzExplicit-tagged fields can be re-tagged to
/// PersonaDefault for fields the record did not independently set.
pub(crate) fn resolve_effective_prompt_model_provider(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    record_prompt: Option<String>,
    record_model: Option<String>,
    record_provider: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let fallback = crate::managed_agents::persona_events::persona_field_with_record_fallback;
    match persona_id.and_then(|pid| personas.iter().find(|p| p.id == pid)) {
        Some(p) => (
            Some(p.system_prompt.clone()),
            fallback(p.model.as_deref(), record_model.as_deref()), // fallback: record.model
            fallback(p.provider.as_deref(), record_provider.as_deref()), // fallback: record.provider
        ),
        None => (record_prompt, record_model, record_provider),
    }
}
