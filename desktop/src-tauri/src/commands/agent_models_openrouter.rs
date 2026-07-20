use std::collections::BTreeMap;

use serde::Deserialize;

use crate::managed_agents::{AgentModelInfo, AgentModelsResponse};

#[cfg(test)]
use super::env_value;
use super::{env_or_process_value, redaction_env_with_value};

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct OpenRouterModelListResponse {
    pub data: Vec<OpenRouterModelListItem>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct OpenRouterModelListItem {
    pub id: String,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
}

pub(super) fn is_openrouter_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openrouter")
    )
}

#[cfg(test)]
pub(super) fn openrouter_models_url(env: &BTreeMap<String, String>) -> String {
    let base_url = env_value(env, "OPENROUTER_BASE_URL")
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn openrouter_models_url_for_discovery(env: &BTreeMap<String, String>) -> String {
    let base_url = env_or_process_value(env, "OPENROUTER_BASE_URL")
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

pub(super) async fn discover_openrouter_models(
    client: &reqwest::Client,
    provider: Option<&str>,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    if !is_openrouter_provider(provider) {
        return Ok(None);
    }

    let api_key = env_or_process_value(env, "OPENROUTER_API_KEY")
        .ok_or_else(|| "config: OPENROUTER_API_KEY required".to_string())?;
    let redaction_env = redaction_env_with_value(env, "OPENROUTER_API_KEY", &api_key);
    let url = openrouter_models_url_for_discovery(env);
    let response = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|error| format!("OpenRouter model discovery request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = crate::managed_agents::redact_env_values_in(&body, &redaction_env);
        return Err(format!("OpenRouter model discovery HTTP {status}: {body}"));
    }

    let response = response
        .json::<OpenRouterModelListResponse>()
        .await
        .map_err(|error| format!("OpenRouter model discovery response parse failed: {error}"))?;

    filter_openrouter_models(response, selected_model)
}

pub(super) fn filter_openrouter_models(
    response: OpenRouterModelListResponse,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let models: Vec<AgentModelInfo> = response
        .data
        .into_iter()
        .filter(|m| m.supported_parameters.iter().any(|p| p == "tools"))
        .map(|m| AgentModelInfo {
            id: m.id.clone(),
            name: Some(m.id),
            description: None,
        })
        .collect();

    if models.is_empty() {
        return Err("OpenRouter model discovery returned no tools-capable models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: "openrouter".to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}
