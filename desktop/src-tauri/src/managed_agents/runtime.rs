use std::collections::HashMap;

use tauri::AppHandle;

use super::agent_env::build_buzz_agent_provider_defaults;

use crate::{
    managed_agents::{
        append_log_marker, known_acp_runtime, login_shell_path, managed_agent_log_path,
        missing_command_message, normalize_agent_args, open_log_file, resolve_command,
        spawn_key_refusal, ManagedAgentPairRuntime, ManagedAgentRecord, ManagedAgentRuntimeKey,
        ManagedAgentRuntimeLifecycle, ManagedAgentSummary,
    },
    util::now_iso,
};

mod adapter;
#[cfg(test)]
use adapter::is_bundled_sibling;
use adapter::{
    resolve_canonical_bundled_buzz_agent, resolve_canonical_bundled_executable,
    validate_managed_adapter_descriptor,
};

mod path;
pub(in crate::managed_agents) use path::build_augmented_path;
pub(crate) use path::compose_path_entries;
pub(crate) use path::should_skip_claude_executable;
pub(crate) use path::should_use_inherited;

mod metadata;
pub(crate) use metadata::{
    apply_agent_display_env, resolve_session_title, runtime_metadata_env_vars,
    DISPLAY_NAME_ENV_VAR, SESSION_TITLE_ENV_VAR,
};

mod stop;
pub(crate) use stop::managed_agent_runtime_keys;
pub use stop::{stop_managed_agent_process, stop_managed_agent_workspace_pair};

mod environment;
pub(crate) use environment::{
    acp_turn_limits_log_line, build_respond_to_env, configure_managed_acp_environment,
    configure_runtime_cli, managed_runtime_feature_gates, ManagedRuntimeLaunchMode,
};
#[cfg(test)]
pub(crate) use environment::{
    configure_managed_acp_turn_environment, configure_rollout_gate_environment,
    effective_acp_turn_limits, ManagedRuntimeFeatureGates,
};

mod process;
#[cfg(test)]
pub(crate) use process::process_is_running;
pub(crate) use process::{
    adopt_schema_v2_runtime, current_instance_id, legacy_migration_gate, pair_lock_is_held,
    select_rollout_launch_mode, stop_verified_legacy_runtime, terminate_process,
    verify_runtime_lock_proof, LegacyMigrationGate,
};

mod migration;
pub use migration::clear_legacy_runtime_pids;

/// Classify an agent's persona against the live catalog for the Agents-menu
/// drift indicator. Returns `(out_of_date, orphaned)`.
///
/// Drift basis is the RECORD's `persona_source_version`, never the engram:
/// - persona_id set + persona present: out_of_date when the snapshot hash
///   differs from the persona's current content hash.
/// - persona_id set + persona gone: orphaned (no current hash to respawn into,
///   so never out_of_date — we must not tell the user to respawn into nothing).
/// - no persona_id: neither — a hand-built agent has no persona to drift from.
fn persona_drift_state(
    record: &ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
) -> (bool, bool) {
    let Some(persona_id) = record.persona_id.as_deref() else {
        return (false, false);
    };
    let Some(persona) = personas.iter().find(|p| p.id == persona_id) else {
        return (false, true);
    };
    let current = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(persona),
    );
    let out_of_date = record
        .persona_source_version
        .as_deref()
        .is_some_and(|pinned| pinned != current);
    (out_of_date, false)
}

/// Resolve the runtime-pair key this record maps to for the active
/// workspace: always the active workspace relay (the legacy per-record relay
/// pin is ignored — see `effective_agent_relay_url`). Returns `None` for
/// records that cannot form a valid pair key yet (e.g. key-less agents that
/// mint keys on first start).
pub(crate) fn workspace_pair_key(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Option<ManagedAgentRuntimeKey> {
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    resolve_workspace_pair_key(
        &record.pubkey,
        &record.relay_url,
        &crate::relay::relay_ws_url_with_override(&state),
    )
}

/// Pure core of [`workspace_pair_key`]: workspace-relay resolution (legacy
/// record pins ignored) plus canonical key construction, kept `AppHandle`-free
/// so summary/stop scoping semantics are unit-testable.
pub(crate) fn resolve_workspace_pair_key(
    pubkey: &str,
    record_relay_url: &str,
    workspace_relay_url: &str,
) -> Option<ManagedAgentRuntimeKey> {
    let effective_relay =
        crate::relay::effective_agent_relay_url(record_relay_url, workspace_relay_url);
    ManagedAgentRuntimeKey::new(pubkey.to_string(), &effective_relay).ok()
}

pub fn build_managed_agent_summary(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    global_config: &crate::managed_agents::GlobalAgentConfig,
) -> Result<ManagedAgentSummary, String> {
    use crate::managed_agents::BackendKind;

    // Community-scoped truth: this summary describes the pair for the active
    // workspace relay. An agent running only in another community must read
    // as stopped here — matching by pubkey alone would show every community a
    // green light as long as any pair anywhere is alive.
    let pair_key = workspace_pair_key(app, record);
    let pair_runtime = pair_key.as_ref().and_then(|key| runtimes.get(key));

    let (status, pid, log_path) = if record.backend != BackendKind::Local {
        // Two-axis status model for remote agents:
        //
        //   Control-plane (this field): "deployed" = provider has been invoked and
        //   returned a backend_agent_id. "not_deployed" = no deploy call yet (or it
        //   failed). This axis tracks whether infrastructure *exists*, not whether
        //   the process is currently running.
        //
        //   Live axis (relay presence, polled by frontend): online/away/offline.
        //   Shown as a PresenceDot next to the agent name. This is the real-time
        //   signal for whether the harness is connected.
        //
        // After !shutdown the agent goes offline (presence) but stays "deployed"
        // (infrastructure still exists). This is intentional — the provider may
        // have allocated a VM/container that persists across process restarts.
        // A future provider `undeploy` operation (v2) will handle teardown.
        let status = if record.backend_agent_id.is_some() {
            "deployed".to_string()
        } else {
            "not_deployed".to_string()
        };
        (status, None, String::new())
    } else {
        if let Some(runtime) = pair_runtime {
            (
                match runtime.lifecycle {
                    ManagedAgentRuntimeLifecycle::Failed => "failed",
                    ManagedAgentRuntimeLifecycle::LegacyRuntimeActive => "legacy_runtime_active",
                    ManagedAgentRuntimeLifecycle::ManualLegacyStopRequired => {
                        "manual_legacy_stop_required"
                    }
                    _ => "running",
                }
                .to_string(),
                Some(runtime.pid()),
                runtime
                    .log_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| {
                        pair_key
                            .as_ref()
                            .and_then(|key| super::managed_agent_runtime_log_path(app, key).ok())
                            .map(|path| path.display().to_string())
                            .unwrap_or_default()
                    }),
            )
        } else {
            (
                "stopped".to_string(),
                None,
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
            )
        }
    };

    let (persona_out_of_date, persona_orphaned) = persona_drift_state(record, personas);

    let global_for_summary =
        crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let effective_cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record,
        personas,
        &global_for_summary,
    );
    let (effective_model, effective_provider, effective_prompt, model_source) = match effective_cfg
    {
        crate::managed_agents::effective_config::EffectiveConfigResult::Resolved(cfg) => {
            let source = cfg.model.source.clone();
            (
                cfg.model.value,
                cfg.provider.value,
                cfg.system_prompt.value,
                Some(source),
            )
        }
        crate::managed_agents::effective_config::EffectiveConfigResult::OrphanedInstance {
            record_pubkey,
            missing_persona_id,
        } => {
            eprintln!(
                "orphaned agent instance: pubkey={record_pubkey}, missing_persona_id={missing_persona_id}"
            );
            (None, None, None, None)
        }
    };

    // Restart badge: the running process stamped the effective spawn config
    // it was launched with; recompute a prospective one from current disk
    // state and report every differing field. Only the tracked live pair for
    // THIS workspace can drift — stopped agents spawn fresh, adopted
    // (runtime_pid-only) processes have no stamp to compare, and pairs running
    // for other communities are judged in their own community (comparing them
    // against this workspace's relay would flag a spurious restart on every
    // community switch).
    //
    // Adapter-availability drift (codex only) contributes its own synthetic
    // entry, so an out-of-band adapter change (manual npm install/downgrade)
    // that Phase-1 auto-restart doesn't cover still shows the user what moved.
    // The cache is read-only here — no subprocess is spawned.
    //
    // Global config drives both the prospective snapshot and the descriptor
    // env layering below — the caller loads it once and passes it in, so
    // list-style callers pay one disk read per call rather than one per record.

    // The prospective side is computed only for a tracked pair: it costs a
    // teams-store read, and an unstamped agent has nothing to compare against.
    let tracked_spawn = pair_key.as_ref().zip(pair_runtime).map(|(key, runtime)| {
        let teams = crate::managed_agents::load_teams(app).unwrap_or_default();
        let current = crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
            record,
            personas,
            &teams,
            &key.relay_url,
            global_config,
        );
        (runtime, current)
    });
    let restart_diff = crate::managed_agents::spawn_snapshot::eligible_restart_diff(
        persona_orphaned,
        tracked_spawn.as_ref().and_then(|(runtime, current)| {
            let process = runtime.process.as_ref()?;
            Some(crate::managed_agents::spawn_snapshot::TrackedSpawnState {
                stamped: &process.spawn_config,
                current,
                stamped_availability: process.adapter_availability.as_ref(),
                current_availability: super::adapter_availability_cached(),
            })
        }),
    );
    // Active durable work defers configuration replacement until the runtime
    // is idle: a restart would kill running jobs and abandon a live
    // assignment. Availability drift is already folded into restart_diff.
    let has_active_jobs = pair_runtime.is_some_and(|runtime| !runtime.active_jobs.is_empty());
    let active_assignment = pair_runtime
        .and_then(|runtime| runtime.active_assignment.as_ref())
        .map(|assignment| assignment.state);
    let needs_restart = restart_eligible(
        has_active_jobs,
        active_assignment,
        persona_orphaned,
        !restart_diff.is_empty(),
        false,
    );

    // Resolve the effective harness via the single typed descriptor — same resolver
    // as spawn, so the UI reflects the persona's current harness (or explicit pin).
    let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
        record,
        personas,
        global_config,
    )
    .unwrap_or_else(|e| {
        // Dangling harness — surface the missing id so the UI tells the same
        // story as spawn (which refuses with a sentence), rather than silently
        // showing the default-command fallback as if the agent were healthy.
        let cmd = match crate::managed_agents::dangling_harness_id(&e) {
            Some(id) => crate::managed_agents::dangling_harness_display(id),
            None => crate::managed_agents::record_agent_command(record, personas),
        };
        let args = normalize_agent_args(&cmd, record.agent_args.clone());
        crate::managed_agents::readiness::EffectiveHarnessDescriptor {
            command: cmd,
            args,
            env: Default::default(),
        }
    });
    let effective_mcp_command = known_acp_runtime(&descriptor.command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("")
        .to_string();

    Ok(ManagedAgentSummary {
        pubkey: record.pubkey.clone(),
        name: record.name.clone(),
        persona_id: record.persona_id.clone(),
        runtime: record.runtime.clone(),
        team_id: record.team_id.clone(),
        relay_url: record.relay_url.clone(),
        acp_command: record.acp_command.clone(),
        agent_command: descriptor.command,
        agent_command_override: record.agent_command_override.clone(),
        agent_args: descriptor.args,
        mcp_command: effective_mcp_command,
        turn_timeout_seconds: record.turn_timeout_seconds,
        idle_timeout_seconds: record.idle_timeout_seconds,
        max_turn_duration_seconds: record.max_turn_duration_seconds,
        parallelism: record.parallelism,
        system_prompt: effective_prompt,
        avatar_url: record.avatar_url.clone(),
        model: effective_model,
        model_source,
        provider: effective_provider,
        persona_out_of_date,
        persona_orphaned,
        needs_restart,
        restart_diff,
        env_vars: record.env_vars.clone(),
        backend: record.backend.clone(),
        backend_agent_id: record.backend_agent_id.clone(),
        status,
        pid,
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        last_started_at: record.last_started_at.clone(),
        last_stopped_at: record.last_stopped_at.clone(),
        last_exit_code: record.last_exit_code,
        last_error: record.last_error.clone(),
        last_error_code: record.last_error_code,
        start_on_app_launch: record.start_on_app_launch,
        auto_restart_on_config_change: record.auto_restart_on_config_change,
        log_path,
        respond_to: record.respond_to,
        respond_to_allowlist: record.respond_to_allowlist.clone(),
    })
}

/// Pure predicate: should the "Restart required" badge fire?
///
/// Active durable work defers configuration replacement until the runtime is
/// idle. An orphaned linked instance can never be restarted successfully.
fn restart_eligible(
    has_active_jobs: bool,
    active_assignment: Option<buzz_runtime_pkg::protocol::AssignmentState>,
    persona_orphaned: bool,
    hash_drift: bool,
    availability_drift: bool,
) -> bool {
    let assignment_is_nonterminal = active_assignment.is_some_and(|state| !state.is_terminal());
    !has_active_jobs
        && !assignment_is_nonterminal
        && !persona_orphaned
        && (hash_drift || availability_drift)
}

pub fn find_managed_agent_mut<'a>(
    records: &'a mut [ManagedAgentRecord],
    pubkey: &str,
) -> Result<&'a mut ManagedAgentRecord, String> {
    records
        .iter_mut()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))
}

/// Spawn an agent process without holding any locks on records or runtimes.
/// Returns the child process and log path on success. The caller is responsible
/// for updating `ManagedAgentRecord` fields and inserting into the runtimes map.
///
/// `owner_hex`: the workspace owner's pubkey, used as a fallback for legacy
/// records that have no NIP-OA `auth_tag`. See `build_respond_to_env`.
pub fn spawn_agent_child(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    relay_url: &str,
    lazy: bool,
    owner_hex: Option<&str>,
    launch_mode: ManagedRuntimeLaunchMode,
) -> Result<crate::managed_agents::ManagedAgentProcess, String> {
    if let Some(error) = spawn_key_refusal(record) {
        return Err(error);
    }
    let runtime_key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), relay_url)?;
    let runtime_lock_path = super::managed_agent_runtime_lock_path(app, &runtime_key)?;
    // Resolve the effective harness (agent command) from the linked persona, so
    // persona harness edits propagate on the next spawn; an explicit per-agent
    // override wins. `agent_args` and `mcp_command` are pure derivations of the
    // command, so we recompute them from the effective value rather than the
    // frozen record snapshot. Mirrors the model resolution below.
    let personas = super::load_personas(app).unwrap_or_default();
    let teams = super::load_teams(app).unwrap_or_default();
    // Load global config once; used for runtime_metadata_env_vars (model/provider fallback)
    // and for the env-var merge at spawn time.
    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();

    // Resolve model/provider/prompt ONCE, here, at the shared spawn boundary —
    // the single source both the env writes below and the spawn-config snapshot
    // read from. Previously prompt was read from the record's own (possibly
    // stale, Phase-A-snapshot) bytes while model/provider were resolved live
    // from `personas`; a definition edit landing between a caller's snapshot
    // apply and this spawn could hand a fresh model/provider to a stale
    // prompt. This also folds in orphan refusal via `require_resolved`: every
    // caller (interactive start, launch restore, `start_managed_agent_process`)
    // inherits it — no caller can bypass this by reaching `spawn_agent_child`
    // directly. Checked before any side effect (log marker, log file, process
    // spawn) so a refused spawn leaves no trace.
    let effective_cfg = crate::managed_agents::effective_config::resolve_effective_config(
        record, &personas, &global,
    )
    .require_resolved()?;

    // Single typed resolver: validates runtime id (dangling harness → Err), resolves
    // command, args (instance wins over definition default), and the full env layer stack.
    // This is the sole path for harness-definition lookup — spawn, snapshot,
    // summary, and model probes all consume this descriptor rather than
    // assembling values inline.
    // Like the orphan refusal above, this runs before any side effect so a refused
    // spawn leaves no trace.
    let descriptor =
        crate::managed_agents::resolve_effective_harness_descriptor(record, &personas, &global)
            .map_err(|e| {
                format!(
                    "cannot spawn agent {}: {}",
                    record.pubkey,
                    crate::managed_agents::user_facing_harness_error(&e)
                )
            })?;
    let effective_command = &descriptor.command;
    let agent_args = &descriptor.args;
    validate_managed_adapter_descriptor(effective_command, agent_args)?;

    let log_path = super::managed_agent_runtime_log_path(app, &runtime_key)?;
    append_log_marker(
        &log_path,
        &format!(
            "\n=== starting {} ({}) at {} ===",
            record.name,
            record.pubkey,
            now_iso()
        ),
    )?;
    append_log_marker(&log_path, &acp_turn_limits_log_line(record))?;

    let stdout = open_log_file(&log_path)?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone log handle: {error}"))?;
    let resolved_acp_command = resolve_command(&record.acp_command)
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let effective_mcp_command = known_acp_runtime(effective_command)
        .and_then(|runtime| runtime.mcp_command)
        .filter(|command| *command == "buzz-dev-mcp")
        .ok_or_else(|| {
            "unsupported_managed_adapter: durable managed mode requires the canonical bundled buzz-dev-mcp executable"
                .to_string()
        })?;
    let resolved_mcp_command = Some(resolve_canonical_bundled_executable(
        effective_mcp_command,
        "buzz-dev-mcp",
    )?);
    let resolved_agent_command = resolve_canonical_bundled_buzz_agent()?
        .display()
        .to_string();

    // The caller supplies the explicit canonical pair relay. This is the only
    // relay this child may connect to, regardless of the record/workspace default.
    let effective_relay_url = runtime_key.relay_url.clone();

    // Augment PATH for DMG launches so child processes can find:
    //   - bundled CLI via ~/.local/bin symlink
    //   - nvm-managed node/npm (nvm initializes only in interactive shells)
    //   - bundled sidecars (buzz, buzz-acp, etc.) via exe parent (Contents/MacOS/)
    //   - runtimes (node, python, etc.) via login shell PATH
    let nvm_bin = dirs::home_dir()
        .as_deref()
        .and_then(super::find_nvm_default_bin);
    let augmented_path = build_augmented_path(
        dirs::home_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        login_shell_path(),
        nvm_bin,
    );

    let mut command = std::process::Command::new(&resolved_acp_command);
    if let Some(home) = super::default_agent_workdir() {
        command.current_dir(home);
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::from(stdout));
    command.stderr(std::process::Stdio::from(stderr));
    if let Some(ref path) = augmented_path {
        command.env("PATH", path);
    }
    command.env("RUST_LOG", child_rust_log_filter());
    command.env("BUZZ_PRIVATE_KEY", &record.private_key_nsec);
    command.env("BUZZ_RELAY_URL", &effective_relay_url);
    command.env("BUZZ_ACP_LAZY_POOL", if lazy { "true" } else { "false" });
    command.env("BUZZ_ACP_AGENT_COMMAND", &resolved_agent_command);
    command.env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));
    match &resolved_mcp_command {
        Some(mcp_cmd) => {
            command.env("BUZZ_ACP_MCP_COMMAND", mcp_cmd);
        }
        None => {
            command.env("BUZZ_ACP_MCP_COMMAND", "");
        }
    }
    // Enable MCP hook tools (_Stop, _PostCompact) for agents that need them.
    // Uses "*" because build_mcp_servers() hard-codes the server name to "buzz-mcp".
    let runtime_meta = known_acp_runtime(effective_command);
    if runtime_meta.is_some_and(|r| r.mcp_hooks) {
        command.env("MCP_HOOK_SERVERS", "*");
    }

    // ── Readiness check: set setup-payload if agent is not ready ─────────────
    //
    // Build the effective env the agent would have at start-time, run the
    // readiness predicate, and if anything is missing, serialize the payload
    // into BUZZ_ACP_SETUP_PAYLOAD.  buzz-acp detects this env var on startup
    // and enters the minimal setup-listener mode instead of the agent pool.
    //
    // SECURITY: BUZZ_ACP_SETUP_PAYLOAD is in RESERVED_ENV_KEYS so user env
    // cannot set it, but we also explicitly remove it after writing user env
    // to guard against the parent-process environment. We then set it only
    // when desktop has computed NotReady — the desktop is the sole readiness
    // source and buzz-acp only transports the payload.
    //
    // The JSON format mirrors `setup_mode::SetupPayload` in buzz-acp:
    //   { "agent_name": "...", "agent_pubkey": "...", "requirements": [{ "surface": "...", ... }] }
    //
    // `spawned_setup_mode` is captured outside the block so it can be stamped
    // on `ManagedAgentProcess` — used by `install_acp_runtime` to target only
    // stuck agents for auto-restart.
    let spawned_setup_mode;
    {
        use crate::managed_agents::readiness::EffectiveAgentEnv;
        use crate::managed_agents::{agent_readiness, AgentReadiness, Requirement};

        // Construct EffectiveAgentEnv from the descriptor computed above — no second
        // resolver call; the descriptor's env is already the fully layered result.
        let effective = EffectiveAgentEnv {
            env: descriptor.env.clone(),
            config_file_path: runtime_meta.and_then(|r| r.config_file_path),
            effective_command: descriptor.command.clone(),
        };
        // Compute the optional payload before touching the command.
        let setup_payload_json =
            if let AgentReadiness::NotReady { requirements } = agent_readiness(&effective) {
                let reqs: Vec<serde_json::Value> = requirements
                    .into_iter()
                    .map(|r| match r {
                        Requirement::NormalizedField { field } => serde_json::json!({
                            "surface": "normalized_field",
                            "field": field,
                        }),
                        Requirement::EnvKey { key } => serde_json::json!({
                            "surface": "env_key",
                            "key": key,
                        }),
                        Requirement::CliLogin {
                            probe_args,
                            setup_copy,
                            availability,
                        } => serde_json::json!({
                            "surface": "cli_login",
                            "probe_args": probe_args,
                            "setup_copy": setup_copy,
                            "availability": availability,
                        }),
                        Requirement::CliConfigInvalid {
                            probe_args,
                            setup_copy,
                            diagnostic,
                        } => serde_json::json!({
                            "surface": "cli_config_invalid",
                            "probe_args": probe_args,
                            "setup_copy": setup_copy,
                            "diagnostic": diagnostic,
                        }),
                        Requirement::GitBash => serde_json::json!({
                            "surface": "git_bash",
                        }),
                        Requirement::MissingBinary { command } => serde_json::json!({
                            "surface": "missing_binary",
                            "command": command,
                        }),
                    })
                    .collect();
                let payload = serde_json::json!({
                    "agent_name": record.name,
                    "agent_pubkey": record.pubkey,
                    "requirements": reqs,
                });
                match serde_json::to_string(&payload) {
                    Ok(json) => Some(json),
                    Err(e) => {
                        eprintln!(
                            "buzz-desktop: failed to serialize setup payload for {}: {e}",
                            record.name
                        );
                        None
                    }
                }
            } else {
                None
            };

        spawned_setup_mode = setup_payload_json.is_some();

        // Strip the key from the process-spawned command on every path.
        // Two independent guards protect the invariant:
        //   1. BUZZ_ACP_SETUP_PAYLOAD is in RESERVED_ENV_KEYS, so
        //      merged_user_env() can never write it via saved/persona env.
        //   2. This env_remove() clears any ambient parent-process value
        //      inherited by std::process::Command before we conditionally
        //      set the desktop-computed trusted value below.
        // Note: merged_user_env() is written further below in this function;
        // ordering relative to that call is NOT what makes this safe — the
        // reserved-key strip (guard 1) handles user env regardless of order.
        command.env_remove("BUZZ_ACP_SETUP_PAYLOAD");

        // Set the payload only when desktop computed NotReady.
        if let Some(json) = setup_payload_json {
            command.env("BUZZ_ACP_SETUP_PAYLOAD", json);
            eprintln!(
                "buzz-desktop: agent {} not ready — spawning in setup-listener mode",
                record.name
            );
        }
    }
    // Managed ACP controls are applied after user environment layering below.
    // This prevents ambient or persisted legacy timeout values from defeating
    // harness defaults and protects the pair-scoped exclusivity lock path.
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                command.env(key, value);
            }
        }
    }
    let team_instructions = super::spawn_snapshot::effective_team_instructions(record, &teams);
    if let Some(instructions) = &team_instructions {
        command.env("BUZZ_ACP_TEAM_INSTRUCTIONS", instructions);
    } else {
        command.env_remove("BUZZ_ACP_TEAM_INSTRUCTIONS");
    }

    // Prompt, model, and provider all come from the single `effective_cfg`
    // resolved at the top of this function — the SAME resolve the spawn-config
    // snapshot reads, so env write and restart badge cannot disagree. Linked
    // instances never consult the record's own model/provider/prompt bytes;
    // definition-less instances fall back to their own fields, then global.
    //
    // Derive the mesh decision BEFORE moving fields out — `relay_mesh_model_id`
    // is the single authoritative gate; the mesh-llm block below MUST use it
    // rather than re-deriving from `effective_provider` to keep preflight and
    // spawn semantics in lock-step (see `EffectiveAgentConfig::relay_mesh_model_id`).
    #[cfg(feature = "mesh-llm")]
    let mesh_model_id = effective_cfg.relay_mesh_model_id();
    let effective_prompt = effective_cfg.system_prompt.value;
    let effective_model = effective_cfg.model.value;
    let effective_provider = effective_cfg.provider.value;

    if let Some(prompt) = &effective_prompt {
        command.env("BUZZ_ACP_SYSTEM_PROMPT", prompt);
    } else {
        command.env_remove("BUZZ_ACP_SYSTEM_PROMPT");
    }
    if let Some(model) = effective_model.as_deref() {
        command.env("BUZZ_ACP_MODEL", model);
    } else {
        command.env_remove("BUZZ_ACP_MODEL");
    }
    // Session title for the harness to pass out-of-band on `session/new`. The
    // adapter names the session after it; it never reaches the prompt, so this
    // is display metadata only. The spawn-config snapshot records the same
    // resolve, so a rename raises the restart badge instead of leaving the
    // process stale.
    apply_agent_display_env(
        &mut command,
        resolve_session_title(record.display_name.as_deref(), &record.name),
    );
    build_buzz_agent_provider_defaults(&mut command);
    if let Some(meta) = runtime_meta {
        for (key, value) in runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            effective_model.as_deref(),
            effective_provider.as_deref(),
        ) {
            command.env(key, value);
        }
    }
    command.env_remove("BUZZ_ACP_PRIVATE_KEY");
    command.env_remove("BUZZ_ACP_API_TOKEN");
    command.env_remove("BUZZ_API_TOKEN");

    if let Some(ref auth_tag) = record.auth_tag {
        command.env("BUZZ_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("BUZZ_AUTH_TAG");
    }

    // Inbound author gate: who is this agent allowed to respond to?
    // Validation is strict here — a malformed allowlist on disk fails before
    // we spawn anything (the harness would also reject it, but we'd rather
    // fail with a clear error than crash-loop the child).
    let (gate_set, gate_remove) = build_respond_to_env(record, owner_hex)?;
    for (key, value) in &gate_set {
        command.env(key, value);
    }
    for key in &gate_remove {
        command.env_remove(key);
    }

    command.env("BUZZ_ACP_RELAY_OBSERVER", "true");

    // ── Git credential helper for Buzz relay ──────────────────────────
    //
    // Agents need to clone/push repos hosted on the Buzz relay's git
    // server, which authenticates via NIP-98. The `git-credential-nostr`
    // binary signs auth events using the agent's nostr key.
    //
    // We configure git via GIT_CONFIG_COUNT env vars (ephemeral, no
    // filesystem writes) scoped to the relay's git URL so we don't
    // interfere with other remotes (e.g. GitHub).
    //
    // NOSTR_PRIVATE_KEY mirrors BUZZ_PRIVATE_KEY — keep in sync.
    if let Some(cred_helper) = resolve_command("git-credential-nostr") {
        let relay_http_url = crate::relay::relay_http_base_url(&effective_relay_url);

        command.env("NOSTR_PRIVATE_KEY", &record.private_key_nsec);
        command.env("GIT_TERMINAL_PROMPT", "0");
        command.env("GIT_CONFIG_COUNT", "2");
        command.env(
            "GIT_CONFIG_KEY_0",
            format!("credential.{relay_http_url}/git.helper"),
        );
        let helper = cred_helper.to_string_lossy().replace('\\', "/");
        command.env("GIT_CONFIG_VALUE_0", helper);
        command.env(
            "GIT_CONFIG_KEY_1",
            format!("credential.{relay_http_url}/git.useHttpPath"),
        );
        command.env("GIT_CONFIG_VALUE_1", "true");
    } else {
        eprintln!(
            "buzz-desktop: git-credential-nostr not found — agent {} will not have automatic Buzz git auth",
            record.name,
        );
    }

    // ── User env vars: definition floor + global + live persona + agent overrides ──
    //
    // `descriptor.env` is the fully-layered result from `resolve_effective_harness_descriptor`:
    // baked floor → runtime metadata → definition env (harness author defaults) →
    // global → live persona → per-agent, with reserved-key and malformed-key filtering
    // applied. Runtime-owned ACP timeout, queue, and lock controls are reapplied
    // immediately below so persisted or ambient values cannot weaken them.
    for (key, value) in &descriptor.env {
        command.env(key, value);
    }
    configure_managed_acp_environment(
        app,
        &mut command,
        record,
        &runtime_key,
        &runtime_lock_path,
        launch_mode,
    )?;
    configure_runtime_cli(&mut command, runtime_meta);

    // Buzz shared compute is stored as a native provider; derive the OpenAI-compatible
    // transport at spawn time and scrub any unrelated ambient OpenAI key.
    // Gate on `mesh_model_id` (derived from `effective_cfg.relay_mesh_model_id()`
    // above) — not on `effective_provider` directly — so the mesh gate here
    // uses the same trim semantics as the preflight callers.
    #[cfg(feature = "mesh-llm")]
    if let Some(ref mesh_model_id) = mesh_model_id {
        let mesh_env = super::relay_mesh_process_env(&descriptor.env, mesh_model_id);
        command.env_remove("OPENAI_API_KEY");
        for (key, value) in mesh_env {
            command.env(key, value);
        }
    }

    // Stamp a non-authoritative diagnostic origin plus the observer-frame nonce.
    let start_nonce = uuid::Uuid::new_v4().simple().to_string();
    command
        .env("BUZZ_MANAGED_AGENT", current_instance_id(app))
        .env("BUZZ_MANAGED_AGENT_START_NONCE", &start_nonce);

    // Stamp the effective spawn config from the values that populated the
    // `Command` above, BEFORE spawning. Re-resolving after `spawn()` would let
    // a persona/harness/global edit landing in between stamp the NEW config
    // onto a child running the OLD one, silently suppressing the badge.
    let spawn_config = super::spawn_snapshot::SpawnConfigSnapshot::from_inputs(
        super::spawn_snapshot::SpawnConfigInputs {
            record,
            descriptor: &descriptor,
            relay_url: &effective_relay_url,
            team_instructions: team_instructions.as_deref(),
            system_prompt: effective_prompt.as_deref(),
            model: effective_model.as_deref(),
            provider: effective_provider.as_deref(),
        },
    );

    // Spawn the harness in its own process group so we can kill the entire
    // tree (harness + MCP servers + agent subprocesses) on shutdown.

    // A durable harness is a separate session/process-group leader. Desktop
    // may disconnect or exit without owning its lifetime.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and does not access parent memory.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    // Windows: suppress the harness console window. Without this a bare
    // terminal pops for buzz-acp.exe and lingers (the app itself sets
    // windows_subsystem="windows", but the spawned child does not inherit it).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }

    let child = command.spawn().map_err(|error| {
        format!(
            "failed to spawn `{}` for agent {}: {error}",
            resolved_acp_command.display(),
            record.name
        )
    })?;

    // Stamp the adapter availability for runtimes with a version gate (codex
    // only). The summary builder compares this against the current cached value
    // to detect out-of-band adapter changes after spawn (Phase-2 badge fallback).
    // Non-codex runtimes get `None` — nothing changes for them.
    // When the cache is cold (e.g. Doctor just installed and cleared the cache),
    // `adapter_availability_cached()` returns `None`, so the stamp is `None` and
    // the drift check is skipped until discovery warms the cache — preventing a
    // false restart badge immediately after auto-restart.
    let spawned_adapter_availability = if runtime_meta.is_some_and(|r| r.id == "codex") {
        super::adapter_availability_cached()
    } else {
        None
    };

    // Receipt persistence belongs to the caller's atomic register transition.

    // Windows: retain a non-killing Job Object only as connected-session tree
    // identity. Runtime lifetime remains independent of the Desktop handle.
    #[cfg(windows)]
    return Ok(super::process_lifecycle::finish_spawn(
        child,
        log_path,
        spawn_config,
        spawned_setup_mode,
        spawned_adapter_availability,
        start_nonce,
        &record.name,
    ));
    #[cfg(not(windows))]
    Ok(crate::managed_agents::ManagedAgentProcess {
        child,
        log_path,
        spawn_config,
        setup_mode: spawned_setup_mode,
        adapter_availability: spawned_adapter_availability,
        start_nonce,
    })
}

fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

pub fn start_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    owner_hex: Option<&str>,
) -> Result<(), String> {
    let relay_url = {
        use tauri::Manager;
        let state = app.state::<crate::app_state::AppState>();
        crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &crate::relay::relay_ws_url_with_override(&state),
        )
    };
    let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)?;
    if let Some(runtime) = runtimes.get(&key) {
        if runtime.is_legacy()
            && runtime.legacy_receipt.as_ref().is_some_and(|receipt| {
                buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker)
            })
        {
            return Ok(());
        }
        if runtime
            .controller
            .as_ref()
            .is_some_and(|controller| super::block_on_runtime_io(controller.status()).is_ok())
        {
            return Ok(());
        }
        runtimes.remove(&key);
    }

    let preferred_launch_mode = managed_runtime_feature_gates().launch_mode();
    let receipt_path = super::managed_agent_runtime_receipt_path(app, &key)?;
    let had_v2_receipt = receipt_path.exists();
    if had_v2_receipt {
        if let Ok(runtime) =
            super::runtime_commands::connect_runtime_receipt(app, &key, None, false)
        {
            runtimes.insert(key, runtime);
            return Ok(());
        }
        if pair_lock_is_held(app, &key)? {
            return Err(
                "recovering: runtime pair lock is held but receipt is not adoptable".into(),
            );
        }
        if matches!(
            preferred_launch_mode,
            ManagedRuntimeLaunchMode::LegacyPhase0
        ) {
            return Err("durable runtime recovery required; refusing schema-v1 fallback".into());
        }
        super::quarantine_agent_runtime_receipt_path(&receipt_path)?;
    }

    let durable_store_exists = super::managed_agent_runtime_state_path(app, &key)?
        .join("runtime.sqlite3")
        .exists();
    let needs_phase_zero_decision = !matches!(
        preferred_launch_mode,
        ManagedRuntimeLaunchMode::LegacyPhase0
    ) && !had_v2_receipt
        && !durable_store_exists;
    let (proof_exists, migration_gate) = if needs_phase_zero_decision {
        (
            super::managed_agent_legacy_runtime_receipt_path(app, &key)?.exists(),
            legacy_migration_gate(app, &key, record.runtime_pid)?,
        )
    } else {
        (false, LegacyMigrationGate::Clear)
    };
    let launch_mode = match process::select_rollout_launch_mode(
        preferred_launch_mode,
        had_v2_receipt || durable_store_exists,
        proof_exists,
        migration_gate,
    ) {
        Ok(mode) => mode,
        Err(LegacyMigrationGate::LegacyRuntimeActive) => {
            return Err("legacy_runtime_active".into());
        }
        Err(LegacyMigrationGate::ManualLegacyStopRequired) => {
            return Err("manual_legacy_stop_required".into());
        }
        Err(LegacyMigrationGate::Clear) => unreachable!("clear migration gate is not blocking"),
    };
    if matches!(launch_mode, ManagedRuntimeLaunchMode::LegacyPhase0) && durable_store_exists {
        return Err("durable runtime state exists; refusing schema-v1 fallback".into());
    }

    let process = spawn_agent_child(app, record, &key.relay_url, false, owner_hex, launch_mode)?;
    let runtime = match launch_mode {
        ManagedRuntimeLaunchMode::LegacyPhase0 => {
            super::runtime_commands::connect_legacy_runtime_receipt(app, &key, process)?
        }
        ManagedRuntimeLaunchMode::DurableV2 { .. } => {
            super::runtime_commands::connect_runtime_receipt(app, &key, Some(process), true)?
        }
    };
    record.runtime_pid = runtime.is_legacy().then(|| runtime.pid());
    let now = now_iso();
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;
    record.last_error_code = None;
    runtimes.insert(key, runtime);
    Ok(())
}

#[cfg(test)]
mod tests;
