use super::*;

pub(super) async fn start_local_agent_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    allow_fresh_create_start: bool,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
    replay_floor_unix: Option<u64>,
) -> Result<ManagedAgentSummary, String> {
    let (record_snapshot, start_scope) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        crate::relay::bind_expected_relay_scope(expected_relay_url, scope.relay_url.clone())?;
        crate::relay::assert_expected_signer(
            expected_signer_pubkey,
            &scope.owner_keys.public_key().to_hex(),
        )?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        (
            crate::managed_agents::private_config_overlay::resolved_local_record(state, record)?,
            scope.db_path,
        )
    };

    if record_snapshot.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is not a local agent"));
    }

    // Preflight against the same resolution spawn uses — `resolve_effective_config`
    // (definition → global fallback). A linked instance's own `provider`/`model`/
    // `relay_mesh` bytes never contribute: this reads the CURRENT definition
    // directly, so a definition edit that flips `provider` to/from relay-mesh
    // between saves is reflected here without needing a prospective re-snapshot;
    // for a global-inherited blank definition, it also folds in the global
    // default, which record-byte sniffing could never see.
    let personas = load_personas(app)?;
    let global = crate::managed_agents::load_global_agent_config(app)?;
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas,
            &global,
        );
    ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), allow_fresh_create_start).await?;

    let _transition_guard = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state
        .shutdown_started
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("desktop shutdown has started".into());
    }
    // Scope writers take the same store lock. Bind only after waiting for it,
    // and keep it through spawn: a pre-lock owner/relay read is not authority.
    let authority = lock_start_authority(app, state, &start_scope)?;
    let workspace_relay_url = crate::relay::bind_expected_relay_scope(
        expected_relay_url,
        authority.scope.relay_url.clone(),
    )?;
    let workspace_owner = crate::relay::bind_expected_signer(
        expected_signer_pubkey,
        authority.scope.owner_keys.public_key().to_hex(),
    )?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let disk_record = find_managed_agent_mut(&mut records, pubkey)?;
    let mut resolved_record =
        crate::managed_agents::private_config_overlay::resolved_local_record(state, disk_record)?;
    if resolved_record.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is no longer a local agent"));
    }
    // Re-snapshot the persona onto the resolved spawn record at every start so
    // local persona state retains its established precedence without writing
    // relay-owned configuration into the device-local migration record.
    // Load personas once: used for snapshot application below and summary build
    // at the end — avoids a second disk read for the same file in the same call.
    let personas = load_personas(app)?;
    let current_mesh_model =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &resolved_record,
            &personas,
            &crate::managed_agents::load_global_agent_config(app)?,
        );
    if current_mesh_model != mesh_model_id {
        return Err("agent mesh configuration changed during Start; retry Start".into());
    }
    if let Some(persona_id) = resolved_record.persona_id.clone() {
        match personas.iter().find(|p| p.id == persona_id) {
            Some(persona) => {
                crate::managed_agents::persona_events::apply_persona_snapshot(
                    &mut resolved_record,
                    persona,
                );
                resolved_record.updated_at = crate::util::now_iso();
            }
            None => {
                return Err(
                    crate::managed_agents::effective_config::ORPHANED_INSTANCE_ERROR.to_string(),
                );
            }
        }
    }
    start_managed_agent_process(
        app,
        &mut resolved_record,
        &mut runtimes,
        Some(workspace_owner.as_str()),
        &workspace_relay_url,
        replay_floor_unix,
    )?;
    // Persist operational lifecycle metadata only. Relay-owned configuration
    // remains an in-memory overlay and is never copied over device-local fields.
    crate::managed_agents::private_config_overlay::copy_lifecycle_state(
        disk_record,
        &resolved_record,
    );
    save_managed_agents(app, &records)?;
    // Retain the relay-resolved configuration. The projection equality guard
    // makes a runtime-only start a no-op, while avoiding resurrection of stale
    // disk config when this device is following a newer relay snapshot.
    retain_managed_agent_pending(app, state, &resolved_record)?;
    build_managed_agent_summary(
        app,
        &resolved_record,
        &runtimes,
        &personas,
        &load_teams(app).unwrap_or_default(),
        &crate::managed_agents::load_global_agent_config(app).unwrap_or_default(),
    )
}

struct StartAuthority<'a> {
    _store: std::sync::MutexGuard<'a, ()>,
    scope: crate::managed_agents::retention::RetentionScope,
}

// Caller holds runtime transition. The returned guard spans every launch effect.
fn lock_start_authority<'a, R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &'a AppState,
    expected_scope: &std::path::Path,
) -> Result<StartAuthority<'a>, String> {
    let store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    if scope.db_path != expected_scope {
        return Err("workspace changed during Start; retry Start".into());
    }
    crate::managed_agents::private_config_overlay::require_authority_ready(state)?;
    Ok(StartAuthority {
        _store: store,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    #[test]
    fn start_scope_is_rechecked_after_waiting_for_store_and_held_through_launch() {
        let _env = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().unwrap();
        struct Env(&'static str, Option<std::ffi::OsString>);
        impl Drop for Env {
            fn drop(&mut self) {
                match self.1.take() {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
        }
        let _vars: Vec<_> = ["HOME", "XDG_DATA_HOME"]
            .into_iter()
            .map(|key| {
                let guard = Env(key, std::env::var_os(key));
                std::env::set_var(key, temp.path());
                guard
            })
            .collect();
        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        state
            .managed_agent_authority_ready
            .store(true, std::sync::atomic::Ordering::Release);
        let expected =
            crate::managed_agents::retention::active_retention_scope(app.handle(), &state)
                .unwrap()
                .db_path;
        let transition = state.managed_agent_runtime_transition.lock().unwrap();
        let authority = lock_start_authority(app.handle(), &state, &expected).unwrap();
        assert!(state.managed_agents_store_lock.try_lock().is_err());
        drop(authority);
        drop(transition);
        // Deterministically model the lock wait: scope changes while the store
        // is occupied, before the launch boundary is allowed to acquire it.
        let store = state.managed_agents_store_lock.lock().unwrap();
        *state.keys.lock().unwrap() = nostr::Keys::generate();
        drop(store);
        let _transition = state.managed_agent_runtime_transition.lock().unwrap();
        assert!(lock_start_authority(app.handle(), &state, &expected)
            .err()
            .unwrap()
            .contains("workspace changed"));
    }
}
