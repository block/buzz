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
pub fn run_event_sync(
    app: &tauri::AppHandle,
    owner_keys: &nostr::Keys,
    db_path: &Path,
) -> Result<(), String> {
    // Persona and agent legs stay best-effort: they log and swallow, and their
    // failure does not undo the boot team-membership repair. The team leg is
    // fatal — it establishes the superseding local head (a monotonic
    // `created_at`) that lets `retain_inbound_event`'s equal/older guard reject
    // a stale relay roster. If it fails, the caller must not let the frontend
    // expose the community and start inbound replay against an un-superseded
    // disk state.
    migrate_personas_to_events(app, owner_keys, db_path);
    migrate_teams_to_events(app, owner_keys, db_path)?;
    reconcile_team_catalog_heads(app, owner_keys, db_path);
    crate::managed_agents::reconcile::reconcile_agents_to_events(app, owner_keys, db_path);
    Ok(())
}

/// Run the scoped event reconcile to completion on the blocking pool.
///
/// Callers that must not let downstream work observe a not-yet-retained disk
/// state (e.g. `apply_workspace` before the frontend can start inbound history
/// replay) await this so the repaired local heads are durably retained — with a
/// superseding `monotonic_created_at` — before an old relay head can race in.
/// The owner keys are moved in so the task never touches the `AppState::keys`
/// mutex; the reconcile itself is synchronous JSON/SQLite/signing work, so it
/// runs on the blocking pool rather than an async worker.
///
/// Returns `Err` if the task fails to join or the fatal team leg errors, so the
/// caller can withhold community exposure until the superseding head is durable.
pub async fn run_event_sync_blocking(
    app: tauri::AppHandle,
    owner_keys: nostr::Keys,
    db_path: std::path::PathBuf,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run_event_sync(&app, &owner_keys, &db_path))
        .await
        .map_err(|e| format!("event-sync: spawn_blocking failed: {e}"))?
}

/// Reconcile `personas.json` into the persona-event retention store.
///
/// Must run AFTER `fold_personas_into_agent_store` and
/// `detach_directory_backed_teams` (depends on field renames and store
/// unification being complete) and AFTER the persisted identity is resolved
/// (it signs every retained event with the owner's keys).
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
pub fn migrate_personas_to_events(app: &tauri::AppHandle, keys: &nostr::Keys, db_path: &Path) {
    use crate::managed_agents::managed_agents_base_dir;

    let Ok(base_dir) = managed_agents_base_dir(app) else {
        return;
    };

    match migrate_personas_in_dir_at(&base_dir, keys, db_path) {
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
#[cfg(test)]
fn migrate_personas_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    migrate_personas_in_dir_at(base_dir, keys, &base_dir.join("retention.db"))
}

fn migrate_personas_in_dir_at(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    use crate::managed_agents::{
        persona_events::{build_persona_event, monotonic_created_at, persona_d_tag},
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();

    // Post-fold (Phase 1A.2): definitions live as key-less records in the
    // unified agent store, presented in the legacy shape. Pre-fold boots
    // (run_event_sync runs after run_boot_migrations, so the fold has
    // already happened) never reach this path with personas.json present —
    // but read it as a fallback for one release in case the fold errored.
    let records = read_persona_definitions(base_dir)?;

    if records.is_empty() {
        return Ok(0);
    }

    // Open (or create) the retention database.
    let conn =
        open_retention_db(db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

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

        let mut scoped_record = record.clone();
        scoped_record.shared = existing
            .as_ref()
            .and_then(|row| nostr::Event::from_json(&row.raw_event).ok())
            .is_some_and(|event| buzz_core_pkg::kind::event_is_shared(&event));
        let event = build_persona_event(&scoped_record)
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
pub fn migrate_teams_to_events(
    app: &tauri::AppHandle,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<(), String> {
    use crate::managed_agents::managed_agents_base_dir;

    let base_dir = managed_agents_base_dir(app)
        .map_err(|e| format!("team-event-migration: base dir unavailable: {e}"))?;

    match migrate_teams_in_dir_at(&base_dir, keys, db_path) {
        Ok(0) => Ok(()),
        Ok(migrated) => {
            eprintln!("buzz-desktop: team-event-migration: {migrated} teams migrated to retention");
            Ok(())
        }
        Err(e) => Err(format!("team-event-migration: {e}")),
    }
}

/// Core team reconcile logic, decoupled from the Tauri `AppHandle` for testing.
///
/// Returns the number of teams (re)written to the retention store. The
/// per-coordinate content compare matches [`migrate_personas_in_dir`]: an
/// unchanged team is skipped so a launch does not churn `pending_sync`.
#[cfg(test)]
fn migrate_teams_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    migrate_teams_in_dir_at(base_dir, keys, &base_dir.join("retention.db"))
}

fn migrate_teams_in_dir_at(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
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

    let conn =
        open_retention_db(db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

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

/// Reconcile every shared team's kind:30178 catalog head against the team as
/// it exists on disk now.
///
/// The publish path rebuilds a catalog head only when the owner touches the
/// team itself. A team's *members* are separate records, so editing or
/// deleting one changes what the team is while leaving a stale projection
/// published. This seam catches that drift, over currently-shared heads only —
/// an unshared head is not discoverable, so nothing is stale to correct.
///
/// Two outcomes, both keeping the published catalog truthful:
///
/// - Still projects, bytes changed → republish a newer shared head.
/// - Can no longer be projected (a member was deleted, or it outgrew the size
///   contract) → **purge + tombstone** (I4). An unshared stale body is not a
///   true retraction — it leaves the coordinate live with no opt-in tag, so
///   the team must fully disappear. A typed `team-catalog-auto-retracted`
///   notice names the team and reason so the owner knows why the toggle
///   changed.
///
/// Deliberately not wired into `save_teams()`: that disk-store primitive has
/// many callers (import, repair, cascade delete), and signing a relay event
/// inside it would publish on paths that never intended to.
fn reconcile_team_catalog_heads(app: &tauri::AppHandle, keys: &nostr::Keys, db_path: &Path) {
    use crate::managed_agents::managed_agents_base_dir;

    let Ok(base_dir) = managed_agents_base_dir(app) else {
        return;
    };

    match reconcile_team_catalog_heads_at(app, &base_dir, keys, db_path) {
        Ok(0) => {}
        Ok(reconciled) => {
            eprintln!(
                "buzz-desktop: team-catalog-reconcile: {reconciled} shared team heads refreshed"
            );
        }
        Err(e) => {
            eprintln!("buzz-desktop: team-catalog-reconcile: {e}");
        }
    }
}

/// Core catalog reconcile, decoupled from the Tauri `AppHandle` for testing.
///
/// Returns the number of heads (re)written — republished or tombstoned.
fn reconcile_team_catalog_heads_at(
    app: &tauri::AppHandle,
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    reconcile_team_catalog_heads_core(Some(app), base_dir, keys, db_path)
}

#[cfg(test)]
pub(crate) fn reconcile_team_catalog_heads_at_for_test(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    reconcile_team_catalog_heads_core(None, base_dir, keys, db_path)
}

/// Inner reconcile, `app` is `None` only in unit tests (no Tauri runtime).
fn reconcile_team_catalog_heads_core(
    app: Option<&tauri::AppHandle>,
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_events_by_kind, open_retention_db, retain_event, RetainedEvent},
        team_catalog::{
            build_team_catalog_event, resolve_team_members, tombstone_team_catalog_coordinate,
        },
        TeamRecord,
    };
    use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();
    let conn =
        open_retention_db(db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

    // Enumerate retained 30178 heads as the authoritative worklist. A team
    // deleted after a shared head was written is still visible here; iterating
    // only the current team store would miss the orphan.
    let all_heads = get_retained_events_by_kind(&conn, KIND_TEAM_CATALOG, &pubkey)?;
    if all_heads.is_empty() {
        return Ok(0);
    }

    // Load teams once; missing is equivalent to empty (owner cleared the
    // store). Load personas only when at least one shared head is found.
    let teams: Vec<TeamRecord> = read_json_store(&base_dir.join("teams.json"))?;
    let personas = read_persona_definitions(base_dir)?;

    let mut reconciled = 0u32;

    for head in &all_heads {
        let head_event = nostr::Event::from_json(&head.raw_event).map_err(|e| {
            format!(
                "failed to parse retained head for d-tag '{}': {e}",
                head.d_tag
            )
        })?;

        // Only shared heads represent live community-visible state. An
        // already-unshared head cannot be made worse by leaving it; a
        // tombstone covers whole-coordinate deletion (delete_team).
        if !event_is_shared(&head_event) {
            continue;
        }

        // F1: the team no longer exists → the owner deleted it after sharing.
        // Tombstone the coordinate so the community catalog stops showing it.
        // The team-first loop could never see this case.
        let Some(team) = teams.iter().find(|t| t.id == head.d_tag) else {
            // Team name from the head's content for the notice, falling back
            // to the d-tag when content is unparseable.
            let team_name = (|| -> Option<String> {
                let content: serde_json::Value =
                    serde_json::from_str(head_event.content.as_ref()).ok()?;
                content.get("name")?.as_str().map(str::to_string)
            })()
            .unwrap_or_else(|| head.d_tag.clone());
            let reason = "team no longer exists".to_string();
            eprintln!("buzz-desktop: team-catalog-reconcile: tombstoning '{team_name}' — {reason}");
            // `tombstone_team_catalog_coordinate` opens its own WAL connection;
            // `conn` is kept alive for the retain_event calls in later
            // iterations.
            if let Err(e) = tombstone_team_catalog_coordinate(db_path, keys, &head.d_tag) {
                eprintln!(
                    "buzz-desktop: team-catalog-reconcile: tombstone failed for '{}': {e}",
                    head.d_tag
                );
            } else {
                reconciled += 1;
                if let Some(app) = app {
                    emit_team_catalog_auto_retracted(app, &team_name, &reason);
                }
            }
            continue;
        };

        // Built-in teams can never have been shared, but be defensive.
        if team.is_builtin {
            continue;
        }

        // Reproject from the current on-disk team and members. A failure is
        // the retraction trigger: purge + tombstone the coordinate and notify
        // the owner via a typed event. A stale-body "retraction" was rejected
        // because an unshared-but-retained coordinate leaves the event live on
        // the relay with no opt-in tag.
        let rebuilt = resolve_team_members(team, &personas)
            .and_then(|members| build_team_catalog_event(team, &members, true));
        let builder = match rebuilt {
            Ok(builder) => builder,
            Err(reason) => {
                eprintln!(
                    "buzz-desktop: team-catalog-reconcile: tombstoning '{}' — {reason}",
                    team.name
                );
                // `tombstone_team_catalog_coordinate` opens its own WAL
                // connection; NOT dropping `conn` is what lets the loop keep
                // processing remaining heads (I2 — multi-head continuation).
                if let Err(e) = tombstone_team_catalog_coordinate(db_path, keys, &team.id) {
                    eprintln!(
                        "buzz-desktop: team-catalog-reconcile: tombstone failed for '{}': {e}",
                        team.name
                    );
                } else {
                    reconciled += 1;
                    if let Some(app) = app {
                        emit_team_catalog_auto_retracted(app, &team.name, &reason);
                    }
                }
                // Continue to the next head — do not stop after the first
                // tombstone (the original `drop(conn); return` was the I2 bug).
                continue;
            }
        };

        let event = builder
            // Supersede the retained head even when future-dated, as the
            // persona and team reconciles do.
            .custom_created_at(monotonic_created_at(Some(head.created_at)))
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign catalog head for '{}': {e}", team.name))?;

        // Compare the tag too, not just the body: an unshare replays the
        // retained content verbatim, so bytes alone would report "unchanged"
        // and leave the stale head shared.
        if head.content == event.content && event_is_shared(&event) {
            continue;
        }

        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_TEAM_CATALOG,
                pubkey: pubkey.clone(),
                d_tag: team.id.clone(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
        .map_err(|e| format!("failed to retain catalog head for '{}': {e}", team.name))?;
        reconciled += 1;
    }

    Ok(reconciled)
}

/// Emit a typed Tauri event so the frontend can show the owner a notice when
/// the boot reconcile automatically retracts a shared team.
///
/// Best-effort: a failed emit is logged but does not block reconcile.
fn emit_team_catalog_auto_retracted(app: &tauri::AppHandle, team_name: &str, reason: &str) {
    use serde::Serialize;
    use tauri::Emitter;

    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct TeamCatalogAutoRetractedPayload<'a> {
        team_name: &'a str,
        reason: &'a str,
    }

    if let Err(e) = app.emit(
        "team-catalog-auto-retracted",
        TeamCatalogAutoRetractedPayload { team_name, reason },
    ) {
        eprintln!("buzz-desktop: team-catalog-reconcile: failed to emit retraction notice: {e}");
    }
}

/// Read a JSON array store, treating an absent file as empty.
fn read_json_store<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read {name}: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("failed to parse {name}: {e}"))
}

/// Test-accessible alias for `read_json_store`, used by the `pending` module's
/// `refresh_for_persona_at` testable seam without re-exporting the private fn.
#[cfg(test)]
pub(crate) fn read_json_store_pub<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Vec<T>, String> {
    read_json_store(path)
}

/// Read every persona definition in the legacy shape, from whichever store
/// holds them.
///
/// Post-fold (Phase 1A.2) definitions are key-less records in the unified
/// agent store; `personas.json` survives only on a boot where the fold
/// errored. Both callers must read the same set — a reconcile that saw an
/// empty persona list would conclude every team's members were deleted.
fn read_persona_definitions(
    base_dir: &Path,
) -> Result<Vec<crate::managed_agents::AgentDefinition>, String> {
    let personas: Vec<crate::managed_agents::AgentDefinition> =
        read_json_store(&base_dir.join("personas.json"))?;
    if !personas.is_empty() {
        return Ok(personas);
    }
    let all: Vec<crate::managed_agents::ManagedAgentRecord> =
        read_json_store(&base_dir.join("managed-agents.json"))?;
    Ok(all
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(|record| record.to_definition_view())
        .collect())
}

#[cfg(test)]
#[path = "event_sync_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "event_sync_team_events_tests.rs"]
mod team_events_tests;

#[cfg(test)]
#[path = "event_sync_team_catalog_tests.rs"]
mod team_catalog_tests;
