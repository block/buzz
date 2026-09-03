//! Field normalization for `create_managed_agent` — the pure validators and
//! resolvers its request-to-record mapping runs before any side effect.

use crate::managed_agents::{managed_agent_avatar_url, BackendKind, RelayMeshConfig};

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
