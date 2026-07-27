/// Provider id persisted on agent and global config, and shown in the picker.
pub const TOGETHER_PROVIDER_ID: &str = "together";
/// Together's OpenAI-compatible ingress. Chat Completions only — Together has
/// no Responses endpoint, so `OPENAI_COMPAT_API` is pinned rather than left on
/// `auto`.
pub const TOGETHER_API_BASE_URL: &str = "https://api.together.ai/v1";
/// The single user-owned input for this provider. Everything else about the
/// transport is derived.
pub const TOGETHER_API_KEY_ENV: &str = "TOGETHER_API_KEY";

/// Translate the Together AI provider into the OpenAI-compatible transport
/// understood by buzz-agent. Base URL, wire dialect, and the `OPENAI_COMPAT_*`
/// key names are derived runtime details, not user-owned agent configuration —
/// users only ever supply `TOGETHER_API_KEY` and a model.
///
/// `OPENAI_COMPAT_API_KEY` is written only when Together supplied a key, so a
/// caller that scrubs it first cannot leak an unrelated OpenAI credential to
/// Together's ingress.
pub fn apply_together_env(
    env: &mut std::collections::BTreeMap<String, String>,
    provider: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
) {
    if provider.map(str::trim) != Some(TOGETHER_PROVIDER_ID) {
        return;
    }
    env.insert("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string());
    env.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        TOGETHER_API_BASE_URL.to_string(),
    );
    env.insert("OPENAI_COMPAT_API".to_string(), "chat".to_string());
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        env.insert("BUZZ_AGENT_MODEL".to_string(), model.to_string());
        env.insert("OPENAI_COMPAT_MODEL".to_string(), model.to_string());
    }
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) {
        env.insert("OPENAI_COMPAT_API_KEY".to_string(), api_key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn applied(model: Option<&str>, api_key: Option<&str>) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        apply_together_env(&mut env, Some(TOGETHER_PROVIDER_ID), model, api_key);
        env
    }

    #[test]
    fn native_provider_maps_to_the_openai_chat_transport() {
        let env = applied(Some("moonshotai/Kimi-K2.6"), Some("tgp-secret"));

        assert_eq!(
            env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
            Some("openai")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_BASE_URL").map(String::as_str),
            Some(TOGETHER_API_BASE_URL)
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_API").map(String::as_str),
            Some("chat")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_API_KEY").map(String::as_str),
            Some("tgp-secret")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_MODEL").map(String::as_str),
            Some("moonshotai/Kimi-K2.6")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some("moonshotai/Kimi-K2.6")
        );
    }

    #[test]
    fn other_providers_are_left_alone() {
        let mut env = BTreeMap::new();
        apply_together_env(&mut env, Some("openai"), Some("gpt-5"), Some("sk-openai"));
        assert!(env.is_empty());

        apply_together_env(&mut env, None, Some("gpt-5"), Some("sk-openai"));
        assert!(env.is_empty());
    }

    #[test]
    fn a_missing_key_writes_no_credential_so_a_scrubbed_one_stays_scrubbed() {
        for api_key in [None, Some(""), Some("   ")] {
            let env = applied(Some("zai-org/GLM-5.2"), api_key);
            assert!(
                !env.contains_key("OPENAI_COMPAT_API_KEY"),
                "api_key {api_key:?} must not produce a credential"
            );
            // The rest of the transport is still derived, so the agent fails on
            // the missing key rather than on an api.openai.com base URL.
            assert_eq!(
                env.get("OPENAI_COMPAT_BASE_URL").map(String::as_str),
                Some(TOGETHER_API_BASE_URL)
            );
        }
    }

    #[test]
    fn a_blank_model_leaves_model_selection_to_the_agents_own_resolution() {
        for model in [None, Some(""), Some("  ")] {
            let env = applied(model, Some("tgp-secret"));
            assert!(!env.contains_key("OPENAI_COMPAT_MODEL"));
            assert!(!env.contains_key("BUZZ_AGENT_MODEL"));
        }
    }
}
