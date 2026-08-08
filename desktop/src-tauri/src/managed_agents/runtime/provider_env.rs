use std::collections::{BTreeMap, BTreeSet};

use crate::managed_agents::KnownAcpRuntime;

#[derive(Debug, Clone, Copy)]
struct RuntimeProviderEnvMapping {
    provider_id: &'static str,
    runtime_provider_id: &'static str,
    provider_url_env_var: Option<&'static str>,
    env_aliases: &'static [(&'static str, &'static str)],
}

const GOOSE_OPENAI_API_KEY_ALIASES: &[(&str, &str)] = &[
    ("OPENAI_COMPAT_API_KEY", "GOOSE_PROVIDER__API_KEY"),
    ("OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"),
];

const GOOSE_OPENAI_COMPAT_ENV_ALIASES: &[(&str, &str)] = &[
    ("OPENAI_COMPAT_API_KEY", "GOOSE_PROVIDER__API_KEY"),
    ("OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"),
    ("OPENAI_COMPAT_BASE_URL", "GOOSE_PROVIDER__HOST"),
    ("OPENAI_COMPAT_BASE_URL", "OPENAI_HOST"),
    ("OPENAI_COMPAT_BASE_URL", "OPENAI_BASE_URL"),
];

const GOOSE_PROVIDER_ENV_MAPPINGS: &[RuntimeProviderEnvMapping] = &[
    RuntimeProviderEnvMapping {
        provider_id: "openai",
        runtime_provider_id: "openai",
        provider_url_env_var: None,
        env_aliases: GOOSE_OPENAI_API_KEY_ALIASES,
    },
    RuntimeProviderEnvMapping {
        provider_id: "openai-compat",
        runtime_provider_id: "openai",
        provider_url_env_var: Some("OPENAI_COMPAT_BASE_URL"),
        env_aliases: GOOSE_OPENAI_COMPAT_ENV_ALIASES,
    },
];

fn provider_env_mappings(runtime: &KnownAcpRuntime) -> &'static [RuntimeProviderEnvMapping] {
    match runtime.id {
        "goose" => GOOSE_PROVIDER_ENV_MAPPINGS,
        _ => &[],
    }
}

const OPENAI_COMPAT_BASE_URL_KEYS: &[&str] = &[
    "OPENAI_COMPAT_BASE_URL",
    "GOOSE_PROVIDER__HOST",
    "OPENAI_HOST",
    "OPENAI_BASE_URL",
];

fn runtime_provider_env_mapping(
    runtime: &KnownAcpRuntime,
    configured_provider: &str,
) -> Option<(&'static RuntimeProviderEnvMapping, bool)> {
    let mappings = provider_env_mappings(runtime);
    if let Some(mapping) = mappings
        .iter()
        .find(|mapping| mapping.provider_id == configured_provider)
    {
        return Some((mapping, false));
    }

    provider_is_http_base_url(configured_provider)
        .then(|| {
            mappings
                .iter()
                .find(|mapping| mapping.provider_id == "openai-compat")
                .map(|mapping| (mapping, true))
        })
        .flatten()
}

/// Mirror canonical/runtime-native aliases inside one precedence layer.
///
/// Runtime-native values win only when both spellings occur in the same
/// layer. Normalizing before layers are merged makes a higher-layer value win
/// regardless of which spelling that layer uses.
fn normalize_runtime_provider_env_layer(
    runtime: &KnownAcpRuntime,
    configured_provider: Option<&str>,
    env: &mut BTreeMap<String, String>,
) {
    let Some(configured_provider) = configured_provider else {
        return;
    };
    let Some((mapping, _)) = runtime_provider_env_mapping(runtime, configured_provider) else {
        return;
    };

    let mut normalized_source_keys = BTreeSet::new();
    for (source_key, _) in mapping.env_aliases {
        if !normalized_source_keys.insert(*source_key) {
            continue;
        }
        let resolved_value = mapping
            .env_aliases
            .iter()
            .filter(|(candidate_source_key, _)| candidate_source_key == source_key)
            .find_map(|(_, runtime_key)| env.get(*runtime_key).cloned())
            .or_else(|| env.get(*source_key).cloned());
        if let Some(mut value) = resolved_value {
            if mapping.provider_url_env_var == Some(*source_key) {
                value = value.trim().to_string();
            }
            env.insert((*source_key).to_string(), value.clone());
            for (_, runtime_key) in mapping
                .env_aliases
                .iter()
                .filter(|(candidate_source_key, _)| candidate_source_key == source_key)
            {
                env.insert((*runtime_key).to_string(), value.clone());
            }
        }
    }
}

/// Merge filtered env layers while preserving alias-aware precedence, then
/// adapt Buzz's canonical provider configuration to the runtime's names.
///
/// `configured_provider` must come from the effective structured config. It is
/// deliberately not inferred from a layered `GOOSE_PROVIDER`/
/// `BUZZ_AGENT_PROVIDER`, because linked definitions are authoritative.
pub(crate) fn merge_runtime_provider_env_layers(
    runtime: &KnownAcpRuntime,
    configured_provider: Option<&str>,
    env: &mut BTreeMap<String, String>,
    layers: impl IntoIterator<Item = BTreeMap<String, String>>,
) {
    let configured_provider = configured_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty());
    normalize_runtime_provider_env_layer(runtime, configured_provider, env);
    for mut layer in layers {
        normalize_runtime_provider_env_layer(runtime, configured_provider, &mut layer);
        env.extend(layer);
    }
    apply_runtime_provider_env_mapping(runtime, configured_provider, env);
}

/// Apply the effective structured provider and its runtime-specific aliases.
pub(crate) fn apply_runtime_provider_env_mapping(
    runtime: &KnownAcpRuntime,
    configured_provider: Option<&str>,
    env: &mut BTreeMap<String, String>,
) {
    let Some(provider_env_var) = runtime.provider_env_var else {
        return;
    };
    let Some(configured_provider) = configured_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    else {
        return;
    };

    let Some((mapping, provider_is_url)) =
        runtime_provider_env_mapping(runtime, configured_provider)
    else {
        return;
    };

    if provider_is_url {
        if let Some(base_url_env_var) = mapping.provider_url_env_var {
            let provider_url = configured_provider.to_string();
            // Legacy v0.4.24 records stored the endpoint in `provider`. That
            // selected endpoint must beat any stale inherited base URL.
            env.insert(base_url_env_var.to_string(), provider_url.clone());
            for (source_key, runtime_key) in mapping.env_aliases {
                if *source_key == base_url_env_var {
                    env.insert((*runtime_key).to_string(), provider_url.clone());
                }
            }
        }
    } else if mapping.provider_id == "openai" {
        // The ordinary OpenAI provider must not inherit a compatibility host
        // from an earlier openai-compat selection.
        for key in OPENAI_COMPAT_BASE_URL_KEYS {
            env.remove(*key);
        }
    }
    normalize_runtime_provider_env_layer(runtime, Some(configured_provider), env);

    env.insert(
        provider_env_var.to_string(),
        mapping.runtime_provider_id.to_string(),
    );
}

pub(crate) fn validate_provider_value(provider: &str) -> Result<(), String> {
    let trimmed = provider.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if !normalized.starts_with("http://") && !normalized.starts_with("https://") {
        return Ok(());
    }
    validate_openai_compat_base_url(trimmed)
}

pub(crate) fn validate_openai_compat_base_url(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    let url = url::Url::parse(trimmed)
        .map_err(|_| "OpenAI-compatible base URL must be a valid HTTP(S) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("OpenAI-compatible base URL must include a valid HTTP(S) host".to_string());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "OpenAI-compatible base URL cannot include credentials, query parameters, or fragments"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn validate_provider_env_urls(
    env_vars: &BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(value) = env_vars
        .get("OPENAI_COMPAT_BASE_URL")
        .filter(|value| !value.trim().is_empty())
    {
        validate_openai_compat_base_url(value)?;
    }
    Ok(())
}

pub(crate) fn provider_is_http_base_url(provider: &str) -> bool {
    validate_openai_compat_base_url(provider).is_ok()
}

#[cfg(test)]
mod tests;
