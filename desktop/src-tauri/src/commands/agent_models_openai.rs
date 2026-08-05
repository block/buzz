use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use crate::managed_agents::{AgentModelInfo, AgentModelsResponse};

#[cfg(test)]
use super::env_value;
use super::{env_or_process_value, redaction_env_with_value, DiscoveryProvider};

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiModelListResponse {
    pub data: Vec<OpenAiModelListItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiModelListItem {
    pub id: String,
    #[serde(default)]
    pub created: Option<i64>,
}

pub(super) fn is_openai_compatible_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openai" | "openai-compat")
    )
}

#[cfg(test)]
pub(super) fn openai_compatible_models_url(env: &BTreeMap<String, String>) -> String {
    let base_url = env_value(env, "OPENAI_COMPAT_BASE_URL")
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn openai_compatible_models_url_for_discovery(env: &BTreeMap<String, String>) -> String {
    let base_url = env_or_process_value(env, "OPENAI_COMPAT_BASE_URL")
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn is_agent_text_model_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if [
        "audio",
        "dall-e",
        "embedding",
        "image",
        "moderation",
        "realtime",
        "speech",
        "transcribe",
        "tts",
        "whisper",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return false;
    }

    lower.starts_with("gpt-") || lower.starts_with('o') || lower.starts_with("chatgpt-")
}

fn openai_dated_snapshot_alias(id: &str) -> Option<String> {
    let (base, date) = id.rsplit_once('-')?;
    if date.len() != 2 || !date.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, month) = base.rsplit_once('-')?;
    if month.len() != 2 || !month.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, year) = base.rsplit_once('-')?;
    if year.len() != 4 || !year.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    Some(base.to_string())
}

fn openai_model_display_name(id: &str) -> String {
    let canonical = openai_dated_snapshot_alias(id).unwrap_or_else(|| id.to_string());
    if let Some(rest) = canonical.strip_prefix("chatgpt-") {
        return format!("ChatGPT {}", title_case_model_suffix(rest));
    }
    if let Some(rest) = canonical.strip_prefix("gpt-") {
        return format!("GPT-{}", title_case_model_suffix(rest));
    }

    canonical
}

fn title_case_model_suffix(value: &str) -> String {
    value
        .split('-')
        .enumerate()
        .map(|(index, part)| {
            let part = if part.eq_ignore_ascii_case("pro") {
                "Pro".to_string()
            } else if part.eq_ignore_ascii_case("mini") {
                "mini".to_string()
            } else if part.eq_ignore_ascii_case("nano") {
                "nano".to_string()
            } else {
                part.to_string()
            };

            if index == 0 {
                part
            } else {
                format!(" {part}")
            }
        })
        .collect::<String>()
}

pub(super) fn normalize_openai_compatible_models(
    response: OpenAiModelListResponse,
    provider: Option<&str>,
) -> Vec<AgentModelInfo> {
    let mut seen = HashSet::new();
    let mut items = response.data;
    let filter_to_openai_text_models = matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openai")
    );
    let all_ids = items
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<String>>();
    items.sort_by(|left, right| {
        right
            .created
            .cmp(&left.created)
            .then_with(|| left.id.cmp(&right.id))
    });

    items
        .into_iter()
        .filter(|item| !filter_to_openai_text_models || is_agent_text_model_id(&item.id))
        .filter(|item| match openai_dated_snapshot_alias(&item.id) {
            Some(alias) if filter_to_openai_text_models => !all_ids.contains(&alias),
            Some(_) | None => true,
        })
        .filter(|item| seen.insert(item.id.clone()))
        .map(|item| AgentModelInfo {
            name: Some(openai_model_display_name(&item.id)),
            id: item.id,
            description: None,
        })
        .collect()
}

pub(super) async fn discover_openai_compatible_models(
    client: &reqwest::Client,
    provider: &DiscoveryProvider,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let relay_mesh =
        provider.as_deref().map(str::trim) == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID);
    if !relay_mesh && !is_openai_compatible_provider(provider.as_deref()) {
        return Ok(None);
    }

    let api_key = if relay_mesh {
        crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER.to_string()
    } else {
        match provider.required_env(env, "OPENAI_COMPAT_API_KEY")? {
            Some(api_key) => api_key,
            None => return Ok(None),
        }
    };
    let redaction_env = redaction_env_with_value(env, "OPENAI_COMPAT_API_KEY", &api_key);
    let url = if relay_mesh {
        format!("{}/models", crate::managed_agents::RELAY_MESH_API_BASE_URL)
    } else {
        openai_compatible_models_url_for_discovery(env)
    };
    let response = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|error| format!("OpenAI model discovery request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = crate::managed_agents::redact_env_values_in(&body, &redaction_env);
        return Err(format!("OpenAI model discovery HTTP {status}: {body}"));
    }

    let response = response
        .json::<OpenAiModelListResponse>()
        .await
        .map_err(|error| format!("OpenAI model discovery response parse failed: {error}"))?;
    let models = normalize_openai_compatible_models(response, provider.as_deref());
    if models.is_empty() {
        return Err("OpenAI model discovery returned no compatible text models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: provider.as_deref().unwrap_or("openai").trim().to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}
