use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::admission::{
    evaluate_offline_admission, OfflineModelAdmission, OfflineModelAdmissionState,
    OfflineModelPolicy,
};
use super::env_value;
use crate::managed_agents::{
    known_acp_runtime, resolve_command, AgentModelInfo, AgentModelsResponse,
};

const LMSTUDIO_EXPOSURE_UNVERIFIED_WARNING: &str = "LM Studio listener exposure is unverified.";
const LMSTUDIO_AUTHENTICATION_DISABLED_WARNING: &str =
    "LM Studio API authentication is not enabled.";

const MAX_LMSTUDIO_MODELS: usize = 256;
const MAX_LMSTUDIO_MODEL_ID_BYTES: usize = 256;
const MAX_LMSTUDIO_DISPLAY_NAME_BYTES: usize = 512;
const MAX_LMSTUDIO_DESCRIPTION_BYTES: usize = 4_096;
const MAX_LMSTUDIO_LOADED_INSTANCES: usize = 32;
const MAX_LMSTUDIO_CONTEXT_LENGTH: u64 = 16_777_216;
const MAX_LMSTUDIO_CAPABILITIES_BYTES: usize = 16_384;
const MAX_LMSTUDIO_CAPABILITIES_DEPTH: usize = 8;
const MAX_LMSTUDIO_CAPABILITIES_NODES: usize = 256;
const MAX_LMSTUDIO_CAPABILITY_STRING_BYTES: usize = 1_024;
const MAX_LMSTUDIO_CAPABILITY_KEY_BYTES: usize = 128;

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
    #[serde(default)]
    config: Option<LmStudioLoadedInstanceConfig>,
}

#[derive(Debug, Deserialize)]
struct LmStudioLoadedInstanceConfig {
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    parallel: Option<u64>,
}

pub(super) fn normalize_lmstudio_models(
    value: serde_json::Value,
) -> Result<Vec<AgentModelInfo>, String> {
    let response = serde_json::from_value::<LmStudioModelListResponse>(value)
        .map_err(|error| format!("LM Studio models response parse failed: {error}"))?;
    if response.models.len() > MAX_LMSTUDIO_MODELS {
        return Err("LM Studio model catalog exceeds the maximum model count".to_string());
    }
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for model in response.models {
        if model.model_type != "llm" {
            continue;
        }
        validate_catalog_identifier(
            &model.key,
            MAX_LMSTUDIO_MODEL_ID_BYTES,
            "LM Studio model catalog contains an invalid model identifier",
        )?;
        validate_optional_catalog_text(
            model.display_name.as_deref(),
            MAX_LMSTUDIO_DISPLAY_NAME_BYTES,
            "LM Studio model catalog contains an oversized display name",
        )?;
        validate_optional_catalog_text(
            model.description.as_deref(),
            MAX_LMSTUDIO_DESCRIPTION_BYTES,
            "LM Studio model catalog contains an oversized description",
        )?;
        if model.loaded_instances.len() > MAX_LMSTUDIO_LOADED_INSTANCES {
            return Err("LM Studio model catalog contains too many loaded instances".to_string());
        }
        let mut loaded_instance_ids = Vec::with_capacity(model.loaded_instances.len());
        let mut loaded_context_length = None;
        let mut loaded_parallelism = None;
        for instance in model.loaded_instances {
            validate_catalog_identifier(
                &instance.id,
                MAX_LMSTUDIO_MODEL_ID_BYTES,
                "LM Studio model catalog contains an invalid loaded instance identifier",
            )?;
            if loaded_instance_ids.is_empty() {
                loaded_context_length = instance.config.as_ref().and_then(|c| c.context_length);
                loaded_parallelism = instance.config.as_ref().and_then(|c| c.parallel);
            }
            loaded_instance_ids.push(instance.id);
        }
        if model
            .max_context_length
            .is_some_and(|length| length == 0 || length > MAX_LMSTUDIO_CONTEXT_LENGTH)
        {
            return Err("LM Studio model catalog contains an invalid context length".to_string());
        }
        if let Some(capabilities) = model.capabilities.as_ref() {
            validate_capabilities(capabilities)?;
        }
        if !seen.insert(model.key.clone()) {
            continue;
        }
        models.push(AgentModelInfo {
            id: model.key,
            name: model.display_name,
            description: model.description,
            is_loaded: !loaded_instance_ids.is_empty(),
            loaded_instance_ids,
            max_context_length: model.max_context_length,
            capabilities: model.capabilities,
            loaded_context_length,
            loaded_parallelism,
        });
    }
    Ok(models)
}

fn validate_catalog_identifier(
    value: &str,
    max_bytes: usize,
    diagnostic: &str,
) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(diagnostic.to_string());
    }
    Ok(())
}

fn validate_optional_catalog_text(
    value: Option<&str>,
    max_bytes: usize,
    oversized_diagnostic: &str,
) -> Result<(), String> {
    if let Some(value) = value {
        if value.len() > max_bytes {
            return Err(oversized_diagnostic.to_string());
        }
        if value.chars().any(char::is_control) {
            return Err("LM Studio model catalog contains invalid text metadata".to_string());
        }
    }
    Ok(())
}

fn validate_capabilities(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("LM Studio model catalog contains invalid capabilities metadata".to_string());
    }
    if serde_json::to_vec(value)
        .map_err(|_| "LM Studio model catalog contains invalid capabilities metadata".to_string())?
        .len()
        > MAX_LMSTUDIO_CAPABILITIES_BYTES
    {
        return Err("LM Studio model catalog contains oversized capabilities metadata".to_string());
    }

    fn walk(value: &serde_json::Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
        *nodes += 1;
        if depth > MAX_LMSTUDIO_CAPABILITIES_DEPTH || *nodes > MAX_LMSTUDIO_CAPABILITIES_NODES {
            return Err(
                "LM Studio model catalog contains overly complex capabilities metadata".to_string(),
            );
        }
        match value {
            serde_json::Value::Object(entries) => {
                for (key, child) in entries {
                    if key.len() > MAX_LMSTUDIO_CAPABILITY_KEY_BYTES
                        || key.chars().any(char::is_control)
                    {
                        return Err(
                            "LM Studio model catalog contains invalid capabilities metadata"
                                .to_string(),
                        );
                    }
                    walk(child, depth + 1, nodes)?;
                }
            }
            serde_json::Value::Array(entries) => {
                for child in entries {
                    walk(child, depth + 1, nodes)?;
                }
            }
            serde_json::Value::String(text)
                if text.len() > MAX_LMSTUDIO_CAPABILITY_STRING_BYTES
                    || text.chars().any(char::is_control) =>
            {
                return Err(
                    "LM Studio model catalog contains invalid capabilities metadata".to_string(),
                );
            }
            _ => {}
        }
        Ok(())
    }

    walk(value, 1, &mut 0)
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
    token: Option<&str>,
) -> Result<buzz_agent_pkg::LmStudioNativeClient, String> {
    let base_url_key = runtime
        .base_url_env_var
        .ok_or_else(|| "LM Studio runtime catalog is missing its base URL key".to_string())?;
    let base_url =
        env_value(env, base_url_key).unwrap_or_else(|| "http://127.0.0.1:1234".to_string());
    let integrations = runtime
        .integrations_env_var
        .and_then(|key| env_value(env, key));
    let config = buzz_agent_pkg::egress::LmStudioRuntimeConfig::parse_with_token(
        Some("OFFICIAL"),
        &base_url,
        None,
        integrations.as_deref(),
        token,
    )?;
    buzz_agent_pkg::LmStudioNativeClient::new(config, std::time::Duration::from_secs(10))
        .map_err(|error| error.to_string())
}

struct LmStudioCatalogProbe {
    value: serde_json::Value,
    authentication_enforced: bool,
}

async fn fetch_lmstudio_catalog_tokenless_first<F>(
    runtime: &crate::managed_agents::KnownAcpRuntime,
    env: &BTreeMap<String, String>,
    token_loader: F,
) -> Result<LmStudioCatalogProbe, buzz_agent_pkg::AgentError>
where
    F: FnOnce() -> Option<String>,
{
    let tokenless_client = lmstudio_native_client(runtime, env, None)
        .map_err(buzz_agent_pkg::AgentError::InvalidParams)?;
    match tokenless_client.discover_models().await {
        Ok(value) => Ok(LmStudioCatalogProbe {
            value,
            authentication_enforced: false,
        }),
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
            let token = token_loader().ok_or_else(|| {
                buzz_agent_pkg::AgentError::LlmAuth("LM Studio authentication required".to_string())
            })?;
            let authenticated_client = lmstudio_native_client(runtime, env, Some(&token))
                .map_err(buzz_agent_pkg::AgentError::InvalidParams)?;
            let value = authenticated_client.discover_models().await?;
            Ok(LmStudioCatalogProbe {
                value,
                authentication_enforced: true,
            })
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn discover_lmstudio_native_models(
    agent_command: &str,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    discover_lmstudio_native_models_with_token_loader(agent_command, env, selected_model, || {
        known_acp_runtime(agent_command).and_then(lmstudio_runtime_token)
    })
    .await
}

async fn discover_lmstudio_native_models_with_token_loader<F>(
    agent_command: &str,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
    token_loader: F,
) -> Result<Option<AgentModelsResponse>, String>
where
    F: FnOnce() -> Option<String>,
{
    let Some(runtime) = known_acp_runtime(agent_command) else {
        return Ok(None);
    };
    if runtime.native_model_discovery
        != Some(crate::managed_agents::NativeModelDiscovery::LmStudioV1)
    {
        return Ok(None);
    }
    let probe = fetch_lmstudio_catalog_tokenless_first(runtime, env, token_loader)
        .await
        .map_err(|error| error.to_string())?;
    let models = normalize_lmstudio_models(probe.value)?;
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

#[derive(Clone, Debug, Serialize)]
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
    /// Admission of the exact qualified model and loaded runtime configuration.
    pub admission: OfflineModelAdmission,
    /// Qualified output budget projected into every managed adviser.
    pub max_output_tokens: u32,
    /// Effective resident generation capacity.
    pub generation_capacity: u64,
}

fn unavailable_admission() -> OfflineModelAdmission {
    evaluate_offline_admission(&OfflineModelPolicy::command_adviser(), &[])
}

fn lmstudio_application_installed() -> bool {
    #[cfg(target_os = "macos")]
    if std::path::Path::new("/Applications/LM Studio.app").is_dir() {
        return true;
    }
    resolve_command("lms").is_some()
}

fn lmstudio_security_warnings(authentication_enforced: Option<bool>) -> Vec<String> {
    let mut warnings = Vec::with_capacity(2);
    if authentication_enforced == Some(false) {
        warnings.push(LMSTUDIO_AUTHENTICATION_DISABLED_WARNING.to_string());
    }
    warnings.push(LMSTUDIO_EXPOSURE_UNVERIFIED_WARNING.to_string());
    warnings
}

pub(super) fn lmstudio_readiness_from_models(
    app_installed: bool,
    configured_model: Option<String>,
    models: Vec<AgentModelInfo>,
    authentication_enforced: bool,
) -> LmStudioReadiness {
    if !app_installed {
        return LmStudioReadiness {
            status: LmStudioReadinessState::AppMissing,
            detail: "LM Studio is not installed or discoverable on this Mac.".to_string(),
            configured_model,
            loaded_models: Vec::new(),
            security_warnings: lmstudio_security_warnings(None),
            bind_exposure: "unknown",
            admission: unavailable_admission(),
            max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
            generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
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
            security_warnings: lmstudio_security_warnings(Some(authentication_enforced)),
            bind_exposure: "unknown",
            admission: unavailable_admission(),
            max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
            generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
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
            security_warnings: lmstudio_security_warnings(Some(authentication_enforced)),
            bind_exposure: "unknown",
            admission: evaluate_offline_admission(&OfflineModelPolicy::command_adviser(), &models),
            max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
            generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
        };
    }
    let admission = evaluate_offline_admission(&OfflineModelPolicy::command_adviser(), &models);
    if admission.state != OfflineModelAdmissionState::Ready {
        let detail = match admission.state {
            OfflineModelAdmissionState::NotLoaded => "No local model is loaded.",
            OfflineModelAdmissionState::WrongModel => {
                "The exact qualified Gemma 4 26B runtime is not loaded."
            }
            OfflineModelAdmissionState::MissingCapability => {
                "The loaded model does not advertise required tool and vision capabilities."
            }
            OfflineModelAdmissionState::InsufficientContext => {
                "The qualified model is loaded below the admitted 64K context tier."
            }
            OfflineModelAdmissionState::InvalidRuntime => {
                "The loaded model runtime is not configured for one 64K generation slot."
            }
            OfflineModelAdmissionState::Ready => unreachable!(),
        };
        return LmStudioReadiness {
            status: LmStudioReadinessState::ConfiguredModelUnavailable,
            detail: detail.to_string(),
            configured_model,
            loaded_models,
            security_warnings: lmstudio_security_warnings(Some(authentication_enforced)),
            bind_exposure: "unknown",
            admission,
            max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
            generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
        };
    }
    let security_warnings = lmstudio_security_warnings(Some(authentication_enforced));
    LmStudioReadiness {
        status: LmStudioReadinessState::Ready,
        detail: if authentication_enforced {
            "Loaded LM Studio model is ready.".to_string()
        } else {
            "Loaded model is ready; authentication is not enabled.".to_string()
        },
        configured_model,
        loaded_models,
        security_warnings,
        bind_exposure: "unknown",
        admission,
        max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
        generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
    }
}

fn lmstudio_unreachable(configured_model: Option<String>) -> LmStudioReadiness {
    LmStudioReadiness {
        status: LmStudioReadinessState::ApiUnreachable,
        detail: "The native LM Studio API is unreachable on the configured loopback endpoint."
            .to_string(),
        configured_model,
        loaded_models: Vec::new(),
        security_warnings: lmstudio_security_warnings(None),
        bind_exposure: "unknown",
        admission: unavailable_admission(),
        max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
        generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
    }
}

fn lmstudio_auth_required(configured_model: Option<String>) -> LmStudioReadiness {
    LmStudioReadiness {
        status: LmStudioReadinessState::AuthRequired,
        detail: "LM Studio requires authentication; add its token to the macOS Keychain."
            .to_string(),
        configured_model,
        loaded_models: Vec::new(),
        security_warnings: lmstudio_security_warnings(None),
        bind_exposure: "unknown",
        admission: unavailable_admission(),
        max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
        generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
    }
}

async fn probe_lmstudio_readiness(
    runtime: &crate::managed_agents::KnownAcpRuntime,
    env: &BTreeMap<String, String>,
    configured_model: Option<String>,
    token_loader: impl FnOnce() -> Option<String>,
) -> LmStudioReadiness {
    let probe = match fetch_lmstudio_catalog_tokenless_first(runtime, env, token_loader).await {
        Ok(probe) => probe,
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
            return lmstudio_auth_required(configured_model);
        }
        Err(_) => return lmstudio_unreachable(configured_model),
    };

    let models = match normalize_lmstudio_models(probe.value) {
        Ok(models) => models,
        Err(error) => {
            return LmStudioReadiness {
                status: LmStudioReadinessState::ApiUnreachable,
                detail: error,
                configured_model,
                loaded_models: Vec::new(),
                security_warnings: lmstudio_security_warnings(None),
                bind_exposure: "unknown",
                admission: unavailable_admission(),
                max_output_tokens: crate::managed_agents::runtime::QUALIFIED_OUTPUT_TOKENS,
                generation_capacity: crate::managed_agents::runtime::QUALIFIED_GENERATION_CAPACITY,
            };
        }
    };
    lmstudio_readiness_from_models(
        true,
        configured_model,
        models,
        probe.authentication_enforced,
    )
}

pub(crate) async fn read_lmstudio_readiness(app: AppHandle) -> Result<LmStudioReadiness, String> {
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
    Ok(
        probe_lmstudio_readiness(runtime, &env, configured_model, || {
            lmstudio_runtime_token(runtime)
        })
        .await,
    )
}

/// Read-only health probe for the Command Console's distinct LM Studio source.
#[tauri::command]
pub async fn get_lmstudio_readiness(app: AppHandle) -> Result<LmStudioReadiness, String> {
    let readiness = read_lmstudio_readiness(app.clone()).await;
    crate::startup::notify_lmstudio_readiness(&app, &readiness);
    readiness
}

#[cfg(test)]
#[path = "lmstudio_tests.rs"]
mod tests;
