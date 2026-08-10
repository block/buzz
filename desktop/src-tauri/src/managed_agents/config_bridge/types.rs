use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Sanitized inherited config tiers passed to the reader.
///
/// Built at the `agent_config` command boundary with spawn-equivalent
/// sanitization: reserved, malformed, NUL-value, and oversize-value env keys
/// are stripped (matching `merged_user_env`). Structured fields are
/// normalized: blank/whitespace-only values collapse to `None`.
///
/// Orphaned persona links (persona_id references a missing persona) produce
/// an empty persona env tier and `None` for all structured persona fields —
/// the panel still renders from record/global. This diverges deliberately from
/// spawn's `OrphanedInstance` refusal, which is a spawn-safety property the
/// display surface does not need to enforce.
#[derive(Debug, Clone, Default)]
pub struct InheritedConfigTiers {
    /// Sanitized env vars from the linked persona definition.
    pub persona_env: BTreeMap<String, String>,
    /// Sanitized env vars from the global agent config.
    pub global_env: BTreeMap<String, String>,
    /// Sanitized env vars from the resolved harness definition (`HarnessDefinition::env`).
    /// Sits below global env and above structured values, matching spawn Layer 2b.
    /// Empty for preset harnesses (all shipped presets have `env: {}`); only
    /// user-authored custom harness JSONs with a non-empty `env` block contribute here.
    pub definition_env: BTreeMap<String, String>,
    /// Structured model from the linked persona (non-blank only).
    pub persona_model: Option<String>,
    /// Structured provider from the linked persona (non-blank only).
    pub persona_provider: Option<String>,
    /// Structured system_prompt from the linked persona (non-blank only).
    pub persona_prompt: Option<String>,
    /// Structured model from global config (non-blank only).
    pub global_model: Option<String>,
    /// Structured provider from global config (non-blank only).
    pub global_provider: Option<String>,
}

/// Where a config value came from — determines precedence and UI annotations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigOrigin {
    /// Explicitly set in Buzz UI / ManagedAgentRecord (highest precedence).
    BuzzExplicit,
    /// Returned by ACP `_goose/unstable/config/read` (tier 1a).
    AcpNativeRead,
    /// Returned by ACP `session/new` configOptions (tier 1b).
    AcpConfigOption,
    /// Set via env var at spawn time (tier 2a).
    EnvVar,
    /// Read from harness config file on disk (tier 2b, lowest precedence).
    ConfigFile,
    /// Value inherited from persona defaults.
    /// Populated when a persona's env var or structured field wins for this
    /// field in the reader's candidate resolution.
    PersonaDefault,
    /// Value inherited from global agent configuration defaults.
    /// The lowest user-settable layer — active when neither the agent record nor
    /// the linked persona specifies a value.
    GlobalDefault,
    /// Live runtime model override applied via the ModelPicker (Phase 3).
    /// The ACP session's current model diverges from the persona model because
    /// the user picked a different model on the running instance. Runtime-only —
    /// never persisted; reverts to the persona model on restart/respawn.
    RuntimeOverride,
    /// Value is fixed by the harness itself — not from any user-set config or
    /// env var. E.g. Claude Code only supports Anthropic as a provider; the
    /// "locked" display is synthesized by the config bridge, not read from disk.
    HarnessConstraint,
    /// Value comes from a custom harness definition's `env` block.
    /// Sits below global env and above structured persona/global values,
    /// matching spawn Layer 2b. Only reachable for user-authored custom harness
    /// JSONs with a non-empty `env` block; preset harnesses always have empty env.
    HarnessDefault,
}

/// How a config field can be written back to the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConfigWriteMechanism {
    /// Update record env vars, save, stop + restart agent.
    RespawnWithEnvVar { env_key: String },
    /// Send `session/set_config_option` via ACP (live, no restart).
    AcpSetConfigOption { config_id: String },
    /// Send `session/set_model` via ACP (live, no restart).
    AcpSetSessionModel,
    /// Send `_goose/unstable/config/write` sparse patch (live, no restart).
    /// Reserved for tier 1a — blocked on upstream goose PR landing.
    /// Not yet constructed by any reader; will be wired when config/read+write
    /// are available in the harness.
    GooseNativeConfigWrite { config_key: String },
    /// Not writable through Buzz.
    ReadOnly,
}

/// A single normalized config field with provenance and write metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedField {
    pub value: Option<String>,
    pub origin: ConfigOrigin,
    pub write_via: ConfigWriteMechanism,
    /// When this field overrides a lower-precedence value, show what it overrode.
    pub overridden_value: Option<String>,
    pub overridden_origin: Option<ConfigOrigin>,
    /// True if this field must be set for the harness to function.
    /// Populated from `KnownAcpRuntime::required_normalized_fields`.
    pub is_required: bool,
}

/// Normalized cross-runtime config concepts (~8 fields that span all runtimes).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedConfig {
    pub model: Option<NormalizedField>,
    pub provider: Option<NormalizedField>,
    pub mode: Option<NormalizedField>,
    pub thinking_effort: Option<NormalizedField>,
    pub max_output_tokens: Option<NormalizedField>,
    pub context_limit: Option<NormalizedField>,
    pub system_prompt: Option<NormalizedField>,
}

/// A runtime-specific config field not covered by normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    pub value: Option<String>,
    pub origin: ConfigOrigin,
    pub schema_type: ConfigFieldType,
    pub write_via: ConfigWriteMechanism,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConfigFieldType {
    String,
    Number,
    Boolean,
    Enum { options: Vec<String> },
}

/// Status of each config tier for the sources footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigTierStatus {
    Available,
    Pending,
    NotApplicable,
}

/// Report of which config tiers were consulted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSourceReport {
    pub acp_native: ConfigTierStatus,
    pub acp_config_options: ConfigTierStatus,
    pub env_vars: ConfigTierStatus,
    pub config_file: ConfigTierStatus,
    pub config_file_path: Option<String>,
    pub mcp_config_file_path: Option<String>,
}

/// Full config surface returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigSurface {
    pub runtime_id: Option<String>,
    pub runtime_label: Option<String>,
    pub is_pre_spawn: bool,
    pub normalized: NormalizedConfig,
    pub advanced: Vec<ConfigField>,
    pub extensions: Vec<ExtensionEntry>,
    pub sources: ConfigSourceReport,
}

/// Raw config values extracted from a runtime's config file.
#[derive(Debug, Clone, Default)]
pub struct RuntimeFileConfig {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub mode: Option<String>,
    pub thinking_effort: Option<String>,
    pub max_output_tokens: Option<String>,
    pub context_limit: Option<String>,
    pub system_prompt: Option<String>,
    pub extensions: Vec<ExtensionEntry>,
    pub extra: BTreeMap<String, String>,
}

/// A detected MCP server or extension from a config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionEntry {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}

/// Cached ACP session config from a running agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigCache {
    pub config_options: Vec<AcpConfigOptionEntry>,
    pub available_modes: Vec<String>,
    pub available_models: Vec<AcpModelEntry>,
    pub current_model: Option<String>,
    /// Whether the harness's `desired_model` was set by a live `SwitchModel`
    /// control signal (true) vs derived from config/persona at spawn (false).
    /// Used by the reader to distinguish a genuine runtime override from a
    /// stale session whose persona model was edited mid-life.
    #[serde(default)]
    pub model_overridden: bool,
    pub goose_native_config: Option<serde_json::Value>,
    pub captured_at: String,
}

/// A single ACP configOption from session/new.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigOptionEntry {
    pub config_id: String,
    pub category: Option<String>,
    pub display_name: Option<String>,
    pub current_value: Option<String>,
    pub options: Vec<AcpConfigOptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpConfigOptionValue {
    pub value: String,
    pub display_name: Option<String>,
}

/// A model entry from ACP session/new.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpModelEntry {
    pub model_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}
