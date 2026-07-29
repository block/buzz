use tauri::State;

use crate::{
    app_state::AppState,
    managed_agents::{
        command_availability, is_npm_global_install, AcpRuntimeCatalogEntry,
        DiscoverManagedAgentPrereqsRequest, InstallRuntimeResult, ManagedAgentPrereqsInfo,
        RelayAgentInfo, DEFAULT_ACP_COMMAND,
    },
    nostr_convert,
    relay::query_relay,
};

mod post_install_verification;

fn active_installs() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Returns the adapter install commands that `install_acp_runtime_blocking` would
/// run for `runtime_id` given a resolved adapter binary at `adapter_path` (or
/// `None` if none was found).
///
/// Returns `None` when no install is needed (adapter is present and current).
/// Returns `Some(cmds)` when the adapter is missing or (for codex) below its
/// minimum supported version.
///
/// For the codex **outdated** case the returned sequence is a two-step
/// reinstall: first uninstall the old `@zed-industries/codex-acp` package
/// (idempotent — exit 0 when absent), then install the new
/// `@agentclientprotocol/codex-acp`.  This is required because both packages
/// install a global binary named `codex-acp`, and npm ≥7 refuses to overwrite
/// a bin file owned by a different package with `EEXIST`.
///
/// For the **missing** case the catalog's `adapter_install_commands` are used
/// as-is (no prior package to remove).
///
/// This is a pure planning function: it never spawns a process.  Tests use it to
/// assert the correct install command is selected without touching real npm.
pub(crate) fn plan_adapter_install<'c>(
    runtime_id: &str,
    adapter_path: Option<&std::path::Path>,
    adapter_install_commands: &'c [&'c str],
    adapter_probe_path: Option<&str>,
) -> Option<Vec<&'c str>> {
    match adapter_path {
        // Adapter present and current — no install needed.
        Some(_) if runtime_id != "codex" => None,
        Some(path)
            if !crate::managed_agents::codex_adapter_is_outdated_with_path(
                path,
                adapter_probe_path,
            ) =>
        {
            None
        }
        // Codex adapter is outdated: uninstall the old package first so npm
        // doesn't hit EEXIST on the shared `codex-acp` bin-link, then install.
        Some(_) => Some(vec![
            "npm uninstall -g @zed-industries/codex-acp",
            "npm install -g @agentclientprotocol/codex-acp",
        ]),
        // Adapter missing: use the catalog's install commands directly.
        None => Some(adapter_install_commands.to_vec()),
    }
}

#[tauri::command]
pub async fn discover_acp_providers(
    app: tauri::AppHandle,
) -> Result<Vec<AcpRuntimeCatalogEntry>, String> {
    tokio::task::spawn_blocking(move || {
        use tauri::Manager;
        crate::managed_agents::clear_resolve_cache();
        crate::managed_agents::refresh_login_shell_path();
        let custom_dir = app
            .path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("custom_harnesses"));
        crate::managed_agents::discover_acp_runtimes_from(custom_dir.as_deref())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Write a user-defined harness definition to `<app-data>/custom_harnesses/<id>.json`.
///
/// Validates the definition (id regex, builtin-id collision, non-empty command
/// and label, env well-formedness) before touching the filesystem. Returns the
/// merged catalog entry so the UI can update the provider list without triggering
/// a full re-discover.
///
/// `original_id` handles the rename case: when the user edits an existing
/// harness and changes its id, pass the old id here so the old file is removed
/// atomically as part of the write. If the id is unchanged or this is a new
/// harness, omit `original_id` (or pass `None`).
///
/// The file is written using `atomic-write-file` (unique temp file + commit)
/// so concurrent saves do not race on a fixed temp path, and a partial write
/// never produces a corrupted JSON file.
#[tauri::command]
pub async fn save_custom_harness(
    definition: crate::managed_agents::custom_harnesses::HarnessDefinition,
    original_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<AcpRuntimeCatalogEntry, String> {
    use crate::managed_agents::{
        custom_harnesses, AcpAvailabilityStatus, AuthStatus, HarnessSource,
    };
    use tauri::Manager;

    // ── Phase 1: full validation before touching the filesystem ─────────────
    // validate_harness_definition_pub now covers: id format, non-empty command/label,
    // env key well-formedness + reserved-key check + NUL/size limits, and
    // install_instructions_url scheme.
    custom_harnesses::validate_harness_definition_pub(&definition)?;
    custom_harnesses::check_id_collision(&definition.id)?;

    // Validate original_id BEFORE any filesystem mutation (validate-before-mutate).
    let rename_old_id: Option<String> = original_id.and_then(|oid| {
        let oid = oid.trim().to_string();
        if oid.is_empty() || oid == definition.id {
            None
        } else {
            Some(oid)
        }
    });
    if let Some(ref old_id) = rename_old_id {
        custom_harnesses::check_id_collision(old_id)
            .map_err(|_| format!("original_id {old_id:?} is a built-in and cannot be deleted"))?;
        if !custom_harnesses::is_valid_harness_id_pub(old_id) {
            return Err(format!("invalid original_id {old_id:?}"));
        }
    }

    let custom_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?
        .join("custom_harnesses");
    std::fs::create_dir_all(&custom_dir)
        .map_err(|e| format!("failed to create custom_harnesses dir: {e}"))?;

    // ── Phase 2+3: backup-swap write + rename (Windows-safe, rollback on failure)
    // `save_and_warm` holds the persist mutex for the write + registry-warm pair
    // so concurrent saves never produce a stale registry snapshot (B-6).
    custom_harnesses::save_and_warm(&custom_dir, &definition, rename_old_id.as_deref())?;

    // Resolve availability for the returned catalog entry.
    let (availability, command_opt, binary_path) =
        match crate::managed_agents::find_command(&definition.command) {
            Some(path) => (
                AcpAvailabilityStatus::Available,
                Some(definition.command.clone()),
                Some(path.display().to_string()),
            ),
            None => (AcpAvailabilityStatus::NotInstalled, None, None),
        };

    let default_args =
        crate::managed_agents::normalize_agent_args(&definition.command, definition.args.clone());

    Ok(AcpRuntimeCatalogEntry {
        id: definition.id,
        label: definition.label,
        // Security: no user-supplied avatar URL in catalog entries.
        avatar_url: String::new(),
        availability,
        command: command_opt,
        binary_path,
        default_args,
        mcp_command: None,
        model_env_var: None,
        provider_env_var: None,
        thinking_env_var: None,
        install_hint: definition.install_hint,
        install_instructions_url: definition.install_instructions_url,
        can_auto_install: false,
        requires_external_cli: false,
        underlying_cli_path: None,
        node_required: false,
        auth_status: AuthStatus::NotApplicable,
        login_hint: None,
        source: HarnessSource::Custom,
        // Carry definition env back so the edit form can read and preserve it.
        definition_env: definition.env,
    })
}

/// Remove a user-defined harness definition from `<app-data>/custom_harnesses/`.
///
/// Only `source: custom` harnesses may be deleted. Attempting to delete a
/// built-in id (goose, claude, codex, buzz-agent) returns an error without
/// touching the filesystem.
#[tauri::command]
pub async fn delete_custom_harness(id: String, app: tauri::AppHandle) -> Result<(), String> {
    use crate::managed_agents::custom_harnesses;
    use tauri::Manager;

    // Reject built-in ids early — they have no backing file to delete and
    // must never be removable from the catalog.
    custom_harnesses::check_id_collision(&id)
        .map_err(|_| format!("harness {id:?} is a built-in and cannot be deleted"))?;

    // Validate the id so callers cannot use path-traversal tricks.
    if !custom_harnesses::is_valid_harness_id_pub(&id) {
        return Err(format!("invalid harness id {id:?}"));
    }

    let custom_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?
        .join("custom_harnesses");

    // `delete_and_warm` holds the persist mutex for the delete + registry-warm
    // pair so concurrent save/delete calls never produce a stale snapshot (B-6).
    custom_harnesses::delete_and_warm(&custom_dir, &id)?;

    Ok(())
}

#[tauri::command]
pub async fn install_acp_runtime(
    runtime_id: String,
    app: tauri::AppHandle,
) -> Result<InstallRuntimeResult, String> {
    // ── Phase 1: blocking install ────────────────────────────────────────────
    //
    // Run the npm install steps synchronously in spawn_blocking.  The
    // active_installs guard is dropped when install_acp_runtime_blocking
    // returns (Guard impl Drop) — so Phase 2's restart path runs outside
    // the guard and cannot re-enter the mutex.
    let runtime_id_clone = runtime_id.clone();
    let install_result =
        tokio::task::spawn_blocking(move || install_acp_runtime_blocking(&runtime_id_clone))
            .await
            .map_err(|e| format!("install task panicked: {e}"))??;

    if !install_result.success {
        return Ok(install_result);
    }

    // ── Phase 2: async restart of stuck agents ───────────────────────────────
    //
    // Mirror set_global_agent_config: after a successful install, restart any
    // local agents that were spawned in setup-listener mode for this runtime
    // and whose readiness now computes Ready.  Best-effort — errors are logged
    // and returned as failed_restart_count without failing the command.
    let (restarted_count, failed_restart_count) =
        restart_setup_mode_agents_after_install(&app, &runtime_id).await;

    Ok(InstallRuntimeResult {
        success: true,
        steps: install_result.steps,
        restarted_count,
        failed_restart_count,
    })
}

/// Err(_) = infrastructure failure (panic, concurrency guard).
/// Ok({success: false}) = an install step failed (stderr captured in steps).
fn install_acp_runtime_blocking(runtime_id: &str) -> Result<InstallRuntimeResult, String> {
    // Re-fetch the login-shell PATH so a Node.js installation that happened
    // after app launch (or after a previous failed install) is visible to this
    // run and to the subsequent discover_acp_providers call.
    crate::managed_agents::refresh_login_shell_path();
    // Clear the resolve cache so newly-installed binaries are found.
    crate::managed_agents::clear_resolve_cache();

    // Prevent concurrent installs for the same runtime.
    {
        let mut set = active_installs()
            .lock()
            .map_err(|_| "install lock poisoned".to_string())?;
        if !set.insert(runtime_id.to_string()) {
            return Err(format!(
                "an install is already in progress for {runtime_id}"
            ));
        }
    }

    struct Guard(String);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(mut set) = active_installs().lock() {
                set.remove(&self.0);
            }
        }
    }
    let _guard = Guard(runtime_id.to_string());

    let runtime = crate::managed_agents::known_acp_runtime_exact(runtime_id)
        .ok_or_else(|| format!("unknown runtime: {runtime_id}"))?;

    let mut steps = Vec::new();

    // Phase 1: Install CLI if missing and commands are available.
    // Today every entry in `cli_install_commands` is a curl-pipe; npm-backed
    // adapter installs live in Phase 2 below where they are rewritten to a
    // Buzz-private prefix before execution.
    if let Some(cli) = runtime.underlying_cli {
        if crate::managed_agents::resolve_command(cli).is_none() {
            for cmd in runtime.cli_install_commands_for_os() {
                let result = run_install_command_with_retry("cli", cmd);
                let success = result.success;
                steps.push(result);
                if !success {
                    return Ok(InstallRuntimeResult {
                        success: false,
                        steps,
                        restarted_count: 0,
                        failed_restart_count: 0,
                    });
                }
            }
        }
    }

    // Phase 2: Install adapter if missing (or outdated) and commands are available.
    // For the codex runtime, "found" is not enough — the resolved binary must also
    // pass the 1.x version gate. An outdated 0.16.x adapter must be overwritten by
    // the new npm install so the CODEX_CONFIG spawn contract works correctly.
    let adapter_path = runtime
        .commands
        .iter()
        .find_map(|cmd| crate::managed_agents::resolve_command(cmd));
    let adapter_probe_path = crate::managed_agents::readiness::cli_probe::augmented_path();
    if let Some(cmds) = plan_adapter_install(
        runtime_id,
        adapter_path.as_deref(),
        runtime.adapter_install_commands,
        adapter_probe_path.as_deref(),
    ) {
        let use_managed_npm =
            cmds.iter().any(|cmd| is_npm_global_install(cmd)) && managed_node_runtime_supported();
        if use_managed_npm {
            if let Err(step) = ensure_managed_node_runtime_blocking() {
                steps.push(*step);
                return Ok(InstallRuntimeResult {
                    success: false,
                    steps,
                    restarted_count: 0,
                    failed_restart_count: 0,
                });
            }
        }

        for cmd in cmds {
            let planned = match if use_managed_npm {
                managed_npm_command(cmd)
            } else {
                Ok(None)
            } {
                Ok(Some(command)) => command,
                Ok(None) => cmd.to_string(),
                Err(step) => {
                    steps.push(*step);
                    return Ok(InstallRuntimeResult {
                        success: false,
                        steps,
                        restarted_count: 0,
                        failed_restart_count: 0,
                    });
                }
            };

            let mut result = run_install_command_with_retry("adapter", &planned);
            if !result.success && result.hint.is_none() && is_npm_global_install(cmd) {
                result.hint = npm_eacces_hint(&result.stderr, cmd);
            }
            let success = result.success;
            steps.push(result);
            if !success {
                return Ok(InstallRuntimeResult {
                    success: false,
                    steps,
                    restarted_count: 0,
                    failed_restart_count: 0,
                });
            }
        }
    }

    post_install_verification::run(runtime_id, &mut steps);

    Ok(InstallRuntimeResult {
        success: steps.iter().all(|step| step.success),
        steps,
        restarted_count: 0,
        failed_restart_count: 0,
    })
}

// ── Post-install auto-restart (Phase 2 of install_acp_runtime) ───────────────
//
// After a successful adapter install, restart any local agents that:
//   1. are local backend + have a live PID,
//   2. their effective command maps to the just-installed runtime,
//   3. were spawned in setup-listener mode (setup_mode stamp), AND
//   4. their readiness now computes Ready.
//
// Mirrors the two-phase shape of set_global_agent_config.

/// Outcome of a single per-agent restart attempt during post-install restart.
#[derive(Debug)]
enum InstallRestartOutcome {
    Restarted,
    FailedAfterStop,
    Skipped,
}

/// Pure predicate: should this agent be restarted after an adapter install?
///
/// Extracted for unit testing — callers must still re-verify under the lock.
/// The caller is responsible for computing `pid_alive` (via `process_is_running`)
/// before invoking this function, keeping the predicate OS-agnostic and testable
/// on all platforms.
///
/// An agent qualifies iff:
/// - it is a local backend with a live PID (`pid_alive`),
/// - its effective command maps to `runtime_id`,
/// - it was **spawned in setup-listener mode** (`setup_mode`), AND
/// - its readiness **now computes `Ready`** (install fixed the blocker).
fn should_restart_after_install(
    is_local: bool,
    pid_alive: bool,
    runtime_matches: bool,
    setup_mode: bool,
    now_ready: bool,
) -> bool {
    is_local && pid_alive && runtime_matches && setup_mode && now_ready
}

/// Restart all setup-mode agents whose runtime matches `runtime_id` and whose
/// readiness now computes Ready.  Returns `(restarted_count, failed_restart_count)`.
async fn restart_setup_mode_agents_after_install(
    app: &tauri::AppHandle,
    runtime_id: &str,
) -> (u32, u32) {
    use crate::{
        app_state::AppState,
        managed_agents::{
            agent_readiness, known_acp_runtime, load_global_agent_config, load_managed_agents,
            load_personas, record_agent_command, resolve_effective_agent_env, AgentReadiness,
            BackendKind,
        },
    };
    use tauri::Manager;

    // ── Pre-scan: collect candidate pubkeys without holding locks ────────────
    let app_for_scan = app.clone();
    let runtime_id_owned = runtime_id.to_string();
    let candidates = tokio::task::spawn_blocking(move || {
        let records = load_managed_agents(&app_for_scan).unwrap_or_default();
        let personas = load_personas(&app_for_scan).unwrap_or_default();
        let global = load_global_agent_config(&app_for_scan).unwrap_or_default();

        // Read the runtimes map to check setup_mode stamps.
        let state_inner = app_for_scan.state::<AppState>();
        let runtimes = state_inner
            .managed_agent_processes
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        records
            .iter()
            .filter(|record| {
                let is_local = record.backend == BackendKind::Local;
                let effective_cmd = record_agent_command(record, &personas);
                let runtime_matches =
                    known_acp_runtime(&effective_cmd).is_some_and(|r| r.id == runtime_id_owned);
                let setup_mode = runtimes
                    .iter()
                    .find(|(key, _)| key.pubkey == record.pubkey)
                    .map(|(_, p)| p.setup_mode)
                    .unwrap_or(false);
                let effective = resolve_effective_agent_env(
                    record,
                    &personas,
                    known_acp_runtime(&effective_cmd),
                    &global,
                );
                let now_ready = matches!(agent_readiness(&effective), AgentReadiness::Ready);
                let pid_alive = runtimes.iter().any(|(key, runtime)| {
                    key.pubkey.eq_ignore_ascii_case(&record.pubkey)
                        && crate::managed_agents::process_is_running(runtime.child.id())
                });
                should_restart_after_install(
                    is_local,
                    pid_alive,
                    runtime_matches,
                    setup_mode,
                    now_ready,
                )
            })
            .map(|r| r.pubkey.clone())
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();

    if candidates.is_empty() {
        return (0, 0);
    }

    let mut restarted_count: u32 = 0;
    let mut failed_restart_count: u32 = 0;

    for pubkey in &candidates {
        let outcome = restart_single_agent_after_install(app, pubkey, runtime_id).await;
        match outcome {
            InstallRestartOutcome::Restarted => restarted_count += 1,
            InstallRestartOutcome::FailedAfterStop => failed_restart_count += 1,
            InstallRestartOutcome::Skipped => {}
        }
    }

    (restarted_count, failed_restart_count)
}

/// Stop-then-start a single setup-mode agent after a successful adapter install.
///
/// Mirrors `restart_local_agent_on_config_change` from `global_agent_config.rs`:
/// eligibility is re-verified under the store lock before the stop, then the
/// agent is restarted via `start_local_agent_with_preflight`.
async fn restart_single_agent_after_install(
    app: &tauri::AppHandle,
    pubkey: &str,
    runtime_id: &str,
) -> InstallRestartOutcome {
    use crate::{
        app_state::AppState,
        managed_agents::{
            agent_readiness, current_instance_id, find_managed_agent_mut, known_acp_runtime,
            load_global_agent_config, load_managed_agents, load_personas, record_agent_command,
            resolve_effective_agent_env, save_managed_agents, stop_managed_agent_process,
            sync_managed_agent_processes, AgentReadiness, BackendKind,
        },
    };
    use tauri::Manager;

    let app_for_stop = app.clone();
    let pubkey_owned = pubkey.to_string();
    let runtime_id_owned = runtime_id.to_string();

    let stop_result = tokio::task::spawn_blocking(move || {
        let state = app_for_stop.state::<AppState>();

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| format!("failed to acquire store lock: {e}"))?;

        let mut records = load_managed_agents(&app_for_stop)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| format!("failed to acquire runtimes lock: {e}"))?;

        // Sync process state so PID liveness reflects current reality.
        let (sync_changed, _) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &current_instance_id(&app_for_stop),
        );
        if sync_changed {
            save_managed_agents(&app_for_stop, &records)?;
        }

        // Re-verify eligibility under lock.
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey_owned)
            .ok_or_else(|| format!("agent {pubkey_owned} not found"))?;

        if record.backend != BackendKind::Local {
            return Err(format!("agent {pubkey_owned} is no longer a local agent"));
        }
        let runtime_keys =
            crate::managed_agents::managed_agent_runtime_keys(&runtimes, &pubkey_owned);
        if runtime_keys.is_empty() {
            return Err(format!(
                "agent {pubkey_owned} no longer has a live pair runtime after sync"
            ));
        }

        let personas = load_personas(&app_for_stop).unwrap_or_default();
        let global = load_global_agent_config(&app_for_stop).unwrap_or_default();

        let effective_cmd = record_agent_command(record, &personas);
        let runtime_matches =
            known_acp_runtime(&effective_cmd).is_some_and(|r| r.id == runtime_id_owned);
        if !runtime_matches {
            return Err(format!(
                "agent {pubkey_owned} runtime no longer matches {runtime_id_owned} under lock"
            ));
        }

        let setup_mode = runtimes
            .iter()
            .find(|(key, _)| key.pubkey == pubkey_owned)
            .map(|(_, p)| p.setup_mode)
            .unwrap_or(false);
        if !setup_mode {
            return Err(format!(
                "agent {pubkey_owned} is not in setup mode under lock — skipping"
            ));
        }

        let runtime_meta = known_acp_runtime(&effective_cmd);
        let effective = resolve_effective_agent_env(record, &personas, runtime_meta, &global);
        if !matches!(agent_readiness(&effective), AgentReadiness::Ready) {
            return Err(format!(
                "agent {pubkey_owned} readiness is still NotReady after install — not bouncing"
            ));
        }

        // Stop the process.
        let record_mut = find_managed_agent_mut(&mut records, &pubkey_owned)?;
        stop_managed_agent_process(&app_for_stop, record_mut, &mut runtimes)?;
        save_managed_agents(&app_for_stop, &records)?;

        Ok(runtime_keys)
    })
    .await;

    let runtime_keys = match stop_result {
        Ok(Ok(runtime_keys)) => runtime_keys,
        Ok(Err(e)) => {
            eprintln!("buzz-desktop: install_acp_runtime: skipping restart of {pubkey}: {e}");
            return InstallRestartOutcome::Skipped;
        }
        Err(e) => {
            eprintln!(
                "buzz-desktop: install_acp_runtime: spawn_blocking failed for stop of {pubkey}: {e}"
            );
            return InstallRestartOutcome::Skipped;
        }
    };

    let relay_urls: Vec<_> = runtime_keys.into_iter().map(|key| key.relay_url).collect();
    let state = app.state::<AppState>();
    match super::agents::start_local_agent_pairs_with_preflight(app, &state, pubkey, &relay_urls)
        .await
    {
        Ok(_) => {
            eprintln!(
                "buzz-desktop: install_acp_runtime: restarted setup-mode agent {pubkey} after install"
            );
            InstallRestartOutcome::Restarted
        }
        Err(e) => {
            eprintln!(
                "buzz-desktop: install_acp_runtime: failed to start {pubkey} after install: {e}"
            );
            if let Err(save_err) = persist_last_error_on_install(app, pubkey, &e) {
                eprintln!(
                    "buzz-desktop: install_acp_runtime: failed to persist last_error for {pubkey}: {save_err}"
                );
            }
            InstallRestartOutcome::FailedAfterStop
        }
    }
}

/// Persist a `last_error` on the agent record under the store lock.
/// Best-effort: called only after a failed restart.
fn persist_last_error_on_install(
    app: &tauri::AppHandle,
    pubkey: &str,
    error: &str,
) -> Result<(), String> {
    use crate::{
        app_state::AppState,
        managed_agents::{find_managed_agent_mut, load_managed_agents, save_managed_agents},
    };
    use tauri::Manager;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire store lock: {e}"))?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = crate::util::now_iso();
    save_managed_agents(app, &records)
}

/// Build the `-l -c` argument list for the install shell.
///
/// The body runs under `pipefail`: every CLI install command is a `curl … |
/// bash` / `| sh` pipe, and without it the pipeline's status is the right-hand
/// side's — `bash`/`sh` fed an empty stdin exits 0 — so a `curl` that fails (or
/// isn't on PATH at all) was recorded as a successful `cli` step, leaving the
/// user an unactionable `verify` error instead of curl's own stderr. Every
/// install shell supports it; the Windows PowerShell path bypasses this shell.
/// `SHELLOPTS` is not exported, so the piped-to vendor script keeps its own
/// defaults.
///
/// Off Windows, `composed_path` is passed as a positional and re-exported
/// *inside* the body, because `-l` sources the user's login startup files after
/// the process environment is installed: a profile assigning PATH overwrites
/// `cmd.env("PATH", …)` before the vendor command runs. `export PATH=` empties
/// it outright; macOS `/etc/zprofile` runs `path_helper`, which reorders it and
/// costs Buzz's managed Node/npm dirs their precedence. A positional rather than
/// an interpolated body keeps entries containing spaces or quotes intact.
///
/// The prelude is omitted where it would do harm:
/// - `composed_path` is `None` — `export PATH="$1"` with `$1` unset sets an
///   *empty* PATH, worse than the ambient one.
/// - `is_windows` — `join_paths` uses the platform separator, so the positional
///   would be `;`-joined while bash splits PATH on `:`, collapsing every entry
///   into one nonsense path; and Windows is where the inherited fallback always
///   fires (`login_shell_path()` is unconditionally `None` there), so this would
///   be the steady state. `cmd.env("PATH", …)` already delivers the native form
///   Git Bash translates on entry, and Windows has no login startup files doing
///   the clobbering this prelude defends against.
///
/// `is_windows` is a parameter rather than a `#[cfg]` so the Windows shape stays
/// asserted on Unix CI — the same reason `should_skip_claude_executable` takes
/// one. Extracted from `install_shell_command` for that testability, not because
/// it has more than one caller.
fn install_shell_args(
    command: &str,
    composed_path: Option<&std::ffi::OsStr>,
    is_windows: bool,
) -> Vec<std::ffi::OsString> {
    let Some(path) = composed_path.filter(|_| !is_windows) else {
        return vec![
            "-l".into(),
            "-c".into(),
            format!("set -o pipefail; {command}").into(),
        ];
    };
    vec![
        "-l".into(),
        "-c".into(),
        format!("export PATH=\"$1\"; set -o pipefail; {command}").into(),
        // `$0` is the shell-name slot, so the PATH must be the second positional.
        "buzz-install".into(),
        path.to_os_string(),
    ]
}

/// Build a login-shell `Command` for `command` with hermit env vars stripped,
/// Buzz-managed npm locations set, and the user's PATH set. This is the
/// single source of truth for
/// the shell selection and environment cleanup shared by `run_install_command`
/// and managed npm install path — keeping them in sync so the hermit-strip list
/// can't drift between command execution paths.
///
/// On Windows, resolves Git Bash via `resolve_bash_path` (skips `BUZZ_SHELL`
/// since install commands require bash syntax). Returns `Err` when no shell
/// can be found.
fn install_shell_command(command: &str) -> Result<std::process::Command, String> {
    let shell: std::path::PathBuf = resolve_install_shell()?;

    let mut cmd = std::process::Command::new(&shell);

    // Strip hermit vars and set managed npm paths (see apply_npm_env).
    apply_npm_env(&mut cmd);

    // Compose the PATH for the install shell using the same kernel as the
    // runtime/probe path so the two can never drift.  managed entries first
    // (Node/npm bins keep precedence); login-shell entries next; inherited
    // process PATH appended last when no login-shell PATH exists — the case
    // where the composed PATH would otherwise be Buzz's managed Node dirs
    // alone, with no `curl`/`sh`/`tar` for the vendor install pipes
    // (cmd.env("PATH", …) replaces rather than extends). On Windows that case
    // is the steady state: login_shell_path() always returns None there
    // because Git Bash paths are POSIX-shaped and poison native children.
    //
    // The composed PATH is set twice on purpose off Windows: `cmd.env` so the
    // login startup files themselves run with a usable PATH, and the `$1`
    // export in `install_shell_args` so their own PATH assignments cannot undo
    // it. Neither is redundant — see `install_shell_args`, which also explains
    // why the export is suppressed on Windows.
    let login_path = crate::managed_agents::login_shell_path();
    let had_login = login_path.is_some();
    let managed: Vec<std::path::PathBuf> = [
        crate::managed_agents::buzz_managed_node_bin_dir(),
        crate::managed_agents::buzz_managed_npm_bin_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let login: Vec<std::path::PathBuf> = login_path
        .as_deref()
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    let inherited: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let use_inherited = crate::managed_agents::should_use_inherited(had_login, true);
    let path_parts =
        crate::managed_agents::compose_path_entries(managed, login, inherited, use_inherited);
    let composed_path = (!path_parts.is_empty())
        .then(|| std::env::join_paths(path_parts).ok())
        .flatten();
    if let Some(path) = composed_path.as_deref() {
        cmd.env("PATH", path);
    }
    cmd.args(install_shell_args(
        command,
        composed_path.as_deref(),
        cfg!(windows),
    ));

    // Detach from the controlling terminal so install scripts that read from
    // /dev/tty (e.g. Codex's "Start Codex now? [y/N]") fall back to stdin
    // (which is /dev/null) instead of blocking forever.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    // Suppress the console window on Windows.
    apply_no_window(&mut cmd);

    Ok(cmd)
}

/// Resolve the shell binary for install commands.
///
/// Unix: `/bin/zsh` if present, else `/bin/bash`.
/// Windows: Git Bash via `resolve_bash_path` — skips `BUZZ_SHELL` because install
/// commands use bash-only `-l -c` syntax. A `BUZZ_SHELL=pwsh` user gets a green
/// Doctor prereq (their agents work) but installs use the Git Bash fallback chain.
fn resolve_install_shell() -> Result<std::path::PathBuf, String> {
    #[cfg(not(windows))]
    {
        if std::path::Path::new("/bin/zsh").exists() {
            return Ok(std::path::PathBuf::from("/bin/zsh"));
        }
        Ok(std::path::PathBuf::from("/bin/bash"))
    }

    #[cfg(windows)]
    {
        install_shell_from(crate::managed_agents::git_bash::resolve_bash_path())
    }
}

/// Pure mapping from a resolved bash path to the install-shell result.
/// `None` → `Err(GIT_BASH_INSTALL_HINT)`, `Some(path)` → `Ok(path)`.
#[cfg(windows)]
pub(crate) fn install_shell_from(
    resolved: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    resolved.ok_or_else(|| crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT.to_string())
}

/// Returns `true` when `command` is a Windows-native PowerShell invocation
/// (i.e. begins with `powershell.exe`). These commands must NOT be routed
/// through Git Bash: the Bash login shell prepends POSIX dirs to PATH, so
/// bare `tar` inside the PowerShell script resolves to GNU tar
/// (`/usr/bin/tar`) instead of Windows bsdtar. GNU tar parses the drive
/// letter in `C:\…` as a remote host and fails with "Cannot connect to C:
/// resolve failed", which is the exact failure Will observed in the Codex
/// PowerShell installer. Non-PowerShell commands (e.g. `npm install -g …`
/// adapter steps) are unaffected.
///
/// The check is case-insensitive because Windows file-system conventions do
/// not mandate casing, and the install command constants could change.
#[cfg(windows)]
fn is_powershell_command(command: &str) -> bool {
    command
        .split_ascii_whitespace()
        .next()
        .is_some_and(|tok| tok.eq_ignore_ascii_case("powershell.exe"))
}

/// Apply the shared npm env cleanup and managed-prefix setup to an install child.
/// Strips hermit-managed vars and establishes the Buzz-managed npm prefix so adapters
/// installed via either path (shell or native PowerShell) land in the same location.
fn apply_npm_env(cmd: &mut std::process::Command) {
    cmd.env_remove("NPM_CONFIG_PREFIX");
    cmd.env_remove("NPM_CONFIG_CACHE");
    cmd.env_remove("COREPACK_HOME");

    if let Some(prefix) = crate::managed_agents::buzz_managed_npm_prefix() {
        cmd.env("NPM_CONFIG_PREFIX", &prefix);
        cmd.env("npm_config_prefix", &prefix);
        cmd.env("COREPACK_HOME", prefix.join("corepack"));
        cmd.env("NPM_CONFIG_CACHE", prefix.join("cache"));
        cmd.env("npm_config_cache", prefix.join("cache"));
    }
}

/// Suppress the console window for an install child on Windows (no-op elsewhere).
fn apply_no_window(_cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Build a native `powershell.exe` [`Command`][std::process::Command] for the given command
/// string, bypassing Git Bash so POSIX PATH entries never leak into the child.
///
/// Finds the first `-Command` token (case-insensitive, token-boundary match), passes preceding
/// tokens as individual flags, and passes the rest as a single body arg. One outer double-quote
/// pair is stripped from the body — the catalog strings (`discovery.rs:107`, `:138`) wrap it for
/// Bash serialization; stripping here delivers the bare pipeline to PowerShell. Tokens with no
/// `-Command` are forwarded individually. Only called from [`build_install_command`] on Windows.
#[cfg(windows)]
fn install_powershell_command(command: &str) -> std::process::Command {
    // Strip the leading `powershell.exe` token.
    let after_exe = command
        .split_once(|c: char| c.is_ascii_whitespace())
        .map(|(_, rest)| rest.trim())
        .unwrap_or("");

    let mut cmd = std::process::Command::new("powershell.exe");

    // Walk tokens to find -Command on a token boundary (not as a substring of
    // a preceding argument). The comparison is case-insensitive because
    // PowerShell itself is case-insensitive for parameters.
    let mut rest = after_exe;
    let mut found_command_flag = false;
    loop {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            break;
        }
        // Find the end of the current token.
        let tok_end = trimmed
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(trimmed.len());
        let tok = &trimmed[..tok_end];
        if tok.eq_ignore_ascii_case("-command") {
            // Everything after this token (trimmed) is the body.
            let body_raw = trimmed[tok_end..].trim();
            // Strip one matching outer double-quote pair inserted by the
            // Bash-layer catalog serialization. PowerShell does not need them.
            let body =
                if body_raw.starts_with('"') && body_raw.ends_with('"') && body_raw.len() >= 2 {
                    &body_raw[1..body_raw.len() - 1]
                } else {
                    body_raw
                };
            cmd.arg("-Command");
            if !body.is_empty() {
                cmd.arg(body);
            }
            found_command_flag = true;
            break;
        }
        cmd.arg(tok);
        rest = &trimmed[tok_end..];
    }

    if !found_command_flag {
        // No -Command boundary — forward remaining tokens individually.
        for arg in rest.split_ascii_whitespace() {
            cmd.arg(arg);
        }
    }

    apply_npm_env(&mut cmd);

    // Compose PATH: managed Buzz dirs first, then inherited process PATH.
    // No login-shell path: login_shell_path() always returns None on Windows,
    // and we deliberately skip it here to avoid POSIX-shaped entries.
    let managed: Vec<std::path::PathBuf> = [
        crate::managed_agents::buzz_managed_node_bin_dir(),
        crate::managed_agents::buzz_managed_npm_bin_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let inherited: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // No login path → should_use_inherited returns true so inherited appended.
    let path_parts = crate::managed_agents::compose_path_entries(managed, vec![], inherited, true);
    if !path_parts.is_empty() {
        if let Ok(path) = std::env::join_paths(path_parts) {
            cmd.env("PATH", path);
        }
    }

    apply_no_window(&mut cmd);
    cmd
}

/// Select the right [`Command`][std::process::Command] builder for `command`.
///
/// On Windows, PowerShell-prefixed commands are spawned natively (via
/// [`install_powershell_command`]) to avoid the Git Bash PATH poisoning that
/// causes GNU `tar` to be resolved instead of Windows bsdtar.  All other
/// commands — including `npm install -g …` adapter steps — keep the existing
/// Git Bash path via [`install_shell_command`].
///
/// On non-Windows this is always `install_shell_command`.
fn build_install_command(command: &str) -> Result<std::process::Command, String> {
    #[cfg(windows)]
    if is_powershell_command(command) {
        return Ok(install_powershell_command(command));
    }
    install_shell_command(command)
}

// ── install command execution ─────────────────────────────────────────────────
mod install_exec;
use install_exec::run_install_command_with_retry;

// ── managed Node/npm runtime ──────────────────────────────────────────────────
mod managed_node;
use managed_node::{
    ensure_managed_node_runtime_blocking, managed_node_runtime_supported, managed_npm_command,
    npm_eacces_hint,
};

#[tauri::command]
pub async fn discover_managed_agent_prereqs(
    input: DiscoverManagedAgentPrereqsRequest,
) -> Result<ManagedAgentPrereqsInfo, String> {
    tokio::task::spawn_blocking(move || {
        let acp_command = input
            .acp_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_ACP_COMMAND);
        let mcp_command = input
            .mcp_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");

        ManagedAgentPrereqsInfo {
            acp: command_availability(acp_command),
            mcp: command_availability(mcp_command),
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn list_relay_agents(state: State<'_, AppState>) -> Result<Vec<RelayAgentInfo>, String> {
    // Query kind:10100 agent profile events from the relay.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [10100],
        })],
    )
    .await?;

    // The convert helper returns `{"agents": [...]}`. Extract and re-deserialize
    // into the strongly-typed `Vec<RelayAgentInfo>` the frontend expects.
    let value = nostr_convert::agents_from_events(&events);
    let agents = value
        .get("agents")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    serde_json::from_value(agents).map_err(|e| format!("agent parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_npm_global_install ─────────────────────────────────────────────────

    #[test]
    fn test_is_npm_global_install_accepts_catalog_claude_command() {
        assert!(is_npm_global_install(
            "npm install -g @agentclientprotocol/claude-agent-acp"
        ));
    }

    #[test]
    fn test_is_npm_global_install_accepts_catalog_codex_command() {
        assert!(is_npm_global_install(
            "npm install -g @agentclientprotocol/codex-acp"
        ));
    }

    #[test]
    fn test_is_npm_global_install_accepts_short_flag() {
        assert!(is_npm_global_install("npm i -g some-package"));
    }

    #[test]
    fn test_is_npm_global_install_accepts_uninstall() {
        assert!(is_npm_global_install(
            "npm uninstall -g @zed-industries/codex-acp"
        ));
    }

    #[test]
    fn test_is_npm_global_install_accepts_leading_whitespace() {
        assert!(is_npm_global_install("  npm install -g foo"));
    }

    #[test]
    fn test_is_npm_global_install_rejects_curl_pipe() {
        assert!(!is_npm_global_install(
            "curl -fsSL https://example.com/install.sh | bash"
        ));
    }

    #[test]
    fn test_is_npm_global_install_rejects_non_global_install() {
        assert!(!is_npm_global_install("npm install foo"));
    }

    #[test]
    fn test_is_npm_global_install_rejects_unrelated_command() {
        assert!(!is_npm_global_install("cargo install some-tool"));
    }

    // ── npm_eacces_hint ───────────────────────────────────────────────────────

    #[test]
    fn test_npm_eacces_hint_detects_old_format() {
        let stderr = "npm ERR! code EACCES\nnpm ERR! syscall mkdir\nnpm ERR! path /usr/local/lib/node_modules\nnpm ERR! errno -13\nnpm ERR! Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
        assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
    }

    #[test]
    fn test_npm_eacces_hint_detects_new_format() {
        let stderr = "npm error EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
        assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
    }

    #[test]
    fn test_npm_eacces_hint_returns_none_for_404_stderr() {
        let stderr = "npm error 404 Not Found - GET https://registry.npmjs.org/no-such-pkg";
        assert!(npm_eacces_hint(stderr, "npm install -g no-such-pkg").is_none());
    }

    // ── adapter_needs_install (codex version gate) ────────────────────────────

    /// plan_adapter_install is the pure install-plan seam used by
    /// install_acp_runtime_blocking. These tests verify:
    ///   - A 0.x binary (AdapterOutdated) → uninstall-then-install sequence returned
    ///   - A current 1.x binary (Available) → None (no reinstall)
    ///   - A 1.x binary below the floor → install plan returned
    ///   - Missing binary (None path) → catalog install commands returned
    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_selects_npm_command_for_outdated_0x_codex_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex-acp");
        // Simulate old 0.16.x: --version exits non-zero (unrecognised flag)
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

        assert!(
            plan.is_some(),
            "0.x codex adapter must trigger install plan"
        );
        let cmds = plan.unwrap();
        // Outdated arm: must uninstall the old package first, then install new.
        assert_eq!(
            cmds,
            vec![
                "npm uninstall -g @zed-industries/codex-acp",
                "npm install -g @agentclientprotocol/codex-acp",
            ],
            "outdated codex adapter must produce uninstall-then-install sequence; got {cmds:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_returns_none_for_current_1x_codex_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex-acp");
        // Simulate the minimum supported adapter version.
        std::fs::write(
            &bin,
            "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.7'\nexit 0\n",
        )
        .expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

        assert!(
            plan.is_none(),
            "current codex adapter must not trigger install plan (no reinstall needed)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_updates_older_1x_codex_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex-acp");
        // A 1.x adapter below MIN_CODEX_ACP_VERSION must still be reinstalled.
        std::fs::write(
            &bin,
            "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.5'\nexit 0\n",
        )
        .expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

        assert!(
            plan.is_some(),
            "older 1.x codex adapter must trigger update plan"
        );
    }

    #[test]
    fn test_plan_adapter_install_returns_catalog_cmds_when_no_adapter_path() {
        let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
        let plan = plan_adapter_install("codex", None, install_cmds, None);
        assert!(plan.is_some(), "missing adapter must trigger install plan");
        // Missing arm: use the catalog's install commands directly (no prior
        // package to uninstall — fresh install, not a reinstall).
        assert_eq!(
            plan.unwrap(),
            vec!["npm install -g @agentclientprotocol/codex-acp"],
            "missing codex adapter must use catalog install commands only"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_plan_adapter_install_non_codex_runtime_never_reinstalls() {
        use std::os::unix::fs::PermissionsExt;

        // For non-codex runtimes, any resolved binary means no install needed.
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("goose-acp");
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let install_cmds = &["npm install -g @block/goose-acp"];
        let plan = plan_adapter_install("goose", Some(&bin), install_cmds, None);
        assert!(
            plan.is_none(),
            "non-codex runtime with resolved binary must not trigger reinstall"
        );
    }

    // ── should_restart_after_install ─────────────────────────────────────────

    /// Setup-mode agent on matching runtime that is now Ready → restart.
    #[test]
    fn test_should_restart_after_install_setup_mode_now_ready_is_candidate() {
        assert!(
            should_restart_after_install(true, true, true, true, true),
            "setup-mode codex agent that became Ready must be restarted after install"
        );
    }

    /// Setup-mode agent still NotReady after install (e.g. logged out) → no restart.
    #[test]
    fn test_should_restart_after_install_still_not_ready_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, true, true, false),
            "setup-mode agent still NotReady must NOT be restarted (would re-enter setup mode)"
        );
    }

    /// Healthy in-pool agent (setup_mode=false) → no restart, even if now Ready.
    #[test]
    fn test_should_restart_after_install_healthy_agent_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, true, false, true),
            "healthy in-pool agent (setup_mode=false) must NOT be bounced on install"
        );
    }

    /// Agent on a different runtime_id → no restart.
    #[test]
    fn test_should_restart_after_install_different_runtime_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, true, false, true, true),
            "agent on a different runtime must NOT be restarted by this install"
        );
    }

    /// Remote/provider-backend agent → no restart (not local).
    #[test]
    fn test_should_restart_after_install_non_local_is_not_candidate() {
        assert!(
            !should_restart_after_install(false, true, true, true, true),
            "non-local (provider-backend) agent must NOT be restarted"
        );
    }

    /// Dead process (pid_alive=false) → no restart.
    #[test]
    fn test_should_restart_after_install_dead_pid_is_not_candidate() {
        assert!(
            !should_restart_after_install(true, false, true, true, true),
            "agent whose process is no longer running must NOT be restarted"
        );
    }

    // ── badge availability-drift (Phase 2) ───────────────────────────────────
    //
    // `availability_drift` is a pure predicate over two `Option` values —
    // no global state, no parallelism hazard.

    /// Both sides known and different → drift detected.
    #[test]
    fn test_availability_drift_detected_when_stamped_differs_from_current() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            availability_drift(
                Some(&AcpAvailabilityStatus::Available),
                Some(AcpAvailabilityStatus::AdapterOutdated),
            ),
            "Available stamped vs AdapterOutdated current must be detected as drift"
        );
    }

    /// Both sides known and equal → no drift.
    #[test]
    fn test_availability_drift_no_drift_when_stamped_equals_current() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(
                Some(&AcpAvailabilityStatus::Available),
                Some(AcpAvailabilityStatus::Available),
            ),
            "matching stamped and current must not show drift"
        );
    }

    /// Stamped is None (cold cache at spawn) → no drift regardless of current.
    #[test]
    fn test_availability_drift_none_stamp_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(None, Some(AcpAvailabilityStatus::Available)),
            "None stamp (cold cache at spawn) must never signal drift"
        );
    }

    /// Current is None (cache cold now) → no drift regardless of stamp.
    #[test]
    fn test_availability_drift_none_current_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        assert!(
            !availability_drift(Some(&AcpAvailabilityStatus::Available), None),
            "None current (cache cold) must never signal drift"
        );
    }

    /// Non-codex agent (stamp is None) → no drift (None case).
    #[test]
    fn test_availability_drift_non_codex_none_never_drifts() {
        use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
        // Non-codex agents have `adapter_availability = None` — must never flip.
        assert!(
            !availability_drift(None, Some(AcpAvailabilityStatus::AdapterMissing)),
            "non-codex agent (None stamp) must never trigger drift badge"
        );
    }

    // ── Phase A: install shell selection ─────────────────────────────────────

    /// On Unix, resolve_install_shell always succeeds (returns zsh or bash).
    #[cfg(unix)]
    #[test]
    fn test_resolve_install_shell_succeeds_on_unix() {
        let result = super::resolve_install_shell();
        assert!(result.is_ok(), "Unix must always resolve a shell");
        let shell = result.unwrap();
        assert!(
            shell == std::path::Path::new("/bin/zsh") || shell == std::path::Path::new("/bin/bash"),
            "expected /bin/zsh or /bin/bash, got {shell:?}"
        );
    }

    /// install_shell_command returns a valid Command on Unix.
    #[cfg(unix)]
    #[test]
    fn test_install_shell_command_returns_ok_on_unix() {
        let result = super::install_shell_command("echo test");
        assert!(result.is_ok(), "install_shell_command must succeed on Unix");
    }

    // ── pipefail: install pipes must not mask a failing left-hand side ────────

    /// The command handed to the install shell must run under `set -o pipefail;`
    /// with the vendor command preserved verbatim, so `curl … | bash` fails when
    /// `curl` does. Platform-agnostic: only the PATH prelude differs by OS, and
    /// `test_install_shell_args_shape_per_platform` pins that.
    #[test]
    fn test_install_shell_command_enables_pipefail() {
        let cmd = super::install_shell_command("curl -fsSL https://example.test/i.sh | bash")
            .expect("install shell must resolve on a test host");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let body = &args[2];
        assert!(
            body.contains("set -o pipefail; "),
            "the install body must set pipefail; got: {body}"
        );
        assert!(
            body.ends_with("curl -fsSL https://example.test/i.sh | bash"),
            "the vendor command must be preserved verbatim; got: {body}"
        );
    }

    /// The PATH prelude is emitted only where it helps, and the exact argument
    /// vector is the contract: a stray trailing positional with no `$1` reader,
    /// or an export whose `$1` the shell cannot split, both corrupt PATH.
    /// Windows is excluded because `join_paths` is `;`-separated there while bash
    /// splits PATH on `:` — and it is the platform where the inherited fallback
    /// always fires. See `install_shell_args` for the full reasoning.
    #[test]
    fn test_install_shell_args_shape_per_platform() {
        let composed = std::ffi::OsString::from("/buzz/node/bin:/usr/bin");
        let windows_composed = std::ffi::OsString::from(r"C:\buzz\node;C:\Windows\system32");
        let bare = ["-l", "-c", "set -o pipefail; echo hi"].map(std::ffi::OsString::from);

        assert_eq!(
            super::install_shell_args("echo hi", Some(&composed), false),
            [
                "-l",
                "-c",
                "export PATH=\"$1\"; set -o pipefail; echo hi",
                "buzz-install",
                "/buzz/node/bin:/usr/bin",
            ]
            .map(std::ffi::OsString::from),
            "Unix must re-export the composed PATH after login init"
        );
        assert_eq!(
            super::install_shell_args("echo hi", Some(&windows_composed), true),
            bare,
            "Windows must not re-export a `;`-joined PATH inside bash"
        );
        assert_eq!(
            super::install_shell_args("echo hi", None, false),
            bare,
            "no composed PATH must yield the bare pipefail body and no positionals"
        );
    }

    /// Regression for the login-startup-file overwrite: `cmd.env("PATH", …)` is
    /// installed *before* `-l` sources the user's profile, so a profile that
    /// assigns PATH silently discards the composed one. Uses `/bin/bash`
    /// explicitly — the planted profile is bash-specific, so resolving the host
    /// shell (which prefers zsh) would make this vacuous.
    #[cfg(unix)]
    #[test]
    fn test_composed_path_survives_a_profile_that_clears_it() {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::write(home.path().join(".bash_profile"), "export PATH=\n")
            .expect("plant a hostile login profile");
        let composed = std::ffi::OsString::from("/buzz/sentinel/bin:/usr/bin:/bin");

        // `echo` is a shell builtin, so the child needs no PATH to report one.
        let out = std::process::Command::new("/bin/bash")
            .args(super::install_shell_args(
                "echo \"$PATH\"",
                Some(&composed),
                false,
            ))
            .env("HOME", home.path())
            .env("PATH", &composed)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("bash must spawn");

        let path = String::from_utf8_lossy(&out.stdout);
        assert!(
            path.contains("/buzz/sentinel/bin"),
            "the composed PATH must survive login init; got: {path:?}"
        );
    }

    /// End-to-end on the real resolved install shell (no network): a pipeline
    /// whose left-hand side fails must exit non-zero, while a fully successful
    /// pipeline must still succeed. Without `pipefail` the status is the
    /// right-hand side's and the left-hand failure is invisible.
    #[cfg(unix)]
    #[test]
    fn test_install_shell_pipeline_status_follows_left_side() {
        for (command, expect_success) in [("false | true", false), ("echo ok | cat", true)] {
            let status = super::install_shell_command(command)
                .expect("Unix must always resolve an install shell")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("install shell must spawn");
            assert_eq!(
                status.success(),
                expect_success,
                "`{command}` must report success={expect_success}; got {status:?}"
            );
        }
    }

    // ── Phase A: Windows install shell selection ───────────────────────────────

    /// On Windows (CI runner has Git pre-installed), resolve_install_shell succeeds.
    #[cfg(windows)]
    #[test]
    fn test_resolve_install_shell_succeeds_on_windows_with_git() {
        let result = super::resolve_install_shell();
        assert!(
            result.is_ok(),
            "Windows CI runner has Git — resolve_install_shell must succeed; got: {:?}",
            result.err()
        );
        let shell = result.unwrap();
        // The resolved path must end with bash.exe (Git Bash).
        let fname = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            fname.eq_ignore_ascii_case("bash.exe"),
            "Windows install shell must be bash.exe, got: {shell:?}"
        );
    }

    /// On Windows, when no Git Bash is found, the error carries the Doctor hint.
    #[cfg(windows)]
    #[test]
    fn test_resolve_install_shell_error_contains_doctor_hint() {
        // We can't force resolve_install_shell to fail on CI (Git is installed),
        // but we can verify the error string it would use matches the hint.
        let hint = crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT;
        assert!(
            hint.contains("Git for Windows"),
            "GIT_BASH_INSTALL_HINT must mention Git for Windows; got: {hint}"
        );
        assert!(
            hint.contains("PATH"),
            "GIT_BASH_INSTALL_HINT must mention PATH option; got: {hint}"
        );
    }

    /// install_shell_command returns a valid Command on Windows.
    #[cfg(windows)]
    #[test]
    fn test_install_shell_command_returns_ok_on_windows() {
        let result = super::install_shell_command("echo test");
        assert!(
            result.is_ok(),
            "install_shell_command must succeed on Windows with Git; got: {:?}",
            result.err()
        );
    }

    /// On Windows, `install_shell_command` must set PATH to a value that
    /// includes the inherited process PATH, so node/npm are visible inside
    /// the install shell even when no managed Node runtime is present.
    #[cfg(windows)]
    #[test]
    fn test_install_shell_command_includes_process_path_on_windows() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let previous = std::env::var_os("PATH");
        // Plant a sentinel in the process PATH that the test can detect.
        let sentinel = r"C:\TestSentinel\bin";
        std::env::set_var("PATH", sentinel);

        let result = super::install_shell_command("echo test");

        match previous {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        let cmd = result.expect("install_shell_command must succeed on Windows with Git");
        let path_value = cmd
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, val)| val)
            .map(|v| v.to_string_lossy().into_owned())
            .expect("install_shell_command must always set a PATH env var on Windows");

        // The sentinel (inherited process PATH) must appear in the composed PATH.
        assert!(
            path_value.contains(sentinel),
            "install_shell_command PATH must include the inherited process PATH; got: {path_value}"
        );
        // The sentinel must appear LAST — managed Buzz dirs must have precedence.
        assert!(
            path_value.ends_with(sentinel),
            "inherited process PATH must be appended LAST so managed dirs keep precedence; got: {path_value}"
        );
    }

    // ── Phase B: per-OS install commands ──────────────────────────────────────

    /// On non-Windows, cli_install_commands_for_os returns the default commands.
    #[cfg(not(windows))]
    #[test]
    fn test_cli_install_commands_for_os_returns_default_on_unix() {
        let claude = crate::managed_agents::known_acp_runtime_exact("claude").unwrap();
        assert_eq!(
            claude.cli_install_commands_for_os(),
            claude.cli_install_commands,
            "on Unix, cli_install_commands_for_os must return the default install.sh commands"
        );
    }

    /// buzz-agent has no install commands on any platform.
    #[test]
    fn test_buzz_agent_has_no_install_commands() {
        let buzz = crate::managed_agents::known_acp_runtime_exact("buzz-agent").unwrap();
        assert!(
            buzz.cli_install_commands_for_os().is_empty(),
            "buzz-agent ships with the app — must never have install commands"
        );
    }

    // ── PowerShell routing ────────────────────────────────────────────────────

    /// Commands beginning with `powershell.exe` (any casing) must be identified
    /// as PowerShell commands; all others must not.
    #[cfg(windows)]
    #[test]
    fn test_is_powershell_command_detects_powershell_commands() {
        assert!(
            super::is_powershell_command(
                r#"powershell.exe -NoProfile -NonInteractive -Command "irm https://chatgpt.com/codex/install.ps1 | iex""#
            ),
            "canonical codex install command must be detected as PowerShell"
        );
        assert!(
            super::is_powershell_command("POWERSHELL.EXE -Command foo"),
            "is_powershell_command must be case-insensitive"
        );
        assert!(
            !super::is_powershell_command("npm install -g @agentclientprotocol/claude-agent-acp"),
            "npm commands must NOT be detected as PowerShell"
        );
        assert!(
            !super::is_powershell_command(r"curl -fsSL https://example.com | bash"),
            "bash pipe commands must NOT be detected as PowerShell"
        );
        assert!(
            !super::is_powershell_command(""),
            "empty string must not be detected as PowerShell"
        );
    }

    /// On Windows, `build_install_command` must return a `Command` whose
    /// program is `powershell.exe` (not `bash.exe`) for PowerShell commands.
    #[cfg(windows)]
    #[test]
    fn test_build_install_command_uses_powershell_natively_on_windows() {
        let ps_command = r#"powershell.exe -NoProfile -NonInteractive -Command "irm https://chatgpt.com/codex/install.ps1 | iex""#;
        let result = super::build_install_command(ps_command);
        assert!(
            result.is_ok(),
            "build_install_command must succeed for a PowerShell command; got: {:?}",
            result.err()
        );
        let cmd = result.unwrap();
        let program = cmd.get_program().to_string_lossy().to_lowercase();
        assert!(
            program.contains("powershell"),
            "PowerShell install command must use powershell.exe, not bash; got: {program}"
        );
        assert!(
            !program.contains("bash"),
            "PowerShell install command must NOT go through bash; got: {program}"
        );
    }

    /// On Windows, `build_install_command` must route non-PowerShell commands
    /// through Git Bash (program must be bash.exe).
    #[cfg(windows)]
    #[test]
    fn test_build_install_command_uses_git_bash_for_non_powershell_on_windows() {
        let npm_command = "npm install -g @agentclientprotocol/claude-agent-acp";
        let result = super::build_install_command(npm_command);
        assert!(
            result.is_ok(),
            "build_install_command must succeed for an npm command on Windows with Git; got: {:?}",
            result.err()
        );
        let cmd = result.unwrap();
        let program = cmd.get_program().to_string_lossy().to_lowercase();
        assert!(
            program.contains("bash"),
            "non-PowerShell install command must still use bash.exe on Windows; got: {program}"
        );
    }

    /// On non-Windows, `build_install_command` must always use the Unix shell
    /// (zsh or bash), never powershell.exe.
    #[cfg(not(windows))]
    #[test]
    fn test_build_install_command_uses_unix_shell_on_non_windows() {
        let command = r"curl -fsSL https://example.com/install.sh | bash";
        let result = super::build_install_command(command);
        assert!(
            result.is_ok(),
            "build_install_command must succeed on Unix; got: {:?}",
            result.err()
        );
        let cmd = result.unwrap();
        let program = cmd.get_program().to_string_lossy();
        assert!(
            program.contains("bash") || program.contains("zsh"),
            "Unix install command must use bash or zsh, got: {program}"
        );
    }

    /// On Windows, `install_powershell_command` must build an exact argv:
    /// flags before `-Command` forwarded, body unquoted (outer catalog quotes stripped),
    /// no bash flags, and `-Command` found on token boundary not as substring.
    #[cfg(windows)]
    #[test]
    fn test_powershell_command_argv_exact() {
        // Catalog format: body wrapped in one outer double-quote pair (Bash-layer serialization).
        let body = "irm https://chatgpt.com/codex/install.ps1 | iex";
        let cmd = super::install_powershell_command(&format!(
            r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "{body}""#
        ));
        assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", body],
            "argv must be exact with outer quotes stripped"
        );
    }

    /// Token that merely contains `-command` as a substring must not be treated
    /// as the `-Command` boundary; only an exact token match (case-insensitive) counts.
    #[cfg(windows)]
    #[test]
    fn test_powershell_command_token_boundary_not_substring() {
        let cmd = super::install_powershell_command(
            r#"powershell.exe -x-command-y -Command "echo hello""#,
        );
        assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["-x-command-y", "-Command", "echo hello"],
            "substring must not consume -Command boundary early"
        );
    }

    /// Claude Code catalog command (discovery.rs:107) must dequote to the bare pipeline.
    #[cfg(windows)]
    #[test]
    fn test_powershell_command_claude_catalog_dequoted() {
        let cmd = super::install_powershell_command(
            r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://claude.ai/install.ps1 | iex""#,
        );
        assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "irm https://claude.ai/install.ps1 | iex",
            ],
            "Claude catalog command must be dequoted correctly"
        );
    }

    /// Goose Windows catalog command (discovery.rs:78) must dequote to a bare pipeline
    /// with a literal `$env:` prefix — no backslash before the dollar sign.
    /// This proves the `\$` → `$` escape fix: post-#2750 the spawn is native and
    /// PowerShell receives the body verbatim, so a residual `\` would produce
    /// `\$env:CONFIGURE='false'` which is a malformed statement.
    #[cfg(windows)]
    #[test]
    fn test_powershell_command_goose_catalog_dequoted() {
        let cmd = super::install_powershell_command(
            r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:CONFIGURE='false'; irm https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1 | iex""#,
        );
        assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$env:CONFIGURE='false'; irm https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1 | iex",
            ],
            "Goose catalog command must dequote with bare $env: (no backslash before $)"
        );
    }
}

/// Returns the Windows-only Git Bash prerequisite used by buzz-agent's shell MCP.
/// `None` on other platforms keeps the shared Doctor surfaces platform-neutral.
#[tauri::command]
pub async fn discover_git_bash_prerequisite(
) -> Result<Option<crate::managed_agents::GitBashPrerequisite>, String> {
    tokio::task::spawn_blocking(crate::managed_agents::discover_git_bash)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}
