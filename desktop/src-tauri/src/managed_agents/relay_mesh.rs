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
    // Keep the requested response inside smaller local-model context windows.
    // These are defaults, not policy: the effective agent/persona/global env
    // may deliberately choose a smaller cap or a different effort. This function
    // runs after those layers during readiness, so never clobber their values.
    insert_default_if_unset(env, "BUZZ_AGENT_MAX_OUTPUT_TOKENS", "4096");
    // Deliberately no BUZZ_AGENT_THINKING_EFFORT default: mesh translates
    // `reasoning_effort` into the chat template's `enable_thinking` flag, so any
    // value we pick overrides each model's own template default — and the right
    // value is model-specific. Measured with the real prompt and toolset:
    // gemma-4-E4B delivers 0/8 at `none` but 6/6 with the field absent, while
    // Qwen3-8B delivers 8/8 either way and burns ~4x the output tokens once
    // thinking is on (121 -> ~470), risking the 4096 cap. Omitting the field
    // lets every model use its own default; explicit agent/persona/global
    // values still apply.
}

#[cfg(feature = "mesh-llm")]
fn insert_default_if_unset(
    env: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    value: &str,
) {
    if env.get(key).is_none_or(|current| current.trim().is_empty()) {
        env.insert(key.to_string(), value.to_string());
    }
}

/// Build the final Mesh-specific process overrides from the already-resolved
/// harness environment. Only user-owned generation controls are seeded: the
/// derived provider/base URL/model values remain authoritative, and unrelated
/// credentials (notably `OPENAI_API_KEY`) must not be copied back after the
/// spawn path removes them.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_process_env(
    effective_env: &std::collections::BTreeMap<String, String>,
    model: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    for key in ["BUZZ_AGENT_MAX_OUTPUT_TOKENS", "BUZZ_AGENT_THINKING_EFFORT"] {
        if let Some(value) = effective_env.get(key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    apply_relay_mesh_env(&mut env, Some(RELAY_MESH_PROVIDER_ID), Some(model));
    env
}

#[cfg(all(test, feature = "mesh-llm"))]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn native_provider_uses_context_safe_tool_calling_budget() {
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
        // Must stay unset: any value we pick overrides the model's own chat
        // template default, and the right value is model-specific ("none"
        // stops gemma tool-calling; enabling thinking makes Qwen3 burn ~4x the
        // output budget).
        assert_eq!(env.get("BUZZ_AGENT_THINKING_EFFORT"), None);
        assert_eq!(
            env.get(RELAY_MESH_PREFER_MESH_FOR_AUTO_ENV)
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn native_provider_preserves_explicit_generation_controls() {
        let mut env = BTreeMap::from([
            (
                "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
                "2048".to_string(),
            ),
            ("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string()),
        ]);
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("2048")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_THINKING_EFFORT").map(String::as_str),
            Some("high")
        );
    }

    #[test]
    fn process_env_seeds_controls_without_restoring_unrelated_credentials() {
        let effective_env = BTreeMap::from([
            (
                "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
                "1024".to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "must-not-leak".to_string()),
        ]);

        let env = relay_mesh_process_env(&effective_env, "Gemma-4");

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("1024")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_MODEL").map(String::as_str),
            Some("Gemma-4")
        );
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }
}
