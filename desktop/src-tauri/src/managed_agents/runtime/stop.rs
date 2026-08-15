use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    append_log_marker, current_instance_id, now_iso, process_belongs_to_us,
    process_has_buzz_marker, process_is_running, terminate_process,
    tracked_pair_runtime_for_target, ManagedAgentPairRuntime, ManagedAgentRecord,
    ManagedAgentRuntimeKey,
};

pub(crate) fn managed_agent_runtime_keys<T>(
    runtimes: &HashMap<ManagedAgentRuntimeKey, T>,
    pubkey: &str,
) -> Vec<ManagedAgentRuntimeKey> {
    runtimes
        .keys()
        .filter(|key| key.pubkey.eq_ignore_ascii_case(pubkey))
        .cloned()
        .collect()
}

/// The relay URLs to dial when restarting `pubkey`'s live pairs: each pair's
/// stamped connection URL. Restart paths must use this instead of
/// `key.relay_url` — the canonical spelling folds loopback hosts, and on a
/// host-scoped multi-tenant relay that lands the child in the wrong (empty)
/// community. Collect before stopping: stopping removes the pair, and with it
/// the only in-memory copy of the configured spelling.
pub(crate) fn managed_agent_restart_targets(
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    pubkey: &str,
) -> Vec<String> {
    runtimes
        .iter()
        .filter(|(key, _)| key.pubkey.eq_ignore_ascii_case(pubkey))
        .map(|(_, runtime)| runtime.connect_relay_url.clone())
        .collect()
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

/// Stop the single tracked runtime pair at `key`, if present.
///
/// Terminates the child, records the exit code, removes the pair receipt,
/// and appends a stop marker to the pair log. On teardown failure the
/// runtime is reinserted so the pair stays visible and stoppable instead of
/// becoming an invisible orphan. Touches no other pair for the agent and
/// does no record-level stop bookkeeping — callers own that.
fn stop_managed_agent_pair(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: &ManagedAgentRuntimeKey,
    requested_relay_url: &str,
) -> Result<(), String> {
    if tracked_pair_runtime_for_target(runtimes, key, requested_relay_url)?.is_none() {
        return Ok(());
    }
    super::ensure_pair_receipt_connection_matches(app, key, requested_relay_url)?;
    let Some(mut runtime) = runtimes.remove(key) else {
        return Ok(());
    };
    let result = (|| -> Result<(), String> {
        #[cfg(unix)]
        terminate_process(runtime.child.id())?;
        #[cfg(windows)]
        match runtime.job.take() {
            Some(job) => drop(job),
            None => runtime
                .child
                .kill()
                .map_err(|error| format!("failed to kill agent process: {error}"))?,
        }
        #[cfg(not(any(unix, windows)))]
        runtime
            .child
            .kill()
            .map_err(|error| format!("failed to kill agent process: {error}"))?;
        let status = runtime
            .child
            .wait()
            .map_err(|error| format!("failed to wait for agent shutdown: {error}"))?;
        record.last_exit_code = status.code();
        super::super::remove_agent_runtime_receipt(app, key);
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
        // Keep failed teardown visible/manageable instead of orphaning it.
        runtimes.insert(key.clone(), runtime);
        return Err(error);
    }
    Ok(())
}

/// Terminate a legacy scalar-PID child (pre-pair records) and remove the
/// agent-scoped pid file. Pair receipts are restored separately.
fn stop_legacy_scalar_pid(app: &AppHandle, record: &mut ManagedAgentRecord) -> Result<(), String> {
    if let Some(pid) = record.runtime_pid {
        if process_is_running(pid)
            && process_belongs_to_us(pid)
            && process_has_buzz_marker(pid, &current_instance_id(app))
        {
            terminate_process(pid)?;
        }
        record.runtime_pid = None;
        record.updated_at = now_iso();
    }
    super::super::remove_agent_pid_file(app, &record.pubkey);
    Ok(())
}

fn clear_inactive_legacy_scalar_pid_with(
    record: &mut ManagedAgentRecord,
    is_live_owned: impl FnOnce(u32) -> bool,
) -> Result<bool, String> {
    let Some(pid) = record.runtime_pid else {
        return Ok(false);
    };
    if is_live_owned(pid) {
        return Err(concat!(
            "connection-target conflict: a live legacy managed-agent runtime has no ",
            "connection-target receipt; stop all runtimes for this agent before managing ",
            "one community"
        )
        .into());
    }
    record.runtime_pid = None;
    record.updated_at = now_iso();
    Ok(true)
}

/// Pair-scoped stop may clear stale scalar bookkeeping, but a live legacy PID
/// has no tenant stamp and must be left to the explicit agent-wide stop path.
pub(crate) fn clear_inactive_legacy_scalar_pid(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
) -> Result<(), String> {
    let cleared = clear_inactive_legacy_scalar_pid_with(record, |pid| {
        process_is_running(pid)
            && process_belongs_to_us(pid)
            && process_has_buzz_marker(pid, &current_instance_id(app))
    })?;
    if cleared {
        super::super::remove_agent_pid_file(app, &record.pubkey);
    }
    Ok(())
}

/// Stop the runtime pair this record resolves to for the active workspace
/// (explicit relay pin, else the active workspace relay) — the pair-scoped
/// counterpart of [`stop_managed_agent_process`], which drains every pair.
///
/// Community-scoped surfaces (profile panel, Agents tab, auto-restart) stop
/// through here so stopping an agent in one community never tears down its
/// pairs in other communities. Clears the matching agent session cache
/// (pair-scoped when a pair target resolves). A prior-session receipt must
/// match that same target before any pair-scoped cleanup proceeds.
pub fn stop_managed_agent_workspace_pair(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    match super::workspace_pair_target(app, record) {
        Some((pair_key, requested_relay)) if runtimes.contains_key(&pair_key) => {
            stop_managed_agent_pair(app, record, runtimes, &pair_key, &requested_relay)?;
            state.clear_agent_session_cache(&pair_key);
            let now = now_iso();
            record.updated_at = now.clone();
            record.last_stopped_at = Some(now);
            record.last_error = None;
            record.last_error_code = None;
        }
        Some((pair_key, requested_relay)) => {
            // No tracked pair here — a pubkey-wide cache clear would disturb
            // live pairs in other communities, so validate any receipt and
            // stay pair-scoped.
            super::terminate_untracked_pair_runtime(app, &pair_key, &requested_relay)?;
            clear_inactive_legacy_scalar_pid(app, record)?;
            state.clear_agent_session_cache(&pair_key);
        }
        None => {
            stop_legacy_scalar_pid(app, record)?;
            state.clear_agent_session_caches(&record.pubkey);
        }
    }
    Ok(())
}

pub fn stop_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    let keys = managed_agent_runtime_keys(runtimes, &record.pubkey);
    if keys.is_empty() {
        return stop_legacy_scalar_pid(app, record);
    }

    let mut errors = Vec::new();
    for key in keys {
        let Some(requested_relay) = runtimes
            .get(&key)
            .map(|runtime| runtime.connect_relay_url.clone())
        else {
            continue;
        };
        if let Err(error) = stop_managed_agent_pair(app, record, runtimes, &key, &requested_relay) {
            errors.push(error);
        }
    }

    if !errors.is_empty() {
        return Err(format!(
            "failed to stop one or more managed-agent runtimes: {}",
            errors.join("; ")
        ));
    }

    // This is the explicit agent-wide path, so it is also the safe migration
    // escape hatch for a live scalar PID whose community cannot be proven.
    stop_legacy_scalar_pid(app, record)?;
    let now = now_iso();
    record.updated_at = now.clone();
    record.last_stopped_at = Some(now);
    record.last_error = None;
    record.last_error_code = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pair_scoped_stop_refuses_a_live_unscoped_legacy_pid() {
        let mut record = super::super::test_fixtures::minimal_record(&"aa".repeat(32));
        record.runtime_pid = Some(42);

        let error = clear_inactive_legacy_scalar_pid_with(&mut record, |_| true).unwrap_err();

        assert!(error.contains("connection-target conflict"));
        assert!(!error.contains("ws://"));
        assert_eq!(record.runtime_pid, Some(42));
    }

    #[test]
    fn pair_scoped_stop_clears_only_inactive_legacy_bookkeeping() {
        let mut record = super::super::test_fixtures::minimal_record(&"aa".repeat(32));
        record.runtime_pid = Some(42);

        assert!(clear_inactive_legacy_scalar_pid_with(&mut record, |_| false).unwrap());
        assert_eq!(record.runtime_pid, None);
    }
}
