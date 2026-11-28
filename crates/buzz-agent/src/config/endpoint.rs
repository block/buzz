//! Endpoint resolution shared by agent startup and desktop security suggestions.
use super::Provider;

pub fn provider_base_url(
    provider: &Provider,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let (key, default) = match provider {
        Provider::Anthropic => ("ANTHROPIC_BASE_URL", Some("https://api.anthropic.com")),
        Provider::OpenAi => ("OPENAI_COMPAT_BASE_URL", Some("https://api.openai.com/v1")),
        Provider::OpenRouter => ("OPENROUTER_BASE_URL", Some("https://openrouter.ai/api/v1")),
        Provider::Databricks | Provider::DatabricksV2 => ("DATABRICKS_HOST", None),
    };
    lookup(key)
        .or_else(|| default.map(str::to_owned))
        .ok_or_else(|| format!("config: {key} required"))
}

/// Resolve without fetching credentials, signing in, or making network requests.
pub fn configured_endpoint(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let provider = super::resolve_provider(
        lookup("BUZZ_AGENT_PROVIDER").as_deref(),
        Some("configured"),
        Some("configured"),
        Some("configured"),
    )
    .ok()?;
    provider_base_url(&provider, lookup).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_follow_selected_provider_and_overrides() {
        for (provider, expected) in [
            ("anthropic", "https://api.anthropic.com"),
            ("openai-compat", "https://api.openai.com/v1"),
            ("openrouter", "https://openrouter.ai/api/v1"),
        ] {
            assert_eq!(
                configured_endpoint(|key| (key == "BUZZ_AGENT_PROVIDER").then(|| provider.into()))
                    .as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            configured_endpoint(|key| match key {
                "BUZZ_AGENT_PROVIDER" => Some("databricks_v2".into()),
                "DATABRICKS_HOST" => Some("https://workspace.example.com".into()),
                _ => None,
            })
            .as_deref(),
            Some("https://workspace.example.com")
        );
        assert!(configured_endpoint(|_| None).is_none());
        assert_eq!(
            provider_base_url(&Provider::OpenAi, |_| Some(
                "http://localhost:1234/v1".into()
            ))
            .unwrap(),
            "http://localhost:1234/v1"
        );
    }
}
