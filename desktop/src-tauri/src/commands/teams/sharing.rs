//! The `set_team_shared` command: publish a team's kind:30178 catalog head,
//! or replace it with an untagged head to unshare.
//!
//! Reuses the persona sharing shape (`commands::personas::sharing`): same
//! strict `prepare → submit → mark_synced` path, same `published | queued`
//! contract, same rule that a relay rejection or unreachable relay leaves the
//! head durably queued for the flush loop rather than failing the command.
//! Only the projection input is new — a team plus its ordered members.

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_personas, load_teams,
        retention::{get_retained_event, open_retention_db},
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
}

/// Share a team to the community catalog, or retract it from discovery.
///
/// Unsharing publishes a NEWER, still-valid 30178 head WITHOUT the `shared`
/// tag rather than deleting the coordinate. The relay's read gate keys off the
/// tag, so the untagged head is invisible to the community while remaining
/// readable by its author — which lets a later re-share replace it
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

/// Publish the retained head through the serialized flush publisher, then
/// report whether the relay accepted it.
///
/// This must NOT submit the prepared event directly. A direct submit runs
/// outside `managed_agents_store_lock` and races `delete_team`: the delete
/// atomically purges this head's retained row and enqueues a newer 30178
/// tombstone (`tombstone_team_catalog_coordinate`, one `BEGIN IMMEDIATE`), and
/// a delayed direct submit could land the old shared head *after* the
/// tombstone — and 30178 replacement has no deletion watermark, so the deleted
/// team would be publicly live again with no local retry witness.
///
/// `flush_pending_events_at` closes the race on two counts. It re-reads each
/// row immediately before publishing, so once the delete's transaction has
/// committed this head's row is gone and the flush skips it. And it holds the
/// per-scope publisher lock (keyed by the retention db_path) across its entire
/// invocation, so no *second* flush of the same scope can publish the tombstone
/// in the await gap between this flush's re-read and its POST. Serialized flush
/// ⟹ the only interleavings are head-before-tombstone (head lands first, then
/// dominated by the later tombstone) or purged-row-skip (delete committed
/// first, so the re-read skips the head) — a purged head can never publish
/// after its tombstone. The lock is scope-keyed, not process-wide, so a stalled
/// relay in another community never blocks this toggle, and each relay await is
/// bounded so a non-responding relay releases the lock rather than pinning it.
async fn publish_prepared_team(
    state: &AppState,
    prepared: PreparedTeamPublication,
) -> Result<SetTeamSharedResult, String> {
    let scope = &prepared.scope;
    // Best-effort: the head is already durably retained (pending) under the
    // store lock, so a flush hiccup leaves it queued rather than failing the
    // toggle. A relay rejection is swallowed by the flush loop's own log, and a
    // local DB fault surfaces through the status re-read below, so the flush's
    // own error carries nothing this command must report.
    let _ = crate::managed_agents::persona_events::flush_pending_events_at(
        &scope.db_path,
        state,
        &scope.relay_url,
        &scope.owner_keys,
    )
    .await;

    // Re-read the row the flush just processed. A concurrent delete may have
    // purged it between the flush and here; an absent row means the team is
    // being (or has been) deleted and nothing published, so Queued is the
    // honest answer.
    let conn = open_retention_db(&scope.db_path)?;
    let published = get_retained_event(
        &conn,
        prepared.retained.kind,
        &prepared.retained.pubkey,
        &prepared.retained.d_tag,
    )?
    .is_some_and(|row| !row.pending_sync);

    let publication_status = if published {
        TeamSharePublicationStatus::Published
    } else {
        TeamSharePublicationStatus::Queued
    };
    Ok(SetTeamSharedResult {
        team: prepared.team,
        publication_status,
    })
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests;
