use std::collections::BTreeMap;

use serde::Serialize;

use super::global_config::GlobalAgentConfig;
use super::relay_mesh::{
    RELAY_MESH_API_BASE_URL, RELAY_MESH_API_KEY_PLACEHOLDER, RELAY_MESH_AUTO_MODEL_ID,
    RELAY_MESH_PROVIDER_ID,
};
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

/// The API key value the relay-mesh preset wrote before the Jun-11 rename
/// window (#960, `8f580f308`) changed `RELAY_MESH_API_KEY_PLACEHOLDER` from
/// `"sprout-mesh-local"` to its current value. The old string is persisted as a
/// *value* in `env_vars` on records created before that, and no migration ever
/// rewrote it.
const LEGACY_MESH_API_KEY_PLACEHOLDER: &str = "sprout-mesh-local";

/// The provider key the preset wrote before #971 (`8c8312932`) renamed it to
/// `BUZZ_AGENT_PROVIDER`. That commit changed source literals only — persisted
/// `env_vars` keys were never migrated.
const LEGACY_MESH_PROVIDER_ENV_KEY: &str = "SPROUT_AGENT_PROVIDER";

/// The legacy env discriminator: recognizes the relay-mesh preset purely from
/// the env vars a pre-typed-field record carries, returning its served model id.
///
/// All three sentinels must match — the local base URL alone is not enough,
/// since a user may point their own OpenAI-compatible provider at the same
/// port. The placeholder API key is what makes this Buzz's own preset.
///
/// Two of those sentinels were renamed in the same Jun-11 window, in separate
/// commits, with neither migrating persisted records: the provider env *key*
/// (#971) and the API key *value* (#960). Each is therefore accepted under
/// either spelling, independently — a record straddling the window carries one
/// old and one new. In both cases the current spelling is authoritative when
/// present, so a record that has since been rewritten with a non-mesh value is
/// not resurrected by the stale leftover beside it.
///
/// `OPENAI_COMPAT_BASE_URL` and `OPENAI_COMPAT_MODEL` were never renamed.
/// Nothing beyond these two either/ors is loosened: every sentinel dropped
/// widens the false-positive surface for a user's own openai-compatible agent.
fn mesh_preset_env_model_id(env_vars: &BTreeMap<String, String>) -> Option<String> {
    let base_url = env_vars.get("OPENAI_COMPAT_BASE_URL")?.trim();
    if base_url.trim_end_matches('/') != RELAY_MESH_API_BASE_URL {
        return None;
    }
    let provider = env_vars
        .get("BUZZ_AGENT_PROVIDER")
        .or_else(|| env_vars.get(LEGACY_MESH_PROVIDER_ENV_KEY))?
        .trim();
    if provider != "openai" {
        return None;
    }
    let api_key = env_vars.get("OPENAI_COMPAT_API_KEY")?.trim();
    if api_key != RELAY_MESH_API_KEY_PLACEHOLDER && api_key != LEGACY_MESH_API_KEY_PLACEHOLDER {
        return None;
    }
    non_blank(env_vars.get("OPENAI_COMPAT_MODEL").map(String::as_str)).map(str::to_owned)
}

/// The mesh model id a definition-less record carries in legacy form, or `None`
/// when it is not a legacy mesh record.
///
/// Two shipped record generations predate `provider: "relay-mesh"` and are
/// never rewritten on load, so they are still on disk:
///
/// - the typed `relay_mesh` marker, added before the record had a `provider`
///   field at all;
/// - before that, the mesh preset written straight into `env_vars`.
///
/// Consulted only by [`resolve_definition_less`]: a record with no definition
/// has nothing authoritative to be overridden by, so its own bytes are the
/// highest-precedence signal it has. A linked instance never reaches here.
fn legacy_record_mesh_model_id(record: &ManagedAgentRecord) -> Option<String> {
    match &record.relay_mesh {
        // The marker itself is the mesh signal; a blank `model_ref` still means
        // mesh, resolved to the auto model exactly as `apply_relay_mesh_env`
        // and `relay_mesh_model_id` treat a blank model.
        Some(config) => Some(
            non_blank(Some(config.model_ref.as_str()))
                .unwrap_or(RELAY_MESH_AUTO_MODEL_ID)
                .to_owned(),
        ),
        None => mesh_preset_env_model_id(&record.env_vars),
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

    let mut config = EffectiveAgentConfig {
        model,
        provider,
        system_prompt,
    };

    // Legacy mesh compatibility. A record with an explicit `provider` has
    // already stated its intent — including switching AWAY from mesh, which
    // leaves the old marker and env bytes behind — so its legacy bytes are
    // never consulted. Only a record that never carried a provider at all
    // falls back, and both fields move together so the single mesh gate
    // (`relay_mesh_model_id`) and the spawned model agree.
    if non_blank(record.provider.as_deref()).is_none() {
        if let Some(model_ref) = legacy_record_mesh_model_id(record) {
            config.provider = ResolvedField {
                value: Some(RELAY_MESH_PROVIDER_ID.to_owned()),
                source: ConfigSource::InstanceLegacy,
            };
            config.model = ResolvedField {
                value: Some(model_ref),
                source: ConfigSource::InstanceLegacy,
            };
        }
    }

    config
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
