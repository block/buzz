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

/// Resolve the session title for an agent: its `display_name` when it has one,
/// otherwise its unique `name` handle. `None` when both are blank, so the
/// caller clears the env var rather than exporting an empty title.
///
/// Sanitization (control chars, whitespace, length cap) and channel
/// qualification are the harness's job — see `sanitize_session_title` and
/// `compose_session_title` in `buzz-acp`.
pub(crate) fn resolve_session_title<'a>(
    display_name: Option<&'a str>,
    name: &'a str,
) -> Option<&'a str> {
    [display_name, Some(name)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
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

#[cfg(test)]
mod tests {
    use super::resolve_session_title;

    #[test]
    fn resolve_session_title_prefers_display_name() {
        assert_eq!(resolve_session_title(Some("Fizz"), "fizz-1"), Some("Fizz"));
    }

    #[test]
    fn resolve_session_title_falls_back_to_name_when_display_name_blank() {
        assert_eq!(resolve_session_title(None, "fizz-1"), Some("fizz-1"));
        assert_eq!(resolve_session_title(Some("  "), "fizz-1"), Some("fizz-1"));
    }

    #[test]
    fn resolve_session_title_returns_none_when_both_are_blank() {
        assert_eq!(resolve_session_title(Some(""), "   "), None);
    }

    #[test]
    fn resolve_session_title_trims_surrounding_whitespace() {
        assert_eq!(
            resolve_session_title(Some("  Fizz  "), "fizz-1"),
            Some("Fizz")
        );
    }
}
