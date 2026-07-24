pub const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
pub const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";
pub const RELAY_MESH_PROVIDER_ID: &str = "relay-mesh";
pub const RELAY_MESH_AUTO_MODEL_ID: &str = "auto";
#[cfg(feature = "mesh-llm")]
pub const RELAY_MESH_PREFER_MESH_FOR_AUTO_ENV: &str = "BUZZ_AGENT_PREFER_MESH_FOR_AUTO";

/// Translate the native Buzz shared compute provider into the OpenAI-compatible
/// transport understood by buzz-agent. These are derived runtime details, not
/// user-owned agent configuration.
#[cfg(feature = "mesh-llm")]
pub fn apply_relay_mesh_env(
    env: &mut std::collections::BTreeMap<String, String>,
    provider: Option<&str>,
    model: Option<&str>,
) {
    if provider.map(str::trim) != Some(RELAY_MESH_PROVIDER_ID) {
        return;
    }
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(RELAY_MESH_AUTO_MODEL_ID)
        .to_string();
    env.insert("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string());
    env.insert("BUZZ_AGENT_MODEL".to_string(), model.clone());
    env.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        RELAY_MESH_API_BASE_URL.to_string(),
    );
    env.insert("OPENAI_COMPAT_MODEL".to_string(), model);
    env.insert(
        "OPENAI_COMPAT_API_KEY".to_string(),
        RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
    );
    env.insert("OPENAI_COMPAT_API".to_string(), "chat".to_string());
    // Buzz owns the meaning of relay-mesh `auto`: buzz-agent dynamically uses
    // mesh-llm's virtual Mixture-of-Agents model whenever the live catalog says
    // at least two distinct models are available, and otherwise keeps the
    // router's normal single-model `auto` behavior.
    env.insert(
        RELAY_MESH_PREFER_MESH_FOR_AUTO_ENV.to_string(),
        "1".to_string(),
    );
    // Keep the requested response inside smaller local-model context windows,
    // and spend that budget on an answer/tool call instead of hidden reasoning.
    // Without both settings Qwen3 either fails the router's fit check at the
    // agent default (32K) or can consume a tight cap before serializing a tool.
    // Defaults only — never clobber a user/record override already present (#2558).
    insert_default_if_unset(env, "BUZZ_AGENT_MAX_OUTPUT_TOKENS", "4096");
    insert_default_if_unset(env, "BUZZ_AGENT_THINKING_EFFORT", "none");
}

/// Insert `value` only when `key` is missing or empty/whitespace.
#[cfg(feature = "mesh-llm")]
fn insert_default_if_unset(
    env: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    value: &str,
) {
    let unset = env.get(key).map(|v| v.trim().is_empty()).unwrap_or(true);
    if unset {
        env.insert(key.to_string(), value.to_string());
    }
}

#[cfg(all(test, feature = "mesh-llm"))]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn native_provider_uses_context_safe_non_reasoning_budget() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("4096")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_THINKING_EFFORT").map(String::as_str),
            Some("none")
        );
        assert_eq!(
            env.get(RELAY_MESH_PREFER_MESH_FOR_AUTO_ENV)
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn apply_relay_mesh_env_preserves_user_max_output_tokens() {
        let mut env = BTreeMap::from([
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(), "1024".to_string()),
            ("BUZZ_AGENT_THINKING_EFFORT".to_string(), "low".to_string()),
        ]);
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );
        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("1024")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_THINKING_EFFORT").map(String::as_str),
            Some("low")
        );
    }
}
