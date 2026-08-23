//! Desktop-safe projection of `buzz-agent` provider metadata.

use serde::Serialize;

/// Secret metadata safe to expose over IPC. Credential values never appear in
/// the runtime catalog.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpProviderCredential {
    /// Environment key used by standalone and inherited configuration.
    pub env: String,
    /// Human-readable credential label.
    pub label: String,
    /// Whether Desktop stores the value in its device keyring.
    pub device_keyring: bool,
}

/// Provider configuration facts projected from `buzz-agent`'s Rust-owned
/// provider catalog. Only runtimes that consume these profiles expose rows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpProviderProfile {
    /// Canonical provider identifier.
    pub id: String,
    /// Human-readable picker label.
    pub label: String,
    /// Compatibility identifiers accepted by the provider runtime.
    pub aliases: Vec<String>,
    /// Provider-specific model environment key.
    pub model_env: String,
    /// Optional base-URL override environment key.
    pub base_url_env: Option<String>,
    /// Default provider API base URL.
    pub default_base_url: String,
    /// Credential metadata; never a credential value.
    pub credential: Option<AcpProviderCredential>,
    /// Environment keys required for provider readiness.
    pub required_env: Vec<String>,
    /// Whether this provider supports Buzz's reasoning-effort controls.
    pub supports_reasoning_effort: bool,
}

impl From<&buzz_agent_pkg::provider_profiles::ProviderProfile> for AcpProviderProfile {
    fn from(profile: &buzz_agent_pkg::provider_profiles::ProviderProfile) -> Self {
        Self {
            id: profile.id.to_string(),
            label: profile.label.to_string(),
            aliases: profile
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
            model_env: profile.model_env.to_string(),
            base_url_env: profile.base_url_env.map(str::to_string),
            default_base_url: profile.default_base_url.to_string(),
            credential: profile.credential.map(|credential| AcpProviderCredential {
                env: credential.env.to_string(),
                label: credential.label.to_string(),
                device_keyring: credential.device_keyring,
            }),
            required_env: profile
                .required_env
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            supports_reasoning_effort: profile.supports_reasoning_effort,
        }
    }
}

pub(crate) fn provider_profiles_for_runtime(runtime_id: &str) -> Vec<AcpProviderProfile> {
    buzz_agent_pkg::provider_profiles::PROVIDER_PROFILES
        .iter()
        .filter(|profile| runtime_id == "buzz-agent" && profile.supports_buzz_agent)
        .map(AcpProviderProfile::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_runtime_scoped_and_secret_free() {
        let buzz = provider_profiles_for_runtime("buzz-agent");
        assert!(buzz.iter().any(|profile| profile.id == "ollama"));
        let hugging_face = buzz
            .iter()
            .find(|profile| profile.id == "huggingface")
            .expect("Hugging Face profile");
        assert_eq!(hugging_face.required_env, vec!["HF_TOKEN"]);
        assert!(hugging_face
            .credential
            .as_ref()
            .is_some_and(|credential| credential.device_keyring));
        let wire = serde_json::to_string(hugging_face).expect("profile serializes");
        assert!(!wire.contains("secret"));
        assert!(provider_profiles_for_runtime("goose").is_empty());
        assert!(provider_profiles_for_runtime("claude").is_empty());
    }
}
