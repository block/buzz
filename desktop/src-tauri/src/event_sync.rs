//! Boot-time disk→relay event reconcile ("event sync").
//!
//! Reconciles the on-disk JSON stores (`personas.json`, `teams.json`,
//! `managed-agents.json`) into signed retention events queued for relay
//! publish. Runs after identity resolution (event signing needs the owner
//! keys), unlike the pre-identity migrations in [`crate::migration`].

use crate::active_user_signer::ActiveUserSigner;
use crate::managed_agents::team_catalog::{
    prepare_team_catalog_tombstone_from_head, CatalogInputs, CatalogRetraction,
};
use std::path::Path;
use tauri::Manager;

/// Reconcile personas, teams, and managed agents into signed retention
/// events. All readers consume the already-synced
/// `personas.json`/`teams.json`/`managed-agents.json` that
/// `sync_team_personas` wrote in [`crate::migration::run_boot_migrations`]
/// (see its `# Ordering` guard). Event signing needs the resolved owner keys,
/// so this runs after identity resolution, not in the boot migrations.
pub async fn run_event_sync_blocking<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    signer: ActiveUserSigner,
    db_path: std::path::PathBuf,
) -> Result<(), String> {
    // Await each leg in order. In particular, a failed team-head prerequisite
    // must return BEFORE catalog work, agents, deletion sweep, or inbound replay.
    migrate_personas_to_events(&app, &signer, &db_path).await;
    migrate_teams_to_events(&app, &signer, &db_path).await?;
    let work = tauri::async_runtime::spawn_blocking({
        let signer = signer.clone();
        let app = app.clone();
        let db_path = db_path.clone();
        move || {
            let state = app.state::<crate::app_state::AppState>();
            let _guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(reconcile_team_catalog_heads(&app, &signer, &db_path))
        }
    })
    .await
    .map_err(|e| format!("event-sync: prepare join failed: {e}"))??;
    crate::managed_agents::team_catalog::finish_catalog_retractions(&app, work).await;
    crate::managed_agents::reconcile::reconcile_agents_to_events(&app, &signer, &db_path).await;
    // Negative-side backstop is LAST, never detached.
    let state = app.state::<crate::app_state::AppState>();
    if let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(&app) {
        if let Err(e) = reconcile_deleted_heads_core(
            &base_dir,
            &signer,
            &db_path,
            &state.managed_agents_store_lock,
        )
        .await
        {
            eprintln!("buzz-desktop: deletion-reconcile: {e}");
        }
    }
    Ok(())
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
pub(crate) async fn migrate_personas_to_events<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    signer: &ActiveUserSigner,
    db_path: &Path,
) {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    let state = app.state::<crate::app_state::AppState>();
    match migrate_personas_in_dir_at(&base_dir, signer, db_path, &state.managed_agents_store_lock)
        .await
    {
        Ok(0) => {}
        Ok(count) => eprintln!(
            "buzz-desktop: persona-event-migration: {count} personas migrated to retention"
        ),
        Err(e) => eprintln!("buzz-desktop: persona-event-migration: {e}"),
    }
}

#[cfg(test)]
async fn migrate_personas_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    migrate_personas_in_dir_at(
        base_dir,
        &ActiveUserSigner::local(keys.clone()),
        &base_dir.join("retention.db"),
        &std::sync::Mutex::new(()),
    )
    .await
}

async fn migrate_personas_in_dir_at(
    base_dir: &Path,
    signer: &ActiveUserSigner,
    db_path: &Path,
    store_lock: &std::sync::Mutex<()>,
) -> Result<u32, String> {
    use crate::managed_agents::persona_events::definition::prepare_persona_head;
    let records = {
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        read_persona_definitions(base_dir)?
    };
    let mut migrated = 0;
    for record in records.iter().filter(|record| !record.is_builtin) {
        let work = {
            let _guard = store_lock.lock().map_err(|e| e.to_string())?;
            let current = read_persona_definitions(base_dir)?;
            let Some(record) = current
                .iter()
                .find(|current| current.id == record.id && !current.is_builtin)
            else {
                continue;
            };
            prepare_persona_head(db_path, signer, record, None, false)?
        };
        if work.unchanged() {
            continue;
        }
        let signed = work.sign().await?;
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        signed.commit(&read_persona_definitions(base_dir)?)?;
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
pub async fn migrate_teams_to_events<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    signer: &ActiveUserSigner,
    db_path: &Path,
) -> Result<(), String> {
    let base_dir = crate::managed_agents::managed_agents_base_dir(app)
        .map_err(|e| format!("team-event-migration: base dir unavailable: {e}"))?;
    let state = app.state::<crate::app_state::AppState>();
    match migrate_teams_in_dir_at(&base_dir, signer, db_path, &state.managed_agents_store_lock)
        .await
    {
        Ok(0) => Ok(()),
        Ok(migrated) => {
            eprintln!("buzz-desktop: team-event-migration: {migrated} teams migrated to retention");
            Ok(())
        }
        Err(e) => Err(format!("team-event-migration: {e}")),
    }
}

#[cfg(test)]
async fn migrate_teams_in_dir(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    migrate_teams_in_dir_at(
        base_dir,
        &ActiveUserSigner::local(keys.clone()),
        &base_dir.join("retention.db"),
        &std::sync::Mutex::new(()),
    )
    .await
}

async fn migrate_teams_in_dir_at(
    base_dir: &Path,
    signer: &ActiveUserSigner,
    db_path: &Path,
    store_lock: &std::sync::Mutex<()>,
) -> Result<u32, String> {
    use crate::managed_agents::team_events::publication::prepare_team_head;
    let records = {
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        read_teams_strict(base_dir)?
    };
    let mut migrated = 0;
    for record in records.iter().filter(|record| !record.is_builtin) {
        let work = {
            let _guard = store_lock.lock().map_err(|e| e.to_string())?;
            let current = read_teams_strict(base_dir)?;
            let Some(record) = current
                .iter()
                .find(|current| current.id == record.id && !current.is_builtin)
            else {
                continue;
            };
            prepare_team_head(db_path, signer, record)?
        };
        if work.unchanged() {
            continue;
        }
        let signed = work.sign().await?;
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        signed.commit(&read_teams_strict(base_dir)?)?;
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
fn reconcile_team_catalog_heads<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    signer: &ActiveUserSigner,
    db_path: &Path,
) -> Vec<CatalogRetraction> {
    let Ok(base_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return Vec::new();
    };
    match reconcile_team_catalog_heads_core(&base_dir, signer, db_path) {
        Ok((_, work)) => work,
        Err(e) => {
            eprintln!("buzz-desktop: team-catalog-reconcile: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
pub(crate) async fn reconcile_team_catalog_heads_at_for_test(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    let (mut count, work) = reconcile_team_catalog_heads_core(
        base_dir,
        &ActiveUserSigner::local(keys.clone()),
        db_path,
    )?;
    for job in work {
        let id = job.plan.d_tag().to_owned();
        let signed = job.plan.sign(&job.signer).await?;
        if CatalogInputs::capture(
            &id,
            &read_teams_strict(base_dir)?,
            &read_persona_definitions(base_dir)?,
        )? != job.inputs
        {
            return Err("catalog tombstone conflict: disk inputs changed".into());
        }
        // Same per-head best-effort contract as the production job runner.
        if signed.commit().is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

/// Short catalog PREPARE: return inert refresh/retraction plans for the awaited
/// async writer. No connection or
/// store guard escapes in the work list.
fn reconcile_team_catalog_heads_core(
    base_dir: &Path,
    signer: &ActiveUserSigner,
    db_path: &Path,
) -> Result<(u32, Vec<CatalogRetraction>), String> {
    use crate::managed_agents::{
        retention::{get_retained_events_by_kind, open_retention_db},
        team_catalog::{build_team_catalog_event, resolve_team_members},
        TeamRecord,
    };
    use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
    use nostr::JsonUtil;

    let pubkey = signer.public_key().to_hex();
    let conn =
        open_retention_db(db_path).map_err(|e| format!("failed to open retention db: {e}"))?;

    // Enumerate retained 30178 heads as the authoritative worklist. A team
    // deleted after a shared head was written is still visible here; iterating
    // only the current team store would miss the orphan.
    let all_heads = get_retained_events_by_kind(&conn, KIND_TEAM_CATALOG, &pubkey)?;
    if all_heads.is_empty() {
        return Ok((0, Vec::new()));
    }

    // Load teams once; missing is equivalent to empty (owner cleared the
    // store). Load personas only when at least one shared head is found.
    let teams: Vec<TeamRecord> = read_json_store(&base_dir.join("teams.json"))?;
    let personas = read_persona_definitions(base_dir)?;

    let reconciled = 0u32;
    let mut work = Vec::new();

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
            match prepare_team_catalog_tombstone_from_head(
                db_path,
                &pubkey,
                &head.d_tag,
                Some(head),
            ) {
                Ok(plan) => work.push(CatalogRetraction {
                    plan: plan.into(),
                    inputs: CatalogInputs::capture(&head.d_tag, &teams, &personas)?,
                    signer: signer.clone(),
                    notice: Some((team_name, reason)),
                    boot_inputs: true,
                }),
                Err(e) => eprintln!("buzz-desktop: team-catalog-reconcile: {e}"),
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
                match prepare_team_catalog_tombstone_from_head(
                    db_path,
                    &pubkey,
                    &team.id,
                    Some(head),
                ) {
                    Ok(plan) => work.push(CatalogRetraction {
                        plan: plan.into(),
                        inputs: CatalogInputs::capture(&team.id, &teams, &personas)?,
                        signer: signer.clone(),
                        notice: Some((team.name.clone(), reason)),
                        boot_inputs: true,
                    }),
                    Err(e) => eprintln!("buzz-desktop: team-catalog-reconcile: {e}"),
                }
                // Continue to the next head — do not stop after the first
                // tombstone (the original `drop(conn); return` was the I2 bug).
                continue;
            }
        };

        let event = builder.clone().build(signer.public_key());

        // The retained head passed the shared-tag gate above and this
        // builder explicitly uses shared=true; equal bodies need no refresh.
        if head.content == event.content {
            continue;
        }

        let plan =
            crate::managed_agents::team_catalog::publication::prepare_catalog_head_from_builder(
                db_path,
                &pubkey,
                &team.id,
                Some(head),
                builder,
            );
        work.push(CatalogRetraction {
            plan: crate::managed_agents::team_catalog::publication::CatalogPlan::Refresh(plan),
            inputs: CatalogInputs::capture(&team.id, &teams, &personas)?,
            signer: signer.clone(),
            notice: None,
            boot_inputs: true,
        });
    }

    Ok((reconciled, work))
}

/// Read `teams.json` strictly: an absent file is an empty store (every team
/// was deleted), but a malformed file is a fail-loud error — never an empty
/// read that would orphan every retained team head.
pub(crate) fn read_teams_strict(
    base_dir: &Path,
) -> Result<Vec<crate::managed_agents::TeamRecord>, String> {
    read_json_store(&base_dir.join("teams.json"))
}

/// Validate `managed-agents.json` for the deletion sweep: absent is an empty
/// store, but a malformed file is preserved as `.invalid` and fails loud
/// (mirrors [`crate::managed_agents::reconcile`]'s contract) — a truncated file
/// backs the persona coordinates too, so it must never read as empty and orphan
/// live personas. The returned records are unused (managed-agent heads are not
/// swept), but reading strictly here aborts before `read_persona_definitions`
/// re-reads the same store.
fn read_agents_strict(
    base_dir: &Path,
) -> Result<Vec<crate::managed_agents::ManagedAgentRecord>, String> {
    let path = base_dir.join("managed-agents.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read managed-agents.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| {
        crate::managed_agents::storage::backup_invalid_store(&path);
        format!("failed to parse managed-agents.json (preserved as .invalid): {e}")
    })
}

/// Read disk stores strictly before deciding that a coordinate is orphaned.
/// Managed agents are validated but never swept: missing device-local keys are
/// normal on another device, not evidence of deletion.
fn live_definition_tags(
    base_dir: &Path,
) -> Result<
    (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ),
    String,
> {
    read_agents_strict(base_dir)?;
    Ok((
        read_persona_definitions(base_dir)?
            .iter()
            .map(crate::managed_agents::persona_events::persona_d_tag)
            .collect(),
        read_teams_strict(base_dir)?
            .into_iter()
            .map(|t| t.id)
            .collect(),
    ))
}

async fn reconcile_deleted_heads_core(
    base_dir: &Path,
    signer: &ActiveUserSigner,
    db_path: &Path,
    store_lock: &std::sync::Mutex<()>,
) -> Result<u32, String> {
    use crate::commands::{tombstone_persona_at, tombstone_team_at};
    use crate::managed_agents::retention::{get_retained_events_by_kind, open_retention_db};
    use buzz_core_pkg::kind::{KIND_PERSONA, KIND_TEAM};
    let work = {
        let _guard = store_lock.lock().map_err(|e| e.to_string())?;
        let (personas, teams) = live_definition_tags(base_dir)?;
        let conn = open_retention_db(db_path)?;
        let mut work = Vec::new();
        for (kind, live) in [(KIND_PERSONA, personas), (KIND_TEAM, teams)] {
            for head in get_retained_events_by_kind(&conn, kind, &signer.public_key().to_hex())? {
                if live.contains(&head.d_tag) {
                    continue;
                }
                let plan = if kind == KIND_PERSONA {
                    tombstone_persona_at(db_path, signer, &head.d_tag)
                } else {
                    tombstone_team_at(db_path, signer, &head.d_tag)
                };
                work.push((kind, head.d_tag, plan));
            }
        }
        work
    };
    let mut count = 0;
    for (kind, tag, plan) in work {
        let result = async {
            let signed = plan?.sign(signer).await;
            let _guard = store_lock.lock().map_err(|e| e.to_string())?;
            let (personas, teams) = live_definition_tags(base_dir)?;
            let live = if kind == KIND_PERSONA {
                personas
            } else {
                teams
            };
            if live.contains(&tag) {
                return Err("definition deletion conflict: disk record restored".into());
            }
            signed.commit()
        }
        .await;
        match result {
            Ok(()) => count += 1,
            Err(e) => eprintln!("buzz-desktop: deletion-reconcile kind:{kind} '{tag}': {e}"),
        }
    }
    Ok(count)
}

#[cfg(test)]
async fn reconcile_deleted_heads_at(
    base_dir: &Path,
    keys: &nostr::Keys,
    db_path: &Path,
) -> Result<u32, String> {
    reconcile_deleted_heads_core(
        base_dir,
        &ActiveUserSigner::local(keys.clone()),
        db_path,
        &std::sync::Mutex::new(()),
    )
    .await
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
pub(crate) fn read_persona_definitions(
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
