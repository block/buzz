use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        delete_team_with_cascade, ensure_persona_ids_are_active, load_managed_agents,
        load_personas, load_teams, save_managed_agents, save_teams, try_regenerate_nest,
        AgentDefinition, CreateTeamRequest, TeamRecord, UpdateTeamRequest,
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

/// A staged team membership change. The record persists before the team and
/// agent stores change so a later save or launch can replay the original delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::commands) struct PendingTeamMembershipUpdate {
    pub(in crate::commands) team_id: String,
    pub(in crate::commands) previous_persona_ids: Vec<String>,
    pub(in crate::commands) current_persona_ids: Vec<String>,
}

fn pending_team_membership_path<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(crate::managed_agents::managed_agents_base_dir(app)?.join("pending-team-membership.json"))
}

pub(in crate::commands) fn save_pending_team_membership_at(
    path: &std::path::Path,
    pending: Option<&PendingTeamMembershipUpdate>,
) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(&pending)
        .map_err(|error| format!("failed to serialize pending team update: {error}"))?;
    crate::managed_agents::storage::atomic_write_json(path, &payload)
}

pub(in crate::commands) fn load_pending_team_membership_at(
    path: &std::path::Path,
) -> Result<Option<PendingTeamMembershipUpdate>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let payload = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read pending team update: {error}"))?;
    serde_json::from_str(&payload)
        .map_err(|error| format!("failed to parse pending team update: {error}"))
}

pub(in crate::commands) fn save_pending_team_membership<R: tauri::Runtime>(
    app: &AppHandle<R>,
    pending: &PendingTeamMembershipUpdate,
) -> Result<(), String> {
    save_pending_team_membership_at(&pending_team_membership_path(app)?, Some(pending))
}

fn load_pending_team_membership<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<Option<PendingTeamMembershipUpdate>, String> {
    load_pending_team_membership_at(&pending_team_membership_path(app)?)
}

pub(in crate::commands) fn clear_pending_team_membership<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    // Write `null` through the link instead of unlinking it. The pending file
    // is shared by dev worktrees, and `atomic_write_json` preserves the link.
    save_pending_team_membership_at(&pending_team_membership_path(app)?, None)
}

/// The part of a staged delta that still agrees with the current team roster.
///
/// An inbound event can extend or reorder the roster while a local agent-store
/// write is pending. It does not erase the local add or removal evidence that
/// still holds. An inbound reversal does erase that evidence, so the replay
/// leaves that membership direction alone.
fn pending_replay_delta(
    pending: &PendingTeamMembershipUpdate,
    current_persona_ids: &[String],
) -> (Vec<String>, Vec<String>) {
    let removed = pending
        .previous_persona_ids
        .iter()
        .filter(|id| {
            !pending.current_persona_ids.contains(*id) && !current_persona_ids.contains(*id)
        })
        .cloned()
        .collect();
    let added = pending
        .current_persona_ids
        .iter()
        .filter(|id| {
            !pending.previous_persona_ids.contains(*id) && current_persona_ids.contains(*id)
        })
        .cloned()
        .collect();
    (removed, added)
}

/// Replay a staged membership delta. Callers hold `managed_agents_store_lock`.
pub(crate) fn replay_pending_team_membership<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let Some(pending) = load_pending_team_membership(app)? else {
        return Ok(());
    };
    let teams = load_teams(app)?;
    let Some(team) = teams.iter().find(|team| team.id == pending.team_id) else {
        eprintln!(
            "buzz-desktop: pending-team-membership: discarding staged update for missing team {:?}",
            pending.team_id
        );
        return clear_pending_team_membership(app);
    };
    let (previous_persona_ids, current_persona_ids) =
        pending_replay_delta(&pending, &team.persona_ids);
    propagate_membership_with_roster(
        &pending.team_id,
        &previous_persona_ids,
        &current_persona_ids,
        &team.persona_ids,
        || load_managed_agents(app),
        |records| save_managed_agents(app, records),
    )
    .map_err(|error| format!("could not replay pending team update: {error}"))?;
    clear_pending_team_membership(app)
}

/// Clear `team_id` on every instance that is bound to `team_id` but whose
/// persona is absent from `current_persona_ids` or unset. Reports whether
/// anything changed.
///
/// This reads the current roster, not a delta. That is what makes a failed
/// detach recoverable: after a failed agent-store write the team is already
/// saved with the new roster, so the prior→current delta is empty on the next
/// save and a delta-only pass would do nothing. The invariant here is
/// state-based — an instance bound to a team must have its persona on that
/// team's roster — so a second save still repairs the binding.
///
/// Bindings to other teams are untouched, and an unbound instance is left to
/// the delta's backfill branch.
fn detach_agents_outside_roster(
    records: &mut [crate::managed_agents::ManagedAgentRecord],
    team_id: &str,
    current_persona_ids: &[String],
) -> bool {
    let mut changed = false;
    for record in records.iter_mut() {
        if record.pubkey.is_empty() || record.team_id.as_deref() != Some(team_id) {
            continue;
        }
        if !current_persona_ids
            .iter()
            .any(|id| record.persona_id.as_deref() == Some(id))
        {
            record.team_id = None;
            changed = true;
        }
    }
    changed
}

/// Reports a membership propagation failure.
#[derive(Debug)]
pub(in crate::commands) enum MembershipPropagationError {
    Load(String),
    Save(String),
}

impl std::fmt::Display for MembershipPropagationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(error) | Self::Save(error) => formatter.write_str(error),
        }
    }
}

/// Propagate a team's membership to its members' already-running instances.
/// Loads the agent store, applies the roster delta via
/// [`apply_team_membership_delta`], reconciles stale bindings via
/// [`detach_agents_outside_roster`], and re-saves only when something changed.
/// Reports the failure type so the caller can choose its error policy.
pub(in crate::commands) fn propagate_membership(
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), MembershipPropagationError> {
    propagate_membership_with_roster(
        team_id,
        previous_persona_ids,
        current_persona_ids,
        current_persona_ids,
        load_agents,
        save_agents,
    )
}

/// Apply a membership delta, then reconcile bindings with the authoritative
/// roster. Replay uses a smaller delta when an inbound edit has changed the
/// roster after staging, while the reconciliation always uses that latest roster.
fn propagate_membership_with_roster(
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
    authoritative_persona_ids: &[String],
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), MembershipPropagationError> {
    let mut records = load_agents().map_err(MembershipPropagationError::Load)?;
    let delta_changed = apply_team_membership_delta(
        &mut records,
        team_id,
        previous_persona_ids,
        current_persona_ids,
    );
    let detached = detach_agents_outside_roster(&mut records, team_id, authoritative_persona_ids);
    if delta_changed || detached {
        save_agents(&records).map_err(MembershipPropagationError::Save)?;
    }
    Ok(())
}

/// Apply a membership change without failing a metadata-only producer. The
/// caller must use [`propagate_membership`] when a roster change needs durable
/// recovery.
pub(in crate::commands) fn propagate_membership_best_effort(
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
) {
    if let Err(error) = propagate_membership(
        team_id,
        previous_persona_ids,
        current_persona_ids,
        load_agents,
        save_agents,
    ) {
        eprintln!("buzz-desktop: team-membership-propagate: {error}");
    }
}

/// In-memory core of [`create_team`]: stage, persist, and bind the new team's
/// full roster to already-running instances. The function clears the stage
/// only after both stores succeed. A failure returns an error because a success
/// response must mean the membership binding is durable.
fn commit_team_create(
    teams: &mut Vec<TeamRecord>,
    team: TeamRecord,
    save_pending: impl FnOnce(&PendingTeamMembershipUpdate) -> Result<(), String>,
    persist_teams: impl FnOnce(&[TeamRecord]) -> Result<(), String>,
    load_agents: impl FnOnce() -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[crate::managed_agents::ManagedAgentRecord]) -> Result<(), String>,
    clear_pending: impl FnOnce() -> Result<(), String>,
) -> Result<TeamRecord, String> {
    let membership_changed = !team.persona_ids.is_empty();
    if membership_changed {
        save_pending(&PendingTeamMembershipUpdate {
            team_id: team.id.clone(),
            previous_persona_ids: Vec::new(),
            current_persona_ids: team.persona_ids.clone(),
        })?;
    }
    teams.push(team.clone());
    persist_teams(teams)?;
    if membership_changed {
        propagate_membership(&team.id, &[], &team.persona_ids, load_agents, save_agents)
            .map_err(|error| format!("could not update the new team's agents: {error}"))?;
        clear_pending()?;
    }
    Ok(team)
}

/// In-memory core of [`update_team`]: mutate the matching team, capturing its
/// roster *before* the edit, persist teams authoritatively, then propagate the
/// prior→current delta to live instances. The prior-roster capture
/// and its use as the delta baseline live here — not at a command call site —
/// so a miswire to the wrong baseline is caught by a test. Injected persistence
/// keeps it `AppHandle`-free; a `persist_teams` error propagates. Returns the
/// updated team.
///
/// Keeps the team roster and instance bindings recoverable across two stores.
/// It stages the prior→current delta before it writes either store. A failed
/// agent write leaves the stage file in place. The next update or launch replays
/// that original delta before it accepts another team edit.
///
/// A create also stages its full roster before it writes the team. A retry
/// replays that stage, so the command never reports a lost member binding.
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
    let membership_changed = previous_persona_ids != updated.persona_ids;
    persist_teams(teams)?;
    if let Err(error) = propagate_membership(
        &updated.id,
        &previous_persona_ids,
        &updated.persona_ids,
        load_agents,
        save_agents,
    ) {
        if membership_changed || matches!(error, MembershipPropagationError::Save(_)) {
            return Err(format!(
                "Saved the team, but could not update its agents: {error}. Save the team again."
            ));
        }
        eprintln!("buzz-desktop: team-membership-propagate: {error}");
    }
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
mod sharing;
pub use adopt::add_team_from_catalog;
pub use sharing::set_team_shared;

/// Refresh the shared 30178 catalog heads of every team that includes
/// `persona_id` as a member, after a successful persona edit.
///
/// `pub(crate)` so persona-edit commands can trigger a catalog refresh without
/// crossing into the `commands::teams` private module. Best-effort: failures
/// are logged, not returned.
pub(crate) fn refresh_team_catalog_heads_for_persona<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    persona_id: &str,
) {
    pending::refresh_shared_team_catalog_heads_for_persona(app, state, persona_id);
}

/// Refresh (or retract) one team's shared 30178 catalog head after an inbound
/// 30176 team edit landed on this device.
///
/// `pub(crate)` so the inbound reconcile can converge the catalog without
/// reaching into the private `commands::teams` module. Best-effort: failures
/// are logged, not returned. The idempotency skip inside the refresh makes this
/// a no-op when the editing device already published the identical head.
pub(crate) fn refresh_team_catalog_head<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) {
    pending::refresh_shared_team_catalog_head_resolving(app, state, team, personas);
}

/// Purge and tombstone a team's 30178 catalog coordinate after an inbound
/// 30176 team tombstone removed the team on this device.
///
/// `pub(crate)` for the inbound reconcile. Best-effort: the catalog head is a
/// separate coordinate from the 30176 team head, so a team tombstone does not
/// retract it — this closes that gap on the receiving device.
pub(crate) fn tombstone_team_catalog_head<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    d_tag: &str,
) {
    pending::tombstone_team_catalog_pending(app, state, d_tag);
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
pub(super) fn retain_team_pending(app: &AppHandle, state: &AppState, team: &TeamRecord) {
    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        retain_team_pending_at(&scope, team)
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-retain: {e}");
    }
}

/// Scope-level team retention: sign and durably enqueue a team head in an
/// already-resolved retention scope. Team adoption resolves the scope once for
/// its batch and calls this alongside [`personas::retain_persona_pending_at`];
/// [`retain_team_pending`] is the `AppHandle` wrapper for single writes.
pub(super) fn retain_team_pending_at(
    scope: &crate::managed_agents::retention::RetentionScope,
    team: &TeamRecord,
) -> Result<(), String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        team_events::build_team_event,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    let conn = open_retention_db(&scope.db_path)?;
    let pubkey = scope.owner_keys.public_key().to_hex();
    // Monotonic created_at: bump past the retained head (NIP-AP step 3).
    let prior = get_retained_event(&conn, KIND_TEAM, &pubkey, &team.id)?.map(|row| row.created_at);
    let event = build_team_event(team)?
        .custom_created_at(monotonic_created_at(prior))
        .sign_with_keys(&scope.owner_keys)
        .map_err(|e| format!("failed to sign team event: {e}"))?;
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM,
            pubkey,
            d_tag: team.id.clone(),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )
}

/// Purge a deleted team's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body.
///
/// Mirrors `commands::personas::tombstone_persona_pending`: the team row is
/// purged first so an unpublished edit can never resurrect it after the
/// tombstone publishes, then the kind:5 tombstone is retained at its own
/// `(5, pubkey, d_tag)` coordinate with `pending_sync = 1`. Best-effort: a
/// failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative delete.
///
/// Timestamp-domination invariant: the retained 30176 head may be future-dated
/// (`retain_team_pending` signs it with `monotonic_created_at`), and the relay
/// only soft-deletes coordinate versions with `created_at <=` the tombstone's.
/// So the kind:5 is signed with `monotonic_created_at(Some(head.created_at))` —
/// the head's `created_at` read before the purge — so a future-dated head cannot
/// survive its own tombstone. Without a head, fall back to
/// `monotonic_created_at(None)`.
fn tombstone_team_pending(app: &AppHandle, state: &AppState, d_tag: &str) {
    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        tombstone_team_at(&scope.db_path, &scope.owner_keys, d_tag)
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-tombstone: {e}");
    }
}

/// Scope-free core of [`tombstone_team_pending`], so the purge and enqueue can
/// be asserted directly against a retention database (mirrors
/// `pending::tombstone_team_catalog_at` for the 30178 coordinate).
pub(crate) fn tombstone_team_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    d_tag: &str,
) -> Result<(), String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{
            delete_retained_event, get_retained_event, open_retention_db, retain_event,
            tombstone_retention_d_tag, RetainedEvent,
        },
        team_events::build_team_delete,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let pubkey = keys.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    // Single transaction: a kill between the head purge and the tombstone
    // enqueue would otherwise leave the 30176 head shared with no local retry
    // witness. Reading the head's `created_at` inside the same `BEGIN
    // IMMEDIATE` also closes the read-then-sign race — no concurrent writer can
    // bump the head between the read and the purge. Mirrors
    // `team_catalog::tombstone_team_catalog_coordinate` for the 30178
    // coordinate; the two cannot share one helper because they target distinct
    // kinds and builders.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("failed to begin team tombstone transaction: {e}"))?;
    let result = (|| -> Result<(), String> {
        // Read the retained head's created_at inside the transaction, then sign
        // the kind:5 strictly past it so the relay cannot reject the deletion.
        let prior_head =
            get_retained_event(&conn, KIND_TEAM, &pubkey, d_tag)?.map(|row| row.created_at);
        let event = build_team_delete(d_tag, &pubkey)?
            .custom_created_at(monotonic_created_at(prior_head))
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign team tombstone: {e}"))?;
        delete_retained_event(&conn, KIND_TEAM, &pubkey, d_tag)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey: pubkey.clone(),
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_TEAM, d_tag),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("failed to commit team tombstone transaction: {e}")),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn list_teams(app: AppHandle) -> Result<Vec<TeamRecord>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let name = trim_required(&input.name, "Team name")?;
        let description = trim_optional(input.description);
        let instructions = trim_optional(input.instructions);
        let now = now_iso();

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        replay_pending_team_membership(&app)?;
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
            |pending| save_pending_team_membership(&app, pending),
            |teams| save_teams(&app, teams),
            || load_managed_agents(&app),
            |records| save_managed_agents(&app, records),
            || clear_pending_team_membership(&app),
        )?;
        // Created teams are always non-builtin. Retain only after the agent
        // binding is durable, so a remote device cannot consume lost intent.
        retain_team_pending(&app, &state, &team);
        Ok(team)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn update_team(input: UpdateTeamRequest, app: AppHandle) -> Result<TeamRecord, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let name = trim_required(&input.name, "Team name")?;
        let description = trim_optional(input.description);
        let instructions = trim_optional(input.instructions);

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        replay_pending_team_membership(&app)?;
        let personas = load_personas(&app)?;
        ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
        let mut teams = load_teams(&app)?;
        let previous_persona_ids = teams
            .iter()
            .find(|team| team.id == input.id)
            .ok_or_else(|| format!("team {} not found", input.id))?
            .persona_ids
            .clone();
        let membership_changed = previous_persona_ids != input.persona_ids;
        if membership_changed {
            save_pending_team_membership(
                &app,
                &PendingTeamMembershipUpdate {
                    team_id: input.id.clone(),
                    previous_persona_ids,
                    current_persona_ids: input.persona_ids.clone(),
                },
            )?;
        }
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
        if membership_changed {
            clear_pending_team_membership(&app)?;
        }
        // Built-in teams are not owner-authored — never publish them.
        if !updated.is_builtin {
            retain_team_pending(&app, &state, &updated);
            // Reproject the shared 30178 head immediately so the catalog
            // reflects the edit. Resolution failure (a member was deleted
            // mid-edit) is treated as a projection failure — the shared head
            // is tombstoned and the owner is notified via a typed notice.
            // Best-effort: a retention hiccup never blocks the team edit.
            pending::refresh_shared_team_catalog_head_resolving(&app, &state, &updated, &personas);
        }
        Ok(updated)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn delete_team(id: String, app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        replay_pending_team_membership(&app)?;
        let cascaded_persona_d_tags = delete_team_with_cascade(&app, &id)?;
        // delete_team_with_cascade rejects built-in teams via validate_team_deletion,
        // so reaching here means this team was owner-published — tombstone it. The
        // d_tag is the team id, captured before the record left the store.
        tombstone_team_pending(&app, &state, &id);
        // The catalog projection is a separate coordinate with its own
        // retained head, so the 30176 tombstone above does not retract it.
        // Without this, deleting a shared team would leave a live catalog
        // entry the owner can no longer see or unshare.
        pending::tombstone_team_catalog_pending(&app, &state, &id);
        // Tombstone the cascaded personas too, so their orphaned kind:30175 heads
        // don't linger on the relay (F4). Each d-tag was captured pre-removal.
        for persona_d_tag in &cascaded_persona_d_tags {
            super::personas::tombstone_persona_pending(&app, &state, persona_d_tag);
        }
        try_regenerate_nest(&app);
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests;
