use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::env_value;
use crate::managed_agents::{
    known_acp_runtime, resolve_command, AgentModelInfo, AgentModelsResponse,
};

#[derive(Debug, Deserialize)]
struct LmStudioModelListResponse {
    models: Vec<LmStudioModelListItem>,
}

#[derive(Debug, Deserialize)]
struct LmStudioModelListItem {
    #[serde(rename = "type")]
    model_type: String,
    key: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    loaded_instances: Vec<LmStudioLoadedInstance>,
    #[serde(default)]
    max_context_length: Option<u64>,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LmStudioLoadedInstance {
    id: String,
}

pub(super) fn normalize_lmstudio_models(
    value: serde_json::Value,
) -> Result<Vec<AgentModelInfo>, String> {
    let response = serde_json::from_value::<LmStudioModelListResponse>(value)
        .map_err(|error| format!("LM Studio models response parse failed: {error}"))?;
    let mut seen = HashSet::new();
    let models = response
        .models
        .into_iter()
        .filter(|model| model.model_type == "llm")
        .filter(|model| !model.key.trim().is_empty())
        .filter(|model| seen.insert(model.key.clone()))
        .map(|model| {
            let loaded_instance_ids = model
                .loaded_instances
                .into_iter()
                .map(|instance| instance.id)
                .filter(|id| !id.trim().is_empty())
                .collect::<Vec<_>>();
            AgentModelInfo {
                id: model.key,
                name: model.display_name,
                description: model.description,
                is_loaded: !loaded_instance_ids.is_empty(),
                loaded_instance_ids,
                max_context_length: model.max_context_length,
                capabilities: model.capabilities,
            }
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err("LM Studio returned no installed LLM models".to_string());
    }
    Ok(models)
}

fn lmstudio_runtime_token(runtime: &crate::managed_agents::KnownAcpRuntime) -> Option<String> {
    let token_key = runtime.keychain_token_key?;
    crate::secret_store::SecretStore::shared(crate::app_state::keyring_service())
        .load(token_key)
        .ok()
        .flatten()
}

fn lmstudio_native_client(
    runtime: &crate::managed_agents::KnownAcpRuntime,
    env: &BTreeMap<String, String>,
) -> Result<buzz_agent_pkg::LmStudioNativeClient, String> {
    let base_url_key = runtime
        .base_url_env_var
        .ok_or_else(|| "LM Studio runtime catalog is missing its base URL key".to_string())?;
    let base_url =
        env_value(env, base_url_key).unwrap_or_else(|| "http://127.0.0.1:1234".to_string());
    let integrations = runtime
        .integrations_env_var
        .and_then(|key| env_value(env, key));
    let token = lmstudio_runtime_token(runtime);
    let config = buzz_agent_pkg::egress::LmStudioRuntimeConfig::parse_with_token(
        Some("OFFICIAL"),
        &base_url,
        None,
        integrations.as_deref(),
        token.as_deref(),
    )?;
    buzz_agent_pkg::LmStudioNativeClient::new(config, std::time::Duration::from_secs(10))
        .map_err(|error| error.to_string())
}

pub(super) async fn discover_lmstudio_native_models(
    agent_command: &str,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let Some(runtime) = known_acp_runtime(agent_command) else {
        return Ok(None);
    };
    if runtime.native_model_discovery
        != Some(crate::managed_agents::NativeModelDiscovery::LmStudioV1)
    {
        return Ok(None);
    }
    let client = lmstudio_native_client(runtime, env)?;
    let models = normalize_lmstudio_models(
        client
            .discover_models()
            .await
            .map_err(|error| error.to_string())?,
    )?;
    Ok(Some(AgentModelsResponse {
        agent_name: runtime.label.to_string(),
        agent_version: "lm-studio-native-v1".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: runtime.supports_acp_model_switching,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Readiness states reported by the desktop's native LM Studio probe.
pub enum LmStudioReadinessState {
    /// The LM Studio application and CLI were not found.
    AppMissing,
    /// The configured loopback native API did not respond successfully.
    ApiUnreachable,
    /// The native API requires a bearer token not available in Keychain.
    AuthRequired,
    /// No installed model has a loaded runtime instance.
    NoLoadedModel,
    /// The configured model is not currently loaded.
    ConfiguredModelUnavailable,
    /// The configured model and native service are ready.
    Ready,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Reader-facing facts from the native LM Studio readiness probe.
pub struct LmStudioReadiness {
    /// Normalized readiness state.
    pub status: LmStudioReadinessState,
    /// Operator-facing explanation of the current state.
    pub detail: String,
    /// Catalog-selected model, when configured.
    pub configured_model: Option<String>,
    /// Installed model IDs with at least one loaded instance.
    pub loaded_models: Vec<String>,
    /// Security facts that require operator attention.
    pub security_warnings: Vec<String>,
    /// LM Studio's models API does not attest its listener bind address.
    /// The desktop therefore reports this as unknown rather than inferring
    /// wildcard exposure from a successful loopback request.
    pub bind_exposure: &'static str,
}

fn lmstudio_application_installed() -> bool {
    #[cfg(target_os = "macos")]
    if std::path::Path::new("/Applications/LM Studio.app").is_dir() {
        return true;
    }
    resolve_command("lms").is_some()
}

pub(super) fn lmstudio_readiness_from_models(
    app_installed: bool,
    configured_model: Option<String>,
    models: Vec<AgentModelInfo>,
    token_present: bool,
) -> LmStudioReadiness {
    if !app_installed {
        return LmStudioReadiness {
            status: LmStudioReadinessState::AppMissing,
            detail: "LM Studio is not installed or discoverable on this Mac.".to_string(),
            configured_model,
            loaded_models: Vec::new(),
            security_warnings: Vec::new(),
            bind_exposure: "unknown",
        };
    }
    let loaded_models = models
        .iter()
        .filter(|model| model.is_loaded)
        .map(|model| model.id.clone())
        .collect::<Vec<_>>();
    if loaded_models.is_empty() {
        return LmStudioReadiness {
            status: LmStudioReadinessState::NoLoadedModel,
            detail: "LM Studio is reachable, but no LLM model is loaded.".to_string(),
            configured_model,
            loaded_models,
            security_warnings: if token_present {
                Vec::new()
            } else {
                vec!["LM Studio API authentication is not enabled.".to_string()]
            },
            bind_exposure: "unknown",
        };
    }
    if configured_model
        .as_deref()
        .is_some_and(|configured| !loaded_models.iter().any(|loaded| loaded == configured))
    {
        return LmStudioReadiness {
            status: LmStudioReadinessState::ConfiguredModelUnavailable,
            detail: "The configured LM Studio model is not currently loaded.".to_string(),
            configured_model,
            loaded_models,
            security_warnings: if token_present {
                Vec::new()
            } else {
                vec!["LM Studio API authentication is not enabled.".to_string()]
            },
            bind_exposure: "unknown",
        };
    }
    let security_warnings = if token_present {
        Vec::new()
    } else {
        vec!["LM Studio API authentication is not enabled.".to_string()]
    };
    LmStudioReadiness {
        status: LmStudioReadinessState::Ready,
        detail: if security_warnings.is_empty() {
            "Loaded LM Studio model is ready.".to_string()
        } else {
            "Loaded model is ready; authentication is not enabled.".to_string()
        },
        configured_model,
        loaded_models,
        security_warnings,
        bind_exposure: "unknown",
    }
}

/// Read-only health probe for the Command Console's distinct LM Studio source.
#[tauri::command]
pub async fn get_lmstudio_readiness(app: AppHandle) -> Result<LmStudioReadiness, String> {
    let runtime = known_acp_runtime("buzz-lmstudio-agent")
        .ok_or_else(|| "LM Studio runtime is missing from the catalog".to_string())?;
    let configured_model = crate::managed_agents::load_global_agent_config(&app)
        .ok()
        .filter(|config| config.preferred_runtime.as_deref() == Some(runtime.id))
        .and_then(|config| config.model);
    if !lmstudio_application_installed() {
        return Ok(lmstudio_readiness_from_models(
            false,
            configured_model,
            Vec::new(),
            false,
        ));
    }
    let mut env = BTreeMap::new();
    crate::managed_agents::apply_runtime_security_env(&mut env, Some(runtime));
    let token_present = lmstudio_runtime_token(runtime).is_some();
    let client = match lmstudio_native_client(runtime, &env) {
        Ok(client) => client,
        Err(error) => {
            return Ok(LmStudioReadiness {
                status: LmStudioReadinessState::ApiUnreachable,
                detail: error,
                configured_model,
                loaded_models: Vec::new(),
                security_warnings: Vec::new(),
                bind_exposure: "unknown",
            });
        }
    };
    let value = match client.discover_models().await {
        Ok(value) => value,
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
            return Ok(LmStudioReadiness {
                status: LmStudioReadinessState::AuthRequired,
                detail: "LM Studio requires authentication; add its token to the macOS Keychain."
                    .to_string(),
                configured_model,
                loaded_models: Vec::new(),
                security_warnings: Vec::new(),
                bind_exposure: "unknown",
            });
        }
        Err(_) => {
            return Ok(LmStudioReadiness {
                status: LmStudioReadinessState::ApiUnreachable,
                detail:
                    "The native LM Studio API is unreachable on the configured loopback endpoint."
                        .to_string(),
                configured_model,
                loaded_models: Vec::new(),
                security_warnings: Vec::new(),
                bind_exposure: "unknown",
            });
        }
    };
    let models = match normalize_lmstudio_models(value) {
        Ok(models) => models,
        Err(error) => {
            return Ok(LmStudioReadiness {
                status: LmStudioReadinessState::ApiUnreachable,
                detail: error,
                configured_model,
                loaded_models: Vec::new(),
                security_warnings: Vec::new(),
                bind_exposure: "unknown",
            });
        }
    };
    Ok(lmstudio_readiness_from_models(
        true,
        configured_model,
        models,
        token_present,
    ))
}
