use super::{EffectiveAgentEnv, Requirement};

/// Requirements for buzz-agent (provider + model + provider-specific creds).
pub(super) fn buzz_agent_requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let mut missing = Vec::new();

    #[cfg(windows)]
    if !crate::managed_agents::git_bash_available(&effective.env) {
        missing.push(Requirement::GitBash);
    }

    // Provider is required — maps to BUZZ_AGENT_PROVIDER in the effective env.
    // An empty string is treated as absent: a key set to "" is not a valid
    // provider and must not pass the readiness gate.
    let provider = effective
        .env
        .get("BUZZ_AGENT_PROVIDER")
        .filter(|v| !v.is_empty())
        .map(String::as_str);
    if provider.is_none() {
        missing.push(Requirement::NormalizedField {
            field: "provider".to_string(),
        });
    }

    // Also accept provider-specific model fallbacks, matching buzz-agent's
    // Config::from_env order. MiniMax alone has a built-in model default.
    let provider_model_key = match provider {
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2") => {
            Some("DATABRICKS_MODEL")
        }
        Some("anthropic") => Some("ANTHROPIC_MODEL"),
        Some("openai") | Some("openai-compat") => Some("OPENAI_COMPAT_MODEL"),
        Some("openrouter") => Some("OPENROUTER_MODEL"),
        Some("minimax") | Some("minimax-cn") | Some("minimaxi") | Some("minimax_cn") => {
            Some("MINIMAX_MODEL")
        }
        _ => None,
    };
    let provider_has_default_model = matches!(
        provider,
        Some("minimax" | "minimax-cn" | "minimaxi" | "minimax_cn")
    );
    let model_present = effective
        .env
        .get("BUZZ_AGENT_MODEL")
        .filter(|v| !v.is_empty())
        .is_some()
        || provider_model_key
            .and_then(|k| effective.env.get(k))
            .filter(|v| !v.is_empty())
            .is_some();
    if !model_present && !provider_has_default_model {
        missing.push(Requirement::NormalizedField {
            field: "model".to_string(),
        });
    }

    // Provider-specific credential requirements.
    // A key present with an empty value is treated as absent — matching the
    // dialog's (envVars[key] ?? "").length === 0 emptiness check.
    let env_key_missing = |key: &str| effective.env.get(key).is_none_or(|v| v.is_empty());
    match provider {
        Some("anthropic")
            if env_key_missing("ANTHROPIC_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "ANTHROPIC_API_KEY".to_string(),
                });
            }
        Some("openai")
            if env_key_missing("OPENAI_COMPAT_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "OPENAI_COMPAT_API_KEY".to_string(),
                });
            }
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2")
            // DATABRICKS_HOST is hard-required; token optional (OAuth fallback).
            if env_key_missing("DATABRICKS_HOST") => {
                missing.push(Requirement::EnvKey {
                    key: "DATABRICKS_HOST".to_string(),
                });
            }
        Some("openrouter")
            if env_key_missing("OPENROUTER_API_KEY") => {
                missing.push(Requirement::EnvKey {
                    key: "OPENROUTER_API_KEY".to_string(),
                });
            }
        Some("minimax") | Some("minimax-cn") | Some("minimaxi") | Some("minimax_cn")
            if env_key_missing("MINIMAX_API_KEY") => {
                missing.push(Requirement::EnvKey { key: "MINIMAX_API_KEY".to_string() });
            }
        _ => {
            // Unknown provider or no provider yet — only the NormalizedField
            // requirement above captures this gap.
        }
    }

    missing
}
