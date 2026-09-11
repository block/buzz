//! Field normalization for `create_managed_agent` — the pure validators and
//! resolvers its request-to-record mapping runs before any side effect.

use crate::managed_agents::{managed_agent_avatar_url, BackendKind, RelayMeshConfig};

pub(crate) struct CreatedInferenceConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub relay_mesh: Option<RelayMeshConfig>,
}

/// Normalize the provider/model fields written by Create.
///
/// Kept as one pure projection so readiness preview and persistence cannot
/// disagree about implicit provider defaults such as relay-mesh's `auto` model.
pub(crate) fn resolve_created_inference_config(
    provider: Option<&str>,
    model: Option<&str>,
    relay_mesh: Option<RelayMeshConfig>,
) -> CreatedInferenceConfig {
    let provider = provider.and_then(trim_to_optional_string);
    let mut model = model.and_then(trim_to_optional_string);
    if provider.as_deref() == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID) && model.is_none()
    {
        model = Some(crate::managed_agents::RELAY_MESH_AUTO_MODEL_ID.to_string());
    }
    let relay_mesh = if provider.as_deref() == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID) {
        model.clone().map(|model_ref| RelayMeshConfig { model_ref })
    } else {
        relay_mesh
    };
    CreatedInferenceConfig {
        provider,
        model,
        relay_mesh,
    }
}

pub(super) fn normalize_relay_mesh(
    config: Option<&RelayMeshConfig>,
    backend: &BackendKind,
) -> Result<Option<RelayMeshConfig>, String> {
    let Some(config) = config else {
        return Ok(None);
    };

    let model_ref = config.model_ref.trim();
    if model_ref.is_empty() {
        return Err("Buzz shared compute model is required".to_string());
    }
    if backend != &BackendKind::Local {
        return Err("Buzz shared compute agents must use the local backend".to_string());
    }

    Ok(Some(RelayMeshConfig {
        model_ref: model_ref.to_string(),
    }))
}

pub(super) fn trim_to_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn resolve_created_avatar_url(
    requested_avatar_url: Option<&str>,
    persona_avatar_url: Option<String>,
    agent_command: &str,
) -> Option<String> {
    requested_avatar_url
        .and_then(trim_to_optional_string)
        .or_else(|| {
            persona_avatar_url
                .as_deref()
                .and_then(trim_to_optional_string)
        })
        .or_else(|| managed_agent_avatar_url(agent_command))
}
