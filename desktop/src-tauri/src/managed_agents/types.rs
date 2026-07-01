use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, process::Child};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    Local,
    Provider {
        id: String,
        config: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaRecord {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    /// Preferred ACP runtime ID (e.g., 'goose', 'claude', 'codex'). Determines which agent binary
    /// Buzz spawns. When deploying from this persona, this runtime is pre-selected in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Opaque, harness-specific model identifier string. Format depends on the runtime and its LLM
    /// provider (e.g., 'goose-claude-4-6-opus' for Databricks, 'claude-opus-4-7' for Anthropic
    /// direct). Buzz stores and passes through without interpretation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// LLM inference provider (e.g., 'databricks', 'anthropic', 'openai'). Optional — when set,
    /// injected as the runtime's provider env var at agent creation time. When absent, the runtime
    /// falls back to auto-detection (e.g., goose config file or available credentials).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Pool of short, thematic names for bot instances created from this persona.
    /// When a new copy is added to a channel, a random unused name is picked from this pool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default = "default_record_active")]
    pub is_active: bool,
    /// Team ID if this persona was imported from a team directory.
    /// Team personas are non-editable (system_prompt, model locked).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "source_pack"
    )]
    pub source_team: Option<String>,
    /// Internal persona slug within the team (e.g., "lep", "pip").
    /// Used by ACP's `resolve_persona_by_name()` to find the right persona.
    /// Validated: `[a-zA-Z0-9_-]+`, max 64 chars (safe for env vars and paths).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "source_pack_persona_slug"
    )]
    pub source_team_persona_slug: Option<String>,
    /// Harness-level configuration passed to the agent subprocess as environment variables.
    /// Opaque to Buzz — keys and values are runtime-specific.
    ///
    /// Stored as a BTreeMap for deterministic on-disk ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayAgentInfo {
    pub pubkey: String,
    pub name: String,
    pub agent_type: String,
    pub channels: Vec<String>,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    pub capabilities: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub respond_to: Option<RespondTo>,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAgentRecord {
    pub pubkey: String,
    pub name: String,
    #[serde(default)]
    pub persona_id: Option<String>,
    /// nsec private key. Held in memory but persisted to the OS keyring (keyed
    /// by `pubkey`) rather than serialized to `managed-agents.json`. The
    /// storage layer blanks this before writing JSON once the key is safely in
    /// the keyring, and re-hydrates it from the keyring on load.
    ///
    /// It is only serialized inline (the `0o600` JSON fallback) when the
    /// keyring is unreachable — `skip_serializing_if` keeps it out of JSON in
    /// the normal keyring-backed case. `default` also lets an old build parse a
    /// store whose inline key was already migrated out and blanked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key_nsec: String,
    /// NIP-OA auth tag JSON. Computed at agent creation time.
    ///
    /// Pre-existing agents created before NIP-OA will have `None` here.
    /// This is intentional — they continue to work without attestation.
    /// Re-attestation requires agent recreation (v2 migration scope).
    #[serde(default)]
    pub auth_tag: Option<String>,
    pub relay_url: String,
    /// Avatar URL resolved at creation time (user-supplied input, else the
    /// command-based fallback). Persisted so startup reconciliation compares
    /// against what was actually published rather than re-deriving it from
    /// persona config — which would silently overwrite user intent on restart.
    /// `#[serde(default)]` so pre-existing records deserialize as `None`.
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub acp_command: String,
    pub agent_command: String,
    /// Explicit per-instance harness pin. `None` (the default) means inherit
    /// the harness from the linked persona's `runtime`, so persona harness
    /// edits propagate on the next spawn — mirroring the opt-in `model`
    /// override. `Some` is set only when the user deliberately picks a harness
    /// that diverges from the persona. Resolved via `effective_agent_command`;
    /// `agent_command` above is the create-time snapshot kept for avatar/legacy
    /// derivations and is not authoritative for spawn.
    #[serde(default)]
    pub agent_command_override: Option<String>,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub turn_timeout_seconds: u64,
    /// Idle timeout in seconds. If set, overrides turn_timeout_seconds.
    #[serde(default)]
    pub idle_timeout_seconds: Option<u64>,
    /// Absolute wall-clock cap per turn.
    #[serde(default)]
    pub max_turn_duration_seconds: Option<u64>,
    #[serde(default = "default_agent_parallelism")]
    pub parallelism: u32,
    pub system_prompt: Option<String>,
    /// Desired LLM model ID. Matches AgentModelInfo.id from discovery.
    /// The harness re-discovers the correct ACP switching metadata at session
    /// creation by matching this ID against the fresh session/new response.
    #[serde(default)]
    pub model: Option<String>,
    /// LLM inference provider snapshotted from the persona at create time
    /// (e.g. 'databricks', 'anthropic'). Spawn and deploy read this, never the
    /// live persona — so the agent stays pinned to the provider it was created
    /// with across restarts. `#[serde(default)]` so pre-existing records
    /// deserialize as `None` and get backfilled on first load.
    #[serde(default)]
    pub provider: Option<String>,
    /// Content hash of the persona at the time this agent was created — the
    /// `persona_content_hash` of the snapshot in `system_prompt` / `model` /
    /// `provider` / `env_vars`. The Agents menu compares it against the linked
    /// persona's current hash to flag a stale (out-of-date) instance. `None`
    /// for non-persona agents and for pre-existing records pending backfill.
    #[serde(default)]
    pub persona_source_version: Option<String>,
    /// Comma-separated toolset string forwarded as BUZZ_TOOLSETS to the MCP subprocess.
    /// When None, the MCP server uses its own default ("default" toolset).
    #[serde(default)]
    pub mcp_toolsets: Option<String>,
    /// Environment variables injected at spawn time. Layered as: desktop
    /// parent env < persona `env_vars` < this agent's `env_vars` (last wins).
    ///
    /// To "override" a persona env var: set the same key here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    #[serde(default)]
    pub runtime_pid: Option<u32>,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub backend_agent_id: Option<String>,
    #[serde(default)]
    pub provider_binary_path: Option<String>,
    /// Installed team directory path (absolute). Set when agent was created from a team persona.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "persona_pack_path"
    )]
    pub persona_team_dir: Option<PathBuf>,
    /// Persona name within the team.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "persona_name_in_pack"
    )]
    pub persona_name_in_team: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_started_at: Option<String>,
    pub last_stopped_at: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
    /// Inbound author gate mode. Translates to `BUZZ_ACP_RESPOND_TO`.
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Allowlist used when `respond_to == Allowlist`. Stored normalized
    /// (64-char lowercase hex, deduped). Empty when mode is not Allowlist.
    /// Preserved across mode toggles so users don't lose state.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    /// Typed marker for relay-mesh agents. `Some(_)` means this agent runs its
    /// inference through Buzz's relay-mesh local endpoint; the `model_ref` is
    /// the served model id to route to. `None` is a normal agent.
    ///
    /// This is the source of truth for "is this a mesh agent + which model" —
    /// replacing the old practice of sniffing it back out of `env_vars`
    /// (`relay_mesh_config`). Spawn-time env vars are *derived from* this, not
    /// the other way around. `#[serde(default)]` so pre-existing saved records
    /// deserialize as `None` and are resolved via the env-var fallback until
    /// they are rewritten with this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_mesh: Option<RelayMeshConfig>,
}

/// Typed relay-mesh configuration carried on a [`ManagedAgentRecord`].
///
/// Feature-independent on purpose: the field is always present in the record
/// schema so saved agents round-trip identically whether or not the `mesh-llm`
/// feature is compiled in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayMeshConfig {
    /// The served model id this agent routes to (e.g. "Qwen3").
    ///
    /// `alias` because this struct crosses two boundaries with different
    /// casing conventions: the TS create request sends camelCase
    /// (`relayMesh: { modelRef }` — `rename_all` on the request does not
    /// recurse into nested structs), while persisted records use snake_case.
    /// Serialization stays `model_ref` so saved records are stable.
    #[serde(alias = "modelRef")]
    pub model_ref: String,
}

#[derive(Debug)]
pub struct ManagedAgentProcess {
    pub child: Child,
    pub log_path: PathBuf,
    /// Win32 Job Object owning the harness + its entire process tree. Closing
    /// the handle (via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) kills the whole
    /// tree — the Windows mirror of the Unix process-group teardown. `None`
    /// if job creation/assignment failed (we fall back to `Child::kill()`).
    #[cfg(windows)]
    pub job: Option<crate::managed_agents::JobHandle>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagedAgentSummary {
    pub pubkey: String,
    pub name: String,
    pub persona_id: Option<String>,
    pub relay_url: String,
    pub acp_command: String,
    pub agent_command: String,
    /// Mirrors `ManagedAgentRecord.agent_command_override`: `Some` when the user
    /// has explicitly pinned this instance's harness, `None` when it inherits
    /// from the persona. Lets the Edit dialog seed "Inherit from persona" vs a
    /// concrete pin (`agent_command` above is the resolved/effective command).
    pub agent_command_override: Option<String>,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub turn_timeout_seconds: u64,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: u32,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub model: Option<String>,
    /// LLM inference provider, from the agent's pinned record snapshot.
    pub provider: Option<String>,
    /// `true` when the linked persona has been edited since this agent was
    /// created — the running agent uses the older pinned snapshot. The UI
    /// flags it and tells the user to delete + respawn to pick up the edit.
    /// Always `false` for non-persona agents and for orphaned agents (their
    /// persona is gone, so there is nothing newer to drift toward).
    pub persona_out_of_date: bool,
    /// `true` when the agent was created from a persona that no longer exists.
    /// Distinct from out-of-date: there is no current persona to respawn into,
    /// so the UI should not prompt a respawn — the pinned snapshot is all the
    /// config that remains.
    pub persona_orphaned: bool,
    pub mcp_toolsets: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    pub backend: BackendKind,
    pub backend_agent_id: Option<String>,
    pub status: String,
    pub pid: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    pub last_started_at: Option<String>,
    pub last_stopped_at: Option<String>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
    pub start_on_app_launch: bool,
    pub log_path: String,
    pub respond_to: RespondTo,
    pub respond_to_allowlist: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateManagedAgentRequest {
    pub name: String,
    #[serde(default)]
    pub persona_id: Option<String>,
    pub relay_url: Option<String>,
    pub acp_command: Option<String>,
    pub agent_command: Option<String>,
    /// True when `agent_command` is a runtime command the user deliberately
    /// picked for a linked persona. Distinguishes a real selection, including an
    /// installed alias, from a missing-runtime fallback so a persona-backed
    /// create only stores an `agent_command_override` for the former.
    #[serde(default)]
    pub harness_override: bool,
    #[serde(default)]
    pub agent_args: Vec<String>,
    pub mcp_command: Option<String>,
    pub turn_timeout_seconds: Option<u64>,
    pub idle_timeout_seconds: Option<u64>,
    pub max_turn_duration_seconds: Option<u64>,
    pub parallelism: Option<u32>,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub mcp_toolsets: Option<String>,
    /// Environment variables for this agent. Layered on top of persona env.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub spawn_after_create: bool,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Raw allowlist as received from the frontend. Validated and normalized
    /// before being written to the record.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default)]
    pub relay_mesh: Option<RelayMeshConfig>,
}

#[derive(Debug, Serialize)]
pub struct CreateManagedAgentResponse {
    pub agent: ManagedAgentSummary,
    pub private_key_nsec: String,
    pub profile_sync_error: Option<String>,
    pub spawn_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePersonaRequest {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonaRequest {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    ///
    /// Absent (`None`) = don't touch the stored value (caller didn't include
    /// the field). `Some(map)` = replace entirely (empty map clears all).
    /// Defaulting an omitted field to an empty map would silently erase
    /// stored credentials when an unrelated field is edited.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct ManagedAgentLogResponse {
    pub content: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpAvailabilityStatus {
    Available,
    AdapterMissing,
    CliMissing,
    NotInstalled,
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
    pub install_hint: String,
    pub install_instructions_url: String,
    /// true when at least one automated install step is available
    pub can_auto_install: bool,
    pub underlying_cli_path: Option<String>,
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
}

/// Aggregate result of installing a runtime (may include CLI + adapter steps).
#[derive(Debug, Clone, Serialize)]
pub struct InstallRuntimeResult {
    pub success: bool,
    pub steps: Vec<InstallStepResult>,
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

/// Patch request for updating a managed agent's mutable fields.
///
/// Tri-state nullable semantics via `Option<Option<T>>`:
/// - Field absent in JSON → `None` (don't touch)
/// - `"field": null` → `Some(None)` (clear to default)
/// - `"field": "value"` → `Some(Some("value"))` (set)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManagedAgentRequest {
    pub pubkey: String,
    /// Absent = don't touch. Present = rename the agent.
    #[serde(default)]
    pub name: Option<String>,
    /// Absent = don't touch. null = clear to agent default. "id" = set.
    #[serde(default)]
    pub model: Option<Option<String>>,
    #[serde(default)]
    pub system_prompt: Option<Option<String>>,
    #[serde(default)]
    pub mcp_toolsets: Option<Option<String>>,
    /// Absent = don't touch. Present = replace the env_vars map entirely.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub parallelism: Option<u32>,
    #[serde(default)]
    pub turn_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub acp_command: Option<String>,
    #[serde(default)]
    pub agent_command: Option<String>,
    #[serde(default)]
    pub agent_args: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_command: Option<String>,
    /// Absent = don't touch. null = clear to runtime default. "id" = set.
    #[serde(default, deserialize_with = "crate::util::double_option")]
    pub provider: Option<Option<String>>,
    /// Absent = don't touch. Present = set mode.
    #[serde(default)]
    pub respond_to: Option<RespondTo>,
    /// Absent = don't touch. Present = replace the allowlist (validated &
    /// normalized server-side).
    #[serde(default)]
    pub respond_to_allowlist: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct UpdateManagedAgentResponse {
    pub agent: ManagedAgentSummary,
    pub profile_sync_error: Option<String>,
}

/// Response from `get_agent_models` — normalized model info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelsResponse {
    pub agent_name: String,
    pub agent_version: String,
    /// Unified model list (merged from both ACP paths, deduplicated by ID).
    pub models: Vec<AgentModelInfo>,
    /// The agent's default model for a fresh session.
    pub agent_default_model: Option<String>,
    /// The user's persisted model selection (from ManagedAgentRecord.model).
    pub selected_model: Option<String>,
    /// Whether this agent supports model switching.
    pub supports_switching: bool,
}

/// A single model available from an agent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelInfo {
    /// Canonical ID used for persistence and round-tripping.
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub persona_ids: Vec<String>,
    #[serde(default)]
    pub is_builtin: bool,
    /// Absolute path to the team's backing directory (if directory-backed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<PathBuf>,
    /// Whether `source_dir` is a symlink to an external directory.
    #[serde(default)]
    pub is_symlink: bool,
    /// Resolved symlink target path (for display). Only set when `is_symlink` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
    /// Version from the team's `plugin.json` manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

/// Result of syncing a directory-backed team with its backing directory.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub personas_added: Vec<String>,
    pub personas_removed: Vec<String>,
    pub personas_updated: Vec<String>,
    pub metadata_changed: bool,
}

/// Report from the one-time packs→teams migration.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub packs_migrated: usize,
    pub personas_updated: usize,
    pub agents_updated: usize,
    pub errors: Vec<String>,
}

pub const DEFAULT_ACP_COMMAND: &str = "buzz-acp";
/// ~5 min (320s) — matches the CLI harness default (BUZZ_ACP_IDLE_TIMEOUT).
pub const DEFAULT_AGENT_TURN_TIMEOUT_SECONDS: u64 = 320;
/// 1 hour — absolute wall-clock safety cap per turn.
pub const DEFAULT_AGENT_MAX_TURN_DURATION_SECONDS: u64 = 3600;
pub const DEFAULT_AGENT_PARALLELISM: u32 = 24;

fn default_agent_parallelism() -> u32 {
    DEFAULT_AGENT_PARALLELISM
}

fn default_start_on_app_launch() -> bool {
    true
}

fn default_record_active() -> bool {
    true
}

// ── Inbound author gate ──────────────────────────────────────────────────────
//
// Mirrors `buzz-acp`'s `--respond-to` CLI flag and the related
// `--respond-to-allowlist` option. Persisted per agent so the desktop can
// translate the user's choice into `BUZZ_ACP_RESPOND_TO` /
// `BUZZ_ACP_RESPOND_TO_ALLOWLIST` env vars at spawn time.
//
// Wire format is kebab-case (`owner-only`, `allowlist`, `anyone`) to match
// the harness CLI vocabulary and the strings the GUI emits.
//
// `nobody` is intentionally NOT exposed here. The harness supports it, but
// it's a heartbeat-only mode and the desktop has no surface for it.

/// Who the agent should respond to. Defaults to `OwnerOnly`, which matches
/// the harness default → existing agents behave identically.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RespondTo {
    #[default]
    OwnerOnly,
    Allowlist,
    Anyone,
}

impl RespondTo {
    /// CLI/env wire string (matches `buzz-acp`'s `--respond-to`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerOnly => "owner-only",
            Self::Allowlist => "allowlist",
            Self::Anyone => "anyone",
        }
    }
}

/// Validate and normalize a respond-to allowlist.
///
/// Rules mirror `buzz-acp/src/config.rs::validate_allowlist`:
/// - Each entry is exactly 64 hex chars (any case in, lowercase out).
/// - Duplicates removed, insertion order preserved.
///
/// Empty input is allowed here — the boundary check (allowlist mode requires
/// at least one entry) is the caller's job, because an `UpdateManagedAgentRequest`
/// may want to validate a list without yet knowing the final mode.
pub fn validate_respond_to_allowlist(input: &[String]) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(input.len());
    for entry in input {
        let trimmed = entry.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid pubkey in respond-to allowlist: '{trimmed}' (must be 64 hex chars)"
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
