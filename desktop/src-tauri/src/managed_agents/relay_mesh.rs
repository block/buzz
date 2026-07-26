pub const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
pub const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";
pub const RELAY_MESH_PROVIDER_ID: &str = "relay-mesh";
pub const RELAY_MESH_AUTO_MODEL_ID: &str = "auto";

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
    // Keep the requested response inside smaller local-model context windows,
    // and spend that budget on an answer/tool call instead of hidden reasoning.
    // Without both settings Qwen3 either fails the router's fit check at the
    // agent default (32K) or can consume a tight cap before serializing a tool.
    env.insert(
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
        "4096".to_string(),
    );
    env.insert("BUZZ_AGENT_THINKING_EFFORT".to_string(), "none".to_string());
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
    }
}
