use std::collections::BTreeMap;

use crate::managed_agents::{KnownAcpRuntime, NativeModelDiscovery};

const DEFAULT_LMSTUDIO_BASE_URL: &str = "http://127.0.0.1:1234";
const LMSTUDIO_CATALOG_OWNED_ENV_KEYS: &[&str] = &[
    "BUZZ_AGENT_CLASSIFICATION",
    "BUZZ_AGENT_PROVIDER",
    "LM_STUDIO_MODEL",
    "LM_STUDIO_BASE_URL",
    "LM_STUDIO_MCP_INTEGRATIONS",
    "LM_STUDIO_COMMAND_EVIDENCE_POLICY",
    "LM_STUDIO_FALLBACK_PROVIDER",
    "LM_STUDIO_API_TOKEN",
];

pub(crate) struct TrustedLmStudioRuntimeFacts {
    pub(crate) endpoint: String,
    pub(crate) api_token: Option<String>,
}

fn catalog_base_url(runtime: &KnownAcpRuntime) -> String {
    runtime
        .base_url_env_var
        .and_then(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LMSTUDIO_BASE_URL.to_string())
}

pub(crate) fn trusted_lmstudio_runtime_facts() -> Result<TrustedLmStudioRuntimeFacts, &'static str>
{
    let runtime = crate::managed_agents::known_acp_runtime_exact("buzz-lmstudio-agent")
        .filter(|runtime| runtime.native_model_discovery == Some(NativeModelDiscovery::LmStudioV1))
        .ok_or("LM Studio runtime catalog is unavailable")?;
    let api_token = match runtime.keychain_token_key {
        Some(key) => crate::secret_store::SecretStore::shared(crate::app_state::keyring_service())
            .load(key)
            .map_err(|_| "LM Studio Keychain credential is unavailable")?,
        None => None,
    };
    Ok(TrustedLmStudioRuntimeFacts {
        endpoint: catalog_base_url(runtime),
        api_token,
    })
}

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
/// `LM_STUDIO_BASE_URL` is projected from the trusted desktop process
/// environment. `LM_STUDIO_MCP_INTEGRATIONS` is never read from ambient or
/// user-controlled layers: it is built from the short-lived native
/// Memory/RAG admission cache. The API token is intentionally excluded here
/// and loaded separately from the OS Keychain.
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
        env.insert(key.to_string(), catalog_base_url(runtime));
    }
    if let Some(key) = runtime.integrations_env_var {
        let integrations = crate::command_services::policy::catalog_integrations_json(
            crate::command_services::policy::CommandKnowledgeWorkflow::Adviser,
        );
        env.insert(key.to_string(), integrations);
    }
    if let Some(policy) = crate::command_services::policy::catalog_evidence_policy_json() {
        env.insert("LM_STUDIO_COMMAND_EVIDENCE_POLICY".to_string(), policy);
    }
}
