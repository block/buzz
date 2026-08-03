use super::*;

#[cfg(unix)]
pub(crate) fn process_is_running(pid: u32) -> bool {
    // Use libc::kill with signal 0 instead of forking a subprocess.
    // Returns true only if the process exists AND we can signal it.
    // Returns false for non-existent PIDs (ESRCH) and PIDs owned by
    // other users (EPERM) — callers should not interact with those.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub(crate) fn process_is_running(_pid: u32) -> bool {
    false
}

/// Diagnostic origin stamped into managed harnesses. It is intentionally not
/// used for liveness, adoption, or termination; authenticated schema-v2
/// control and pair-lock proof are the only lifecycle authorities.
pub(crate) fn current_instance_id(app: &AppHandle) -> String {
    app.config().identifier.clone()
}

#[cfg(unix)]
fn signal_process_group_or_leader(pid: u32, signal: i32, action: &str) -> Result<(), String> {
    let pgid = -(pid as i32);

    if unsafe { libc::kill(pgid, signal) } == 0 {
        return Ok(());
    }

    let group_err = std::io::Error::last_os_error();
    if !process_is_running(pid) {
        return Ok(());
    }

    // Some local agent trees can no longer be signalled as a process group
    // (for example if the leader changed groups, or macOS returns EPERM for one
    // descendant). Fall back to the leader PID so stop/delete can still recover.
    if matches!(
        group_err.raw_os_error(),
        Some(libc::EPERM) | Some(libc::ESRCH)
    ) {
        if unsafe { libc::kill(pid as i32, signal) } == 0 {
            return Ok(());
        }

        let leader_err = std::io::Error::last_os_error();
        if leader_err.raw_os_error() == Some(libc::ESRCH) || !process_is_running(pid) {
            return Ok(());
        }

        return Err(format!("failed to {action} process {pid}: {leader_err}"));
    }

    Err(format!(
        "failed to {action} process group {pid}: {group_err}"
    ))
}

#[cfg(unix)]
pub(crate) fn terminate_process(pid: u32) -> Result<(), String> {
    // Try graceful shutdown first (SIGTERM to the group).
    signal_process_group_or_leader(pid, libc::SIGTERM, "terminate")?;

    // Wait up to 1s for graceful exit.
    for _ in 0..10 {
        if !process_is_running(pid) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Escalate to SIGKILL on the entire group.
    signal_process_group_or_leader(pid, libc::SIGKILL, "kill")?;

    Ok(())
}

#[cfg(windows)]
pub(crate) fn terminate_process(pid: u32) -> Result<(), String> {
    // No job handle is available on this path (e.g. after an app restart, when
    // we only recovered the PID from the record), so fall back to taskkill on
    // the whole tree.
    super::super::process_lifecycle::taskkill_tree(pid)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn terminate_process(_pid: u32) -> Result<(), String> {
    Err("managed agent shutdown after app restart is not supported on this platform".to_string())
}

pub(crate) fn adopt_schema_v2_runtime(
    receipt_path: &std::path::Path,
    expected_key: &ManagedAgentRuntimeKey,
) -> Result<
    (
        buzz_runtime_pkg::protocol::RuntimeReceipt,
        buzz_runtime_pkg::client::RuntimeClient,
        buzz_runtime_pkg::protocol::RuntimeStatus,
    ),
    String,
> {
    let receipt = buzz_runtime_pkg::artifacts::read_runtime_receipt(receipt_path)
        .map_err(|error| format!("invalid runtime receipt: {error}"))?;
    let canonical_key =
        ManagedAgentRuntimeKey::new(receipt.key.pubkey.clone(), &receipt.key.relay_url)?;
    if canonical_key != *expected_key
        || receipt.key.pubkey != expected_key.pubkey
        || receipt.key.relay_url != expected_key.relay_url
        || receipt.runtime_id != expected_key.runtime_id()
    {
        return Err("runtime receipt identity does not match the requested pair".into());
    }
    if receipt.lock_protocol_version != super::super::RUNTIME_LOCK_PROTOCOL_VERSION
        || receipt.lock_path_hash.is_empty()
    {
        return Err("runtime receipt is missing pair-lock proof".into());
    }
    let observed_marker = buzz_runtime_pkg::artifacts::process_start_marker(receipt.pid)
        .map_err(|error| format!("cannot verify runtime process identity: {error}"))?;
    if observed_marker != receipt.process_start_marker {
        return Err("runtime process start marker does not match receipt".into());
    }
    let controller = super::super::block_on_runtime_io(
        buzz_runtime_pkg::client::RuntimeClient::from_validated_receipt(
            &receipt,
            buzz_runtime_pkg::protocol::Capability::Controller,
        ),
    )
    .map_err(|error| format!("runtime hello authentication failed: {error}"))?;
    let status = super::super::block_on_runtime_io(controller.status())
        .map_err(|error| format!("runtime status authentication failed: {error}"))?;
    if status.runtime_id != receipt.runtime_id || status.generation != receipt.generation {
        return Err("runtime status does not match receipt generation".into());
    }
    Ok((receipt, controller, status))
}

pub(crate) fn verify_runtime_lock_proof(
    receipt: &buzz_runtime_pkg::protocol::RuntimeReceipt,
    lock_path: &std::path::Path,
) -> Result<(), String> {
    if receipt.lock_protocol_version != super::super::RUNTIME_LOCK_PROTOCOL_VERSION
        || receipt.lock_path_hash != super::super::runtime_lock_path_hash(lock_path)
    {
        return Err("runtime receipt pair-lock proof does not match".into());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyMigrationGate {
    Clear,
    LegacyRuntimeActive,
    ManualLegacyStopRequired,
}

pub(super) fn classify_legacy_migration(
    has_lock_proof: bool,
    pair_lock_held: bool,
    receipt_process_is_live: bool,
) -> LegacyMigrationGate {
    if !has_lock_proof || (receipt_process_is_live && !pair_lock_held) {
        LegacyMigrationGate::ManualLegacyStopRequired
    } else if pair_lock_held {
        LegacyMigrationGate::LegacyRuntimeActive
    } else {
        LegacyMigrationGate::Clear
    }
}

pub(crate) fn select_rollout_launch_mode(
    preferred: super::ManagedRuntimeLaunchMode,
    has_v2_artifacts: bool,
    proof_exists: bool,
    migration_gate: LegacyMigrationGate,
) -> Result<super::ManagedRuntimeLaunchMode, LegacyMigrationGate> {
    match preferred {
        super::ManagedRuntimeLaunchMode::LegacyPhase0 => Ok(preferred),
        _ if has_v2_artifacts => Ok(preferred),
        _ => match migration_gate {
            LegacyMigrationGate::Clear if proof_exists => Ok(preferred),
            LegacyMigrationGate::Clear => Ok(super::ManagedRuntimeLaunchMode::LegacyPhase0),
            blocked => Err(blocked),
        },
    }
}

pub(crate) fn pair_lock_is_held(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
) -> Result<bool, String> {
    use fs2::FileExt as _;
    use std::fs::OpenOptions;

    let lock_path = super::super::managed_agent_runtime_lock_path(app, key)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|error| format!("failed to open pair lock {}: {error}", lock_path.display()))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            let _ = lock.unlock();
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(format!(
            "failed to probe pair lock {}: {error}",
            lock_path.display()
        )),
    }
}

pub(super) fn legacy_receipt_has_lock_proof(
    receipt: &super::super::LegacyManagedAgentRuntimeReceipt,
    key: &ManagedAgentRuntimeKey,
    lock_path: &std::path::Path,
) -> bool {
    let canonical_receipt_key =
        ManagedAgentRuntimeKey::new(receipt.key.pubkey.clone(), &receipt.key.relay_url);
    canonical_receipt_key.as_ref().ok() == Some(key)
        && receipt.schema_version == buzz_runtime_pkg::LEGACY_RUNTIME_RECEIPT_SCHEMA_VERSION
        && receipt.lock_protocol_version == super::super::RUNTIME_LOCK_PROTOCOL_VERSION
        && receipt.lock_path_hash == super::super::runtime_lock_path_hash(lock_path)
        && !receipt.process_start_marker.is_empty()
}

pub(super) fn classify_missing_legacy_receipt(
    legacy_runtime_pid: Option<u32>,
    pair_lock_held: bool,
) -> LegacyMigrationGate {
    if legacy_runtime_pid.is_some() || pair_lock_held {
        LegacyMigrationGate::ManualLegacyStopRequired
    } else {
        LegacyMigrationGate::Clear
    }
}

pub(crate) fn legacy_migration_gate(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    legacy_runtime_pid: Option<u32>,
) -> Result<LegacyMigrationGate, String> {
    use fs2::FileExt as _;
    use std::fs::OpenOptions;
    let receipt_path = super::super::managed_agent_legacy_runtime_receipt_path(app, key)?;

    if !receipt_path.exists() {
        return Ok(classify_missing_legacy_receipt(
            legacy_runtime_pid,
            pair_lock_is_held(app, key)?,
        ));
    }
    let lock_path = super::super::managed_agent_runtime_lock_path(app, key)?;
    let receipt = match std::fs::read(&receipt_path).ok().and_then(|bytes| {
        serde_json::from_slice::<super::super::LegacyManagedAgentRuntimeReceipt>(&bytes).ok()
    }) {
        Some(receipt) => receipt,
        None => return Ok(LegacyMigrationGate::ManualLegacyStopRequired),
    };
    let has_lock_proof = legacy_receipt_has_lock_proof(&receipt, key, &lock_path);
    if !has_lock_proof {
        return Ok(classify_legacy_migration(false, false, false));
    }

    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|error| format!("failed to open pair lock {}: {error}", lock_path.display()))?;
    let pair_lock_held = match lock.try_lock_exclusive() {
        Ok(()) => {
            let _ = lock.unlock();
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(error) => {
            return Err(format!(
                "failed to probe pair lock {}: {error}",
                lock_path.display()
            ));
        }
    };
    let receipt_process_is_live =
        buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker);
    let gate = classify_legacy_migration(true, pair_lock_held, receipt_process_is_live);
    if gate == LegacyMigrationGate::Clear {
        super::super::quarantine_agent_runtime_receipt_path(&receipt_path)?;
    }
    Ok(gate)
}

/// Terminate a detached schema-v1 runtime only when its durable receipt, pair
/// lock, and live process identity all agree. Returns `false` when proof is
/// absent or ambiguous so the caller can fail closed.
pub(crate) fn stop_verified_legacy_runtime(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    legacy_runtime_pid: Option<u32>,
) -> Result<bool, String> {
    let receipt_path = super::super::managed_agent_legacy_runtime_receipt_path(app, key)?;
    let lock_path = super::super::managed_agent_runtime_lock_path(app, key)?;
    let Some(receipt) = std::fs::read(&receipt_path).ok().and_then(|bytes| {
        serde_json::from_slice::<super::super::LegacyManagedAgentRuntimeReceipt>(&bytes).ok()
    }) else {
        return Ok(false);
    };
    if !legacy_receipt_has_lock_proof(&receipt, key, &lock_path)
        || legacy_runtime_pid.is_some_and(|pid| pid != receipt.pid)
        || !pair_lock_is_held(app, key)?
        || !buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker)
    {
        return Ok(false);
    }

    super::terminate_process(receipt.pid)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let same_process =
            buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker);
        if !same_process && !pair_lock_is_held(app, key)? {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker) {
        return Err("verified legacy runtime survived explicit stop".into());
    }
    if pair_lock_is_held(app, key)? {
        return Err("legacy runtime pair lock remained held after explicit stop".into());
    }
    Ok(true)
}
