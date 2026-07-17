use std::collections::BTreeMap;

use super::{RELAY_MESH_AUTO_MODEL_ID, RELAY_MESH_PROVIDER_ID};

/// Translate Buzz's provider-neutral/OpenAI-compatible settings into Goose's
/// native environment without changing the saved agent configuration.
///
/// Buzz historically used `OPENAI_COMPAT_*` for buzz-agent. Keeping those as
/// the UI/storage keys preserves existing personas while the bundled Goose
/// sidecar receives the names upstream Goose actually reads.
pub(crate) fn apply_goose_runtime_env(
    env: &mut BTreeMap<String, String>,
    effective_provider: Option<&str>,
    effective_model: Option<&str>,
) {
    copy_alias(env, "OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY");
    copy_alias(env, "OPENAI_COMPAT_BASE_URL", "OPENAI_BASE_URL");

    let provider = env
        .get("GOOSE_PROVIDER")
        .map(String::as_str)
        .or(effective_provider)
        .map(str::trim)
        .map(str::to_string);
    if matches!(
        provider.as_deref(),
        Some("openai-compat" | RELAY_MESH_PROVIDER_ID)
    ) {
        env.insert("GOOSE_PROVIDER".to_string(), "openai".to_string());
    }

    if provider.as_deref() == Some(RELAY_MESH_PROVIDER_ID) {
        let model = effective_model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(RELAY_MESH_AUTO_MODEL_ID);
        env.insert("GOOSE_MODEL".to_string(), model.to_string());
        env.insert("GOOSE_MAX_TOKENS".to_string(), "4096".to_string());
        env.insert("GOOSE_THINKING_EFFORT".to_string(), "none".to_string());
    }
}

fn copy_alias(env: &mut BTreeMap<String, String>, source: &str, target: &str) {
    let target_present = env
        .get(target)
        .is_some_and(|value| !value.trim().is_empty());
    if target_present {
        return;
    }
    if let Some(value) = env
        .get(source)
        .filter(|value| !value.trim().is_empty())
        .cloned()
    {
        env.insert(target.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_buzz_openai_compat_names_for_goose() {
        let mut env = BTreeMap::from([
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                "compat-key".to_string(),
            ),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "https://llm.example/v1".to_string(),
            ),
        ]);

        apply_goose_runtime_env(&mut env, Some("openai-compat"), Some("model"));

        assert_eq!(
            env.get("GOOSE_PROVIDER").map(String::as_str),
            Some("openai")
        );
        assert_eq!(
            env.get("OPENAI_API_KEY").map(String::as_str),
            Some("compat-key")
        );
        assert_eq!(
            env.get("OPENAI_BASE_URL").map(String::as_str),
            Some("https://llm.example/v1")
        );
    }

    #[test]
    fn native_goose_values_win_over_compat_aliases() {
        let mut env = BTreeMap::from([
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                "compat-key".to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "native-key".to_string()),
        ]);

        apply_goose_runtime_env(&mut env, Some("openai"), Some("model"));

        assert_eq!(
            env.get("OPENAI_API_KEY").map(String::as_str),
            Some("native-key")
        );
    }

    #[test]
    fn relay_mesh_becomes_a_bounded_goose_openai_runtime() {
        let mut env = BTreeMap::from([
            (
                "GOOSE_PROVIDER".to_string(),
                RELAY_MESH_PROVIDER_ID.to_string(),
            ),
            ("OPENAI_COMPAT_API_KEY".to_string(), "mesh-key".to_string()),
        ]);

        apply_goose_runtime_env(&mut env, Some(RELAY_MESH_PROVIDER_ID), Some("Qwen3"));

        assert_eq!(
            env.get("GOOSE_PROVIDER").map(String::as_str),
            Some("openai")
        );
        assert_eq!(env.get("GOOSE_MODEL").map(String::as_str), Some("Qwen3"));
        assert_eq!(
            env.get("GOOSE_MAX_TOKENS").map(String::as_str),
            Some("4096")
        );
        assert_eq!(
            env.get("GOOSE_THINKING_EFFORT").map(String::as_str),
            Some("none")
        );
        assert_eq!(
            env.get("OPENAI_API_KEY").map(String::as_str),
            Some("mesh-key")
        );
    }
}
