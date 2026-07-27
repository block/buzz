use std::collections::HashMap;

use tauri::AppHandle;

use super::agent_env::build_buzz_agent_provider_defaults;

use crate::{
    managed_agents::{
        append_log_marker, known_acp_runtime, login_shell_path, managed_agent_log_path,
        missing_command_message, normalize_agent_args, open_log_file, resolve_command,
        spawn_key_refusal, KnownAcpRuntime, ManagedAgentPairRuntime, ManagedAgentRecord,
        ManagedAgentRuntimeKey, ManagedAgentSummary,
    },
    util::now_iso,
};

mod path;
pub(in crate::managed_agents) use path::build_augmented_path;
pub(crate) use path::compose_path_entries;
pub(crate) use path::should_skip_claude_executable;
pub(crate) use path::should_use_inherited;

mod metadata;
pub(crate) use metadata::{resolve_effective_prompt_model_provider, runtime_metadata_env_vars};

mod stop;
pub(crate) use stop::managed_agent_runtime_keys;
pub use stop::{stop_managed_agent_process, stop_managed_agent_workspace_pair};

mod sweep;
pub(crate) use sweep::sweep_untracked_bundle_harnesses;

type RespondToEnv = (Vec<(&'static str, String)>, Vec<&'static str>);

mod process;
#[cfg(test)]
use process::{
    buzz_marker_entry, name_matches_interpreter, name_matches_known_binary,
    terminate_runtime_receipt_with, valid_agent_runtime_receipt_with,
};
pub(crate) use process::{
    current_instance_id, process_belongs_to_us, process_has_buzz_marker, process_is_running,
    terminate_process, terminate_untracked_pair_runtime, valid_agent_runtime_receipt,
};

mod orphan_sweep;
#[cfg(target_os = "macos")]
use orphan_sweep::proc_pidinfo;
pub(crate) use orphan_sweep::{
    sweep_orphaned_agent_processes, sweep_system_agent_processes,
    sweep_system_agent_processes_with_grace,
};
#[cfg(target_os = "macos")]
use orphan_sweep::{BSDInfo, PROC_PIDTBSDINFO};
#[cfg(unix)]
use process::resolve_pgids_and_kill;

mod instance_reaper;
pub(crate) use instance_reaper::reap_dead_instance_agents;
#[cfg(test)]
use instance_reaper::{buffer_contains_identifier, is_desktop_binary};

// Exact-path harness sweep lives in runtime/sweep.rs (re-exported above).

mod lifecycle;
#[cfg(test)]
use lifecycle::kill_stale_tracked_processes_with;
pub use lifecycle::{kill_stale_tracked_processes, sync_managed_agent_processes};

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
        let persisted_pid = record.runtime_pid.filter(|pid| process_is_running(*pid));
        if let Some(runtime) = pair_runtime {
            (
                "running".to_string(),
                Some(runtime.child.id()),
                runtime.log_path.display().to_string(),
            )
        } else if let Some(pid) = persisted_pid {
            (
                "running".to_string(),
                Some(pid),
                managed_agent_log_path(app, &record.pubkey)?
                    .display()
                    .to_string(),
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

    // Restart badge: the running process stamped its effective spawn config
    // at launch; recompute from current disk state and flag drift. Only the
    // tracked live pair for THIS workspace can drift — stopped agents spawn
    // fresh, adopted (runtime_pid-only) processes have no stamped hash to
    // compare, and pairs running for other communities are judged in their
    // own community (hashing them against this workspace's relay would flag
    // a spurious restart on every community switch).
    //
    // Additionally, for runtimes with an adapter version gate (codex only),
    // check whether the cached adapter availability has drifted from the value
    // stamped at spawn.  This catches out-of-band adapter changes (manual
    // npm install/downgrade) that Phase-1 auto-restart doesn't cover.  The
    // cache is read-only here — no subprocess is spawned.
    //
    // Global config drives both the restart-drift hash and descriptor env
    // layering below — the caller loads it once and passes it in, so
    // list-style callers pay one disk read per call rather than one per record.

    let needs_restart = pair_key
        .as_ref()
        .and_then(|key| runtimes.get(key).map(|runtime| (key, runtime)))
        .is_some_and(|(key, runtime)| {
            let teams_for_hash = crate::managed_agents::load_teams(app).unwrap_or_default();
            let hash_drift = runtime.spawn_config_hash
                != crate::managed_agents::spawn_hash::spawn_config_hash(
                    record,
                    personas,
                    &teams_for_hash,
                    &key.relay_url,
                    global_config,
                );
            let availability_drift = super::availability_drift(
                runtime.adapter_availability.as_ref(),
                super::adapter_availability_cached(),
            );
            // An orphan can never be restarted successfully —
            // `spawn_agent_child` refuses it before any process side effect —
            // so `needs_restart` must never fire for one regardless of hash or
            // availability drift. Surfacing "Restart required" here would offer
            // an action guaranteed to fail; the UI shows `persona_orphaned`
            // instead (see `ManagedAgentSummary::persona_orphaned`).
            restart_eligible(persona_orphaned, hash_drift, availability_drift)
        });

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
/// An orphaned linked instance (its persona/definition no longer exists)
/// can never be restarted successfully — `spawn_agent_child` refuses to
/// spawn it before any process side effect. Surfacing "Restart required"
/// for one would offer an action guaranteed to fail, so this always
/// returns `false` for an orphan regardless of drift. Extracted for unit
/// testing without `AppHandle`/global state, following the
/// `availability_drift` pattern in `discovery.rs`.
fn restart_eligible(persona_orphaned: bool, hash_drift: bool, availability_drift: bool) -> bool {
    !persona_orphaned && (hash_drift || availability_drift)
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

/// Pure decision function for the inbound author gate env vars.
///
/// Returns the env vars to **set** and the env vars to **remove**. Removal is
/// belt-and-suspenders: an inherited parent env var must not leak into a
/// child agent and silently change its security posture.
///
/// The `owner_hex` argument is the current workspace owner pubkey. It's used
/// as a fallback for legacy records (`auth_tag.is_none()`) — without it, the
/// harness's owner cache stays empty and `owner-only` / `allowlist` modes
/// drop everything.
///
/// Returns `Err(...)` if the record's allowlist fails validation. The harness
/// validates too, but doing it here means we never spawn a doomed process.
pub(crate) fn build_respond_to_env(
    record: &ManagedAgentRecord,
    owner_hex: Option<&str>,
) -> Result<RespondToEnv, String> {
    // Defensive re-validation: an on-disk record could have been hand-edited.
    let normalized = super::types::validate_respond_to_allowlist(&record.respond_to_allowlist)?;
    if record.respond_to == super::types::RespondTo::Allowlist && normalized.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }

    let mut set: Vec<(&'static str, String)> = Vec::new();
    let mut remove: Vec<&'static str> = Vec::new();

    set.push((
        "BUZZ_ACP_RESPOND_TO",
        record.respond_to.as_str().to_string(),
    ));

    if record.respond_to == super::types::RespondTo::Allowlist {
        set.push(("BUZZ_ACP_RESPOND_TO_ALLOWLIST", normalized.join(",")));
    } else {
        remove.push("BUZZ_ACP_RESPOND_TO_ALLOWLIST");
    }

    // Legacy fallback: agents created before NIP-OA lack `auth_tag`. Without
    // it the harness can't resolve the owner, and owner-dependent gate modes
    // would drop every event. Forwarding the workspace owner pubkey via
    // BUZZ_ACP_AGENT_OWNER keeps those records functional. Modern records
    // (`auth_tag = Some(...)`) use `BUZZ_AUTH_TAG` as before.
    if record.auth_tag.is_none() {
        if let Some(owner) = owner_hex {
            set.push(("BUZZ_ACP_AGENT_OWNER", owner.to_string()));
        } else {
            remove.push("BUZZ_ACP_AGENT_OWNER");
        }
    } else {
        remove.push("BUZZ_ACP_AGENT_OWNER");
    }

    Ok((set, remove))
}

pub(crate) fn configure_runtime_cli(
    command: &mut std::process::Command,
    runtime: Option<&KnownAcpRuntime>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.id != "claude" {
        return;
    }
    if let Some(cli_path) = runtime.underlying_cli.and_then(resolve_command) {
        // On Windows, `.cmd` and `.bat` files are batch shims — they cannot be
        // passed directly to `CreateProcess` and cause EINVAL when the Claude
        // adapter tries to spawn them (issue #2397). Skip setting
        // `CLAUDE_CODE_EXECUTABLE` for shim paths so the adapter falls back to
        // its own PATH lookup and finds the real binary instead.
        // Non-Windows: `.cmd`/`.bat` are valid executables and must be assigned.
        if should_skip_claude_executable(&cli_path, cfg!(windows)) {
            return;
        }
        command.env("CLAUDE_CODE_EXECUTABLE", cli_path);
    }
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
) -> Result<crate::managed_agents::ManagedAgentProcess, String> {
    if let Some(error) = spawn_key_refusal(record) {
        return Err(error);
    }
    let runtime_key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), relay_url)?;
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
    // the single source both the env writes below and `spawn_config_hash`
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
    // This is the sole path for harness-definition lookup — spawn, hash, summary, and
    // model probes all consume this descriptor rather than assembling values inline.
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

    let stdout = open_log_file(&log_path)?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone log handle: {error}"))?;
    let resolved_acp_command = resolve_command(&record.acp_command)
        .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;
    let effective_mcp_command = known_acp_runtime(effective_command)
        .and_then(|r| r.mcp_command)
        .unwrap_or("");
    let resolved_mcp_command: Option<std::path::PathBuf> = if effective_mcp_command.is_empty() {
        None
    } else {
        match resolve_command(effective_mcp_command) {
            Some(path) => Some(path),
            None => {
                eprintln!(
                    "buzz-desktop: mcp_command {effective_mcp_command:?} not found, skipping"
                );
                None
            }
        }
    };
    // Resolve agent command to a full path (DMG launches have minimal PATH).
    let resolved_agent_command = resolve_command(effective_command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| effective_command.clone());

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
    // Only emit BUZZ_ACP_IDLE_TIMEOUT when the user has explicitly set an
    // override. When unset, the buzz-acp harness applies its own default
    // (see `DEFAULT_IDLE_TIMEOUT_SECS` in crates/buzz-acp/src/config.rs),
    // which is the single source of truth. The previously-emitted
    // `BUZZ_ACP_TURN_TIMEOUT` is deprecated upstream and was pinning every
    // agent to the desktop's stale default (320s), bypassing harness bumps.
    if let Some(idle) = record.idle_timeout_seconds {
        command.env("BUZZ_ACP_IDLE_TIMEOUT", idle.to_string());
    }

    if let Some(max_dur) = record.max_turn_duration_seconds {
        command.env("BUZZ_ACP_MAX_TURN_DURATION", max_dur.to_string());
    }
    command.env("BUZZ_ACP_AGENTS", record.parallelism.to_string());
    command.env("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer");
    command.env("BUZZ_ACP_DEDUP", "queue");
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            if std::env::var(key).is_err() {
                command.env(key, value);
            }
        }
    }
    let team_instructions = super::spawn_hash::effective_team_instructions(record, &teams);
    if let Some(instructions) = &team_instructions {
        command.env("BUZZ_ACP_TEAM_INSTRUCTIONS", instructions);
    } else {
        command.env_remove("BUZZ_ACP_TEAM_INSTRUCTIONS");
    }

    // Prompt, model, and provider all come from the single `effective_cfg`
    // resolved at the top of this function — the SAME resolve `spawn_config_hash`
    // performs below, so env write and restart badge cannot disagree. Linked
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
        command.env("GIT_CONFIG_VALUE_0", cred_helper.display().to_string());
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
    // applied. Writing it last lets user-provided values win over every Buzz-set env
    // written above — reserved keys were already stripped from descriptor.env so they
    // cannot clobber BUZZ_PRIVATE_KEY, NOSTR_PRIVATE_KEY, etc.
    for (key, value) in &descriptor.env {
        command.env(key, value);
    }
    configure_runtime_cli(&mut command, runtime_meta);

    // Buzz shared compute is stored as a native provider; derive the OpenAI-compatible
    // transport at spawn time and scrub any unrelated ambient OpenAI key.
    // Gate on `mesh_model_id` (derived from `effective_cfg.relay_mesh_model_id()`
    // above) — not on `effective_provider` directly — so the mesh gate here
    // uses the same trim semantics as the preflight callers.
    #[cfg(feature = "mesh-llm")]
    if let Some(ref mesh_model_id) = mesh_model_id {
        let mut mesh_env = std::collections::BTreeMap::new();
        super::apply_relay_mesh_env(
            &mut mesh_env,
            Some(super::RELAY_MESH_PROVIDER_ID),
            Some(mesh_model_id.as_str()),
        );
        command.env_remove("OPENAI_API_KEY");
        for (key, value) in mesh_env {
            command.env(key, value);
        }
    }

    // Stamp desktop ownership and an unpredictable harness-generation identity.
    let start_nonce = uuid::Uuid::new_v4().simple().to_string();
    command
        .env("BUZZ_MANAGED_AGENT", current_instance_id(app))
        .env("BUZZ_MANAGED_AGENT_START_NONCE", &start_nonce);

    // Spawn the harness in its own process group so we can kill the entire
    // tree (harness + MCP servers + agent subprocesses) on shutdown.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // Windows: suppress the harness console window. Without this a bare
    // terminal pops for buzz-acp.exe and lingers (the app itself sets
    // windows_subsystem="windows", but the spawned child does not inherit it).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command.spawn().map_err(|error| {
        format!(
            "failed to spawn `{}` for agent {}: {error}",
            resolved_acp_command.display(),
            record.name
        )
    })?;

    // Stamp the effective spawn config so the summary builder can flag
    // needs_restart when disk state drifts from what this process runs.
    // `effective_relay_url` is already resolved, and resolution is idempotent,
    // so it serves as the workspace-relay input here.
    let spawn_config_hash = super::spawn_hash::spawn_config_hash(
        record,
        &personas,
        &teams,
        &effective_relay_url,
        &global,
    );

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

    // Windows: assign the harness to a Job Object so its whole tree dies with
    // the handle. The Unix process-group equivalent is set above.
    #[cfg(windows)]
    return Ok(super::process_lifecycle::finish_spawn(
        child,
        log_path,
        spawn_config_hash,
        spawned_setup_mode,
        spawned_adapter_availability,
        start_nonce,
        &record.name,
    ));
    #[cfg(not(windows))]
    Ok(crate::managed_agents::ManagedAgentProcess {
        child,
        log_path,
        spawn_config_hash,
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
    if let Some(runtime) = runtimes.get_mut(&key) {
        if runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect running process: {error}"))?
            .is_none()
        {
            return Ok(());
        }

        runtimes.remove(&key);
        super::remove_agent_runtime_receipt(app, &key);
    }

    // Scalar PIDs are migration-only and never establish pair liveness.
    record.runtime_pid = None;

    let mut process = spawn_agent_child(app, record, &key.relay_url, false, owner_hex)?;
    let now = now_iso();
    let receipt = super::ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(app),
        started_at: now.clone(),
    };
    if let Err(error) = super::write_agent_runtime_receipt(app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }

    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_exit_code = None;
    record.last_error = None;
    record.last_error_code = None;

    runtimes.insert(key, ManagedAgentPairRuntime::starting(process));
    Ok(())
}

#[cfg(test)]
mod tests;
