//! Declarative metadata for LLM providers supported by `buzz-agent`.
//!
//! Provider profiles describe configuration and wire-format differences while
//! [`crate::config::Provider`] remains the small transport enum used by the
//! request dispatcher. Desktop consumes this catalog as well, keeping provider
//! labels, credentials, defaults, and readiness rules in one Rust-owned place.

/// HTTP transport used by a provider profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTransport {
    /// Anthropic Messages API.
    Anthropic,
    /// OpenAI Responses or Chat Completions API.
    OpenAi,
    /// Legacy Databricks serving endpoints.
    Databricks,
    /// Databricks AI Gateway v2.
    DatabricksV2,
    /// OpenRouter's Chat Completions API.
    OpenRouter,
}

/// Name of the output-token field used by Chat Completions requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatTokenLimitField {
    /// OpenAI-native `max_completion_tokens`.
    MaxCompletionTokens,
    /// Broadly compatible `max_tokens` spelling.
    MaxTokens,
}

/// Default API selection for profiles using the OpenAI transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiApiDefault {
    /// Select Responses for official OpenAI hosts and Chat elsewhere.
    Auto,
    /// Use Chat Completions unless explicitly overridden.
    Chat,
}

/// Credential metadata safe to expose to the desktop UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCredential {
    /// Environment variable accepted by standalone/headless deployments.
    pub env: &'static str,
    /// Human-readable input label.
    pub label: &'static str,
    /// Whether Desktop stores this credential in the OS keyring.
    pub device_keyring: bool,
}

/// One named provider configuration profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfile {
    /// Canonical value accepted by `BUZZ_AGENT_PROVIDER`.
    pub id: &'static str,
    /// Human-readable provider name.
    pub label: &'static str,
    /// Alternative provider IDs accepted for compatibility.
    pub aliases: &'static [&'static str],
    /// HTTP transport used by the provider.
    pub transport: ProviderTransport,
    /// Provider-specific default model environment variable.
    pub model_env: &'static str,
    /// Provider-specific base URL environment variable, when configurable.
    pub base_url_env: Option<&'static str>,
    /// Base URL used when no override is supplied.
    pub default_base_url: &'static str,
    /// Required credential, if any.
    pub credential: Option<ProviderCredential>,
    /// Environment keys required by the provider. Device-keyring credentials
    /// are included so Desktop can satisfy the requirement from secure storage.
    pub required_env: &'static [&'static str],
    /// Chat token-limit spelling.
    pub chat_token_limit: ChatTokenLimitField,
    /// OpenAI-family API default.
    pub openai_api_default: OpenAiApiDefault,
    /// Whether reasoning-effort fields are verified for this provider.
    pub supports_reasoning_effort: bool,
    /// Whether the profile is offered for Buzz Agent.
    pub supports_buzz_agent: bool,
}

const fn env_secret(env: &'static str, label: &'static str) -> ProviderCredential {
    ProviderCredential {
        env,
        label,
        device_keyring: false,
    }
}

/// Complete provider catalog. Ordering is the Desktop picker ordering.
pub const PROVIDER_PROFILES: &[ProviderProfile] = &[
    ProviderProfile {
        id: "anthropic",
        label: "Anthropic",
        aliases: &[],
        transport: ProviderTransport::Anthropic,
        model_env: "ANTHROPIC_MODEL",
        base_url_env: Some("ANTHROPIC_BASE_URL"),
        default_base_url: "https://api.anthropic.com",
        credential: Some(env_secret("ANTHROPIC_API_KEY", "Anthropic API Key")),
        required_env: &["ANTHROPIC_API_KEY"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Auto,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "openai",
        label: "OpenAI",
        aliases: &[],
        transport: ProviderTransport::OpenAi,
        model_env: "OPENAI_COMPAT_MODEL",
        base_url_env: Some("OPENAI_COMPAT_BASE_URL"),
        default_base_url: "https://api.openai.com/v1",
        credential: Some(env_secret(
            "OPENAI_COMPAT_API_KEY",
            "OpenAI Runtime API Key",
        )),
        required_env: &["OPENAI_COMPAT_API_KEY"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Auto,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "openai-compat",
        label: "OpenAI-compatible",
        aliases: &[],
        transport: ProviderTransport::OpenAi,
        model_env: "OPENAI_COMPAT_MODEL",
        base_url_env: Some("OPENAI_COMPAT_BASE_URL"),
        default_base_url: "https://api.openai.com/v1",
        credential: Some(env_secret(
            "OPENAI_COMPAT_API_KEY",
            "OpenAI-compatible Runtime API Key",
        )),
        required_env: &["OPENAI_COMPAT_API_KEY"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Auto,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "openrouter",
        label: "OpenRouter",
        aliases: &[],
        transport: ProviderTransport::OpenRouter,
        model_env: "OPENROUTER_MODEL",
        base_url_env: Some("OPENROUTER_BASE_URL"),
        default_base_url: "https://openrouter.ai/api/v1",
        credential: Some(env_secret("OPENROUTER_API_KEY", "OpenRouter API Key")),
        required_env: &["OPENROUTER_API_KEY"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Chat,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "ollama",
        label: "Ollama",
        aliases: &[],
        transport: ProviderTransport::OpenAi,
        model_env: "OLLAMA_MODEL",
        base_url_env: Some("OLLAMA_BASE_URL"),
        default_base_url: "http://127.0.0.1:11434/v1",
        credential: None,
        required_env: &[],
        chat_token_limit: ChatTokenLimitField::MaxTokens,
        openai_api_default: OpenAiApiDefault::Chat,
        supports_reasoning_effort: false,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "huggingface",
        label: "Hugging Face",
        aliases: &["hugging-face", "hf"],
        transport: ProviderTransport::OpenAi,
        model_env: "HUGGINGFACE_MODEL",
        base_url_env: Some("HF_INFERENCE_BASE_URL"),
        default_base_url: "https://router.huggingface.co/v1",
        credential: Some(ProviderCredential {
            env: "HF_TOKEN",
            label: "Hugging Face Token",
            device_keyring: true,
        }),
        required_env: &["HF_TOKEN"],
        chat_token_limit: ChatTokenLimitField::MaxTokens,
        openai_api_default: OpenAiApiDefault::Chat,
        supports_reasoning_effort: false,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "databricks",
        label: "Databricks",
        aliases: &[],
        transport: ProviderTransport::Databricks,
        model_env: "DATABRICKS_MODEL",
        base_url_env: Some("DATABRICKS_HOST"),
        default_base_url: "",
        credential: None,
        required_env: &["DATABRICKS_HOST"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Chat,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
    ProviderProfile {
        id: "databricks_v2",
        label: "Databricks v2",
        aliases: &["databricks-v2"],
        transport: ProviderTransport::DatabricksV2,
        model_env: "DATABRICKS_MODEL",
        base_url_env: Some("DATABRICKS_HOST"),
        default_base_url: "",
        credential: None,
        required_env: &["DATABRICKS_HOST"],
        chat_token_limit: ChatTokenLimitField::MaxCompletionTokens,
        openai_api_default: OpenAiApiDefault::Chat,
        supports_reasoning_effort: true,
        supports_buzz_agent: true,
    },
];

/// Resolve a provider ID or alias case-insensitively.
pub fn provider_profile(id: &str) -> Option<&'static ProviderProfile> {
    let normalized = id.trim().to_ascii_lowercase();
    PROVIDER_PROFILES.iter().find(|profile| {
        profile.id == normalized || profile.aliases.iter().any(|alias| *alias == normalized)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_canonical_profiles() {
        assert_eq!(
            provider_profile(" openai-compat ").map(|p| p.id),
            Some("openai-compat")
        );
        assert_eq!(provider_profile("HF").map(|p| p.id), Some("huggingface"));
        assert_eq!(
            provider_profile("databricks-v2").map(|p| p.id),
            Some("databricks_v2")
        );
    }

    #[test]
    fn local_profiles_use_chat_compatible_token_spelling() {
        for id in ["ollama", "huggingface"] {
            let profile = provider_profile(id).expect("profile");
            assert_eq!(profile.transport, ProviderTransport::OpenAi);
            assert_eq!(profile.openai_api_default, OpenAiApiDefault::Chat);
            assert_eq!(profile.chat_token_limit, ChatTokenLimitField::MaxTokens);
            assert!(!profile.supports_reasoning_effort);
        }
    }
}
