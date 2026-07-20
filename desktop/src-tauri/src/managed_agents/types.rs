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
pub struct AgentDefinition {
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
    /// NIP-AP behavioral defaults, stored in WIRE shape (kebab-case string,
    /// not the `RespondTo` enum) so `persona_event_content` is a verbatim
    /// copy and quad-absent records serialize byte-identically to the
    /// pre-activation era. Copied onto instances at mint time only — spawn
    /// re-snapshot never touches them. Validated at the instance boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
}

impl AgentDefinition {
    /// Project this persona onto a key-less unified [`ManagedAgentRecord`]
    /// (Phase 1A store fold). Identity fields stay empty — keys are minted on
    /// first start. `AgentDefinition.id` becomes `slug`, preserving the 30175
    /// event coordinate (`d_tag = slug`) across the fold.
    pub fn into_agent_record(self) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: String::new(),
            name: self.display_name.clone(),
            persona_id: None,
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            avatar_url: self.avatar_url,
            acp_command: DEFAULT_ACP_COMMAND.to_string(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: Vec::new(),
            mcp_command: String::new(),
            turn_timeout_seconds: DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: default_agent_parallelism(),
            system_prompt: (!self.system_prompt.is_empty()).then_some(self.system_prompt),
            model: self.model,
            provider: self.provider,
            persona_source_version: None,
            env_vars: self.env_vars,
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: BackendKind::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: Vec::new(),
            display_name: Some(self.display_name),
            slug: Some(self.id),
            runtime: self.runtime,
            name_pool: self.name_pool,
            is_builtin: self.is_builtin,
            is_active: self.is_active,
            source_team: self.source_team,
            source_team_persona_slug: self.source_team_persona_slug,
            definition_respond_to: self.respond_to,
            definition_respond_to_allowlist: self.respond_to_allowlist,
            definition_parallelism: self.parallelism,
            relay_mesh: None,
        }
    }
}

impl ManagedAgentRecord {
    /// Present a key-less definition record back in the legacy
    /// [`AgentDefinition`] shape — the compatibility view the persona command
    /// surface serves until Phase 1B unifies the UI. Inverse of
    /// [`AgentDefinition::into_agent_record`] for the fields personas carry.
    pub fn to_definition_view(&self) -> Option<AgentDefinition> {
        let slug = self.slug.clone()?;
        Some(AgentDefinition {
            id: slug,
            display_name: self
                .display_name
                .clone()
                .unwrap_or_else(|| self.name.clone()),
            avatar_url: self.avatar_url.clone(),
            system_prompt: self.system_prompt.clone().unwrap_or_default(),
            runtime: self.runtime.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            name_pool: self.name_pool.clone(),
            is_builtin: self.is_builtin,
            is_active: self.is_active,
            source_team: self.source_team.clone(),
            source_team_persona_slug: self.source_team_persona_slug.clone(),
            env_vars: self.env_vars.clone(),
            respond_to: self.definition_respond_to.clone(),
            respond_to_allowlist: self.definition_respond_to_allowlist.clone(),
            parallelism: self.definition_parallelism,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        })
    }
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
    /// Team this instance was deployed from. Resolves runtime team instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
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
    /// Create-time snapshot of the catalog MCP command. Never read at spawn —
    /// the effective MCP command is always re-derived from the runtime catalog
    /// (`known_acp_runtime`) — and no longer written by updates. Kept for
    /// serde compatibility with existing stores.
    pub mcp_command: String,
    /// Deprecated: `BUZZ_ACP_TURN_TIMEOUT` is ignored by the harness and the
    /// desktop no longer emits or edits it. Kept for serde compatibility with
    /// existing stores; use `idle_timeout_seconds` or
    /// `max_turn_duration_seconds` for turn-length control.
    pub turn_timeout_seconds: u64,
    /// Idle timeout in seconds (`BUZZ_ACP_IDLE_TIMEOUT`): how long the agent
    /// may stay silent on its ACP channel mid-turn before the harness times
    /// the turn out.
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
    /// Environment variables injected at spawn time. Layered as: desktop
    /// parent env < persona `env_vars` < this agent's `env_vars` (last wins).
    ///
    /// To "override" a persona env var: set the same key here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default = "default_start_on_app_launch")]
    pub start_on_app_launch: bool,
    /// Auto-restart this agent when its effective spawn config drifts from
    /// the running process (Chunk F). Default ON; the policy loop in the
    /// frontend only fires when the agent is idle, connected, and local.
    #[serde(default = "default_auto_restart_on_config_change")]
    pub auto_restart_on_config_change: bool,
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
    #[serde(default)]
    pub last_error_code: Option<i64>,
    /// Inbound author gate mode. Translates to `BUZZ_ACP_RESPOND_TO`.
    #[serde(default)]
    pub respond_to: RespondTo,
    /// Allowlist used when `respond_to == Allowlist`. Stored normalized
    /// (64-char lowercase hex, deduped). Empty when mode is not Allowlist.
    /// Preserved across mode toggles so users don't lose state.
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    /// Optional display name distinct from the unique `name` handle. Absorbed
    /// from `AgentDefinition.display_name` (unified agent model, Phase 1A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Stable definition slug — the former `AgentDefinition.id`. Key-less
    /// records (definitions not yet instantiated) publish kind:30175 at
    /// `d_tag = slug`, preserving the pre-merge event coordinates. `None` for
    /// agents created directly (never persona-backed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Absorbed from `AgentDefinition.runtime` — the preferred ACP runtime ID
    /// (e.g. 'goose', 'claude'). Record-first command resolution reads this
    /// before falling back to legacy persona lookup; populated by the store
    /// migration and at create time, and re-mirrored from the linked
    /// definition at every snapshot apply (`apply_persona_snapshot`).
    ///
    /// `None` means "inherit from the linked definition" (the Inherit sentinel
    /// clears it). Serialization then omits the key, so boot-time
    /// `materialize_agent_runtimes` re-inserts a mirror of the definition's
    /// current runtime on the next launch — behaviorally identical, because
    /// every apply site re-mirrors the live definition anyway. A literal
    /// `"runtime": null` in the store (key present, e.g. hand-edited) is
    /// honored: materialization skips it and it deserializes to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Pool of short thematic names for clones of this agent. Absorbed from
    /// `AgentDefinition.name_pool`; feeds clone naming.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    /// Absorbed from `AgentDefinition.is_builtin`.
    #[serde(default)]
    pub is_builtin: bool,
    /// Absorbed from `AgentDefinition.is_active` — `false` means an archived
    /// definition hidden from pickers. Defaults `true` for existing records.
    #[serde(default = "default_record_active")]
    pub is_active: bool,
    /// Absorbed from `AgentDefinition.source_team` — team ID when this
    /// definition was imported from a team directory (team definitions are
    /// non-editable). Distinct from `persona_team_dir`/`persona_name_in_team`,
    /// which are the instance-side spawn plumbing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_team: Option<String>,
    /// Absorbed from `AgentDefinition.source_team_persona_slug` — the
    /// definition's slug within its source team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_team_persona_slug: Option<String>,
    /// NIP-AP definition-level behavioral defaults, absorbed from
    /// `AgentDefinition` in WIRE shape (kebab-case string / optional u32),
    /// distinct from the instance-side `respond_to`/`respond_to_allowlist`/
    /// `parallelism` fields above: these are what a *definition* advertises
    /// and are copied onto instances at mint time only. Wire shape (not the
    /// `RespondTo` enum) so absent-ness and unknown future mode strings
    /// round-trip byte-identically through the store — parsed/validated
    /// solely at the mint boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_respond_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definition_respond_to_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_parallelism: Option<u32>,
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
    /// Digest of the effective spawn config at launch (see
    /// `spawn_hash::spawn_config_hash`). Runtime-only — never persisted. The
    /// summary builder recomputes the hash from current disk state and flags
    /// `needs_restart` on mismatch. Agents adopted via a persisted
    /// `runtime_pid` have no `ManagedAgentProcess` entry, so their spawn
    /// config is unknown and the badge stays off.
    pub spawn_config_hash: u64,
    /// Whether this process was spawned in setup-listener mode (i.e.
    /// `BUZZ_ACP_SETUP_PAYLOAD` was set at launch because the agent was
    /// `NotReady`). Runtime-only — never persisted. Used by
    /// `install_acp_runtime` to target only stuck agents for auto-restart,
    /// excluding healthy in-pool agents.
    pub setup_mode: bool,
    /// Adapter availability status stamped at spawn time for runtimes with a
    /// version gate (currently codex only; `None` for all others). Runtime-only
    /// — never persisted. The summary builder compares this against the current
    /// cached availability and sets `needs_restart` on drift, catching out-of-
    /// band adapter changes that Phase-1 auto-restart doesn't cover.
    pub adapter_availability: Option<AcpAvailabilityStatus>,
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
    pub team_id: Option<String>,
    pub relay_url: String,
    pub acp_command: String,
    pub agent_command: String,
    /// Mirrors `ManagedAgentRecord.agent_command_override`: `Some` when the user
    /// has explicitly pinned this instance's harness, `None` when it inherits
    /// from the persona. Lets the Edit dialog seed "Inherit from persona" vs a
    /// concrete pin (`agent_command` above is the resolved/effective command).
    pub agent_command_override: Option<String>,
    pub agent_args: Vec<String>,
    /// Catalog-derived from the effective harness (not the record's stored
    /// field), so the UI always shows what a spawn would actually use.
    pub mcp_command: String,
    /// Deprecated passthrough of the stored record value; the harness ignores
    /// it. Kept for wire compatibility.
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
    /// `true` when the running process was spawned with a config that no
    /// longer matches what a spawn would use today — a plain restart would
    /// change what runs. Complements `persona_out_of_date`: the badge means
    /// "a restart would change what runs"; out-of-date means "a respawn
    /// would." Always `false` for stopped agents and for processes adopted
    /// via a persisted `runtime_pid` (their spawn config is unknown).
    pub needs_restart: bool,
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
    pub last_error_code: Option<i64>,
    pub start_on_app_launch: bool,
    pub auto_restart_on_config_change: bool,
    pub log_path: String,
    pub respond_to: RespondTo,
    pub respond_to_allowlist: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateManagedAgentResponse {
    pub agent: ManagedAgentSummary,
    pub private_key_nsec: String,
    pub profile_sync_error: Option<String>,
    pub spawn_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ManagedAgentLogResponse {
    pub content: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcpAvailabilityStatus {
    Available,
    AdapterMissing,
    /// Adapter binary is present but is from the deprecated package (< 1.0). Reinstall required.
    AdapterOutdated,
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
    pub install_hint: String,
    pub install_instructions_url: String,
    /// true when at least one automated install step is available
    pub can_auto_install: bool,
    pub underlying_cli_path: Option<String>,
    /// true when an npm adapter step is pending but Node.js / npm is absent.
    /// The UI hides the Install button and shows a Node.js install callout.
    pub node_required: bool,
    /// Login/authentication status for CLI-based runtimes.
    pub auth_status: AuthStatus,
    /// Hint for completing authentication, shown when `auth_status` is not `logged_in`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,
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
    /// Runtime-layered instructions shared by every member deployment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
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
    pub instructions: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    #[serde(default)]
    pub persona_ids: Vec<String>,
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

fn default_auto_restart_on_config_change() -> bool {
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

    /// Parse the NIP-AP wire string. Definitions carry `respond_to` as
    /// opaque data everywhere else; this is the single parse boundary
    /// (instance mint), and an unrecognized mode fails LOUDLY here rather
    /// than silently defaulting — a typo'd definition must not mint an
    /// agent with a different audience than its author intended.
    pub fn parse_wire(value: &str) -> Result<Self, String> {
        match value {
            "owner-only" => Ok(Self::OwnerOnly),
            "allowlist" => Ok(Self::Allowlist),
            "anyone" => Ok(Self::Anyone),
            other => Err(format!(
                "definition respond_to '{other}' is not a recognized mode (expected 'owner-only', 'allowlist', or 'anyone')"
            )),
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

/// The behavioral fields resolved for a new instance at mint time.
#[derive(Debug, PartialEq, Eq)]
pub struct MintBehavioralDefaults {
    pub respond_to: RespondTo,
    pub respond_to_allowlist: Vec<String>,
    /// Validated (1..=32) when present; caller applies its own default.
    pub parallelism: Option<u32>,
}

/// Resolve the NIP-AP behavioral quad for a new instance: explicit input
/// wins, then the linked definition's defaults, then client defaults.
///
/// This is the ONLY place definition behavioral strings are parsed — an
/// unrecognized `respond_to` mode or out-of-range `parallelism` on a
/// definition fails the mint loudly instead of silently substituting a
/// default the definition author did not choose. The empty-allowlist guard
/// fires here too, because inbound definitions bypass the dialog entirely.
///
/// `input_allowlist` must already be normalized via
/// [`validate_respond_to_allowlist`]; the definition's allowlist is
/// validated here since it arrives from the wire.
pub fn resolve_mint_behavioral_defaults(
    input_respond_to: Option<RespondTo>,
    input_allowlist: Vec<String>,
    input_parallelism: Option<u32>,
    definition: Option<&AgentDefinition>,
) -> Result<MintBehavioralDefaults, String> {
    let (respond_to, respond_to_allowlist) = match input_respond_to {
        // Explicit instance-level choice: the definition default is ignored
        // wholesale (mode AND list travel together).
        Some(mode) => (mode, input_allowlist),
        None => match definition.and_then(|d| d.respond_to.as_deref()) {
            Some(wire) => {
                let mode = RespondTo::parse_wire(wire)?;
                let list = if input_allowlist.is_empty() {
                    validate_respond_to_allowlist(
                        definition
                            .map(|d| d.respond_to_allowlist.as_slice())
                            .unwrap_or(&[]),
                    )
                    .map_err(|e| format!("definition respond-to allowlist is invalid: {e}"))?
                } else {
                    input_allowlist
                };
                (mode, list)
            }
            None => (RespondTo::default(), input_allowlist),
        },
    };
    if respond_to == RespondTo::Allowlist && respond_to_allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }

    let parallelism = match input_parallelism {
        // Explicit input is validated here too (not just at the command
        // call sites) so the "validated when present" contract on
        // `MintBehavioralDefaults.parallelism` is unskippable.
        Some(count) if (1..=32).contains(&count) => Some(count),
        Some(count) => {
            return Err(format!(
                "parallelism {count} is out of range (must be between 1 and 32)"
            ))
        }
        None => match definition.and_then(|d| d.parallelism) {
            Some(count) if (1..=32).contains(&count) => Some(count),
            Some(count) => {
                return Err(format!(
                    "parallelism {count} on the linked agent definition is out of range (must be between 1 and 32)"
                ))
            }
            None => None,
        },
    };

    Ok(MintBehavioralDefaults {
        respond_to,
        respond_to_allowlist,
        parallelism,
    })
}

mod requests;
pub use requests::*;

#[cfg(test)]
mod tests;
