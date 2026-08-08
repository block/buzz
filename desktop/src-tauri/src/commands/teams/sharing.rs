//! The `set_team_shared` command: publish a team's kind:30178 catalog head,
//! or replace it with an untagged head to unshare.
//!
//! Delegates wholesale to the persona sharing machinery's shape
//! (`commands::personas::sharing`): the same strict `prepare → submit →
//! mark_synced` path, the same `published | queued` contract, the same rule
//! that a relay rejection or an unreachable relay leaves the head durably
//! queued for the flush loop rather than failing the command. What is new
//! here is only the projection input — a team plus its ordered members.

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas, load_teams,
        retention::{mark_synced, open_retention_db},
        TeamRecord,
    },
};

use super::pending::{prepare_team_publication, PreparedTeamPublication};
use crate::managed_agents::team_catalog::resolve_team_members;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TeamSharePublicationStatus {
    Published,
    Queued,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTeamSharedResult {
    pub team: TeamRecord,
    pub publication_status: TeamSharePublicationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_message: Option<String>,
}

/// Share a team to the community catalog, or retract it from discovery.
///
/// Unsharing publishes a NEWER, still-valid 30178 head WITHOUT the `shared`
/// tag rather than deleting the coordinate. The relay's read gate keys off the
/// tag, so the untagged head is invisible to the community while remaining
/// readable by its author — which is what lets a later re-share replace it
/// monotonically instead of racing a tombstone. Deletion is reserved for
/// deleting the team itself (`delete_team`).
#[tauri::command]
pub async fn set_team_shared(
    id: String,
    shared: bool,
    app: AppHandle,
) -> Result<SetTeamSharedResult, String> {
    let prepared = tokio::task::spawn_blocking({
        let app = app.clone();
        move || {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let teams = load_teams(&app)?;
            let team = teams
                .iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("team {id} not found"))?;

            if team.is_builtin {
                return Err("Built-in teams cannot be shared to the catalog.".to_string());
            }

            let members = resolve_team_members(team, &load_personas(&app)?)?;
            // Strict path: unlike ordinary team saves, an enqueue failure for
            // this privacy-sensitive toggle must reach the command/UI.
            prepare_team_publication(&app, &state, team, &members, Some(shared))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    let state = app.state::<AppState>();
    publish_prepared_team(&state, prepared).await
}

async fn publish_prepared_team(
    state: &AppState,
    prepared: PreparedTeamPublication,
) -> Result<SetTeamSharedResult, String> {
    let api_base_url = crate::relay::relay_http_base_url(&prepared.scope.relay_url);
    let publish_result = crate::relay::submit_signed_event_at_with_keys(
        &prepared.event,
        state,
        &api_base_url,
        &prepared.scope.owner_keys,
    )
    .await;

    match publish_result {
        Ok(_) => {
            let conn = open_retention_db(&prepared.scope.db_path)?;
            mark_synced(
                &conn,
                prepared.retained.kind,
                &prepared.retained.pubkey,
                &prepared.retained.d_tag,
                prepared.retained.created_at,
                &prepared.retained.content,
            )?;
            Ok(SetTeamSharedResult {
                team: prepared.team,
                publication_status: TeamSharePublicationStatus::Published,
                relay_message: None,
            })
        }
        Err(error) => Ok(SetTeamSharedResult {
            team: prepared.team,
            publication_status: TeamSharePublicationStatus::Queued,
            relay_message: Some(error),
        }),
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests;
