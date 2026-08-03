use tauri::AppHandle;

use crate::managed_agents::{
    managed_agent_legacy_runtime_receipt_path, managed_agent_runtime_receipt_path,
    managed_agent_runtime_state_dir, resolve_command,
    types::{validate_respond_to_allowlist, RespondTo},
    KnownAcpRuntime, ManagedAgentRecord, ManagedAgentRuntimeKey,
};

use super::should_skip_claude_executable;

type RespondToEnv = (Vec<(&'static str, String)>, Vec<&'static str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ManagedRuntimeFeatureGates {
    pub durable_runtime: bool,
    pub job_event_publication: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedRuntimeLaunchMode {
    LegacyPhase0,
    DurableV2 { job_event_publication: bool },
}
impl ManagedRuntimeFeatureGates {
    pub(crate) fn from_values(
        durable_runtime: Option<&str>,
        job_event_publication: Option<&str>,
    ) -> Self {
        let enabled = |value: &str| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        };
        Self {
            // The clean cutover is default-on. Explicit false remains the
            // operator rollback/test gate for each independent capability.
            durable_runtime: durable_runtime.map(enabled).unwrap_or(true),
            job_event_publication: job_event_publication.map(enabled).unwrap_or(true),
        }
    }

    pub(crate) fn launch_mode(self) -> ManagedRuntimeLaunchMode {
        if self.durable_runtime {
            ManagedRuntimeLaunchMode::DurableV2 {
                job_event_publication: self.job_event_publication,
            }
        } else {
            ManagedRuntimeLaunchMode::LegacyPhase0
        }
    }
}

pub(crate) fn managed_runtime_feature_gates() -> ManagedRuntimeFeatureGates {
    ManagedRuntimeFeatureGates::from_values(
        std::env::var("BUZZ_ACP_DURABLE_RUNTIME").ok().as_deref(),
        std::env::var("BUZZ_ACP_JOB_EVENT_PUBLICATION")
            .ok()
            .as_deref(),
    )
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
    let normalized = validate_respond_to_allowlist(&record.respond_to_allowlist)?;
    if record.respond_to == RespondTo::Allowlist && normalized.is_empty() {
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

    if record.respond_to == RespondTo::Allowlist {
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

const DEFAULT_ACP_TURN_IDLE_SECONDS: u64 = 900;
const DEFAULT_ACP_MAX_TURN_DURATION_SECONDS: u64 = 7_200;

pub(crate) fn effective_acp_turn_limits(record: &ManagedAgentRecord) -> (u64, u64) {
    (
        record
            .idle_timeout_seconds
            .unwrap_or(DEFAULT_ACP_TURN_IDLE_SECONDS),
        record
            .max_turn_duration_seconds
            .unwrap_or(DEFAULT_ACP_MAX_TURN_DURATION_SECONDS),
    )
}

pub(crate) fn acp_turn_limits_log_line(record: &ManagedAgentRecord) -> String {
    let (idle, max_duration) = effective_acp_turn_limits(record);
    format!(
        "ACP turn limits: idle {idle}s, maximum duration {max_duration}s. \
         These limits apply only to ACP turns and do not bound managed runtime or job-runner lifetime."
    )
}

fn canonical_operator_lh_command(path: Option<std::path::PathBuf>) -> std::ffi::OsString {
    path.and_then(|path| std::fs::canonicalize(path).ok())
        .map(std::path::PathBuf::into_os_string)
        .unwrap_or_default()
}

fn canonical_operator_workspace_roots(raw: Option<&std::ffi::OsStr>) -> std::ffi::OsString {
    let Some(raw) = raw else {
        return std::ffi::OsString::new();
    };
    let mut roots = Vec::new();
    for root in std::env::split_paths(raw) {
        let Ok(canonical) = std::fs::canonicalize(&root) else {
            return std::ffi::OsString::new();
        };
        if !canonical.is_dir() {
            return std::ffi::OsString::new();
        }
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }
    if roots.is_empty() {
        return std::ffi::OsString::new();
    }
    std::env::join_paths(roots).unwrap_or_default()
}

pub(super) fn configure_managed_job_environment(
    command: &mut std::process::Command,
    lh_command: Option<std::path::PathBuf>,
    raw_workspace_roots: Option<&std::ffi::OsStr>,
) {
    // Empty values keep the conversational runtime available while making
    // privileged job starts fail closed in the runtime.
    command.env(
        "BUZZ_ACP_LH_COMMAND",
        canonical_operator_lh_command(lh_command),
    );
    command.env(
        "BUZZ_ACP_JOB_WORKSPACE_ROOTS",
        canonical_operator_workspace_roots(raw_workspace_roots),
    );
}

pub(crate) fn configure_managed_acp_turn_environment(
    command: &mut std::process::Command,
    record: &ManagedAgentRecord,
    runtime_lock_path: &std::path::Path,
) {
    command.env_remove("BUZZ_ACP_TURN_TIMEOUT");
    match record.idle_timeout_seconds {
        Some(idle) => {
            command.env("BUZZ_ACP_IDLE_TIMEOUT", idle.to_string());
        }
        None => {
            command.env_remove("BUZZ_ACP_IDLE_TIMEOUT");
        }
    }
    match record.max_turn_duration_seconds {
        Some(max_duration) => {
            command.env("BUZZ_ACP_MAX_TURN_DURATION", max_duration.to_string());
        }
        None => {
            command.env_remove("BUZZ_ACP_MAX_TURN_DURATION");
        }
    }
    command.env("BUZZ_ACP_AGENTS", record.parallelism.to_string());
    command.env("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer");
    command.env("BUZZ_ACP_DEDUP", "queue");
    command.env("BUZZ_ACP_RUNTIME_LOCK_PATH", runtime_lock_path);
}

pub(crate) fn configure_rollout_gate_environment(
    command: &mut std::process::Command,
    launch_mode: ManagedRuntimeLaunchMode,
) {
    let (durable_runtime, job_event_publication) = match launch_mode {
        ManagedRuntimeLaunchMode::LegacyPhase0 => ("false", "false"),
        ManagedRuntimeLaunchMode::DurableV2 {
            job_event_publication,
        } => (
            "true",
            if job_event_publication {
                "true"
            } else {
                "false"
            },
        ),
    };
    command.env("BUZZ_ACP_DURABLE_RUNTIME", durable_runtime);
    command.env("BUZZ_ACP_JOB_EVENT_PUBLICATION", job_event_publication);
}

pub(crate) fn configure_managed_acp_environment(
    app: &AppHandle,
    command: &mut std::process::Command,
    record: &ManagedAgentRecord,
    runtime_key: &ManagedAgentRuntimeKey,
    runtime_lock_path: &std::path::Path,
    launch_mode: ManagedRuntimeLaunchMode,
) -> Result<(), String> {
    // Deprecated scalar timeouts and all Desktop-owned runtime configuration
    // must never leak from the parent environment or a persisted descriptor.
    for key in crate::managed_agents::env_vars::RESERVED_ENV_KEYS {
        if key.starts_with("BUZZ_ACP_RUNTIME")
            || matches!(
                *key,
                "BUZZ_RUNTIME_RECEIPT"
                    | "BUZZ_ACP_LH_COMMAND"
                    | "BUZZ_ACP_JOB_WORKSPACE_ROOTS"
                    | "BUZZ_ACP_DURABLE_RUNTIME"
                    | "BUZZ_ACP_JOB_EVENT_PUBLICATION"
                    | "BUZZ_ACP_LEGACY_RUNTIME_RECEIPT"
            )
        {
            command.env_remove(key);
        }
    }
    configure_managed_acp_turn_environment(command, record, runtime_lock_path);
    configure_rollout_gate_environment(command, launch_mode);
    match launch_mode {
        ManagedRuntimeLaunchMode::LegacyPhase0 => {
            let receipt_path = managed_agent_legacy_runtime_receipt_path(app, runtime_key)?;
            // Publication stays disabled in Phase-0 even if its independent
            // operator gate was enabled before durability.
            command.env("BUZZ_ACP_LEGACY_RUNTIME_RECEIPT", receipt_path);
        }
        ManagedRuntimeLaunchMode::DurableV2 { .. } => {
            let raw_workspace_roots = std::env::var_os("BUZZ_ACP_JOB_WORKSPACE_ROOTS");
            configure_managed_job_environment(
                command,
                resolve_command("lh"),
                raw_workspace_roots.as_deref(),
            );
            let state_dir = managed_agent_runtime_state_dir(app, runtime_key)?;
            let receipt_path = managed_agent_runtime_receipt_path(app, runtime_key)?;
            // Gate values were applied above; durable paths are injected only
            // after the caller passed the schema-v1 migration proof check.
            command.env("BUZZ_ACP_RUNTIME_ID", runtime_key.runtime_id());
            command.env("BUZZ_ACP_RUNTIME_STATE_DIR", state_dir);
            command.env("BUZZ_RUNTIME_RECEIPT", receipt_path);
        }
    }
    Ok(())
}
