//! Retention-store enqueue helpers for the owner's kind:30178 team catalog
//! heads: build and retain a pending projection on share, retain a newer
//! untagged head on unshare, purge + tombstone on delete.
//!
//! Shares three seams with `commands::personas::pending`: the same retention
//! store, the monotonic `created_at` rule, and the `flush_pending_events`
//! background publisher. It diverges beyond those — a catalog head is built
//! from a team plus its ordered member definitions
//! (`managed_agents::team_catalog`), delete delegates to the single-transaction
//! `tombstone_team_catalog_coordinate`, and this module owns a team-only
//! refresh-or-retract state machine with no persona counterpart.

use tauri::AppHandle;

use crate::app_state::AppState;
use crate::managed_agents::{
    retention::{RetainedEvent, RetentionScope},
    AgentDefinition, TeamRecord,
};

#[cfg(test)]
use crate::active_user_signer::ActiveUserSigner;
use crate::managed_agents::team_catalog::publication::{
    prepare_catalog_head, prepare_catalog_head_from_builder, CatalogPlan, PreparedCatalogHead,
};
use crate::managed_agents::team_catalog::{
    prepare_team_catalog_tombstone, prepare_team_catalog_tombstone_from_head, CatalogInputs,
    CatalogRetraction,
};
use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
type PreparedRefresh = (RefreshOrRetractOutcome, Option<CatalogPlan>);

/// A signed catalog head, retained and awaiting relay acceptance.
///
/// Only the retained-row coordinate is carried, not the signed event itself:
/// publication happens through the flush loop off the durable pending row, so
/// `set_team_shared` never re-submits the event directly (see
/// `sharing::publish_prepared_team`).
pub(super) struct PreparedTeamPublication {
    pub scope: RetentionScope,
    pub retained: RetainedEvent,
    pub team: TeamRecord,
}

/// Outcome of a refresh, or the prospective outcome of a prepared retraction.
///
/// PREPARE returns the removal outcome alongside an inert plan. It becomes
/// queue-accurate only after that plan commits: production carries the notice
/// in `CatalogRetraction` and never emits it during PREPARE. The relay head may
/// still be live until the serialized publisher flushes the committed witness.
#[derive(Debug, PartialEq)]
pub(super) enum RefreshOrRetractOutcome {
    /// No retained shared head — the operation is a no-op.
    Noop,
    /// The shared head was rebuilt and the newer version is now retained.
    Refreshed,
    /// The shared head could not be rebuilt; committing its accompanying plan
    /// enqueues a tombstone.
    RemovalQueued { reason: String },
}

/// Whether a retained catalog head carries the exact `shared` tag.
///
/// Reuses `event_is_shared`, the same fail-closed check the relay applies at
/// its read gate, so the client's notion of "shared" cannot drift from the
/// relay's.
fn retained_team_is_shared(row: Option<&RetainedEvent>) -> bool {
    use buzz_core_pkg::kind::event_is_shared;
    use nostr::JsonUtil;

    row.and_then(|retained| nostr::Event::from_json(&retained.raw_event).ok())
        .is_some_and(|event| event_is_shared(&event))
}

/// Project each team's catalog visibility from the active relay+owner scope's
/// retained 30178 head.
///
/// Infallible by design, like `personas::pending::project_active_persona_sharing`:
/// the scope needs `signing_keys()`, which fails process-wide when the identity
/// is lost or the keyring is locked, and propagating that would break listing,
/// creating, and editing EVERY team. Share state is a view projection, so an
/// unresolvable scope degrades to "not shared" — it can under-report
/// visibility but never present an unshared team as published.
pub(super) fn project_active_team_sharing<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    teams: &mut [TeamRecord],
) {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state);
    project_scoped_team_sharing(scope, teams);
}

fn project_scoped_team_sharing(scope: Result<RetentionScope, String>, teams: &mut [TeamRecord]) {
    let projected = scope.and_then(|scope| {
        project_team_sharing_at(
            &scope.db_path,
            &scope.owner_signer().public_key().to_hex(),
            teams,
        )
    });
    if let Err(error) = projected {
        eprintln!(
            "buzz-desktop: team-share-projection unavailable, reporting every team as unshared: {error}"
        );
        for team in teams {
            team.shared = false;
        }
    }
}

fn project_team_sharing_at(
    db_path: &std::path::Path,
    owner_pubkey: &str,
    teams: &mut [TeamRecord],
) -> Result<(), String> {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};

    let conn = open_retention_db(db_path)?;
    for team in teams {
        if team.is_builtin {
            team.shared = false;
            continue;
        }
        let retained = get_retained_event(&conn, KIND_TEAM_CATALOG, owner_pubkey, &team.id)?;
        team.shared = retained_team_is_shared(retained.as_ref());
    }
    Ok(())
}

/// Build, sign, and durably retain a team's catalog head in the active
/// relay+owner scope.
///
/// `shared_override` follows the persona rule: the explicit toggle passes
/// `Some(shared)`, while a rebuild triggered by an edit passes `None` and
/// preserves whatever the scoped head already says. That is what makes an
/// ordinary team edit unable to silently unshare — belt-and-braces here, since
/// share state lives on 30178 and an edit republishes 30176.
pub(super) struct UnsignedTeamPublication {
    scope: RetentionScope,
    head: PreparedCatalogHead,
    team: TeamRecord,
    inputs: CatalogInputs,
}

pub(super) fn prepare_team_publication<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared_override: Option<bool>,
) -> Result<UnsignedTeamPublication, String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    let inputs = CatalogInputs::capture(&team.id, std::slice::from_ref(team), members)?;
    let (head, team) = prepare_catalog_head(
        &scope.db_path,
        &scope.owner_signer().public_key().to_hex(),
        team,
        members,
        shared_override,
    )?;
    Ok(UnsignedTeamPublication {
        scope,
        head,
        team,
        inputs,
    })
}

pub(super) async fn finish_team_publication<R: tauri::Runtime>(
    app: &AppHandle<R>,
    work: UnsignedTeamPublication,
) -> Result<PreparedTeamPublication, String> {
    use tauri::Manager;
    let signed = work.head.sign(&work.scope.owner_signer()).await?;
    let state = app.state::<AppState>();
    let _guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let teams = crate::managed_agents::load_teams(app)?;
    let personas = crate::managed_agents::load_personas(app)?;
    if CatalogInputs::capture(&work.team.id, &teams, &personas)? != work.inputs {
        return Err("catalog publication conflict: disk inputs changed".into());
    }
    let (_, retained) = signed.commit()?;
    Ok(PreparedTeamPublication {
        scope: work.scope,
        team: work.team,
        retained,
    })
}

#[cfg(test)]
pub(super) async fn prepare_team_publication_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared_override: Option<bool>,
) -> Result<(nostr::Event, RetainedEvent, TeamRecord), String> {
    let (head, team) = prepare_catalog_head(
        db_path,
        &keys.public_key().to_hex(),
        team,
        members,
        shared_override,
    )?;
    let (event, retained) = head
        .sign(&ActiveUserSigner::local(keys.clone()))
        .await?
        .commit()?;
    Ok((event, retained, team))
}

/// Caller holds the store lock; scope may be the captured inbound arrival scope.
pub(crate) fn prepare_catalog_delete_in_scope<R: tauri::Runtime>(
    app: &AppHandle<R>,
    scope: &RetentionScope,
    d_tag: &str,
) -> Result<CatalogRetraction, String> {
    let signer = scope.owner_signer();
    let teams = crate::managed_agents::load_teams(app)?;
    let personas = crate::managed_agents::load_personas(app)?;
    let inputs = CatalogInputs::capture(d_tag, &teams, &personas)?;
    let plan =
        prepare_team_catalog_tombstone(&scope.db_path, &signer.public_key().to_hex(), d_tag)?;
    Ok(CatalogRetraction {
        plan: plan.into(),
        inputs,
        signer,
        boot_inputs: false,
        notice: None,
    })
}

pub(super) fn refresh_shared_team_catalog_head_resolving<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Vec<CatalogRetraction> {
    let result = (|| {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        prepare_catalog_refresh_in_scope(&scope, team, personas)
    })();
    match result {
        Ok(work) => work.into_iter().collect(),
        Err(e) => {
            eprintln!("buzz-desktop: team-catalog-refresh: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn prepare_catalog_refresh_in_scope(
    scope: &RetentionScope,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<Option<CatalogRetraction>, String> {
    let signer = scope.owner_signer();
    let inputs = CatalogInputs::capture(&team.id, std::slice::from_ref(team), personas)?;
    let (outcome, plan) = prepare_resolve_and_refresh_or_retract_at(
        &scope.db_path,
        &scope.owner_signer().public_key().to_hex(),
        team,
        personas,
    )?;
    Ok(plan.map(|plan| CatalogRetraction {
        plan,
        inputs,
        signer,
        boot_inputs: false,
        notice: match outcome {
            RefreshOrRetractOutcome::RemovalQueued { reason } => Some((team.name.clone(), reason)),
            _ => None,
        },
    }))
}

/// Scope-free single-team core: resolve `team`'s members from `personas`,
/// then run the refresh-or-retract state machine.
///
/// On resolution failure the head may already be shared; the function checks
/// and prepares a tombstone if so. This is the ONLY place the
/// "resolution failure → tombstone-if-shared" logic lives — both production
/// and the `#[cfg(test)]` file-based seam call it, so there is no divergence.
pub(super) fn prepare_resolve_and_refresh_or_retract_at(
    db_path: &std::path::Path,
    pubkey: &str,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<PreparedRefresh, String> {
    use crate::managed_agents::team_catalog::resolve_team_members;

    match resolve_team_members(team, personas) {
        Ok(members) => prepare_refresh_or_retract_shared_head_at(db_path, pubkey, team, &members),
        Err(reason) => {
            // Resolution failed (a required member is missing). Treat this
            // like a projection build failure: tombstone the shared head if
            // one exists, so the stale projection is not left live. Done inline
            // (rather than via `refresh_or_retract_shared_head_at`) so the
            // resolution-error reason is preserved in the payload.
            use crate::managed_agents::retention::{get_retained_event, open_retention_db};
            use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
            use nostr::JsonUtil;

            let conn = open_retention_db(db_path)?;
            let Some(existing) = get_retained_event(&conn, KIND_TEAM_CATALOG, pubkey, &team.id)?
            else {
                return Ok((RefreshOrRetractOutcome::Noop, None));
            };
            let head_event = nostr::Event::from_json(&existing.raw_event)
                .map_err(|e| format!("failed to parse retained head: {e}"))?;
            if !event_is_shared(&head_event) {
                return Ok((RefreshOrRetractOutcome::Noop, None));
            }
            // Shared head exists but team is now unresolvable — tombstone it.
            drop(conn);
            let plan = prepare_team_catalog_tombstone_from_head(
                db_path,
                pubkey,
                &team.id,
                Some(&existing),
            )?;
            Ok((
                RefreshOrRetractOutcome::RemovalQueued { reason },
                Some(plan.into()),
            ))
        }
    }
}

/// Core of [`refresh_shared_team_catalog_head_resolving`], scope-free so it is
/// testable without a Tauri `AppHandle`.
pub(super) fn prepare_refresh_or_retract_shared_head_at(
    db_path: &std::path::Path,
    pubkey: &str,
    team: &TeamRecord,
    members: &[AgentDefinition],
) -> Result<PreparedRefresh, String> {
    use crate::managed_agents::{
        retention::{get_retained_event, open_retention_db},
        team_catalog::build_team_catalog_event,
    };
    use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
    use nostr::JsonUtil;

    let conn = open_retention_db(db_path)?;

    // Guard: only act when a retained shared head exists — a never-shared team
    // must never produce a 30178 row.
    let Some(existing) = get_retained_event(&conn, KIND_TEAM_CATALOG, pubkey, &team.id)? else {
        return Ok((RefreshOrRetractOutcome::Noop, None));
    };
    let head_event = nostr::Event::from_json(&existing.raw_event)
        .map_err(|e| format!("failed to parse retained head: {e}"))?;
    if !event_is_shared(&head_event) {
        return Ok((RefreshOrRetractOutcome::Noop, None));
    }

    // Rebuild; on failure, prepare a retraction. The async caller signs outside
    // all storage locks, then revalidates this head and its disk inputs.
    let rebuilt = build_team_catalog_event(team, members, true);
    let builder = match rebuilt {
        Ok(b) => b,
        Err(reason) => {
            // Close the read connection before the tombstone opens a write one.
            drop(conn);
            let plan = prepare_team_catalog_tombstone_from_head(
                db_path,
                pubkey,
                &team.id,
                Some(&existing),
            )?;
            return Ok((
                RefreshOrRetractOutcome::RemovalQueued { reason },
                Some(plan.into()),
            ));
        }
    };

    let event = builder
        .clone()
        .build(nostr::PublicKey::from_hex(pubkey).map_err(|e| e.to_string())?);

    // Idempotency across devices: skip the publish when the rebuilt projection
    // is byte-identical to the retained head and still shared. Without this, an
    // owner's edit on device A refreshes A's head AND is re-applied inbound on
    // device B — where B would rebuild the same content and republish, so the
    // two devices churn identical heads at each other. The retained-head gate
    // above requires a shared event, and this builder explicitly uses shared=true.
    if existing.content == event.content {
        return Ok((RefreshOrRetractOutcome::Noop, None));
    }

    let plan =
        prepare_catalog_head_from_builder(db_path, pubkey, &team.id, Some(&existing), builder);
    Ok((
        RefreshOrRetractOutcome::Refreshed,
        Some(CatalogPlan::Refresh(plan)),
    ))
}

/// Refresh or retract the shared 30178 heads of every team that includes
/// `persona_id` as a member, after a successful persona edit.
///
/// A persona edit changes every catalog projection it is part of; walking all
/// teams is the only way to find them without an inverse index.
///
/// **Privacy invariant**: for each affected team, `resolve_team_members` is
/// called so only that team's own ordered members are projected — never the
/// entire persona store (passing the whole store would embed every local
/// persona in the published 30178).
///
/// Best-effort: per-team failures are logged and do not block each other.
pub(super) fn refresh_shared_team_catalog_heads_for_persona<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    persona_id: &str,
) -> Vec<CatalogRetraction> {
    let result = (|| {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        prepare_catalog_persona_refresh_in_scope(app, &scope, persona_id)
    })();
    match result {
        Ok(work) => work,
        Err(e) => {
            eprintln!("buzz-desktop: team-catalog-refresh-for-persona: {e}");
            Vec::new()
        }
    }
}

pub(crate) fn prepare_catalog_persona_refresh_in_scope<R: tauri::Runtime>(
    app: &AppHandle<R>,
    scope: &RetentionScope,
    persona_id: &str,
) -> Result<Vec<CatalogRetraction>, String> {
    let teams = crate::managed_agents::load_teams(app)?;
    let personas = crate::managed_agents::load_personas(app)?;
    let mut work = Vec::new();
    for team in &teams {
        if team.is_builtin || !team.persona_ids.iter().any(|id| id == persona_id) {
            continue;
        }
        match prepare_catalog_refresh_in_scope(scope, team, &personas) {
            Ok(Some(job)) => work.push(job),
            Ok(None) => {}
            Err(e) => eprintln!("buzz-desktop: team-catalog-refresh-for-persona: {e}"),
        }
    }
    Ok(work)
}

/// Testable seam for [`refresh_shared_team_catalog_heads_for_persona`].
///
/// Reads teams and personas from flat JSON files in `base_dir` rather than
/// through the Tauri store. Calls the SAME `resolve_and_refresh_or_retract_at`
/// that production uses — the seam is a thin file-loading shim with no
/// independent logic. Tests therefore exercise the exact production code path.
#[cfg(test)]
pub(super) async fn refresh_for_persona_at(
    base_dir: &std::path::Path,
    keys: &nostr::Keys,
    db_path: &std::path::Path,
    persona_id: &str,
) -> Result<(), String> {
    use crate::event_sync::read_json_store_pub as read_json_store;

    let teams: Vec<crate::managed_agents::TeamRecord> =
        read_json_store(&base_dir.join("teams.json"))?;
    let personas: Vec<crate::managed_agents::AgentDefinition> =
        read_json_store(&base_dir.join("personas.json"))?;

    for team in &teams {
        if team.is_builtin || !team.persona_ids.iter().any(|id| id == persona_id) {
            continue;
        }
        // Identical call to production — no parallel implementation.
        let _ = resolve_and_refresh_or_retract_at(db_path, keys, team, &personas).await;
    }
    Ok(())
}

// Test adapters drive the production prepare/sign/commit phases without Tauri.
// File-backed disk revalidation is covered by the production retraction tests.
#[cfg(test)]
pub(super) async fn tombstone_team_catalog_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    d_tag: &str,
) -> Result<(), String> {
    prepare_team_catalog_tombstone(db_path, &keys.public_key().to_hex(), d_tag)?
        .sign(&ActiveUserSigner::local(keys.clone()))
        .await?
        .commit()
}

#[cfg(test)]
async fn finish_test_refresh(
    prepared: PreparedRefresh,
    keys: &nostr::Keys,
) -> Result<RefreshOrRetractOutcome, String> {
    let (outcome, plan) = prepared;
    if let Some(plan) = plan {
        plan.sign(&ActiveUserSigner::local(keys.clone()))
            .await?
            .commit()?;
    }
    Ok(outcome)
}

#[cfg(test)]
pub(super) async fn resolve_and_refresh_or_retract_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<RefreshOrRetractOutcome, String> {
    finish_test_refresh(
        prepare_resolve_and_refresh_or_retract_at(
            db_path,
            &keys.public_key().to_hex(),
            team,
            personas,
        )?,
        keys,
    )
    .await
}

#[cfg(test)]
pub(super) async fn refresh_or_retract_shared_head_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    members: &[AgentDefinition],
) -> Result<RefreshOrRetractOutcome, String> {
    finish_test_refresh(
        prepare_refresh_or_retract_shared_head_at(
            db_path,
            &keys.public_key().to_hex(),
            team,
            members,
        )?,
        keys,
    )
    .await
}

#[cfg(test)]
mod tests;
