//! Restart-authorization operations for the two auto-restart flows
//! (`install_acp_runtime` post-install bounce and `set_global_agent_config`).
//!
//! Each flow has a pre-scan that selects candidate pubkeys without holding the
//! store lock, and an under-lock step that re-authorizes a single agent and
//! then stops its process. Both the candidate selection and the
//! authorize-before-stop decision live here as AppHandle-free operations with
//! **injected dependencies**: the global-config loader is a closure the op
//! invokes itself, per-record runtime state is a closure, and the stop is an
//! injected effect. The production async flows in `agent_discovery.rs` and
//! `global_agent_config.rs` are thin shells that acquire locks, load records,
//! and supply those closures.
//!
//! Injecting the loader (rather than a pre-computed `Result`) is what binds the
//! strict-global gates: the op invokes the loader and maps its `Err` to a
//! refusal, so a test can drive the failure path and a stop spy observes that
//! no stop fired. A shell that passed a snapshot would leave the strict choice
//! untested. Injecting the stop effect binds the authorize-before-stop ordering
//! and every pre-stop gate: dropping any gate lets the spy record a stop that
//! should have been refused.

use crate::managed_agents::{
    agent_readiness, effective_secrets_unavailable, known_acp_runtime, record_agent_command,
    resolve_effective_agent_env, AgentDefinition, AgentReadiness, BackendKind, GlobalAgentConfig,
    ManagedAgentRecord,
};

use super::agent_discovery::{
    refuse_restart_on_unavailable_global, refuse_restart_on_unavailable_secrets,
    should_restart_after_install,
};
use super::global_agent_config::should_restart_on_config_change;

/// Select the pubkeys of setup-mode agents to bounce after an adapter install.
///
/// Owns the strict-global gate (via `load_global`) and the eligibility
/// predicate including the `effective_secrets_unavailable` consultation. An
/// unreadable committed global yields **zero candidates** for the scan, because
/// a restart authorized now would stop a live process only to hit the strict
/// reload at spawn (`FailedAfterStop`). `runtime_state` returns
/// `(pid_alive, setup_mode)` for a record from the caller's runtimes map.
pub(super) fn select_post_install_restart_candidates(
    records: &[ManagedAgentRecord],
    personas: &[AgentDefinition],
    runtime_id: &str,
    load_global: impl FnOnce() -> Result<GlobalAgentConfig, String>,
    mut runtime_state: impl FnMut(&ManagedAgentRecord) -> (bool, bool),
) -> Vec<String> {
    let global = match refuse_restart_on_unavailable_global(load_global()) {
        Ok(global) => global,
        Err(e) => {
            eprintln!("buzz-desktop: install_acp_runtime: skipping restart scan — {e}");
            return Vec::new();
        }
    };

    records
        .iter()
        .filter(|record| {
            let is_local = record.backend == BackendKind::Local;
            let effective_cmd = record_agent_command(record, personas);
            let runtime_meta = known_acp_runtime(&effective_cmd);
            let runtime_matches = runtime_meta.is_some_and(|r| r.id == runtime_id);
            let (pid_alive, setup_mode) = runtime_state(record);
            let effective = resolve_effective_agent_env(record, personas, runtime_meta, &global);
            let now_ready = matches!(agent_readiness(&effective), AgentReadiness::Ready);
            should_restart_after_install(
                is_local,
                pid_alive,
                runtime_matches,
                setup_mode,
                now_ready,
                effective_secrets_unavailable(record, personas),
            )
        })
        .map(|r| r.pubkey.clone())
        .collect()
}

/// Re-authorize a single post-install bounce under the store lock and, only if
/// every gate passes, run the injected `stop` effect.
///
/// Gate order mirrors the production flow: strict global load (via
/// `load_global`), runtime-match, setup-mode, readiness, then the under-lock
/// secret-availability refusal — all **before** the stop. Any `Err` short-
/// circuits before `stop` runs, so a stop spy never records a bounce that
/// should have been refused.
pub(super) fn authorize_post_install_restart(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    runtime_id: &str,
    setup_mode: bool,
    load_global: impl FnOnce() -> Result<GlobalAgentConfig, String>,
    stop: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // Strict global load under lock: `spawn_agent_child` reloads it strictly,
    // so an unreadable global here means the respawn would refuse after we
    // already stopped the process. Refuse before the stop.
    let global = refuse_restart_on_unavailable_global(load_global())?;

    let effective_cmd = record_agent_command(record, personas);
    let runtime_meta = known_acp_runtime(&effective_cmd);
    if runtime_meta.is_none_or(|r| r.id != runtime_id) {
        return Err(format!(
            "agent {} runtime no longer matches {runtime_id} under lock",
            record.pubkey
        ));
    }
    if !setup_mode {
        return Err(format!(
            "agent {} is not in setup mode under lock — skipping",
            record.pubkey
        ));
    }

    let effective = resolve_effective_agent_env(record, personas, runtime_meta, &global);
    if !matches!(agent_readiness(&effective), AgentReadiness::Ready) {
        return Err(format!(
            "agent {} readiness is still NotReady after install — not bouncing",
            record.pubkey
        ));
    }

    // Fail-closed under lock: an unavailable secret tier makes the empty
    // hydrated env look Ready, so re-check before the stop (never after).
    refuse_restart_on_unavailable_secrets(record, personas)?;

    stop()
}

/// Select the pubkeys of running local agents to bounce after a global-config
/// change.
///
/// Owns the eligibility predicate including the `effective_secrets_unavailable`
/// consultation. `is_live` reports whether the record still has a live pair
/// runtime (the caller checks its runtimes map). The phase-1 `old_global` /
/// `new_global` snapshots drive the readiness comparison; the committed-global
/// availability gate lives in the under-lock step, not here.
pub(super) fn select_config_change_restart_candidates(
    records: &[ManagedAgentRecord],
    personas: &[AgentDefinition],
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
    mut is_live: impl FnMut(&ManagedAgentRecord) -> bool,
) -> Vec<String> {
    records
        .iter()
        .filter(|record| {
            if record.backend != BackendKind::Local {
                return false;
            }
            if !is_live(record) {
                return false;
            }
            let effective_cmd = record_agent_command(record, personas);
            let runtime_meta = known_acp_runtime(&effective_cmd);
            let old_effective =
                resolve_effective_agent_env(record, personas, runtime_meta, old_global);
            let new_effective =
                resolve_effective_agent_env(record, personas, runtime_meta, new_global);
            let old_ready = matches!(agent_readiness(&old_effective), AgentReadiness::Ready);
            let new_ready = matches!(agent_readiness(&new_effective), AgentReadiness::Ready);
            // NotReady→Ready bypasses the env-diff check (Phase 2 stops-then-
            // starts unconditionally); a Ready agent needs an env delta.
            let env_changed = old_ready && old_effective.env != new_effective.env;
            should_restart_on_config_change(
                old_ready,
                new_ready,
                env_changed,
                effective_secrets_unavailable(record, personas),
            )
        })
        .map(|r| r.pubkey.clone())
        .collect()
}

/// Re-authorize a single config-change bounce under the store lock and, only if
/// every gate passes, run the injected `stop` effect.
///
/// Gate order mirrors the production flow: the restart predicate (readiness
/// transition / env delta, plus the secret-availability consultation), then the
/// strict committed-global re-read (via `load_global`) — the phase-1 snapshots
/// cannot attest the committed ref is still hydratable at stop time — all
/// **before** the stop.
pub(super) fn authorize_config_change_restart(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
    load_global: impl FnOnce() -> Result<GlobalAgentConfig, String>,
    stop: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let effective_cmd = record_agent_command(record, personas);
    let runtime_meta = known_acp_runtime(&effective_cmd);
    let old_effective = resolve_effective_agent_env(record, personas, runtime_meta, old_global);
    let new_effective = resolve_effective_agent_env(record, personas, runtime_meta, new_global);
    let old_ready = matches!(agent_readiness(&old_effective), AgentReadiness::Ready);
    let new_ready = matches!(agent_readiness(&new_effective), AgentReadiness::Ready);
    let env_changed = old_ready && old_effective.env != new_effective.env;
    if !should_restart_on_config_change(
        old_ready,
        new_ready,
        env_changed,
        effective_secrets_unavailable(record, personas),
    ) {
        return Err(format!(
            "agent {} restart condition no longer valid under lock",
            record.pubkey
        ));
    }

    // Strict re-read of the committed global under lock before the stop: the
    // phase-1 snapshots drove the readiness comparison, but cannot attest the
    // committed ref is still hydratable NOW. `spawn_agent_child` reloads it
    // strictly, so an unreadable global here means respawn would refuse after
    // the stop (`FailedAfterStop`).
    refuse_restart_on_unavailable_global(load_global())?;

    stop()
}

#[cfg(test)]
#[path = "restart_ops_tests.rs"]
mod tests;
