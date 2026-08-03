use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    append_log_marker, now_iso, terminate_process, LegacyMigrationGate, ManagedAgentPairRuntime,
    ManagedAgentRecord, ManagedAgentRuntimeKey,
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

/// Stop the single tracked schema-v2 runtime pair through its authenticated,
/// generation-fenced controller. A PID signal is only a bounded fallback after
/// revalidating the process-start marker from the authenticated receipt.
fn stop_managed_agent_pair(
    _app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    let Some(mut runtime) = runtimes.remove(key) else {
        return Ok(());
    };
    if runtime.is_legacy() {
        let receipt = runtime
            .legacy_receipt
            .as_ref()
            .expect("legacy runtime has schema-v1 receipt");
        if runtime
            .process
            .as_ref()
            .is_none_or(|process| process.child.id() != receipt.pid)
            || !buzz_runtime_pkg::process_matches_marker(receipt.pid, &receipt.process_start_marker)
        {
            runtimes.insert(key.clone(), runtime);
            return Err("legacy runtime identity cannot be verified; refusing PID signal".into());
        }
        if let Err(error) = terminate_process(receipt.pid) {
            runtimes.insert(key.clone(), runtime);
            return Err(error);
        }
        if let Some(process) = runtime.process.as_mut() {
            let _ = process.child.wait();
        }
    } else {
        let controller = runtime
            .controller
            .as_ref()
            .ok_or_else(|| "runtime has no authenticated controller".to_string())?;
        if let Err(error) = tauri::async_runtime::block_on(controller.shutdown()) {
            runtimes.insert(key.clone(), runtime);
            return Err(format!(
                "generation-fenced runtime shutdown failed: {error}"
            ));
        }
        let receipt = runtime
            .receipt
            .as_ref()
            .expect("authenticated controller has a schema-v2 receipt");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if buzz_runtime_pkg::process_start_marker(receipt.pid).is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Ok(marker) = buzz_runtime_pkg::process_start_marker(receipt.pid) {
            if marker != receipt.process_start_marker {
                runtimes.insert(key.clone(), runtime);
                return Err(
                    "runtime PID was reused during shutdown; refusing cleanup signal".into(),
                );
            }
            if let Err(error) = terminate_process(receipt.pid) {
                runtimes.insert(key.clone(), runtime);
                return Err(error);
            }
        }
        let _ = super::super::quarantine_agent_runtime_receipt_path(&runtime.receipt_path);
    }
    record.last_exit_code = None;
    if let Some(log_path) = runtime.log_path() {
        if let Err(error) = append_log_marker(
            log_path,
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
    }
    Ok(())
}

fn clear_legacy_scalar_pid(record: &mut ManagedAgentRecord) {
    if record.runtime_pid.take().is_some() {
        record.updated_at = now_iso();
    }
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
            let now = now_iso();
            record.runtime_pid = None;
            record.updated_at = now.clone();
            record.last_stopped_at = Some(now);
            record.last_error = None;
            record.last_error_code = None;
        }
        Some(pair_key) => {
            let receipt_path = super::super::managed_agent_runtime_receipt_path(app, &pair_key)?;
            if receipt_path.exists() {
                let runtime = super::super::runtime_commands::connect_runtime_receipt(
                    app, &pair_key, None, false,
                )?;
                runtimes.insert(pair_key.clone(), runtime);
                stop_managed_agent_pair(app, record, runtimes, &pair_key)?;
                state.clear_agent_session_cache(&pair_key);
            } else {
                match super::legacy_migration_gate(app, &pair_key, record.runtime_pid)? {
                    LegacyMigrationGate::LegacyRuntimeActive => {
                        return Err("legacy_runtime_active".into());
                    }
                    LegacyMigrationGate::ManualLegacyStopRequired => {
                        return Err("manual_legacy_stop_required".into());
                    }
                    LegacyMigrationGate::Clear => clear_legacy_scalar_pid(record),
                }
                state.clear_agent_session_cache(&pair_key);
            }
        }
        None => {
            clear_legacy_scalar_pid(record);
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
        return stop_managed_agent_workspace_pair(app, record, runtimes);
    }

    let mut errors = Vec::new();
    for key in keys {
        if let Err(error) = stop_managed_agent_pair(app, record, runtimes, &key) {
            errors.push(format!("{}: {error}", key.relay_url));
        }
    }

    let now = now_iso();
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_stopped_at = Some(now);
    record.last_error = None;
    record.last_error_code = None;

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to stop one or more managed-agent runtimes: {}",
            errors.join("; ")
        ))
    }
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
}
