use std::{fs, path::PathBuf};

use tauri::AppHandle;

use crate::managed_agents::{
    collect_same_instance_orphans_checked, collect_untracked_bundle_harnesses_checked,
    managed_agents_base_dir, process_has_buzz_marker, process_is_running, ManagedAgentRuntimeKey,
    ManagedAgentRuntimeReceipt,
};

fn runtime_receipt_entries(
    app: &AppHandle,
) -> Result<Vec<(PathBuf, ManagedAgentRuntimeReceipt)>, String> {
    let dir = managed_agents_base_dir(app)?.join("agent-pids");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut receipts = Vec::new();
    for entry in fs::read_dir(&dir)
        .map_err(|error| format!("failed to inspect managed runtime receipts: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("failed to inspect managed runtime receipt entry: {error}"))?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let metadata = path.symlink_metadata().map_err(|error| {
            format!(
                "failed to inspect runtime receipt {}: {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "ambiguous managed runtime receipt path {}",
                path.display()
            ));
        }
        let bytes = fs::read(&path).map_err(|error| {
            format!("failed to read runtime receipt {}: {error}", path.display())
        })?;
        let receipt = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "ambiguous managed runtime receipt {}: {error}",
                path.display()
            )
        })?;
        receipts.push((path, receipt));
    }
    Ok(receipts)
}

fn reconcile_agent_runtime_receipts_with(
    identity_pubkey: &str,
    desktop_instance_id: &str,
    receipts: Vec<(PathBuf, ManagedAgentRuntimeReceipt)>,
    is_running: impl Fn(u32) -> bool,
    has_marker: impl Fn(u32, &str) -> bool,
    mut remove: impl FnMut(&std::path::Path) -> Result<(), String>,
) -> Result<(), String> {
    for (path, receipt) in receipts {
        if !receipt.key.pubkey.eq_ignore_ascii_case(identity_pubkey) {
            continue;
        }
        if !is_running(receipt.pid) {
            remove(&path)?;
            continue;
        }

        let canonical = ManagedAgentRuntimeKey::new(
            receipt.key.pubkey.clone(),
            &receipt.key.relay_url,
        )
        .map_err(|_| {
            format!(
                "ambiguous live runtime receipt for agent {identity_pubkey}; stop it before preparing isolation"
            )
        })?;
        let filename_matches = path.file_name().and_then(|name| name.to_str())
            == Some(&format!("{}.json", canonical.runtime_id()));
        if canonical != receipt.key
            || !filename_matches
            || receipt.desktop_instance_id != desktop_instance_id
            || !has_marker(receipt.pid, desktop_instance_id)
        {
            return Err(format!(
                "ambiguous live runtime receipt for agent {identity_pubkey}; stop it before preparing isolation"
            ));
        }
        return Err(format!(
            "agent {identity_pubkey} still has live harness {} on {}; stop it before preparing isolation",
            receipt.pid, receipt.key.relay_url
        ));
    }
    Ok(())
}

/// Prove that Prepare starts from a process-free boundary. The caller holds
/// both the runtime transition and managed-agent store locks, so receipt
/// reconciliation, the process snapshot, and run-root creation cannot race a
/// managed start in this Desktop process.
pub(crate) fn ensure_no_runtime_before_isolation_prepare(
    app: &AppHandle,
    identity_pubkey: &str,
    desktop_instance_id: &str,
    tracked_pids: &[u32],
) -> Result<(), String> {
    reconcile_agent_runtime_receipts_with(
        identity_pubkey,
        desktop_instance_id,
        runtime_receipt_entries(app)?,
        process_is_running,
        process_has_buzz_marker,
        |path| {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "failed to remove dead runtime receipt {}: {error}",
                    path.display()
                )
            })
        },
    )?;

    let mut ambiguous = collect_same_instance_orphans_checked(desktop_instance_id, tracked_pids)?;
    ambiguous.extend(collect_untracked_bundle_harnesses_checked(tracked_pids)?);
    if !ambiguous.is_empty() {
        let mut pids: Vec<u32> = ambiguous.into_iter().collect();
        pids.sort_unstable();
        return Err(format!(
            "cannot prove the agent harness is stopped; untracked harness or same-instance process(es) {pids:?} remain"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(
        pubkey: &str,
        pid: u32,
        instance: &str,
    ) -> (tempfile::TempDir, PathBuf, ManagedAgentRuntimeReceipt) {
        let dir = tempfile::tempdir().unwrap();
        let key = ManagedAgentRuntimeKey::new(pubkey.to_string(), "wss://relay.example").unwrap();
        let path = dir.path().join(format!("{}.json", key.runtime_id()));
        let receipt = ManagedAgentRuntimeReceipt {
            key,
            pid,
            desktop_instance_id: instance.to_string(),
            started_at: "now".into(),
        };
        fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        (dir, path, receipt)
    }

    #[test]
    fn live_prior_runtime_blocks_prepare_before_any_isolation_run() {
        let identity = format!("{:064x}", std::process::id());
        let instance = "test-isolation-preflight";
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("while :; do sleep 1; done")
            .env("BUZZ_MANAGED_AGENT", instance)
            .spawn()
            .unwrap();
        let (_dir, path, receipt) = receipt(&identity, child.id(), instance);

        let profile = crate::managed_agents::FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let result = reconcile_agent_runtime_receipts_with(
            &identity,
            instance,
            vec![(path.clone(), receipt)],
            process_is_running,
            |_, _| true,
            |_| panic!("live receipt must not be removed"),
        )
        .and_then(|()| {
            crate::managed_agents::prepare_isolated_agent_run(
                &profile,
                &identity,
                instance,
                std::path::Path::new("/bin/sh"),
            )
        });

        child.kill().unwrap();
        child.wait().unwrap();
        if let Ok(prepared) = &result {
            crate::managed_agents::abort_prepared_isolated_agent_run(&identity, &prepared.run_id)
                .unwrap();
        }

        assert!(
            result.unwrap_err().contains("still has live harness"),
            "live persisted runtime did not fail closed"
        );
        assert!(path.exists(), "live prior-session receipt was removed");
        assert_eq!(
            crate::managed_agents::get_prepared_isolated_agent_run(&identity).unwrap(),
            None,
            "Prepare published an isolation run despite the live harness"
        );
    }

    #[test]
    fn dead_exact_receipt_is_cleaned_before_prepare() {
        let identity = "da".repeat(32);
        let (_dir, path, receipt) = receipt(&identity, u32::MAX, "test");
        reconcile_agent_runtime_receipts_with(
            &identity,
            "test",
            vec![(path.clone(), receipt)],
            |_| false,
            |_, _| false,
            |path| fs::remove_file(path).map_err(|error| error.to_string()),
        )
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn ambiguous_live_exact_receipt_fails_closed() {
        let identity = "db".repeat(32);
        let (_dir, path, receipt) = receipt(&identity, 42, "test");
        let error = reconcile_agent_runtime_receipts_with(
            &identity,
            "test",
            vec![(path.clone(), receipt)],
            |_| true,
            |_, _| false,
            |_| panic!("ambiguous live receipt must not be removed"),
        )
        .unwrap_err();
        assert!(error.contains("ambiguous live runtime receipt"));
        assert!(path.exists());
    }
}
