//! Boot-time disk→relay event reconcile ("event sync").
//!
//! Reconciles the on-disk JSON stores (`personas.json`, `teams.json`,
//! `managed-agents.json`) into signed retention events queued for relay
//! publish. Runs after identity resolution (event signing needs the owner
//! keys), unlike the pre-identity migrations in [`crate::migration`].

use std::path::Path;

/// Reconcile personas, teams, and managed agents into signed retention
/// events. All readers consume the already-synced
/// `personas.json`/`teams.json`/`managed-agents.json` that
/// `sync_team_personas` wrote in [`crate::migration::run_boot_migrations`]
/// (see its `# Ordering` guard). Event signing needs the resolved owner keys,
/// so this runs after identity resolution, not in the boot migrations.
pub fn run_event_sync(app: &tauri::AppHandle, owner_keys: &nostr::Keys) {
    migrate_personas_to_events(app, owner_keys);
    migrate_teams_to_events(app, owner_keys);
    crate::managed_agents::reconcile::reconcile_agents_to_events(app, owner_keys);
}

/// Spawn the best-effort event reconcile off the synchronous Tauri setup path.
///
/// The owner keys are cloned before spawning so the task never touches the
/// `AppState::keys` mutex. The reconcile itself is still synchronous JSON,
/// SQLite, and signing work, so it runs on the blocking pool rather than an
/// async worker.
pub fn spawn_event_sync(app: tauri::AppHandle, owner_keys: nostr::Keys) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = tauri::async_runtime::spawn_blocking(move || {
            run_event_sync(&app, &owner_keys);
        })
        .await
        {
            eprintln!("buzz-desktop: event-sync: spawn_blocking failed: {e}");
        }
    });
}

/// Reconcile `personas.json` into the persona-event retention store.
///
/// Must run AFTER `migrate_packs_to_teams` (depends on field renames being
/// complete) and AFTER the persisted identity is resolved (it signs every
/// retained event with the owner's keys).
///
/// Per-record reconcile: for each non-builtin persona it compares the freshly
/// serialized event content against the retained row at the same coordinate
/// and re-retains (marking `pending_sync = 1`) only when the row is absent or
/// its content differs. An unchanged persona is left untouched, so a launch
/// after a no-op edit does not churn `pending_sync`; a persona added or edited
/// on disk between launches is picked up and republished. There is no
/// whole-store sentinel — comparing per coordinate is what lets newly added
/// personas reach the relay.
///
/// Strategy: write to local SQLite retention first (durable copy), mark as
/// `pending_sync = 1` for later relay publish. Migration succeeds on local
/// write, not relay acknowledgment. Every retained row is a real signed
/// event — there is no placeholder path.
pub fn migrate_personas_to_events(app: &tauri::AppHandle, keys: &nostr::Keys) {
    use crate::managed_agents::managed_agents_base_dir;

    let Ok(base_dir) = managed_agents_base_dir(app) else {
        return;
    };

    match migrate_personas_in_dir(&base_dir, keys) {
        Ok(0) => {}
        Ok(migrated) => {
            eprintln!(
                "buzz-desktop: persona-event-migration: {migrated} personas migrated to retention"
            );
        }
        Err(e) => {
            eprintln!("buzz-desktop: persona-event-migration: {e}");
        }
    }
}

/// Core reconcile logic, decoupled from the Tauri `AppHandle` for testing.
///
/// Returns the number of personas (re)written to the retention store. Returns
/// `Ok(0)` when every non-builtin persona already has a matching retained row
/// (or there are none to reconcile).
fn migrate_personas_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    use crate::managed_agents::{
        persona_events::{build_persona_event, monotonic_created_at, persona_d_tag},
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        PersonaRecord,
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();

    // Post-fold (Phase 1A.2): definitions live as key-less records in the
    // unified agent store, presented in the legacy shape. Pre-fold boots
    // (run_event_sync runs after run_boot_migrations, so the fold has
    // already happened) never reach this path with personas.json present —
    // but read it as a fallback for one release in case the fold errored.
    let records: Vec<PersonaRecord> = {
        let personas_path = base_dir.join("personas.json");
        if personas_path.exists() {
            let content = std::fs::read_to_string(&personas_path)
                .map_err(|e| format!("failed to read personas.json: {e}"))?;
            serde_json::from_str(&content)
                .map_err(|e| format!("failed to parse personas.json: {e}"))?
        } else {
            let agents_path = base_dir.join("managed-agents.json");
            if !agents_path.exists() {
                return Ok(0);
            }
            let content = std::fs::read_to_string(&agents_path)
                .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
            let all: Vec<crate::managed_agents::ManagedAgentRecord> =
                serde_json::from_str(&content)
                    .map_err(|e| format!("failed to parse managed-agents.json: {e}"))?;
            all.iter()
                .filter(|record| record.pubkey.is_empty())
                .filter_map(|record| record.to_persona_view())
                .collect()
        }
    };

    if records.is_empty() {
        return Ok(0);
    }

    // Open (or create) the retention database.
    let db_path = base_dir.join("retention.db");
    let conn =
        open_retention_db(&db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

    let mut migrated = 0u32;

    for record in &records {
        // Skip built-in personas — they're always available from code.
        if record.is_builtin {
            continue;
        }

        let d_tag = persona_d_tag(record);

        // Fetch the retained head first so the rebuilt event can supersede it:
        // build at the default `now` and a future-dated head (clock skew, or an
        // interactive same-second `max(now, head+1)` bump) would make
        // `retain_event`'s `created_at >= ...` guard SILENTLY skip the UPDATE
        // while `migrated` over-reports. Mirror the interactive sites' monotonic
        // bump (F1) so a changed body always lands.
        let existing = get_retained_event(&conn, KIND_PERSONA, &pubkey, &d_tag)?;

        let event = build_persona_event(record)
            .map_err(|e| format!("failed to build event for '{}': {e}", record.display_name))?
            .custom_created_at(monotonic_created_at(
                existing.as_ref().map(|row| row.created_at),
            ))
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign event for '{}': {e}", record.display_name))?;

        // Per-coordinate reconcile: skip when an identical body is already
        // retained, so an unchanged persona doesn't reset `pending_sync`.
        // Content is timestamp-independent, so the monotonic bump above never
        // forces a spurious republish.
        let event_content = event.content.to_string();
        if existing
            .as_ref()
            .is_some_and(|row| row.content == event_content)
        {
            continue;
        }

        let retained = RetainedEvent {
            kind: KIND_PERSONA,
            pubkey: pubkey.clone(),
            d_tag,
            content: event_content,
            // Safety: nostr timestamps are seconds and stay below i64::MAX
            // until year 2262.
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        };

        // The monotonic bump guarantees `created_at > head`, so the upsert's
        // `>=` guard always lands the UPDATE — `migrated` counts only real,
        // retained republishes.
        retain_event(&conn, &retained)
            .map_err(|e| format!("failed to retain '{}': {e}", record.display_name))?;
        migrated += 1;
    }

    Ok(migrated)
}

/// Reconcile `teams.json` into kind:30176 team events in the retention store.
///
/// Mirrors [`migrate_personas_to_events`] for teams: it picks up team metadata
/// edits (name/description/persona_ids) made on disk between launches and
/// queues them for relay publish. Managed agents (kind:30177) are deliberately
/// NOT reconciled here — they have no pack/dir source and are backfilled from
/// `managed-agents.json` elsewhere.
///
/// Must run after the persisted identity is resolved (it signs each event with
/// the owner's keys).
pub fn migrate_teams_to_events(app: &tauri::AppHandle, keys: &nostr::Keys) {
    use crate::managed_agents::managed_agents_base_dir;

    let Ok(base_dir) = managed_agents_base_dir(app) else {
        return;
    };

    match migrate_teams_in_dir(&base_dir, keys) {
        Ok(0) => {}
        Ok(migrated) => {
            eprintln!("buzz-desktop: team-event-migration: {migrated} teams migrated to retention");
        }
        Err(e) => {
            eprintln!("buzz-desktop: team-event-migration: {e}");
        }
    }
}

/// Core team reconcile logic, decoupled from the Tauri `AppHandle` for testing.
///
/// Returns the number of teams (re)written to the retention store. The
/// per-coordinate content compare matches [`migrate_personas_in_dir`]: an
/// unchanged team is skipped so a launch does not churn `pending_sync`.
fn migrate_teams_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        team_events::build_team_event,
        TeamRecord,
    };
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();

    let teams_path = base_dir.join("teams.json");
    if !teams_path.exists() {
        return Ok(0);
    }

    let content = std::fs::read_to_string(&teams_path)
        .map_err(|e| format!("failed to read teams.json: {e}"))?;

    let records: Vec<TeamRecord> =
        serde_json::from_str(&content).map_err(|e| format!("failed to parse teams.json: {e}"))?;

    if records.is_empty() {
        return Ok(0);
    }

    let db_path = base_dir.join("retention.db");
    let conn =
        open_retention_db(&db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

    let mut migrated = 0u32;

    for record in &records {
        // Skip built-in teams — they're always available from code.
        if record.is_builtin {
            continue;
        }

        // Team d-tag is the team id (team_events.rs: no slug fallback).
        let d_tag = record.id.clone();

        // Fetch the head first so the monotonic bump can supersede a
        // future-dated head — see migrate_personas_in_dir (F1/F8).
        let existing = get_retained_event(&conn, KIND_TEAM, &pubkey, &d_tag)?;

        let event = build_team_event(record)
            .map_err(|e| format!("failed to build event for team '{}': {e}", record.name))?
            .custom_created_at(monotonic_created_at(
                existing.as_ref().map(|row| row.created_at),
            ))
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign event for team '{}': {e}", record.name))?;

        let event_content = event.content.to_string();
        if existing
            .as_ref()
            .is_some_and(|row| row.content == event_content)
        {
            continue;
        }

        let retained = RetainedEvent {
            kind: KIND_TEAM,
            pubkey: pubkey.clone(),
            d_tag,
            content: event_content,
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        };

        // Monotonic bump guarantees the upsert UPDATE lands — `migrated` counts
        // only real republishes.
        retain_event(&conn, &retained)
            .map_err(|e| format!("failed to retain team '{}': {e}", record.name))?;
        migrated += 1;
    }

    Ok(migrated)
}

#[cfg(test)]
#[path = "event_sync_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "event_sync_team_events_tests.rs"]
mod team_events_tests;
