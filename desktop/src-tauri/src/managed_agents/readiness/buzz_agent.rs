//! Catalog-driven readiness requirements for the bundled Buzz Agent.

use super::{EffectiveAgentEnv, Requirement};

pub(super) fn requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let mut missing = Vec::new();

    #[cfg(windows)]
    if !crate::managed_agents::git_bash_available(&effective.env) {
        missing.push(Requirement::GitBash);
    }

    let provider = effective
        .env
        .get("BUZZ_AGENT_PROVIDER")
        .filter(|value| !value.is_empty())
        .map(String::as_str);
    if provider.is_none() {
        missing.push(Requirement::NormalizedField {
            field: "provider".to_string(),
        });
    }

    let profile = provider.and_then(buzz_agent_pkg::provider_profiles::provider_profile);
    let model_present = effective
        .env
        .get("BUZZ_AGENT_MODEL")
        .filter(|value| !value.is_empty())
        .is_some()
        || profile
            .and_then(|profile| effective.env.get(profile.model_env))
            .filter(|value| !value.is_empty())
            .is_some();
    if !model_present {
        missing.push(Requirement::NormalizedField {
            field: "model".to_string(),
        });
    }

    if let Some(profile) = profile {
        for key in profile.required_env {
            let env_present = effective
                .env
                .get(*key)
                .is_some_and(|value| !value.is_empty());
            let device_secret_present = profile
                .credential
                .filter(|credential| credential.device_keyring && credential.env == *key)
                .is_some_and(|_| {
                    crate::commands::load_provider_secret(profile.id)
                        .ok()
                        .flatten()
                        .is_some()
                });
            if !env_present && !device_secret_present {
                missing.push(Requirement::EnvKey {
                    key: (*key).to_string(),
                });
            }
        }
    }

    missing
}
