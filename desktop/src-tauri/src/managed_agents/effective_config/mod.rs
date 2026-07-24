use serde::Serialize;

use super::global_config::GlobalAgentConfig;
use super::relay_mesh::{RELAY_MESH_AUTO_MODEL_ID, RELAY_MESH_PROVIDER_ID};
use super::types::{AgentDefinition, ManagedAgentRecord};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Definition,
    Global,
    InstanceLegacy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField<T> {
    pub value: Option<T>,
    pub source: ConfigSource,
}

#[derive(Debug, Clone)]
pub struct EffectiveAgentConfig {
    pub model: ResolvedField<String>,
    pub provider: ResolvedField<String>,
    pub system_prompt: ResolvedField<String>,
}

impl EffectiveAgentConfig {
    /// The relay-mesh model id this config resolves to, or `None` when the
    /// effective provider isn't relay-mesh.
    ///
    /// This is the single authoritative mesh decision for this config.  Both
    /// the mesh preflight (interactive start, restore-on-launch) AND spawn's
    /// `apply_relay_mesh_env` block MUST derive their mesh gate from this
    /// method — never from a separate provider comparison — so the two paths
    /// are guaranteed to agree even when the stored provider string has leading
    /// or trailing whitespace.  The provider is trimmed before matching;
    /// a blank effective model falls back to "auto", mirroring
    /// `apply_relay_mesh_env`'s own rule.
    pub fn relay_mesh_model_id(&self) -> Option<String> {
        if self.provider.value.as_deref().map(str::trim) != Some(RELAY_MESH_PROVIDER_ID) {
            return None;
        }
        Some(
            self.model
                .value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(RELAY_MESH_AUTO_MODEL_ID)
                .to_string(),
        )
    }
}

#[derive(Debug, Clone)]
pub enum EffectiveConfigResult {
    Resolved(EffectiveAgentConfig),
    OrphanedInstance {
        record_pubkey: String,
        missing_persona_id: String,
    },
}

fn non_blank(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.trim().is_empty())
}

fn resolve_linked(
    definition: &AgentDefinition,
    global: &GlobalAgentConfig,
) -> EffectiveAgentConfig {
    let model = match non_blank(definition.model.as_deref()) {
        Some(m) => ResolvedField {
            value: Some(m.to_owned()),
            source: ConfigSource::Definition,
        },
        None => ResolvedField {
            value: global.model.clone(),
            source: ConfigSource::Global,
        },
    };

    let provider = match non_blank(definition.provider.as_deref()) {
        Some(p) => ResolvedField {
            value: Some(p.to_owned()),
            source: ConfigSource::Definition,
        },
        None => ResolvedField {
            value: global.provider.clone(),
            source: ConfigSource::Global,
        },
    };

    let system_prompt = ResolvedField {
        value: non_blank(Some(definition.system_prompt.as_str())).map(str::to_owned),
        source: ConfigSource::Definition,
    };

    EffectiveAgentConfig {
        model,
        provider,
        system_prompt,
    }
}

fn resolve_definition_less(
    record: &ManagedAgentRecord,
    global: &GlobalAgentConfig,
) -> EffectiveAgentConfig {
    let model = match non_blank(record.model.as_deref()) {
        Some(m) => ResolvedField {
            value: Some(m.to_owned()),
            source: ConfigSource::InstanceLegacy,
        },
        None => ResolvedField {
            value: global.model.clone(),
            source: ConfigSource::Global,
        },
    };

    let provider = match non_blank(record.provider.as_deref()) {
        Some(p) => ResolvedField {
            value: Some(p.to_owned()),
            source: ConfigSource::InstanceLegacy,
        },
        None => ResolvedField {
            value: global.provider.clone(),
            source: ConfigSource::Global,
        },
    };

    let system_prompt = ResolvedField {
        value: non_blank(record.system_prompt.as_deref()).map(str::to_owned),
        source: ConfigSource::InstanceLegacy,
    };

    EffectiveAgentConfig {
        model,
        provider,
        system_prompt,
    }
}

pub fn resolve_effective_config(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> EffectiveConfigResult {
    match &record.persona_id {
        Some(pid) => match definitions.iter().find(|d| d.id == *pid) {
            Some(def) => EffectiveConfigResult::Resolved(resolve_linked(def, global)),
            None => EffectiveConfigResult::OrphanedInstance {
                record_pubkey: record.pubkey.clone(),
                missing_persona_id: pid.clone(),
            },
        },
        None => EffectiveConfigResult::Resolved(resolve_definition_less(record, global)),
    }
}

pub fn resolve_effective_model_provider_pair(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Option<(Option<String>, Option<String>)> {
    match resolve_effective_config(record, definitions, global) {
        EffectiveConfigResult::Resolved(cfg) => Some((cfg.model.value, cfg.provider.value)),
        EffectiveConfigResult::OrphanedInstance { .. } => None,
    }
}

/// The relay-mesh preflight decision for `record`, resolved the same way
/// spawn resolves its mesh env: through `resolve_effective_config` (which
/// folds in the definition → global fallback), never through the record's
/// own `provider`/`model`/`relay_mesh` bytes.
///
/// `None` covers both "not a mesh agent" and "orphaned instance" — an orphan
/// never spawns (see `require_resolved`), so it never needs a mesh preflight
/// either; the caller's own orphan handling downstream is unaffected, this
/// just avoids tripping mesh bootstrap for a start that will be refused.
pub fn resolve_effective_relay_mesh_model_id(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
    global: &GlobalAgentConfig,
) -> Option<String> {
    match resolve_effective_config(record, definitions, global) {
        EffectiveConfigResult::Resolved(cfg) => cfg.relay_mesh_model_id(),
        EffectiveConfigResult::OrphanedInstance { .. } => None,
    }
}

/// The single user-facing message for a linked instance whose definition no
/// longer exists. Shared by every path that must refuse to act on an orphan:
/// the spawn boundary (`spawn_agent_child`), the interactive start command,
/// and provider deploy.
pub const ORPHANED_INSTANCE_ERROR: &str =
    "This agent's configuration is missing — it may still be \
     syncing or was deleted on another device.";

impl EffectiveConfigResult {
    /// Unwrap into the resolved config, or the shared orphan-refusal error.
    pub fn require_resolved(self) -> Result<EffectiveAgentConfig, String> {
        match self {
            EffectiveConfigResult::Resolved(cfg) => Ok(cfg),
            EffectiveConfigResult::OrphanedInstance { .. } => {
                Err(ORPHANED_INSTANCE_ERROR.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests;
