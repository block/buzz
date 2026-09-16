use tauri::AppHandle;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        ensure_persona_ids_are_active, load_managed_agents, load_personas, load_teams,
        save_managed_agents, save_teams, CreateTeamRequest, TeamRecord, UpdateTeamRequest,
    },
    util::now_iso,
};

fn trim_required(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value.and_then(|candidate| {
        let trimmed = candidate.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

/// Propagate a team's membership *change* to its members' already-running
/// instances, best-effort. Loads the agent store, applies the roster delta via
/// [`apply_team_membership_delta`], and re-saves only when something changed;
/// any load/save error is logged and swallowed. Called after the authoritative
/// `save_teams` succeeds — the team already exists on disk and boot repair is
/// the designed retry for a stale/unset binding, so a secondary-store hiccup
/// must not fail a command whose team write already landed (a UI retry would
/// then mint a duplicate team).
///
/// `load_agents`/`save_agents` are injected so the command wiring (prior-roster
/// capture, delta direction, and this best-effort policy) is unit-testable
/// without an `AppHandle`; the commands pass the real store IO.
///
/// Shared with the inbound reconcile path (`commands::personas::inbound`): a
/// 30176 team edit arriving from another device must bind/detach instances the
/// same way a local edit does, so both call this one wrapper.
pub(in crate::commands) fn propagate_membership_best_effort(
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) {
    let result = (|| -> Result<(), String> {
        let mut records = load_agents()?;
        if apply_team_membership_delta(
            &mut records,
            team_id,
            previous_persona_ids,
            current_persona_ids,
        ) {
            save_agents(&records)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-membership-propagate: {e}");
    }
}

/// In-memory core of [`create_team`]: push the built team, persist teams
/// authoritatively, then propagate its whole roster (no prior members ⇒ the
/// whole roster is the added delta) to live instances best-effort. Decoupled
/// from the `AppHandle` shell via injected persistence so the create wiring is
/// unit-testable. A `persist_teams` error propagates; agent IO is best-effort.
fn commit_team_create(
    teams: &mut Vec<TeamRecord>,
    team: TeamRecord,
    persist_teams: impl FnOnce(&[TeamRecord]) -> Result<(), String>,
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) -> Result<TeamRecord, String> {
    teams.push(team.clone());
    persist_teams(teams)?;
    propagate_membership_best_effort(&team.id, &[], &team.persona_ids, load_agents, save_agents);
    Ok(team)
}

/// In-memory core of [`update_team`]: mutate the matching team, capturing its
/// roster *before* the edit, persist teams authoritatively, then propagate the
/// prior→current delta to live instances best-effort. The prior-roster capture
/// and its use as the delta baseline live here — not at a command call site —
/// so a miswire to the wrong baseline is caught by a test. Injected persistence
/// keeps it `AppHandle`-free; a `persist_teams` error propagates, agent IO is
/// best-effort. Returns the updated team.
#[allow(clippy::too_many_arguments)]
fn commit_team_update(
    teams: &mut [TeamRecord],
    id: &str,
    name: String,
    description: Option<String>,
    instructions: Option<String>,
    persona_ids: Vec<String>,
    now: String,
    persist_teams: impl FnOnce(&[TeamRecord]) -> Result<(), String>,
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) -> Result<TeamRecord, String> {
    let team = teams
        .iter_mut()
        .find(|record| record.id == id)
        .ok_or_else(|| format!("team {id} not found"))?;

    // Capture the pre-edit roster before mutation: the propagation delta
    // (added → backfill, removed → detach) is computed against it.
    let previous_persona_ids = team.persona_ids.clone();
    team.name = name;
    team.description = description;
    team.instructions = instructions;
    team.persona_ids = persona_ids;
    team.updated_at = now;

    let updated = team.clone();
    persist_teams(teams)?;
    propagate_membership_best_effort(
        &updated.id,
        &previous_persona_ids,
        &updated.persona_ids,
        load_agents,
        save_agents,
    );
    Ok(updated)
}

/// Pure core of the membership propagation: apply the roster delta to `records`
/// in place and report whether anything changed. Decoupled from the store IO so
/// the binding rules are unit-testable.
///
/// Two directions, keyed on the delta between the pre-edit and post-edit
/// rosters:
///
/// - **Added** (`current` but not `previous`): backfill `team_id` on the
///   persona's *unbound* instances, so an added persona spawns with the team's
///   instructions (`spawn_snapshot::effective_team_instructions` keys on
///   `record.team_id`). Only an unset field is set — a shared persona keeps an
///   existing binding — and an explicit add is legitimate binding evidence even
///   when the persona belongs to several teams.
/// - **Removed** (`previous` but not `current`): clear `team_id` on instances
///   bound to *this* team, so a "keep agents" removal stops feeding a kept
///   instance the instructions of a team it no longer belongs to. Bindings to
///   other teams are untouched.
///
/// Delta-scoping is what keeps a metadata-only edit inert: with no roster
/// change both sets are empty and no instance is re-pointed — a shared unbound
/// persona is not silently bound to whichever team was last edited. `create`
/// has no prior roster, so it passes an empty `previous` and the whole roster is
/// "added" (the pre-fix whole-roster backfill). A persona both removed and
/// re-added in one edit appears in neither set (set difference, not
/// operation order), so its binding is left as-is.
fn apply_team_membership_delta(
    records: &mut [crate::managed_agents::ManagedAgentRecord],
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
) -> bool {
    let added: Vec<&str> = current_persona_ids
        .iter()
        .filter(|id| !previous_persona_ids.iter().any(|p| p == *id))
        .map(String::as_str)
        .collect();
    let removed: Vec<&str> = previous_persona_ids
        .iter()
        .filter(|id| !current_persona_ids.iter().any(|p| p == *id))
        .map(String::as_str)
        .collect();
    if added.is_empty() && removed.is_empty() {
        return false;
    }

    let mut changed = false;
    for record in records.iter_mut() {
        if record.pubkey.is_empty() {
            continue;
        }
        let Some(persona_id) = record.persona_id.as_deref() else {
            continue;
        };
        if record.team_id.is_none() && added.contains(&persona_id) {
            record.team_id = Some(team_id.to_string());
            changed = true;
        } else if record.team_id.as_deref() == Some(team_id) && removed.contains(&persona_id) {
            record.team_id = None;
            changed = true;
        }
    }
    changed
}

mod adopt;
mod pending;
use crate::managed_agents::team_catalog::{finish_catalog_retractions, CatalogRetraction};
pub(crate) use pending::{
    prepare_catalog_delete_in_scope, prepare_catalog_persona_refresh_in_scope,
    prepare_catalog_refresh_in_scope,
};
mod sharing;
pub use adopt::add_team_from_catalog;
pub use sharing::set_team_shared;

/// Refresh the shared 30178 catalog heads of every team that includes
/// `persona_id` as a member, after a successful persona edit.
///
/// `pub(crate)` so persona-edit commands can trigger a catalog refresh without
/// crossing into the `commands::teams` private module. Called under the store
/// lock; the caller MUST carry returned retractions out and await completion.
/// Best-effort failures are logged, not returned.
pub(crate) fn refresh_team_catalog_heads_for_persona<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    persona_id: &str,
) -> Vec<CatalogRetraction> {
    pending::refresh_shared_team_catalog_heads_for_persona(app, state, persona_id)
}

/// Retain a freshly authored team event in the local store, flagged for relay
/// sync. Called inside a command's `managed_agents_store_lock`-held body after
/// `save_teams`; the background flush loop publishes it out-of-band.
///
/// Mirrors `commands::personas::retain_persona_pending`. The caller skips
/// built-in teams, so this assumes the team is publishable. Best-effort: a
/// failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative write.
///
/// Unlike `retain_managed_agent_pending`, no projection-equality short-circuit:
/// teams have no start/stop runtime churn, so a republish only happens on an
/// actual user edit.
pub(super) fn retain_team_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    team: &TeamRecord,
) -> TeamRetentionWork {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    retain_team_pending_at(&scope, team)
}

pub(super) type TeamRetentionWork =
    Result<crate::managed_agents::team_events::publication::PreparedTeamHead, String>;

/// Capture the owner and exact retained head under the caller's store lock.
/// Every caller must carry the plan out and await `finish_team_pending`.
pub(super) fn retain_team_pending_at(
    scope: &crate::managed_agents::retention::RetentionScope,
    team: &TeamRecord,
) -> TeamRetentionWork {
    crate::managed_agents::team_events::publication::prepare_team_head(
        &scope.db_path,
        &scope.owner_signer(),
        team,
    )
}

pub(super) async fn finish_team_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    work: TeamRetentionWork,
) {
    use tauri::Manager;
    let result = async {
        let signed = work?.sign().await?;
        let state = app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        signed.commit(&load_teams(app)?)
    }
    .await;
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-retain: {e}");
    }
}

/// Prepare a team-coordinate deletion under the caller's store lock.
pub(crate) fn tombstone_team_at(
    db_path: &std::path::Path,
    signer: &crate::active_user_signer::ActiveUserSigner,
    d_tag: &str,
) -> Result<super::agents::deletion_witness::AgentTombstone, String> {
    super::agents::deletion_witness::prepare_definition_tombstone(
        db_path,
        signer,
        buzz_core_pkg::kind::KIND_TEAM,
        d_tag,
    )
}

#[tauri::command]
pub async fn list_teams<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation: crate::user_operation::UserOperationScope,
) -> Result<Vec<TeamRecord>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let _admission = operation.admit(&state)?;
        let mut teams = load_teams(&app)?;
        pending::project_active_team_sharing(&app, &state, &mut teams);
        Ok(teams)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_team(input: CreateTeamRequest, app: AppHandle) -> Result<TeamRecord, String> {
    use tauri::Manager;
    let blocking_app = app.clone();
    let (team, work) = tokio::task::spawn_blocking(move || {
        let app = blocking_app;
        let state = app.state::<AppState>();
        let name = trim_required(&input.name, "Team name")?;
        let description = trim_optional(input.description);
        let instructions = trim_optional(input.instructions);
        let now = now_iso();

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let personas = load_personas(&app)?;
        ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
        let mut teams = load_teams(&app)?;
        let team = TeamRecord {
            id: Uuid::new_v4().to_string(),
            name,
            description,
            instructions,
            persona_ids: input.persona_ids,
            is_builtin: false,
            // View projection only — `list_teams` recomputes it from the
            // scoped 30178 head. A new team has no catalog head yet.
            shared: false,
            catalog_source: None,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: now.clone(),
            updated_at: now,
        };
        let team = commit_team_create(
            &mut teams,
            team,
            |teams| save_teams(&app, teams),
            || load_managed_agents(&app),
            |records| save_managed_agents(&app, records),
        )?;
        // Created teams are always non-builtin; publish to the relay.
        let work = retain_team_pending(&app, &state, &team);
        Ok::<_, String>((team, work))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    finish_team_pending(&app, work).await;
    Ok(team)
}

#[tauri::command]
pub async fn update_team(input: UpdateTeamRequest, app: AppHandle) -> Result<TeamRecord, String> {
    update_team_with(input, app).await
}

pub(crate) async fn update_team_with<R: tauri::Runtime>(
    input: UpdateTeamRequest,
    app: AppHandle<R>,
) -> Result<TeamRecord, String> {
    use tauri::Manager;
    let blocking_app = app.clone();
    let (result, work, team_work) = tokio::task::spawn_blocking(move || {
        let app = blocking_app;
        let state = app.state::<AppState>();
        let name = trim_required(&input.name, "Team name")?;
        let description = trim_optional(input.description);
        let instructions = trim_optional(input.instructions);

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let personas = load_personas(&app)?;
        ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
        let mut teams = load_teams(&app)?;
        pending::project_active_team_sharing(&app, &state, &mut teams);
        let updated = commit_team_update(
            &mut teams,
            &input.id,
            name,
            description,
            instructions,
            input.persona_ids,
            now_iso(),
            |teams| save_teams(&app, teams),
            || load_managed_agents(&app),
            |records| save_managed_agents(&app, records),
        )?;
        // Built-in teams are not owner-authored — never publish them.
        let team_work = (!updated.is_builtin).then(|| retain_team_pending(&app, &state, &updated));
        let work = if !updated.is_builtin {
            // Reproject the shared 30178 head immediately so the catalog
            // reflects the edit. Resolution failure (a member was deleted
            // mid-edit) is treated as a projection failure — the shared head
            // is tombstoned and the owner is notified via a typed notice.
            // Best-effort: a retention hiccup never blocks the team edit.
            pending::refresh_shared_team_catalog_head_resolving(&app, &state, &updated, &personas)
        } else {
            Vec::new()
        };
        Ok::<_, String>((updated, work, team_work))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    if let Some(work) = team_work {
        finish_team_pending(&app, work).await;
    }
    finish_catalog_retractions(&app, work).await;
    Ok(result)
}

#[tauri::command]
pub async fn delete_team<R: tauri::Runtime>(id: String, app: AppHandle<R>) -> Result<(), String> {
    deletion::delete_team_with(id, app).await
}

mod deletion;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod signer_tests;
