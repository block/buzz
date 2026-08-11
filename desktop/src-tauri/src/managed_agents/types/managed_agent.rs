use std::{collections::BTreeMap, path::PathBuf, process::Child};

use serde::Serialize;

use super::{AcpAvailabilityStatus, BackendKind, HeartbeatPreflightDesignation, RespondTo};

#[derive(Debug)]
pub struct ManagedAgentProcess {
    pub child: Child,
    pub log_path: PathBuf,
    /// The effective spawn config this process was launched with (see
    /// `spawn_snapshot::SpawnConfigSnapshot`). Runtime-only — never persisted.
    /// The summary builder recomputes a prospective snapshot and reports
    /// differing fields via `ManagedAgentSummary::restart_diff`. Agents
    /// adopted via `runtime_pid` have none; their config is unknown.
    pub spawn_config: crate::managed_agents::spawn_snapshot::SpawnConfigSnapshot,
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
    /// Unpredictable identity shared only with this harness generation.
    pub start_nonce: String,
    /// Exact bundled harness and owner designation used for this process.
    pub heartbeat_harness: Option<crate::managed_agents::HeartbeatHarnessStamp>,
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
    /// The record's harness/runtime id (mirror of `ManagedAgentRecord.runtime`).
    /// Lets the UI count agents referencing a harness definition (e.g. in the
    /// delete-confirmation flow). `None` = inherit from the linked persona.
    pub runtime: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_source: Option<crate::managed_agents::effective_config::ConfigSource>,
    /// LLM inference provider, resolved the same way as `model`/`model_source`
    /// (definition → global for linked instances; instance → global for
    /// definition-less instances). `None` for an orphaned instance.
    pub provider: Option<String>,
    /// `true` when the linked persona has been edited since this agent was
    /// created — the running agent uses the older pinned snapshot. The UI
    /// flags it and tells the user to delete + respawn to pick up the edit.
    /// Always `false` for non-persona agents and for orphaned agents (their
    /// persona is gone, so there is nothing newer to drift toward).
    pub persona_out_of_date: bool,
    /// `true` when the agent was created from a persona that no longer exists.
    /// Distinct from out-of-date: there is no current persona to respawn into.
    /// An orphaned agent also cannot be (re)started — `spawn_agent_child`
    /// refuses it (see `effective_config::resolve_effective_config`'s
    /// `OrphanedInstance` arm via `require_resolved`) — so the UI
    /// should surface that it's stuck, not merely stale.
    pub persona_orphaned: bool,
    /// `true` when the running process's spawn config no longer matches
    /// what a spawn would use today. Derived from `restart_diff` — lit
    /// exactly when there is something to show. Always `false` for stopped,
    /// orphaned, or `runtime_pid`-adopted agents.
    pub needs_restart: bool,
    /// Fields that drifted since launch, redacted for display.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_diff: Vec<crate::managed_agents::spawn_snapshot::RestartDiffEntry>,
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
    pub heartbeat_preflight: Option<HeartbeatPreflightDesignation>,
}
