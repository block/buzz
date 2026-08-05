use std::collections::BTreeMap;

use serde::Deserialize;

use crate::managed_agents::{AgentModelInfo, AgentModelsResponse};

#[cfg(test)]
use super::env_value;
use super::{env_or_process_value, redaction_env_with_value, DiscoveryProvider};

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct VeniceModelListResponse {
    pub data: Vec<VeniceModelListItem>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct VeniceModelListItem {
    pub id: String,
    pub model_spec: VeniceModelSpec,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(Clone))]
pub(super) struct VeniceModelSpec {
    #[serde(default)]
    pub capabilities: Option<VeniceModelCapabilities>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(Clone))]
pub(super) struct VeniceModelCapabilities {
    #[serde(default)]
    pub supports_function_calling: bool,
}

pub(super) fn is_venice_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("venice")
    )
}

#[cfg(test)]
pub(super) fn venice_models_url(env: &BTreeMap<String, String>) -> String {
    let base_url = env_value(env, "VENICE_BASE_URL")
        .unwrap_or_else(|| "https://api.venice.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn venice_models_url_for_discovery(env: &BTreeMap<String, String>) -> String {
    let base_url = env_or_process_value(env, "VENICE_BASE_URL")
        .unwrap_or_else(|| "https://api.venice.ai/api/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

pub(super) async fn discover_venice_models(
    client: &reqwest::Client,
    provider: &DiscoveryProvider,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    if !is_venice_provider(provider.as_deref()) {
        return Ok(None);
    }

    let api_key = match provider.required_env(env, "VENICE_API_KEY")? {
        Some(api_key) => api_key,
        None => return Ok(None),
    };
    let redaction_env = redaction_env_with_value(env, "VENICE_API_KEY", &api_key);
    let url = venice_models_url_for_discovery(env);
    let response = client
        .get(&url)
        .query(&[("type", "text")])
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|error| format!("Venice model discovery request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = crate::managed_agents::redact_env_values_in(&body, &redaction_env);
        return Err(format!("Venice model discovery HTTP {status}: {body}"));
    }

    let response = response
        .json::<VeniceModelListResponse>()
        .await
        .map_err(|error| format!("Venice model discovery response parse failed: {error}"))?;
    filter_venice_models(response, selected_model)
}

pub(super) fn filter_venice_models(
    response: VeniceModelListResponse,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let models = response
        .data
        .into_iter()
        .filter(|model| {
            model
                .model_spec
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.supports_function_calling)
        })
        .map(|model| AgentModelInfo {
            name: Some(model.id.clone()),
            id: model.id,
            description: model.model_spec.description,
        })
        .collect::<Vec<_>>();

    if models.is_empty() {
        return Err("Venice model discovery returned no tools-capable models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: "venice".to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_matches_case_insensitively() {
        assert!(is_venice_provider(Some("venice")));
        assert!(is_venice_provider(Some("  Venice  ")));
        assert!(!is_venice_provider(Some("openai-compat")));
        assert!(!is_venice_provider(None));
    }

    #[test]
    fn models_url_uses_default_and_custom_base_urls() {
        assert_eq!(
            venice_models_url(&BTreeMap::new()),
            "https://api.venice.ai/api/v1/models"
        );
        let env = BTreeMap::from([(
            "VENICE_BASE_URL".to_string(),
            "https://venice-proxy.example/v1/".to_string(),
        )]);
        assert_eq!(
            venice_models_url(&env),
            "https://venice-proxy.example/v1/models"
        );
    }

    #[test]
    fn filter_keeps_only_function_calling_models() {
        let response = VeniceModelListResponse {
            data: vec![
                VeniceModelListItem {
                    id: "zai-org-glm-5".to_string(),
                    model_spec: VeniceModelSpec {
                        capabilities: Some(VeniceModelCapabilities {
                            supports_function_calling: true,
                        }),
                        description: Some("Tools-capable model".to_string()),
                    },
                },
                VeniceModelListItem {
                    id: "venice-text-only".to_string(),
                    model_spec: VeniceModelSpec {
                        capabilities: Some(VeniceModelCapabilities {
                            supports_function_calling: false,
                        }),
                        description: None,
                    },
                },
            ],
        };
        let result = filter_venice_models(response, Some("zai-org-glm-5".to_string()))
            .expect("catalog should normalize")
            .expect("Venice provider should return a catalog");
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].id, "zai-org-glm-5");
        assert_eq!(
            result.models[0].description.as_deref(),
            Some("Tools-capable model")
        );
        assert_eq!(result.selected_model.as_deref(), Some("zai-org-glm-5"));
    }

    #[test]
    fn filter_rejects_catalog_without_function_calling_models() {
        let response = VeniceModelListResponse {
            data: vec![VeniceModelListItem {
                id: "venice-text-only".to_string(),
                model_spec: VeniceModelSpec {
                    capabilities: None,
                    description: None,
                },
            }],
        };
        let error = filter_venice_models(response, None).expect_err("catalog must be rejected");
        assert!(error.contains("no tools-capable models"));
    }
}
