//! Captured-scope agent start helpers, split from `agents.rs` (file-size
//! guard). These variants use a captured [`WorkspaceAgentScope`] so concurrent
//! workspace switches cannot redirect reads/writes to the wrong scope.

/// Spawn auto-start pairs for `pubkey` on each of `relay_urls`, using the
/// live active scope (live reads — relay and owner keys come from current
/// app state). Used by import/restore callers that don't yet hold a captured
/// scope.
///
/// Returns a full [`ManagedAgentSummary`] on success.
pub(crate) async fn start_local_agent_pairs_with_preflight(
    app: &super::AppHandle,
    state: &super::AppState,
    pubkey: &str,
    relay_urls: &[String],
) -> Result<super::ManagedAgentSummary, String> {
    let record_snapshot = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        super::load_managed_agents(app)?
            .into_iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };
    if record_snapshot.backend != super::BackendKind::Local {
        return Err(format!("agent {pubkey} is not a local agent"));
    }
    let personas_for_preflight = super::load_personas(app).unwrap_or_default();
    let global_for_preflight =
        crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas_for_preflight,
            &global_for_preflight,
        );
    super::ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false).await?;

    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = super::load_managed_agents(app)?;
        let record = super::find_managed_agent_mut(&mut records, pubkey)?;
        let personas = super::load_personas(app).unwrap_or_default();
        if let Some(persona_id) = record.persona_id.clone() {
            if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
                crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
                record.updated_at = crate::util::now_iso();
            }
        }
        super::save_managed_agents(app, &records)?;
        if let Some(saved_record) = records.iter().find(|record| record.pubkey == pubkey) {
            super::retain_managed_agent_pending(app, state, saved_record);
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
    let records = super::load_managed_agents(app)?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    super::summarize_from_disk(app, record, &runtimes)
}
