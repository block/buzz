use tauri::AppHandle;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        delete_team_with_cascade, ensure_persona_ids_are_active, load_managed_agents,
        load_personas, load_teams, save_managed_agents, save_teams, try_regenerate_nest,
        CreateTeamRequest, TeamRecord, UpdateTeamRequest,
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
/// instances, in one pass over the store. Runs inside the caller's held
/// `managed_agents_store_lock` after `save_teams`; a persona added/removed on
/// the team otherwise never reaches its live instances.
///
/// Two directions, keyed on the delta between the pre-edit and post-edit
/// rosters:
///
/// - **Added** (on the team now, not before): backfill `team_id` on the
///   persona's *unbound* instances, so an added persona spawns with the team's
///   instructions (`spawn_snapshot::effective_team_instructions` keys on
///   `record.team_id`). Only an unset field is set — a shared persona keeps an
///   existing binding — and an explicit add is legitimate binding evidence even
///   when the persona belongs to several teams.
/// - **Removed** (on the team before, not now): clear `team_id` on instances
///   bound to *this* team, so a "keep agents" removal stops feeding a kept
///   instance the instructions of a team it no longer belongs to. Bindings to
///   other teams are untouched.
///
/// Delta-scoping is what keeps a metadata-only edit inert: with no roster
/// change both sets are empty and no instance is re-pointed — a shared unbound
/// persona is not silently bound to whichever team was last edited. `create_team`
/// has no prior roster, so it passes an empty `previous` and the whole roster is
/// "added" (the pre-fix whole-roster backfill).
fn propagate_team_membership_change(
    app: &AppHandle,
    team_id: &str,
    previous_persona_ids: &[String],
    current_persona_ids: &[String],
) -> Result<(), String> {
    let mut records = load_managed_agents(app)?;
    if apply_team_membership_delta(
        &mut records,
        team_id,
        previous_persona_ids,
        current_persona_ids,
    ) {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

/// Pure core of [`propagate_team_membership_change`]: apply the roster delta to
/// `records` in place and report whether anything changed. Decoupled from the
/// `AppHandle` load/save so the binding rules are unit-testable.
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

/// Retain a freshly authored team event in the local store, flagged for relay
/// sync. Called inside a command's `managed_agents_store_lock`-held body after
/// `save_teams`; the background flush loop publishes it out-of-band.
///
/// Mirrors `commands::personas::retain_persona_pending`. Built-in teams are not
/// owner-authored, so the caller skips them — this helper assumes the team is
/// publishable. Best-effort: a failure here is logged and swallowed so a
/// retention hiccup never blocks the disk-authoritative write.
///
/// Unlike `retain_managed_agent_pending`, this has no projection-equality
/// short-circuit: teams have no start/stop runtime churn, so a republish only
/// happens on an actual user edit. The guard is intentionally omitted.
pub(super) fn retain_team_pending(app: &AppHandle, state: &AppState, team: &TeamRecord) {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        team_events::build_team_event,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let conn = open_retention_db(&scope.db_path)?;
        let pubkey = scope.owner_keys.public_key().to_hex();
        // Monotonic created_at: bump past the retained head (NIP-AP step 3).
        let prior =
            get_retained_event(&conn, KIND_TEAM, &pubkey, &team.id)?.map(|row| row.created_at);
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
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-retain: {e}");
    }
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
fn tombstone_team_pending(app: &AppHandle, state: &AppState, d_tag: &str) {
    use crate::managed_agents::{
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
        team_events::build_team_delete,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let pubkey = scope.owner_keys.public_key().to_hex();
        let event = build_team_delete(d_tag, &pubkey)?
            .sign_with_keys(&scope.owner_keys)
            .map_err(|e| format!("failed to sign team tombstone: {e}"))?;
        let conn = open_retention_db(&scope.db_path)?;
        delete_retained_event(&conn, KIND_TEAM, &pubkey, d_tag)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey,
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
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-tombstone: {e}");
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
        load_teams(&app)
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
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: now.clone(),
            updated_at: now,
        };
        teams.push(team.clone());
        save_teams(&app, &teams)?;
        // Best-effort, like `retain_team_pending` below: the team is already
        // committed to disk (the authoritative mutation), and boot repair is
        // the designed retry for a stale/unset binding — a secondary
        // managed-agents.json write failure must not fail a create whose team
        // now exists, or a UI retry mints a duplicate team. No prior roster, so
        // the whole roster is the added delta.
        if let Err(e) = propagate_team_membership_change(&app, &team.id, &[], &team.persona_ids) {
            eprintln!("buzz-desktop: team-membership-propagate: {e}");
        }
        // Created teams are always non-builtin; publish to the relay.
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
        let personas = load_personas(&app)?;
        ensure_persona_ids_are_active(&personas, &input.persona_ids)?;
        let mut teams = load_teams(&app)?;
        let team = teams
            .iter_mut()
            .find(|record| record.id == input.id)
            .ok_or_else(|| format!("team {} not found", input.id))?;

        // Capture the pre-edit roster before mutation: the propagation delta
        // (added → backfill, removed → detach) is computed against it.
        let previous_persona_ids = team.persona_ids.clone();
        team.name = name;
        team.description = description;
        team.instructions = instructions;
        team.persona_ids = input.persona_ids;
        team.updated_at = now_iso();

        let updated = team.clone();
        save_teams(&app, &teams)?;
        // Best-effort after the authoritative `save_teams`, like create above.
        if let Err(e) = propagate_team_membership_change(
            &app,
            &updated.id,
            &previous_persona_ids,
            &updated.persona_ids,
        ) {
            eprintln!("buzz-desktop: team-membership-propagate: {e}");
        }
        // Built-in teams are not owner-authored — never publish them.
        if !updated.is_builtin {
            retain_team_pending(&app, &state, &updated);
        }
        Ok(updated)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::apply_team_membership_delta;
    use crate::managed_agents::ManagedAgentRecord;

    /// A running instance: `pubkey` set, linked to a persona, optional binding.
    fn instance(seed: char, persona_id: &str, team_id: Option<&str>) -> ManagedAgentRecord {
        let mut record = serde_json::from_value::<ManagedAgentRecord>(serde_json::json!({
            "pubkey": seed.to_string().repeat(64),
            "name": persona_id,
            "persona_id": persona_id,
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "prompt",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        record.team_id = team_id.map(str::to_string);
        record
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A metadata-only edit (no roster change) never re-points an instance —
    /// including an unbound instance of a persona this team shares with another.
    #[test]
    fn metadata_only_edit_leaves_bindings_untouched() {
        let mut records = vec![instance('a', "duncan", None)];
        let roster = ids(&["duncan"]);
        assert!(!apply_team_membership_delta(
            &mut records,
            "team-a",
            &roster,
            &roster
        ));
        assert_eq!(records[0].team_id, None);
    }

    /// Only the *added* persona's unbound instance is bound; an untouched member
    /// already present in the previous roster is not re-pointed.
    #[test]
    fn added_persona_backfills_only_its_unbound_instance() {
        let mut records = vec![
            instance('a', "duncan", None),
            instance('b', "paul", Some("team-b")),
        ];
        assert!(apply_team_membership_delta(
            &mut records,
            "team-a",
            &ids(&["paul"]),
            &ids(&["paul", "duncan"]),
        ));
        assert_eq!(records[0].team_id.as_deref(), Some("team-a"));
        // Paul was already on the team and bound elsewhere — untouched.
        assert_eq!(records[1].team_id.as_deref(), Some("team-b"));
    }

    /// An added persona binds even when shared across teams: an explicit add is
    /// legitimate evidence (unlike the boot-repair's order-blind case).
    #[test]
    fn added_shared_persona_binds_to_the_edited_team() {
        let mut records = vec![instance('a', "duncan", None)];
        assert!(apply_team_membership_delta(
            &mut records,
            "team-a",
            &[],
            &ids(&["duncan"]),
        ));
        assert_eq!(records[0].team_id.as_deref(), Some("team-a"));
    }

    /// Removing a persona ("keep agents") clears its binding to *this* team so a
    /// kept instance stops drawing the team's instructions at spawn.
    #[test]
    fn removed_persona_detaches_instance_bound_to_this_team() {
        let mut records = vec![instance('a', "duncan", Some("team-a"))];
        assert!(apply_team_membership_delta(
            &mut records,
            "team-a",
            &ids(&["duncan"]),
            &[],
        ));
        assert_eq!(records[0].team_id, None);
    }

    /// Removal only clears a binding pointing at *this* team — an instance of
    /// the same persona bound to a different team is left alone.
    #[test]
    fn removed_persona_leaves_other_team_binding_untouched() {
        let mut records = vec![instance('a', "duncan", Some("team-b"))];
        assert!(!apply_team_membership_delta(
            &mut records,
            "team-a",
            &ids(&["duncan"]),
            &[],
        ));
        assert_eq!(records[0].team_id.as_deref(), Some("team-b"));
    }
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
        let cascaded_persona_d_tags = delete_team_with_cascade(&app, &id)?;
        // delete_team_with_cascade rejects built-in teams via validate_team_deletion,
        // so reaching here means this team was owner-published — tombstone it. The
        // d_tag is the team id, captured before the record left the store.
        tombstone_team_pending(&app, &state, &id);
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
