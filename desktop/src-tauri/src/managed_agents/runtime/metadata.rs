/// Classify an agent's persona against the live catalog for the Agents-menu
/// drift indicator. Returns `(out_of_date, orphaned)`.
///
/// Drift basis is the RECORD's `persona_source_version`, never the engram:
/// - persona_id set + persona present: out_of_date when the snapshot hash
///   differs from the persona's current content hash.
/// - persona_id set + persona gone: orphaned (no current hash to respawn into,
///   so never out_of_date — we must not tell the user to respawn into nothing).
/// - no persona_id: neither — a hand-built agent has no persona to drift from.
pub(crate) fn persona_drift_state(
    record: &crate::managed_agents::ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
) -> (bool, bool) {
    let Some(persona_id) = record.persona_id.as_deref() else {
        return (false, false);
    };
    let Some(persona) = personas.iter().find(|p| p.id == persona_id) else {
        return (false, true);
    };
    let current = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(persona),
    );
    let out_of_date = record
        .persona_source_version
        .as_deref()
        .is_some_and(|pinned| pinned != current);
    (out_of_date, false)
}

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

/// Resolve effective prompt/model/provider using definition-authoritative
/// semantics for linked instances.
///
/// Used by `agent_config.rs` to inject persona defaults into the config surface
/// before running the reader.
pub(crate) fn resolve_effective_prompt_model_provider(
    persona_id: Option<&str>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    record_prompt: Option<String>,
    record_model: Option<String>,
    record_provider: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    match persona_id.and_then(|pid| personas.iter().find(|p| p.id == pid)) {
        Some(p) => {
            fn non_blank(v: Option<&str>) -> Option<String> {
                v.filter(|s| !s.trim().is_empty()).map(str::to_owned)
            }
            let prompt = non_blank(Some(&p.system_prompt));
            let model = non_blank(p.model.as_deref());
            let provider = non_blank(p.provider.as_deref());
            (prompt, model, provider)
        }
        None => (record_prompt, record_model, record_provider),
    }
}
