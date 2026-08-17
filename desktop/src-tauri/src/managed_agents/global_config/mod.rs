//! Global agent configuration defaults.
//!
//! A single `global-agent-config.json` record that applies to ALL managed
//! agents. For a linked instance, the definition wins over global; for a
//! definition-less (legacy) instance, the record's own fields win over
//! global. Global is always the lowest user-settable layer.
//!
//! # Precedence (low → high)
//!
//! ```text
//! baked build env  <  GLOBAL  <  definition (linked) / instance (legacy)  <  Buzz-identity
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

use crate::managed_agents::env_vars::{
    validate_user_env_keys, DERIVED_PROVIDER_MODEL_ENV_KEYS, MAX_ENV_VALUE_BYTES,
};
use crate::managed_agents::secret_projection::{
    cancel_gc_candidacy, deserialize_env_map, global_env_key, load_secret, serialize_env_map,
    write_secret, WriteOutcome,
};
use crate::managed_agents::storage::{atomic_write_json_restricted, managed_agents_base_dir};
use crate::managed_agents::types::{AgentDefinition, ManagedAgentRecord};
use crate::secret_store::SecretStore;

/// The global agent configuration record.
///
/// Shape mirrors the per-agent/persona trio (`env_vars` + `provider` + `model`)
/// so the config vocabulary is consistent across all three tiers.
///
/// `env_vars` is the lowest user-settable env layer — global < persona < agent.
/// `provider` / `model` are fallback defaults, resolved via
/// `effective_config::resolve_effective_config`: for a linked instance,
/// definition → global (the record's own `provider`/`model` bytes are never
/// consulted); for a definition-less instance, instance → global.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalAgentConfig {
    /// Global env vars injected into ALL agents unconditionally.
    ///
    /// Lowest user-settable layer — per-agent and persona values win on any
    /// key collision. Reserved and derived keys are rejected at save time and
    /// stripped at spawn time.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,

    /// Keyring generation reference for `env_vars`. When present and `env_vars`
    /// is empty, the env map is stored in the keyring under
    /// `global:env:<env_vars_ref>`. When absent with empty `env_vars`, the env
    /// map is intentionally empty.
    ///
    /// Inline (`env_vars` non-empty) takes precedence over any ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_vars_ref: Option<String>,

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

    /// Preferred ACP runtime for definitions without an explicit runtime.
    #[serde(default)]
    pub preferred_runtime: Option<String>,
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
///
/// `provider` and `model` rules (applied to `Some` values only):
/// - Interior NUL bytes are rejected (they truncate C-string env injection).
/// - Values exceeding [`MAX_ENV_VALUE_BYTES`] are rejected.
/// - Blank / whitespace-only values are normalized to `None` by
///   [`normalize_global_config_fields`], which must be called before
///   persisting (done inside [`save_global_agent_config`]).
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

    // Validate the structured provider and model fields.
    for (field, value) in [("provider", &config.provider), ("model", &config.model)] {
        if let Some(v) = value {
            // Reject interior NUL bytes — they truncate C-string env injection.
            if v.contains('\0') {
                return Err(format!(
                    "global config `{field}` must not contain NUL bytes"
                ));
            }
            // Size cap: match the per-value env-var cap.
            if v.len() > MAX_ENV_VALUE_BYTES {
                return Err(format!(
                    "global config `{field}` exceeds the maximum allowed length \
                     ({} bytes)",
                    MAX_ENV_VALUE_BYTES
                ));
            }
            // Note: blank/whitespace-only values are normalized to None by
            // normalize_global_config_fields, called from save_global_agent_config.
        }
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

/// Normalize `provider` and `model` to `None` when blank or whitespace-only.
///
/// `Some("")` and `Some("  ")` have no meaningful value and break
/// unset/fallback semantics (a blank provider would be treated as "provider
/// explicitly set to nothing" rather than "inherit"). Normalizing to `None`
/// preserves the invariant that `Some(s)` always contains a non-blank string.
///
/// Called from [`save_global_agent_config`] so normalization is applied at
/// every persist boundary.
pub fn normalize_global_config_fields(config: &mut GlobalAgentConfig) {
    if let Some(v) = &config.provider {
        if v.trim().is_empty() {
            config.provider = None;
        }
    }
    if let Some(v) = &config.model {
        if v.trim().is_empty() {
            config.model = None;
        }
    }
}

fn global_config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("global-agent-config.json"))
}

fn global_config_secret_store() -> Option<&'static SecretStore> {
    if cfg!(feature = "system-keyring") {
        Some(SecretStore::shared(crate::app_state::keyring_service()))
    } else {
        None
    }
}

/// Load the global agent config from disk.
///
/// Returns the default (all-empty) config if the file does not exist yet.
/// Hydrates `env_vars` from the keyring when an `env_vars_ref` is present
/// and `env_vars` is empty (inline fallback: non-empty `env_vars` wins).
///
/// # Fail-closed on unavailability
///
/// Returns `Err` (rather than a silently-empty config) when the record
/// carries an `env_vars_ref` but the keyring entry is missing/unreachable or
/// its bytes fail to deserialize. Global env applies to ALL agents; silently
/// dropping it would spawn every agent with an incomplete env under the exact
/// keyring-failure the gen-ref protocol exists to fail closed on. Spawn/deploy
/// gates propagate this `Err` to refuse the launch; display/readiness callers
/// choose degraded-empty via `.unwrap_or_default()` and reflect the failure as
/// not-ready. Absent file and the no-ref empty case remain `Ok(default)`.
pub fn load_global_agent_config(app: &AppHandle) -> Result<GlobalAgentConfig, String> {
    let path = global_config_path(app)?;
    if !path.exists() {
        return Ok(GlobalAgentConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read global agent config: {e}"))?;
    let mut config: GlobalAgentConfig = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse global agent config: {e}"))?;

    // Hydrate env_vars from keyring if inline is empty and a ref is present.
    if config.env_vars.is_empty() {
        if let Some(store) = global_config_secret_store() {
            match load_secret(
                store,
                config.env_vars_ref.as_deref(),
                global_env_key,
                "global env_vars",
            ) {
                Ok(Some(serialized)) => match deserialize_env_map(&serialized) {
                    Ok(map) => config.env_vars = map,
                    // Ref present, bytes retrieved, but unparseable — fail closed.
                    Err(e) => {
                        return Err(format!(
                            "global env_vars unavailable: deserialize failed: {e}"
                        ))
                    }
                },
                Ok(None) => {} // no ref: intentionally empty
                // Ref present but keyring entry missing/unreachable — fail closed.
                Err(e) => return Err(e),
            }
        }
    }
    // else: inline is authoritative.

    Ok(config)
}

/// Save the global agent config to disk.
///
/// Strips empty env values and normalizes blank provider/model to `None`
/// before writing (empty = "inherit" semantics).
/// Written `0o600` — same protection as `managed-agents.json`.
/// Persists `env_vars` to the keyring before writing JSON when the keyring
/// is available (generation-reference protocol).
pub fn save_global_agent_config(app: &AppHandle, config: &GlobalAgentConfig) -> Result<(), String> {
    let mut config = config.clone();
    strip_empty_env_vars(&mut config);
    normalize_global_config_fields(&mut config);

    // Persist env_vars to keyring (generation-reference protocol).
    if let Some(store) = global_config_secret_store() {
        // Cross-process transaction lock: hold across the generation write and
        // the atomic JSON commit below so a second Desktop process's GC cannot
        // delete the just-written generation before its ref lands in JSON. The
        // lock releases when `_txn` drops at end of function. Callers never hold
        // it already (boot migration releases its agent-store span before
        // calling here), so this does not nest. Keyed by the SAME canonical
        // store directory as the agent-store saves and GC (via
        // `acquire_secret_txn_lock`) so every mutator of the shared keyring
        // blob serializes on one inode — global-agent-config.json is not itself
        // shared, but it writes the same blob the agent store does.
        let _txn = crate::managed_agents::storage::acquire_secret_txn_lock(app)?;
        let inline_env = if !config.env_vars.is_empty() {
            serialize_env_map(&config.env_vars).ok()
        } else {
            None
        };
        match write_secret(
            store,
            global_env_key,
            inline_env.as_deref(),
            "global env_vars",
        ) {
            WriteOutcome::Persisted { gen } => {
                cancel_gc_candidacy(store, &global_env_key(&gen));
                config.env_vars.clear();
                config.env_vars_ref = Some(gen);
            }
            WriteOutcome::KeptInline { .. } => {
                config.env_vars_ref = None;
            }
            WriteOutcome::Nothing => {
                config.env_vars_ref = None;
            }
        }

        let path = global_config_path(app)?;
        let payload = serde_json::to_vec_pretty(&config)
            .map_err(|e| format!("failed to serialize global agent config: {e}"))?;
        return atomic_write_json_restricted(&path, &payload);
    }

    let path = global_config_path(app)?;
    let payload = serde_json::to_vec_pretty(&config)
        .map_err(|e| format!("failed to serialize global agent config: {e}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Resolve the effective model and provider for an agent.
///
/// Delegates to `effective_config::resolve_effective_config` which enforces
/// definition-authoritative semantics for linked instances:
///   - **Linked:** definition → global. Record bytes are never consulted.
///   - **Definition-less:** instance → global.
///   - **Orphaned:** returns `(None, None)`. This function is a display/
///     readiness/hash convenience, not the spawn gate — an orphan must never
///     reach a real spawn regardless of what this returns, because
///     `spawn_agent_child` refuses orphans itself via
///     `effective_config::resolve_effective_config(...).require_resolved()`
///     before resolving model/provider/prompt for the process env.
///
/// Returns owned `String`s because the effective value may come from a
/// tier that outlives none of the input borrows (e.g. global config).
pub(crate) fn resolve_effective_model_provider(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> (Option<String>, Option<String>) {
    super::effective_config::resolve_effective_model_provider_pair(record, personas, global)
        .unwrap_or((None, None))
}

#[cfg(test)]
mod tests;
