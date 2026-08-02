use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    append_log_marker, now_iso, ManagedAgentPairRuntime, ManagedAgentRecord, ManagedAgentRuntimeKey,
};
#[cfg(not(windows))]
use super::{
    current_instance_id, process_belongs_to_us, process_has_buzz_marker, process_is_running,
    terminate_process,
};

pub(crate) fn managed_agent_runtime_keys<T>(
    runtimes: &HashMap<ManagedAgentRuntimeKey, T>,
    pubkey: &str,
) -> Vec<ManagedAgentRuntimeKey> {
    let mut keys = runtimes
        .keys()
        .filter(|key| key.pubkey.eq_ignore_ascii_case(pubkey))
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by_key(ManagedAgentRuntimeKey::runtime_id);
    keys
}

#[cfg(test)]
pub(crate) fn managed_agent_runtime_relay_urls<T>(
    runtimes: &HashMap<ManagedAgentRuntimeKey, T>,
    pubkey: &str,
) -> Vec<String> {
    managed_agent_runtime_keys(runtimes, pubkey)
        .into_iter()
        .map(|key| key.relay_url)
        .collect()
}

pub fn persist_stop<T>(
    result: Result<T, String>,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => persist_failed_stop(error, persist),
    }
}

pub fn persist_failed_stop<T>(
    stop_error: String,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    let persistence = persist()
        .err()
        .map(|save_error| format!("; failed to persist Stop recovery state: {save_error}"))
        .unwrap_or_default();
    Err(format!("{stop_error}{persistence}"))
}

/// Stop the single tracked runtime pair at `key`, if present.
///
/// Terminates the child, records the exit code, removes the pair receipt,
/// and appends a stop marker to the pair log. Any failure retains the exact
/// Child/Job authority under the same pair key for retry.
fn stop_managed_agent_pair(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    let Some(mut runtime) = runtimes.remove(key) else {
        return Ok(());
    };
    let result = (|| -> Result<(), String> {
        #[cfg(unix)]
        let status = {
            terminate_process(runtime.child.id())?;
            runtime
                .child
                .wait()
                .map_err(|error| format!("failed to wait for agent shutdown: {error}"))?
        };
        #[cfg(windows)]
        let status =
            super::super::process_lifecycle::finalize_tracked_runtime(app, key, &mut runtime)?;
        #[cfg(not(any(unix, windows)))]
        let status = {
            runtime
                .child
                .kill()
                .map_err(|error| format!("failed to kill agent process: {error}"))?;
            runtime
                .child
                .wait()
                .map_err(|error| format!("failed to wait for agent shutdown: {error}"))?
        };
        record.last_exit_code = status.code();
        #[cfg(not(windows))]
        super::super::remove_agent_runtime_receipt(app, key)?;
        if let Err(error) = append_log_marker(
            &runtime.log_path,
            &format!(
                "=== stopped {} ({}) at {} ===",
                record.name,
                record.pubkey,
                now_iso()
            ),
        ) {
            eprintln!(
                "buzz-desktop: failed to append stop marker for {} on {}: {error}",
                record.pubkey, key.relay_url
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Keep failed teardown and its exact authority visible/manageable.
        #[cfg(windows)]
        let error = super::receipt_failure::mark_unverified_runtime(
            Some(app),
            record,
            key,
            runtime.child.id(),
            now_iso(),
            format!(
                "Stop cleanup is incomplete and exact pair authority remains tracked for retry: {error}"
            ),
        );
        runtimes.insert(key.clone(), runtime);
        return Err(error);
    }
    if let Err(error) = super::super::clear_pair_recovery_with_terminal_proof(app, record, key) {
        super::super::record_terminal_proof_pending_recovery_clear(
            record,
            key,
            runtime.child.id(),
            &error,
        );
        runtimes.insert(key.clone(), runtime);
        return Err(format!(
            "failed to retire exact-pair recovery authority after terminal proof: {error}"
        ));
    }
    Ok(())
}

/// Terminate a legacy scalar-PID child (pre-pair records) and remove the
/// agent-scoped pid file. Pair receipts are restored separately.
fn stop_legacy_scalar_pid(app: &AppHandle, record: &mut ManagedAgentRecord) -> Result<(), String> {
    if super::super::has_unverified_job_reap(record) {
        return Err(
            "prior Windows Job closure remains unverified; preserving PID recovery authority"
                .into(),
        );
    }
    if let Some(pid) = record.runtime_pid {
        #[cfg(windows)]
        {
            let message = format!(
                "cannot safely terminate persisted Windows PID {pid} without its owned Child/Job authority; preserving recovery identity"
            );
            record.updated_at = now_iso();
            record.last_error = Some(message.clone());
            record.last_error_code = None;
            return Err(message);
        }
        #[cfg(not(windows))]
        {
            if process_is_running(pid)
                && process_belongs_to_us(pid)
                && process_has_buzz_marker(pid, &current_instance_id(app))
            {
                terminate_process(pid)?;
            }
            record.runtime_pid = None;
            record.updated_at = now_iso();
        }
    }
    super::super::remove_agent_pid_file(app, &record.pubkey);
    Ok(())
}

fn untracked_runtime_keys_for_agent_from(
    receipts: Result<Vec<(std::path::PathBuf, super::super::ManagedAgentRuntimeReceipt)>, String>,
    pubkey: &str,
) -> Result<Vec<ManagedAgentRuntimeKey>, String> {
    Ok(receipts?
        .into_iter()
        .filter_map(|(_, receipt)| (receipt.key.pubkey == pubkey).then_some(receipt.key))
        .collect())
}

fn untracked_runtime_keys_for_agent(
    app: &AppHandle,
    pubkey: &str,
) -> Result<Vec<ManagedAgentRuntimeKey>, String> {
    untracked_runtime_keys_for_agent_from(
        super::super::read_all_agent_runtime_receipts(app),
        pubkey,
    )
}

/// Stop the runtime pair this record resolves to for the active workspace
/// (explicit relay pin, else the active workspace relay) — the pair-scoped
/// counterpart of [`stop_managed_agent_process`], which drains every pair.
///
/// Community-scoped surfaces (profile panel, Agents tab, auto-restart) stop
/// through here so stopping an agent in one community never tears down its
/// pairs in other communities. Clears the matching agent session cache
/// (pair-scoped when a pair key resolves). When no pair is tracked for this
/// workspace, only legacy scalar-PID cleanup runs.
pub fn stop_managed_agent_workspace_pair(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    match super::workspace_pair_key(app, record) {
        Some(pair_key) if runtimes.contains_key(&pair_key) => {
            stop_managed_agent_pair(app, record, runtimes, &pair_key)?;
            state.clear_agent_session_cache(&pair_key);
            super::super::remove_agent_pid_file(app, &record.pubkey);
            let now = now_iso();
            record.updated_at = now.clone();
            let sibling_active = runtimes.keys().any(|key| key.pubkey == record.pubkey);
            if !sibling_active && !super::super::has_unverified_job_reap(record) {
                record.runtime_pid = None;
                record.last_stopped_at = Some(now);
                if !super::super::finalize_pending_pair_failures(record) {
                    record.last_error = None;
                    record.last_error_code = None;
                }
            }
        }
        Some(pair_key) => {
            // No tracked pair here — discover and resolve exact persisted
            // authority before falling back to the legacy scalar PID evidence.
            super::terminate_untracked_pair_runtime(app, &pair_key)?;
            super::super::clear_pair_recovery_with_terminal_proof(app, record, &pair_key)?;
            stop_legacy_scalar_pid(app, record)?;
            state.clear_agent_session_cache(&pair_key);
        }
        None => {
            stop_legacy_scalar_pid(app, record)?;
            state.clear_agent_session_caches(&record.pubkey);
        }
    }
    Ok(())
}

fn apply_agent_stop_outcome(
    record: &mut ManagedAgentRecord,
    errors: &[String],
    now: String,
) -> Result<(), String> {
    record.updated_at = now.clone();

    if errors.is_empty() && !super::super::has_unverified_job_reap(record) {
        record.runtime_pid = None;
        record.last_stopped_at = Some(now);
        if !super::super::finalize_pending_pair_failures(record) {
            record.last_error = None;
            record.last_error_code = None;
        }
        Ok(())
    } else if errors.is_empty() {
        Err("managed-agent recovery authority remains uncertain after Stop".to_string())
    } else {
        let error = format!(
            "failed to stop one or more managed-agent runtimes: {}",
            errors.join("; ")
        );
        if !super::super::has_unverified_job_reap(record) {
            record.last_error = Some(error.clone());
        }
        Err(error)
    }
}

pub fn stop_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    let keys = managed_agent_runtime_keys(runtimes, &record.pubkey);
    let persisted_keys = untracked_runtime_keys_for_agent(app, &record.pubkey)?;
    if keys.is_empty() && persisted_keys.is_empty() {
        return stop_legacy_scalar_pid(app, record);
    }

    let mut errors = Vec::new();
    for key in &keys {
        if let Err(error) = stop_managed_agent_pair(app, record, runtimes, key) {
            errors.push(format!("{}: {error}", key.relay_url));
        }
    }
    for key in persisted_keys {
        if keys.contains(&key) {
            continue;
        }
        if let Err(error) = super::terminate_untracked_pair_runtime(app, &key) {
            errors.push(format!("{}: {error}", key.relay_url));
        } else {
            super::super::clear_pair_recovery_with_terminal_proof(app, record, &key)?;
        }
    }

    let result = apply_agent_stop_outcome(record, &errors, now_iso());
    if result.is_ok() {
        super::super::remove_agent_pid_file(app, &record.pubkey);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_record() -> ManagedAgentRecord {
        serde_json::from_str(
            r#"{
                "pubkey": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "name": "test",
                "private_key_nsec": "nsec1fake",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "model": null,
                "provider": null,
                "env_vars": {},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": "2026-01-02T00:00:00Z",
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("stop record fixture")
    }

    #[test]
    fn failed_agent_wide_stop_preserves_error_and_does_not_claim_stopped() {
        let mut record = minimal_record();
        record.runtime_pid = Some(4242);
        let original_stopped_at = record.last_stopped_at.clone();
        let errors = vec!["relay: bounded Stop failed".to_string()];

        let result =
            apply_agent_stop_outcome(&mut record, &errors, "2026-01-03T00:00:00Z".to_string());

        assert!(result.is_err());
        assert_eq!(record.last_stopped_at, original_stopped_at);
        assert_eq!(record.runtime_pid, Some(4242));
        assert!(record
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("bounded Stop failed")));
    }

    #[test]
    fn agent_wide_stop_does_not_launder_unverified_reap_marker() {
        let mut record = minimal_record();
        record.runtime_pid = Some(5252);
        record.last_error = Some(format!(
            "{} first pair reap unverified",
            crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX
        ));

        let result = apply_agent_stop_outcome(
            &mut record,
            &["second pair also failed".to_string()],
            "2026-01-03T00:00:00Z".to_string(),
        );

        assert!(result.is_err());
        assert!(crate::managed_agents::has_unverified_job_reap(&record));
    }

    #[test]
    fn successful_retry_stop_preserves_observed_terminal_failure() {
        let mut record = minimal_record();
        let key =
            ManagedAgentRuntimeKey::new(record.pubkey.clone(), "wss://terminal-failure.example")
                .unwrap();
        super::super::super::record_pending_pair_failure(
            &mut record,
            &key,
            Some(13),
            &super::super::super::storage::AgentLogError {
                message: "harness exited with status 13".into(),
                code: None,
            },
        );

        apply_agent_stop_outcome(&mut record, &[], "2026-01-03T00:00:00Z".into()).unwrap();

        assert_eq!(record.last_exit_code, Some(13));
        assert!(record
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("status 13")));
    }

    #[test]
    fn pair_preserving_restart_targets_exact_original_relays() {
        let agent = "aa".repeat(32);
        let other = "bb".repeat(32);
        let first = ManagedAgentRuntimeKey::new(&agent, "wss://one.example").unwrap();
        let second = ManagedAgentRuntimeKey::new(&agent, "wss://two.example").unwrap();
        let unrelated = ManagedAgentRuntimeKey::new(other, "wss://fallback.example").unwrap();
        let runtimes = HashMap::from([(first, ()), (second, ()), (unrelated, ())]);

        let mut relays = managed_agent_runtime_relay_urls(&runtimes, &agent);
        relays.sort();
        assert_eq!(
            relays,
            vec![
                "wss://one.example".to_string(),
                "wss://two.example".to_string()
            ]
        );
    }

    #[test]
    fn pair_scoped_selection_targets_only_the_exact_pair() {
        // stop_managed_agent_workspace_pair resolves one key and removes only
        // that map entry: the same agent's pair on another relay and other
        // agents' pairs must survive a pair-scoped stop.
        let agent = "aa".repeat(32);
        let other = "bb".repeat(32);
        let viewed = ManagedAgentRuntimeKey::new(&agent, "wss://one.example").unwrap();
        let elsewhere = ManagedAgentRuntimeKey::new(&agent, "wss://two.example").unwrap();
        let unrelated = ManagedAgentRuntimeKey::new(other, "wss://one.example").unwrap();
        let mut runtimes = HashMap::from([
            (viewed.clone(), ()),
            (elsewhere.clone(), ()),
            (unrelated.clone(), ()),
        ]);

        // Non-canonical spelling of the viewed workspace relay resolves to
        // the same canonical key that spawn stamped.
        let resolved = ManagedAgentRuntimeKey::new(&agent, "WSS://One.Example:443/").unwrap();
        assert_eq!(resolved, viewed);
        assert!(runtimes.remove(&resolved).is_some());
        assert!(runtimes.contains_key(&elsewhere));
        assert!(runtimes.contains_key(&unrelated));
    }

    #[test]
    fn agent_wide_selection_drains_every_pair_only_for_that_agent() {
        let agent = "aa".repeat(32);
        let other = "bb".repeat(32);
        let first = ManagedAgentRuntimeKey::new(&agent, "wss://one.example").unwrap();
        let second = ManagedAgentRuntimeKey::new(&agent, "wss://two.example").unwrap();
        let unrelated = ManagedAgentRuntimeKey::new(other, "wss://one.example").unwrap();
        let runtimes = HashMap::from([(first.clone(), ()), (second.clone(), ()), (unrelated, ())]);

        let mut selected = managed_agent_runtime_keys(&runtimes, &agent);
        selected.sort_by(|left, right| left.relay_url.cmp(&right.relay_url));
        assert_eq!(selected, vec![first, second]);
    }

    #[test]
    fn failed_stop_persists_recovery_state_before_propagation() {
        let dir = tempfile::tempdir().expect("temporary recovery directory");
        let path = dir.path().join("managed-agents.json");
        let recovery = r#"{"runtime_pid":8181,"last_error":"stop remains uncertain"}"#;
        let result: Result<(), String> =
            persist_failed_stop("bounded Stop failed".to_string(), || {
                std::fs::write(&path, recovery).map_err(|error| error.to_string())
            });

        assert_eq!(result.unwrap_err(), "bounded Stop failed");
        assert_eq!(
            std::fs::read_to_string(path).expect("durable Stop recovery readback"),
            recovery
        );
    }

    #[test]
    fn untracked_agent_wide_stop_propagates_receipt_store_uncertainty() {
        let result = untracked_runtime_keys_for_agent_from(
            Err("receipt store indeterminate".to_string()),
            &"aa".repeat(32),
        );
        assert!(result.unwrap_err().contains("receipt store indeterminate"));
    }

    #[test]
    fn untracked_agent_wide_stop_selects_every_persisted_pair_for_agent() {
        let agent = "aa".repeat(32);
        let other = "bb".repeat(32);
        let first = ManagedAgentRuntimeKey::new(&agent, "wss://one.example").unwrap();
        let second = ManagedAgentRuntimeKey::new(&agent, "wss://two.example").unwrap();
        let unrelated = ManagedAgentRuntimeKey::new(&other, "wss://one.example").unwrap();
        let receipt =
            |key: ManagedAgentRuntimeKey| super::super::super::ManagedAgentRuntimeReceipt {
                key,
                pid: 1,
                desktop_instance_id: "instance".to_string(),
                started_at: "now".to_string(),
                windows_job_contained: true,
            };
        let receipts = vec![
            (
                std::path::PathBuf::from("first.json"),
                receipt(first.clone()),
            ),
            (
                std::path::PathBuf::from("second.json"),
                receipt(second.clone()),
            ),
            (std::path::PathBuf::from("other.json"), receipt(unrelated)),
        ];

        let mut selected = untracked_runtime_keys_for_agent_from(Ok(receipts), &agent).unwrap();
        selected.sort_by(|left, right| left.relay_url.cmp(&right.relay_url));
        assert_eq!(selected, vec![first, second]);
    }

    #[test]
    fn failed_stop_reports_persistence_failure() {
        let result: Result<(), String> =
            persist_failed_stop("bounded Stop failed".to_string(), || {
                Err("disk unavailable".to_string())
            });
        let error = result.unwrap_err();
        assert!(error.contains("bounded Stop failed"));
        assert!(error.contains("failed to persist Stop recovery state: disk unavailable"));
    }
}
