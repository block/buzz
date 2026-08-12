use std::{path::PathBuf, process::Child};

use super::AcpAvailabilityStatus;

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
    /// Owns the fresh run root. Dropping it after the process tree exits
    /// removes all per-run residue before another session can start.
    #[allow(dead_code)] // lifecycle ownership is the read: Drop performs cleanup
    pub isolation_run: Option<crate::managed_agents::FilesystemIsolationRun>,
    /// Win32 Job Object owning the harness + its entire process tree. Closing
    /// the handle (via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) kills the whole
    /// tree — the Windows mirror of the Unix process-group teardown. `None`
    /// if job creation/assignment failed (we fall back to `Child::kill()`).
    #[cfg(windows)]
    pub job: Option<crate::managed_agents::JobHandle>,
}
