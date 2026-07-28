use std::time::Duration;

use buzz_agent_pkg::{
    lmstudio::{LmStudioChatRequest, LmStudioOutput, LmStudioReasoning},
    LmStudioNativeClient,
};
use serde_json::Value;
use tauri::Manager;
use tokio_util::sync::CancellationToken;

use crate::{
    command_brief::cloud::{CloudAdviserClient, CloudProvider},
    command_services::{
        policy::build_adviser_runtime_catalog,
        trusted_lan::{load_optional, ModelRoutingPreference},
    },
};

const COMPLETION_TIMEOUT: Duration = Duration::from_secs(120);
const MAXIMUM_INPUT_BYTES: usize = 768 * 1024;

#[derive(Clone, Copy)]
enum Attempt {
    Local,
    Cloud(CloudProvider),
}

fn attempts(preference: ModelRoutingPreference) -> [Attempt; 3] {
    match preference {
        ModelRoutingPreference::CloudFirst => [
            Attempt::Cloud(CloudProvider::LiteLlm),
            Attempt::Cloud(CloudProvider::OpenAi),
            Attempt::Local,
        ],
        ModelRoutingPreference::LocalFirst => [
            Attempt::Local,
            Attempt::Cloud(CloudProvider::LiteLlm),
            Attempt::Cloud(CloudProvider::OpenAi),
        ],
    }
}

pub(crate) async fn complete_json(
    app: &tauri::AppHandle,
    system_prompt: &str,
    input: &Value,
    _schema_name: &str,
    cancellation: CancellationToken,
) -> Result<Value, String> {
    let input_bytes =
        serde_json::to_vec(input).map_err(|_| "structured input is invalid".to_string())?;
    if input_bytes.len() > MAXIMUM_INPUT_BYTES || system_prompt.len() > 64 * 1024 {
        return Err("structured input exceeds the planning limit".into());
    }
    let config_path = app
        .path()
        .app_config_dir()
        .map_err(|_| "model routing configuration is unavailable".to_string())?
        .join("trusted-lan-sources.json");
    let config = load_optional(&config_path)
        .map_err(|_| "model routing configuration is invalid".to_string())?;
    let preference = config
        .as_ref()
        .map_or(ModelRoutingPreference::LocalFirst, |value| {
            value.routing_preference()
        });
    let cloud = config
        .as_ref()
        .and_then(|value| CloudAdviserClient::from_config(value, COMPLETION_TIMEOUT).ok());
    for attempt in attempts(preference) {
        if cancellation.is_cancelled() {
            return Err("structured completion was cancelled".into());
        }
        let result = match attempt {
            Attempt::Cloud(provider) => match cloud.as_ref() {
                Some(client) if client.available(provider) => client
                    .complete_json(provider, system_prompt, input, cancellation.clone())
                    .await
                    .map_err(|_| ()),
                _ => Err(()),
            },
            Attempt::Local => complete_local(system_prompt, input, cancellation.clone())
                .await
                .map_err(|_| ()),
        };
        if let Ok(value) = result {
            return Ok(value);
        }
    }
    Err("all configured model routes were unavailable".into())
}

async fn complete_local(
    system_prompt: &str,
    input: &Value,
    cancellation: CancellationToken,
) -> Result<Value, String> {
    let facts = crate::managed_agents::trusted_lmstudio_runtime_facts()
        .map_err(|_| "LM Studio runtime is unavailable".to_string())?;
    let catalog = build_adviser_runtime_catalog(&[], &facts.endpoint, facts.api_token.as_deref())
        .map_err(|_| "LM Studio runtime is not admitted".to_string())?;
    let client = LmStudioNativeClient::new(catalog.chief_of_staff_runtime(), COMPLETION_TIMEOUT)
        .map_err(|_| "LM Studio client is unavailable".to_string())?;
    let models = client
        .discover_models()
        .await
        .map_err(|_| "LM Studio model discovery failed".to_string())?;
    let model = first_loaded_model(&models)
        .ok_or_else(|| "LM Studio has no loaded chat model".to_string())?;
    let request = LmStudioChatRequest::new(
        model,
        serde_json::to_string(input).map_err(|_| "structured input is invalid".to_string())?,
        system_prompt,
        Vec::new(),
        LmStudioReasoning::Off,
        8_192,
        32_768,
    )
    .map_err(|_| "LM Studio request is invalid".to_string())?;
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Err("structured completion was cancelled".into()),
        response = client.chat(&request, "battle-rhythm-import") => {
            response.map_err(|_| "LM Studio completion failed".to_string())?
        }
    };
    let terminal = response.output.last().and_then(|output| match output {
        LmStudioOutput::Message { content } => Some(content),
        _ => None,
    });
    serde_json::from_str(
        terminal.ok_or_else(|| "LM Studio returned no structured result".to_string())?,
    )
    .map_err(|_| "LM Studio returned invalid structured JSON".to_string())
}

fn first_loaded_model(catalog: &Value) -> Option<String> {
    catalog
        .get("models")
        .and_then(Value::as_array)?
        .iter()
        .filter(|model| model.get("type").and_then(Value::as_str) == Some("llm"))
        .flat_map(|model| {
            model
                .get("loaded_instances")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find_map(|instance| {
            instance
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 512)
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_model_selection_ignores_embeddings_and_unloaded_models() {
        let catalog = serde_json::json!({
            "models": [
                {"type": "embedding", "loaded_instances": [{"id": "embed"}]},
                {"type": "llm", "loaded_instances": []},
                {"type": "llm", "loaded_instances": [{"id": "qwen/loaded"}]}
            ]
        });
        assert_eq!(first_loaded_model(&catalog).as_deref(), Some("qwen/loaded"));
    }
}
