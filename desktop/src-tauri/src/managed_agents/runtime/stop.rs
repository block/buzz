use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    append_log_marker, current_instance_id, now_iso, process_belongs_to_us,
    process_has_buzz_marker, process_is_running, terminate_process, ManagedAgentPairRuntime,
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

/// Stop the single tracked runtime pair at `key`, if present.
///
/// Terminates the child, records the exit code, removes the pair receipt,
/// and appends a stop marker to the pair log. On teardown failure the
/// runtime is reinserted so the pair stays visible and stoppable instead of
/// becoming an invisible orphan. Touches no other pair for the agent and
/// does no record-level stop bookkeeping — callers own that.
pub(crate) fn stop_managed_agent_pair<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
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
fn stop_legacy_scalar_pid<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &mut ManagedAgentRecord,
) -> Result<(), String> {
    if let Some(pid) = record.runtime_pid.take() {
        if process_is_running(pid)
            && process_belongs_to_us(pid)
            && process_has_buzz_marker(pid, &current_instance_id(app))
        {
            terminate_process(pid)?;
        }
        record.updated_at = now_iso();
    }
    super::super::remove_agent_pid_file(app, &record.pubkey);
    Ok(())
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
pub fn stop_managed_agent_workspace_pair<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    match super::workspace_pair_key(app, record) {
        Some(pair_key) => super::with_pair_runtime_receipt_authority(app, &pair_key, || {
            if runtimes.contains_key(&pair_key) {
                stop_managed_agent_pair(app, record, runtimes, &pair_key)?;
                super::super::remove_agent_pid_file(app, &record.pubkey);
                let now = now_iso();
                record.runtime_pid = None;
                record.updated_at = now.clone();
                record.last_stopped_at = Some(now);
                record.last_error = None;
                record.last_error_code = None;
            } else {
                // No tracked pair here — a pubkey-wide cache clear would
                // disturb live pairs in other communities, so stay scoped.
                super::terminate_untracked_pair_runtime(app, &pair_key)?;
                stop_legacy_scalar_pid(app, record)?;
            }
            state.clear_agent_session_cache(&pair_key);
            Ok(())
        })?,
        None => {
            stop_legacy_scalar_pid(app, record)?;
            state.clear_agent_session_caches(&record.pubkey);
        }
    }
    Ok(())
}

pub fn stop_managed_agent_process<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    let keys = managed_agent_runtime_keys(runtimes, &record.pubkey);
    if keys.is_empty() {
        return stop_legacy_scalar_pid(app, record);
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
    super::super::remove_agent_pid_file(app, &record.pubkey);

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

    #[cfg(unix)]
    fn local_stop_refuses_ambiguous_receipt_before_side_effects(tracked: bool) {
        use tauri::Manager as _;

        let _path_guard = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_xdg = std::env::var_os("XDG_DATA_HOME");
        struct RestoreEnv(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
                match self.1.take() {
                    Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                    None => std::env::remove_var("XDG_DATA_HOME"),
                }
            }
        }
        let _restore_env = RestoreEnv(old_home, old_xdg);
        std::env::set_var("HOME", temp.path());
        std::env::set_var("XDG_DATA_HOME", temp.path());

        let requested_relay = "wss://relay.example/room/";
        let stored_relay = "wss://relay.example/room";
        let pubkey = "aa".repeat(32);
        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some(requested_relay.into());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let instance_id = super::super::current_instance_id(app.handle());

        let mut child =
            Some(super::super::test_fixtures::MarkedTestChild::spawn(&instance_id).unwrap());
        let pid = child.as_ref().unwrap().id();
        let _process_guard = super::super::test_fixtures::MarkedProcessGuard::new(pid);
        assert!(super::super::process_has_buzz_marker(pid, &instance_id));

        let stored_key = ManagedAgentRuntimeKey::new(&pubkey, stored_relay).unwrap();
        let requested_key = ManagedAgentRuntimeKey::new(&pubkey, requested_relay).unwrap();
        let receipt = super::super::super::ManagedAgentRuntimeReceipt {
            authority_version: 0,
            key: stored_key.clone(),
            pid,
            desktop_instance_id: instance_id,
            started_at: "now".into(),
        };
        super::super::super::write_agent_runtime_receipt(app.handle(), &receipt).unwrap();

        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "test",
            "private_key_nsec": "",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "env_vars": {},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "before"
        }))
        .unwrap();
        let mut runtimes = HashMap::new();
        if tracked {
            let process = crate::managed_agents::ManagedAgentProcess {
                child: child.take().unwrap().into_child(),
                log_path: Default::default(),
                spawn_config:
                    crate::managed_agents::spawn_snapshot::prospective_spawn_config_snapshot(
                        &record,
                        &[],
                        &[],
                        requested_relay,
                        &Default::default(),
                        false,
                        crate::managed_agents::AcpSessionPolicy::Channel,
                    ),
                setup_mode: false,
                adapter_availability: None,
                start_nonce: "test-nonce".into(),
            };
            runtimes.insert(
                requested_key.clone(),
                ManagedAgentPairRuntime::starting(process),
            );
        }

        let cache: crate::managed_agents::config_bridge::SessionConfigCache =
            serde_json::from_value(serde_json::json!({
                "configOptions": [],
                "availableModes": [],
                "availableModels": [],
                "currentModel": null,
                "modelOverridden": false,
                "gooseNativeConfig": null,
                "capturedAt": "now"
            }))
            .unwrap();
        app.state::<crate::app_state::AppState>()
            .put_session_cache(requested_key.clone(), cache);

        let error = stop_managed_agent_workspace_pair(app.handle(), &mut record, &mut runtimes)
            .unwrap_err();
        assert!(error.contains("cannot prove the requested community authority"));
        assert_eq!(record.updated_at, "before");
        assert!(record.last_stopped_at.is_none());
        assert!(app
            .state::<crate::app_state::AppState>()
            .get_session_cache(&requested_key)
            .is_some());
        assert!(
            super::super::super::read_all_agent_runtime_receipts(app.handle())
                .iter()
                .any(|(_, candidate)| candidate == &receipt)
        );

        if tracked {
            let runtime = runtimes.get_mut(&requested_key).unwrap();
            assert!(runtime.child.try_wait().unwrap().is_none());
            let mut runtime = runtimes.remove(&requested_key).unwrap();
            let _ = runtime.child.kill();
            let _ = runtime.child.wait();
        } else {
            let child = child.as_mut().unwrap();
            assert!(child.child_mut().try_wait().unwrap().is_none());
        }
        super::super::super::remove_agent_runtime_receipt(app.handle(), &stored_key);
    }

    #[cfg(unix)]
    #[test]
    fn tracked_local_stop_has_no_side_effect_before_ambiguous_receipt_refusal() {
        local_stop_refuses_ambiguous_receipt_before_side_effects(true);
    }

    #[cfg(unix)]
    #[test]
    fn untracked_local_stop_has_no_side_effect_before_ambiguous_receipt_refusal() {
        local_stop_refuses_ambiguous_receipt_before_side_effects(false);
    }
}
