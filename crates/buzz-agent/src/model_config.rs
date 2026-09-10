use anyhow::{bail, Result};
use goose_provider_types::thinking::ThinkingEffort;
use goose_providers::model::ModelConfig;
use std::collections::HashMap;

pub fn from_env(provider_name: &str, model_name: &str) -> Result<ModelConfig> {
    let temperature = parse_optional::<f32>("GOOSE_TEMPERATURE")?;
    if temperature.is_some_and(|value| value < 0.0) {
        bail!("GOOSE_TEMPERATURE must be non-negative");
    }
    let max_tokens = parse_optional::<i32>("GOOSE_MAX_TOKENS")?;
    if max_tokens.is_some_and(|value| value <= 0) {
        bail!("GOOSE_MAX_TOKENS must be greater than zero");
    }
    let toolshim = optional_env("GOOSE_TOOLSHIM")
        .map(|value| parse_bool("GOOSE_TOOLSHIM", &value))
        .transpose()?
        .unwrap_or(false);
    let toolshim_model = optional_env("GOOSE_TOOLSHIM_OLLAMA_MODEL");
    let mut model = ModelConfig::new(model_name)
        .with_temperature(temperature)
        .with_max_tokens(max_tokens)
        .with_toolshim(toolshim)
        .with_toolshim_model(toolshim_model);
    if let Some(effort) = optional_env("GOOSE_THINKING_EFFORT") {
        model = model.with_default_thinking_effort(Some(
            effort
                .parse::<ThinkingEffort>()
                .map_err(anyhow::Error::msg)?,
        ));
    }
    if let Some(ttl) = optional_env("GOOSE_CACHE_TTL") {
        let ttl = ttl.to_ascii_lowercase();
        if !matches!(ttl.as_str(), "5m" | "1h") {
            bail!("GOOSE_CACHE_TTL must be '5m' or '1h'");
        }
        model = model.with_cache_ttl(&ttl);
    }
    if provider_name == "openai" {
        if let Some(store) = optional_env("OPENAI_STORE") {
            model = model.with_merged_request_params(HashMap::from([(
                "store".to_string(),
                serde_json::Value::Bool(parse_bool("OPENAI_STORE", &store)?),
            )]));
        }
    }
    Ok(model.with_canonical_limits(provider_name))
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_optional<T: std::str::FromStr>(key: &str) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    optional_env(key)
        .map(|value| {
            value
                .parse()
                .map_err(|error| anyhow::anyhow!("invalid {key} {value:?}: {error}"))
        })
        .transpose()
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("invalid {key} {value:?}"),
    }
}
