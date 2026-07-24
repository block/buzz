use std::collections::BTreeMap;

use crate::managed_agents::{KnownAcpRuntime, NativeModelDiscovery};

const LMSTUDIO_CATALOG_OWNED_ENV_KEYS: &[&str] = &[
    "BUZZ_AGENT_CLASSIFICATION",
    "BUZZ_AGENT_PROVIDER",
    "LM_STUDIO_MODEL",
    "LM_STUDIO_BASE_URL",
    "LM_STUDIO_MCP_INTEGRATIONS",
    "LM_STUDIO_FALLBACK_PROVIDER",
    "LM_STUDIO_API_TOKEN",
];

/// Environment keys that must be explicitly removed from the inherited
/// desktop process environment before the trusted runtime projection is
/// applied to a child process.
pub(crate) fn runtime_inherited_env_keys_to_remove(
    runtime: Option<&KnownAcpRuntime>,
) -> &'static [&'static str] {
    if runtime.is_some_and(|runtime| {
        runtime.native_model_discovery == Some(NativeModelDiscovery::LmStudioV1)
    }) {
        LMSTUDIO_CATALOG_OWNED_ENV_KEYS
    } else {
        &[]
    }
}

/// Replaces all LM Studio-native security policy keys with catalog-owned
/// values after every user-configurable environment layer.
///
/// `LM_STUDIO_BASE_URL` and `LM_STUDIO_MCP_INTEGRATIONS` may be supplied only
/// by the trusted desktop process environment. The native Rust egress parser
/// validates both before any network request. The API token is intentionally
/// excluded here and loaded separately from the OS Keychain.
pub(crate) fn apply_runtime_security_env(
    env: &mut BTreeMap<String, String>,
    runtime: Option<&KnownAcpRuntime>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.native_model_discovery != Some(NativeModelDiscovery::LmStudioV1) {
        return;
    }

    for key in LMSTUDIO_CATALOG_OWNED_ENV_KEYS
        .iter()
        .copied()
        .filter(|key| *key != "LM_STUDIO_MODEL")
    {
        env.remove(key);
    }

    if let Some(key) = runtime.classification_env_var {
        env.insert(key.to_string(), "OFFICIAL".to_string());
    }
    if let (Some(key), Some(provider)) = (runtime.provider_env_var, runtime.locked_provider_id) {
        env.insert(key.to_string(), provider.to_string());
    }
    if let Some(key) = runtime.base_url_env_var {
        let base_url = std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:1234".to_string());
        env.insert(key.to_string(), base_url);
    }
    if let Some(key) = runtime.integrations_env_var {
        let integrations = std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "[]".to_string());
        env.insert(key.to_string(), integrations);
    }
}
