use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpAvailabilityStatus {
    Available,
    AdapterMissing,
    /// Adapter binary is present but is from the deprecated package (< 1.0). Reinstall required.
    AdapterOutdated,
    /// Vendor CLI is present but below Buzz's minimum supported version.
    CliOutdated,
    CliMissing,
    NotInstalled,
}

/// Authentication/login status for a CLI-based ACP runtime.
///
/// Serializes as a tagged union `{ status: "...", diagnostic?: "..." }` so
/// the TypeScript side can exhaustively switch on `status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum AuthStatus {
    /// The CLI reported a successful login.
    LoggedIn,
    /// The CLI exited non-zero without a config-parse signal.
    LoggedOut,
    /// The CLI exited non-zero and its stderr contains a config-parse error.
    ConfigInvalid {
        /// Trimmed excerpt of the stderr message.
        diagnostic: String,
    },
    /// This runtime does not have a login step (e.g. goose, buzz-agent).
    NotApplicable,
    /// Probe was not attempted (runtime unavailable or probe timed out).
    Unknown,
}

/// Origin of an ACP runtime catalog entry. Serializes as a lowercase string
/// so the TypeScript consumer can switch on it without numeric comparisons.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSource {
    /// Compiled into the app — one of the four first-class runtimes.
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
    /// Detected vendor CLI version, when this runtime has a version gate.
    pub cli_version: Option<String>,
    /// Minimum vendor CLI version required by this Buzz build, when gated.
    pub minimum_cli_version: Option<String>,
    pub default_args: Vec<String>,
    pub mcp_command: Option<String>,
    /// Environment variable used to apply the initial model, when supported.
    pub model_env_var: Option<String>,
    /// Environment variable used to apply the selected LLM provider, when supported.
    pub provider_env_var: Option<String>,
    /// Environment variable used to apply thinking effort, when supported.
    pub thinking_env_var: Option<String>,
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
    /// Hint for completing authentication, shown when `auth_status` is not `logged_in`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,
    /// Whether this entry came from the compiled-in catalog or a user-supplied
    /// JSON file in `custom_harnesses/`. The UI uses this to decide editability.
    pub source: HarnessSource,
    /// Definition-level environment variables for `source: custom` entries.
    ///
    /// Populated from `HarnessDefinition.env` so the edit form can read them
    /// back and the user doesn't silently lose env vars when saving. Always
    /// empty for `builtin` and `preset` entries (those env values come from the
    /// runtime metadata path, not user-editable JSON).
    ///
    /// Skipped in serialization when empty to keep the catalog payload compact.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub definition_env: BTreeMap<String, String>,
}
