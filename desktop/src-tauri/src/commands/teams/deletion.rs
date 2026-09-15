//! Direct team deletion signs every witness before touching its cascade.
use super::*;
use crate::commands::agents::deletion_witness::{
    commit_agent_deletions, prepare_definition_tombstone, sign_agent_deletion,
};
use crate::managed_agents::{
    delete_team_with_cascade, persona_events::persona_d_tag, team_persona_key, try_regenerate_nest,
    validate_team_deletion,
};
use tauri::Manager;

// Fence only the target and the inputs consulted by its cascade. In particular,
// JSON-only teams do not depend on unrelated persona or agent runtime bytes.
fn inputs<R: tauri::Runtime>(app: &AppHandle<R>, id: &str) -> Result<serde_json::Value, String> {
    let teams = load_teams(app)?;
    let team = teams
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("team {id} not found"))?;
    let key = team_persona_key(team);
    let agents = load_managed_agents(app)?;
    let references: Vec<_> = agents
        .iter()
        .filter(|a| {
            a.team_id.as_deref() == Some(id)
                || a.persona_team_dir
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    == Some(key)
        })
        .collect();
    let mut cascade = Vec::new();
    let mut team_refs = Vec::new();
    let mut agent_refs = Vec::new();
    if team.source_dir.is_some() {
        cascade = load_personas(app)?
            .into_iter()
            .filter(|p| p.source_team.as_deref() == Some(key))
            .collect();
    } else if let Some(source) = &team.catalog_source {
        cascade = load_personas(app)?
            .into_iter()
            .filter(|p| {
                !p.is_builtin
                    && p.is_active
                    && p.team_catalog_source.as_ref().is_some_and(|s| {
                        s.owner_pubkey == source.owner_pubkey && s.team_d_tag == source.team_d_tag
                    })
            })
            .collect();
        for other in teams.iter().filter(|t| t.id != id) {
            let relevant: Vec<_> = other
                .persona_ids
                .iter()
                .filter(|id| cascade.iter().any(|p| &p.id == *id))
                .collect();
            if !relevant.is_empty() {
                team_refs.push((&other.id, relevant));
            }
        }
        for agent in &agents {
            if agent
                .persona_id
                .as_ref()
                .is_some_and(|id| cascade.iter().any(|p| &p.id == id))
            {
                agent_refs.push((&agent.pubkey, &agent.persona_id));
            }
        }
    }
    serde_json::to_value((team, cascade, references, team_refs, agent_refs))
        .map_err(|e| e.to_string())
}

pub(super) async fn delete_team_with<R: tauri::Runtime>(
    id: String,
    app: AppHandle<R>,
) -> Result<(), String> {
    crate::owner_authorization::require_owned_workspace(&app.state::<AppState>())?;
    let (disk, work) = {
        let state = app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let teams = load_teams(&app)?;
        let team = teams
            .iter()
            .find(|team| team.id == id)
            .ok_or_else(|| format!("team {id} not found"))?;
        validate_team_deletion(team)?;
        let personas = load_personas(&app)?;
        let disk = inputs(&app, &id)?;
        let scope = crate::managed_agents::retention::active_retention_scope(&app, &state);
        let mut coordinates = vec![
            (buzz_core_pkg::kind::KIND_TEAM, id.clone()),
            (buzz_core_pkg::kind::KIND_TEAM_CATALOG, id.clone()),
        ];
        if team.source_dir.is_some() {
            let key = team_persona_key(team);
            coordinates.extend(
                personas
                    .iter()
                    .filter(|p| p.source_team.as_deref() == Some(key))
                    .map(|p| (buzz_core_pkg::kind::KIND_PERSONA, persona_d_tag(p))),
            );
        }
        let work = coordinates
            .into_iter()
            .map(|(kind, tag)| {
                let scope = scope.as_ref().map_err(Clone::clone)?;
                let signer = scope.owner_signer();
                let plan = match kind {
                    buzz_core_pkg::kind::KIND_TEAM => {
                        tombstone_team_at(&scope.db_path, &signer, &tag)
                    }
                    buzz_core_pkg::kind::KIND_PERSONA => {
                        crate::commands::tombstone_persona_at(&scope.db_path, &signer, &tag)
                    }
                    _ => prepare_definition_tombstone(&scope.db_path, &signer, kind, &tag),
                }?;
                Ok((plan, signer))
            })
            .collect::<Vec<_>>();
        (disk, work)
    };
    let mut witnesses = Vec::new();
    for work in work {
        witnesses.push(sign_agent_deletion(work).await);
    }
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        if inputs(&app, &id)? != disk {
            return Err(
                "team deletion conflict: disk/cascade inputs changed; retry deletion".into(),
            );
        }
        commit_agent_deletions(witnesses, || delete_team_with_cascade(&app, &id))?;
        try_regenerate_nest(&app);
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
