use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AcpAvailabilityStatus, AuthStatus};

/// Operational readiness of an ACP runtime, independent from executable
/// installation and authentication status.
///
/// Authentication remains a separate fact because a runtime can be
/// operational with keyless models while its login state is unknown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeReadinessStatus {
    Ready,
    ConfigurationRequired,
    AuthenticationRequired,
    ModelUnavailable,
    Unknown,
}

/// Input treatment for a runtime-specific setup field projected by the
/// Rust-owned catalog. Values remain in local managed-agent environment maps;
/// the catalog carries only field metadata.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSetupFieldKind {
    Url,
    Secret,
}

/// Validation policy for a runtime setup field. The frontend renders and
/// validates from this catalog value; it never infers rules from a runtime id.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSetupFieldValidation {
    TailnetHttpsOrigin,
    AppPassword,
}

/// One required runtime-specific environment field rendered as first-class
/// setup UX instead of an unlabeled Advanced environment row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeSetupField {
    pub env_key: String,
    pub label: String,
    pub placeholder: String,
    pub helper_text: String,
    pub kind: RuntimeSetupFieldKind,
    pub validation: RuntimeSetupFieldValidation,
    pub required: bool,
}

/// Origin of an ACP runtime catalog entry. Serializes as a lowercase string so the TypeScript consumer can switch on it without numeric comparisons.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSource {
    /// Rich capability metadata compiled into the app. Source/trust is
    /// independent from metadata richness; a rich runtime may still be a
    /// manually installed preset.
    Builtin,
    /// Static preset entry with bundled logo, PATH-probed, not editable/deletable.
    Preset,
    /// Loaded at runtime from the user's `custom_harnesses/` directory.
    Custom,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcpRuntimeCatalogEntry {
    pub id: String,
    pub label: String,
    pub avatar_url: String,
    pub availability: AcpAvailabilityStatus,
    pub command: Option<String>,
    pub binary_path: Option<String>,
    pub default_args: Vec<String>,
    pub mcp_command: Option<String>,
    /// Environment variable used to apply the initial model, when supported.
    pub model_env_var: Option<String>,
    /// Environment variable used to apply the selected LLM provider, when supported.
    pub provider_env_var: Option<String>,
    /// Environment variable used to apply thinking effort, when supported.
    pub thinking_env_var: Option<String>,
    pub max_tokens_env_var: Option<String>,
    pub context_limit_env_var: Option<String>,
    pub max_rounds_env_var: Option<String>,
    pub install_hint: String,
    pub install_instructions_url: String,
    /// true when at least one automated install step is available
    pub can_auto_install: bool,
    /// true when this runtime depends on a separately installed vendor CLI.
    pub requires_external_cli: bool,
    pub underlying_cli_path: Option<String>,
    /// true when an npm adapter step is pending but Node.js / npm is absent.
    /// The UI hides the Install button and shows a Node.js install callout.
    pub node_required: bool,
    /// Login/authentication status for CLI-based runtimes.
    pub auth_status: AuthStatus,
    /// Operational readiness derived from the Rust-owned runtime policy.
    pub runtime_readiness: RuntimeReadinessStatus,
    /// Runtime-specific local setup fields. This metadata never contains the
    /// configured values themselves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup_fields: Vec<RuntimeSetupField>,
    /// Whether this runtime exposes an account/setup connection flow. This is
    /// a metadata capability, not evidence that authentication is required.
    pub can_connect_account: bool,
    /// Hint for completing authentication, shown when `auth_status` is not `logged_in`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,
    /// Whether this entry came from the compiled-in catalog or a user-supplied
    /// JSON file in `custom_harnesses/`. The UI uses this to decide editability.
    pub source: HarnessSource,
    /// Definition-level env vars for `source: custom` entries; populated from
    /// `HarnessDefinition.env` so saves don't silently erase existing vars.
    /// Absent for builtin/preset entries. Skipped when empty in serialization.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub definition_env: BTreeMap<String, String>,
    /// Spawn-time parallelism cap; absent for uncapped harnesses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
}
