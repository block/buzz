//! Wire types for the ACP runtime catalog: which harnesses this install can
//! run, whether each is installed and logged in, and the results of trying to
//! install one.
//!
//! Split out of `types.rs`, which had reached the desktop file-size ceiling;
//! these are the harness/prerequisite DTOs and share no state with the
//! managed-agent record types left behind. Re-exported through
//! `managed_agents::*`, so every existing import path still resolves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpAvailabilityStatus {
    Available,
    AdapterMissing,
    /// Adapter binary is present but unsupported — either the deprecated
    /// package or a version below the supported floor. Reinstall required.
    AdapterOutdated,
    CliMissing,
    NotInstalled,
}

/// Authentication/login status for a CLI-based ACP runtime. Serializes as a tagged union
/// `{ status: "...", diagnostic?: "..." }` so the TypeScript side can exhaustively switch on `status`.
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

/// Origin of an ACP runtime catalog entry. Serializes as a lowercase string so the TypeScript consumer can switch on it without numeric comparisons.
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

/// Result of a single install step (CLI or adapter).
#[derive(Debug, Clone, Serialize)]
pub struct InstallStepResult {
    pub step: String,
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    /// Actionable guidance shown in the UI when this step failed due to a
    /// recognized condition (e.g. EACCES writing Buzz's private npm prefix).
    /// `None` when the step succeeded or no pattern matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Aggregate result of installing a runtime (may include CLI + adapter steps).
#[derive(Debug, Clone, Serialize)]
pub struct InstallRuntimeResult {
    pub success: bool,
    pub steps: Vec<InstallStepResult>,
    /// Number of local agents successfully stopped and restarted after a
    /// successful install. Mirrors `GlobalAgentConfigSaveResult.restarted_count`.
    pub restarted_count: u32,
    /// Number of agents whose stop succeeded but respawn failed.
    /// Mirrors `GlobalAgentConfigSaveResult.failed_restart_count`.
    pub failed_restart_count: u32,
    /// Install log file for this run, when one was written. The UI surfaces it
    /// on failure so a user can read the full retry history instead of only the
    /// last step's truncated output. `None` when no log could be opened.
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandAvailabilityInfo {
    pub command: String,
    pub resolved_path: Option<String>,
    pub available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverManagedAgentPrereqsRequest {
    pub acp_command: Option<String>,
    pub mcp_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedAgentPrereqsInfo {
    pub acp: CommandAvailabilityInfo,
    pub mcp: CommandAvailabilityInfo,
}
