//! Local launch preflight shared by explicit controls and config-driven starts.
use super::*;

#[cfg(feature = "mesh-llm")]
async fn ensure_relay_mesh_for_record(
    app: &AppHandle,
    model_id: Option<&str>,
    allow_fresh_create_start: bool,
) -> Result<(), String> {
    crate::commands::ensure_relay_mesh_for_record(app, model_id, allow_fresh_create_start).await
}

#[cfg(not(feature = "mesh-llm"))]
async fn ensure_relay_mesh_for_record(
    _app: &AppHandle,
    _model_id: Option<&str>,
    _allow_fresh_create_start: bool,
) -> Result<(), String> {
    Ok(())
}

pub(in crate::commands) async fn start_local_agent_pairs_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    relay_urls: &[String],
) -> Result<ManagedAgentSummary, String> {
    let record_snapshot = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        load_managed_agents(app)?
            .into_iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };
    if record_snapshot.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is not a local agent"));
    }
    let personas_for_preflight = load_personas(app).unwrap_or_default();
    let global_for_preflight =
        crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas_for_preflight,
            &global_for_preflight,
        );
    ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false).await?;

    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(app)?;
        let record = find_managed_agent_mut(&mut records, pubkey)?;
        let personas = load_personas(app).unwrap_or_default();
        if let Some(persona_id) = record.persona_id.clone() {
            if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
                crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
                record.updated_at = crate::util::now_iso();
            }
        }
        save_managed_agents(app, &records)?;
        if let Some(saved_record) = records.iter().find(|record| record.pubkey == pubkey) {
            retain_managed_agent_pending(app, state, saved_record);
        }
    }

    let mut errors = Vec::new();
    for relay_url in relay_urls {
        if let Err(error) = crate::managed_agents::start_managed_agent_runtime_pair_lazy(
            pubkey.to_string(),
            relay_url.clone(),
            app.clone(),
        ) {
            errors.push(format!("{relay_url}: {error}"));
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "failed to restart one or more managed-agent runtime pairs: {}",
            errors.join("; ")
        ));
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = load_managed_agents(app)?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    summarize_from_disk(app, record, &runtimes)
}

// Shared continuation for no-journal Start and fresh create-start. Validate
// current state without replacing the captured values consumed by spawn.
fn revalidate_first_start_scope(
    state: &AppState,
    captured_relay_url: &str,
    captured_owner: &str,
) -> Result<(), String> {
    crate::relay::bind_expected_relay_scope(
        Some(captured_relay_url),
        crate::relay::relay_ws_url_with_override(state),
    )?;
    crate::relay::assert_expected_signer(Some(captured_owner), &workspace_owner_hex(state)?)
}

// All local first-start callers share this suspension boundary. Explicit
// Start supplies its clicked scope; fresh create-start binds the command's
// entry scope here, BEFORE mesh discovery can suspend. Never bind a new
// workspace/owner on continuation merely because the caller was unscoped.
async fn scoped_local_preflight(
    state: &AppState,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
    preflight: impl std::future::Future<Output = Result<(), String>>,
) -> Result<
    (
        crate::relay::ScopedWorkspaceRelay,
        crate::relay::ScopedWorkspaceSigner,
    ),
    String,
> {
    let relay = crate::relay::bind_expected_relay_scope(
        expected_relay_url,
        crate::relay::relay_ws_url_with_override(state),
    )?;
    let owner =
        crate::relay::bind_expected_signer(expected_signer_pubkey, workspace_owner_hex(state)?)?;
    preflight.await?;
    revalidate_first_start_scope(state, relay.as_str(), owner.as_str())?;
    Ok((relay, owner))
}

pub(in crate::commands) async fn start_local_agent_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    allow_fresh_create_start: bool,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
) -> Result<ManagedAgentSummary, String> {
    let record_snapshot = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(app)?;
        records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .cloned()
            .ok_or_else(|| format!("agent {pubkey} not found"))?
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
    let personas = load_personas(app).unwrap_or_default();
    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas,
            &global,
        );
    let (workspace_relay_url, workspace_owner) = scoped_local_preflight(
        state,
        expected_relay_url,
        expected_signer_pubkey,
        ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), allow_fresh_create_start),
    )
    .await?;

    if !allow_fresh_create_start {
        let app_for_start = app.clone();
        let agent_for_start = pubkey.to_owned();
        let relay_for_start = workspace_relay_url.as_str().to_owned();
        let owner_for_start = workspace_owner.as_str().to_owned();
        let resumed = tokio::task::spawn_blocking(move || {
            crate::managed_agents::start_after_exact_stop(
                &app_for_start,
                &agent_for_start,
                &relay_for_start,
                &owner_for_start,
            )
        })
        .await
        .map_err(|_| "explicit Start task failed")??;
        if resumed {
            let records = load_managed_agents(app)?;
            let runtimes = state
                .managed_agent_processes
                .lock()
                .map_err(|e| e.to_string())?;
            let record = records
                .iter()
                .find(|r| r.pubkey == pubkey)
                .ok_or("agent not found")?;
            return summarize_from_disk(app, record, &runtimes);
        }
    }

    // Recovery adds another suspension even when there is no journal yet.
    // Revalidate the captured pair/owner before the legacy first-start path;
    // never let that await retarget or authorize a stale workspace action.
    revalidate_first_start_scope(
        state,
        workspace_relay_url.as_str(),
        workspace_owner.as_str(),
    )?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    if record.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is no longer a local agent"));
    }
    // Re-snapshot the persona onto the record at every spawn so the agent always
    // starts with the current persona config (system_prompt, model, provider,
    // runtime). This clears the "out of date" drift badge without requiring a
    // delete+recreate. See `apply_persona_snapshot` for the precedence and
    // env-override self-heal rules.
    // Load personas once: used for snapshot application below and summary build
    // at the end — avoids a second disk read for the same file in the same call.
    let personas = load_personas(app).unwrap_or_default();
    if let Some(persona_id) = record.persona_id.clone() {
        match personas.iter().find(|p| p.id == persona_id) {
            Some(persona) => {
                crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
                record.updated_at = crate::util::now_iso();
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
        record,
        &mut runtimes,
        Some(workspace_owner.as_str()),
        &workspace_relay_url,
    )?;
    save_managed_agents(app, &records)?;
    if let Some(saved_record) = records.iter().find(|r| r.pubkey == pubkey) {
        retain_managed_agent_pending(app, state, saved_record);
    }
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    build_managed_agent_summary(
        app,
        record,
        &runtimes,
        &personas,
        &load_teams(app).unwrap_or_default(),
        &crate::managed_agents::load_global_agent_config(app).unwrap_or_default(),
    )
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[tokio::test]
    async fn create_start_preflight_retains_entry_scope_without_a_caller_pin() {
        let state = crate::app_state::build_app_state();
        let owner = workspace_owner_hex(&state).unwrap();
        for relay in ["ws://localhost:3000", "wss://community.example"] {
            *state.relay_url_override.lock().unwrap() = Some(relay.to_owned());
            let (bound_relay, bound_owner) = scoped_local_preflight(&state, None, None, async {
                tokio::task::yield_now().await;
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(bound_relay.as_str(), relay);
            assert_eq!(bound_owner.as_str(), owner);
        }
    }

    #[tokio::test]
    async fn create_start_preflight_rejects_a_community_switch_without_a_caller_pin() {
        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some("wss://clicked.example".into());
        let error = scoped_local_preflight(&state, None, None, async {
            tokio::task::yield_now().await;
            *state.relay_url_override.lock().unwrap() = Some("wss://other.example".into());
            Ok(())
        })
        .await
        .unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    #[tokio::test]
    async fn create_start_preflight_rejects_an_owner_switch_without_a_caller_pin() {
        let state = crate::app_state::build_app_state();
        let error = scoped_local_preflight(&state, None, None, async {
            tokio::task::yield_now().await;
            *state.keys.lock().unwrap() = Keys::generate();
            Ok(())
        })
        .await
        .unwrap_err();
        assert!(error.contains("active identity changed"), "{error}");
    }

    #[tokio::test]
    async fn scoped_start_rejects_stale_scope_before_polling_preflight() {
        let state = crate::app_state::build_app_state();
        *state.relay_url_override.lock().unwrap() = Some("wss://other.example".into());
        let error = scoped_local_preflight(&state, Some("wss://clicked.example"), None, async {
            panic!("stale Start must not poll mesh preflight");
        })
        .await
        .unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    // This exercises the production continuation (including its WS getter),
    // not just the generic relay helper. State is ephemeral: no app setup,
    // identity resolution, keyring, files, network or child process is used.
    #[test]
    fn first_start_continuation_accepts_unchanged_ws_and_wss_communities() {
        let state = crate::app_state::build_app_state();
        let owner = workspace_owner_hex(&state).unwrap();
        for relay in ["ws://localhost:3000", "wss://community.example"] {
            *state.relay_url_override.lock().unwrap() = Some(relay.to_owned());
            revalidate_first_start_scope(&state, relay, &owner).unwrap();
        }
    }

    #[test]
    fn first_start_continuation_rejects_community_changed_during_recovery() {
        let state = crate::app_state::build_app_state();
        let owner = workspace_owner_hex(&state).unwrap();
        *state.relay_url_override.lock().unwrap() = Some("wss://other.example".to_owned());
        let error =
            revalidate_first_start_scope(&state, "wss://community.example", &owner).unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    #[test]
    fn first_start_continuation_rejects_owner_changed_during_recovery() {
        let state = crate::app_state::build_app_state();
        let owner = workspace_owner_hex(&state).unwrap();
        *state.relay_url_override.lock().unwrap() = Some("wss://community.example".to_owned());
        *state.keys.lock().unwrap() = Keys::generate();
        let error =
            revalidate_first_start_scope(&state, "wss://community.example", &owner).unwrap_err();
        assert!(error.contains("active identity changed"), "{error}");
    }
}
