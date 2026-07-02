//! Global agent configuration defaults.
//!
//! A single `global-agent-config.json` record that applies to ALL managed
//! agents. Per-agent config always wins; global provides the lowest
//! user-settable layer below persona.
//!
//! # Precedence (low → high)
//!
//! ```text
//! baked build env  <  GLOBAL  <  persona  <  per-agent  <  Buzz-identity
//! ```
//!
//! # Semantics
//!
//! Unlike per-agent/persona env (snapshotted at create time), global config is
//! **live-resolved at spawn/readiness/deploy** — change a global key and every
//! agent picks it up on the next restart, with no delete+respawn required.
//!
//! # Storage
//!
//! `<app-data>/agents/global-agent-config.json`, written `0o600` via
//! `atomic_write_json_restricted` (same as the agent store).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::env_vars::{validate_user_env_keys, DERIVED_PROVIDER_MODEL_ENV_KEYS};
use crate::managed_agents::storage::{atomic_write_json_restricted, managed_agents_base_dir};
use crate::managed_agents::types::{ManagedAgentRecord, PersonaRecord};

/// The global agent configuration record.
///
/// Shape mirrors the per-agent/persona trio (`env_vars` + `provider` + `model`)
/// so the config vocabulary is consistent across all three tiers.
///
/// `env_vars` is the lowest user-settable env layer — global < persona < agent.
/// `provider` / `model` are fallback defaults: effective provider/model =
/// `agent → persona → global → None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalAgentConfig {
    /// Global env vars injected into ALL agents unconditionally.
    ///
    /// Lowest user-settable layer — per-agent and persona values win on any
    /// key collision. Reserved and derived keys are rejected at save time and
    /// stripped at spawn time.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,

    /// Global fallback provider (e.g. `"databricks_v2"`, `"anthropic"`).
    ///
    /// Used only when neither the agent record nor the linked persona specifies
    /// a provider. `None` = no global default.
    #[serde(default)]
    pub provider: Option<String>,

    /// Global fallback model identifier.
    ///
    /// Used only when neither the agent record nor the linked persona specifies
    /// a model. `None` = no global default.
    #[serde(default)]
    pub model: Option<String>,
}

/// Validate a `GlobalAgentConfig` before persisting it.
///
/// Rules beyond `validate_user_env_keys`:
/// - `DERIVED_PROVIDER_MODEL_ENV_KEYS` (`GOOSE_PROVIDER`, `GOOSE_MODEL`, …)
///   must NOT be set as global env vars — they would shadow the structured
///   `provider`/`model` fields and break provider/model resolution. Users
///   must use the structured fields instead.
/// - Empty per-key values are stripped before validation so a caller that
///   passes `KEY=""` does not accidentally shadow a real global value.
pub fn validate_global_config(config: &GlobalAgentConfig) -> Result<(), String> {
    // Strip empty values first — they mean "inherit" and must not be stored.
    let non_empty: BTreeMap<String, String> = config
        .env_vars
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Standard env-var key validation (POSIX shape, reserved-key check, NUL/size caps).
    validate_user_env_keys(&non_empty)?;

    // Reject derived provider/model keys in global env_vars.
    let derived: Vec<&str> = non_empty
        .keys()
        .filter(|k| {
            DERIVED_PROVIDER_MODEL_ENV_KEYS
                .iter()
                .any(|d| d.eq_ignore_ascii_case(k.as_str()))
        })
        .map(String::as_str)
        .collect();
    if !derived.is_empty() {
        return Err(format!(
            "the following keys must be set via the structured provider/model fields, \
             not as env vars: {}",
            derived.join(", ")
        ));
    }

    Ok(())
}

/// Strip empty values from `env_vars`.
///
/// Empty per-agent/persona values mean "no value"; if stored they would shadow
/// a real global default with an empty string. Strip them at save time so a
/// caller that clears a row cannot accidentally shadow global.
pub fn strip_empty_env_vars(config: &mut GlobalAgentConfig) {
    config.env_vars.retain(|_, v| !v.is_empty());
}

fn global_config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("global-agent-config.json"))
}

/// Load the global agent config from disk.
///
/// Returns the default (all-empty) config if the file does not exist yet.
pub fn load_global_agent_config(app: &AppHandle) -> Result<GlobalAgentConfig, String> {
    let path = global_config_path(app)?;
    if !path.exists() {
        return Ok(GlobalAgentConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read global agent config: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("failed to parse global agent config: {e}"))
}

/// Save the global agent config to disk.
///
/// Strips empty env values before writing (empty = "inherit" semantics).
/// Written `0o600` — same protection as `managed-agents.json`.
pub fn save_global_agent_config(app: &AppHandle, config: &GlobalAgentConfig) -> Result<(), String> {
    let mut config = config.clone();
    strip_empty_env_vars(&mut config);

    let path = global_config_path(app)?;
    let payload = serde_json::to_vec_pretty(&config)
        .map_err(|e| format!("failed to serialize global agent config: {e}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Resolve the effective model and provider for an agent using the
/// precedence chain: `agent record → linked persona → global defaults → None`.
///
/// This is the single source of truth used by readiness evaluation, spawn,
/// and deploy-payload construction. All three paths must use this function so
/// they agree on what model/provider the agent will actually run with.
///
/// # Arguments
/// * `record` — the `ManagedAgentRecord` (may have `None` for model/provider)
/// * `personas` — all current persona records (looked up by `record.persona_id`)
/// * `global` — global agent config defaults
///
/// # Returns
/// `(effective_model, effective_provider)` — both `Option<&str>`.
pub(crate) fn resolve_effective_model_provider<'a>(
    record: &'a ManagedAgentRecord,
    personas: &'a [PersonaRecord],
    global: &'a GlobalAgentConfig,
) -> (Option<&'a str>, Option<&'a str>) {
    let (persona_model, persona_provider) = record
        .persona_id
        .as_deref()
        .and_then(|pid| personas.iter().find(|p| p.id == pid))
        .map(|p| (p.model.as_deref(), p.provider.as_deref()))
        .unwrap_or((None, None));

    let effective_model = record
        .model
        .as_deref()
        .or(persona_model)
        .or(global.model.as_deref());
    let effective_provider = record
        .provider
        .as_deref()
        .or(persona_provider)
        .or(global.provider.as_deref());

    (effective_model, effective_provider)
}

#[cfg(test)]
mod tests;
