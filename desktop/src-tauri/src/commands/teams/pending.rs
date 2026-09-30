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

use buzz_core_pkg::kind::KIND_TEAM_CATALOG;

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

/// Outcome of a single refresh-or-retract operation.
///
/// Carried through every wrapper so each site can emit the right queue-accurate
/// notice. "Removal" means a tombstone has been *enqueued* for the flush loop —
/// the relay head may still be live until the flush succeeds.
#[derive(Debug, PartialEq)]
pub(super) enum RefreshOrRetractOutcome {
    /// No retained shared head — the operation is a no-op.
    Noop,
    /// The shared head was rebuilt and the newer version is now retained.
    Refreshed,
    /// The shared head could not be rebuilt; a tombstone was enqueued.
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
pub(super) fn project_active_team_sharing(
    app: &AppHandle,
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
            &scope.owner_keys.public_key().to_hex(),
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
pub(super) fn prepare_team_publication(
    app: &AppHandle,
    state: &AppState,
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared_override: Option<bool>,
) -> Result<PreparedTeamPublication, String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    let (_event, retained, team) = prepare_team_publication_at(
        &scope.db_path,
        &scope.owner_keys,
        team,
        members,
        shared_override,
    )?;
    Ok(PreparedTeamPublication {
        scope,
        retained,
        team,
    })
}

pub(super) fn prepare_team_publication_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared_override: Option<bool>,
) -> Result<(nostr::Event, RetainedEvent, TeamRecord), String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event},
        team_catalog::build_team_catalog_event,
    };
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    let existing = get_retained_event(&conn, KIND_TEAM_CATALOG, &pubkey, &team.id)?;
    let mut scoped_team = team.clone();
    scoped_team.shared =
        shared_override.unwrap_or_else(|| retained_team_is_shared(existing.as_ref()));
    // The size contract runs inside the builder, BEFORE signing, so an
    // oversized team fails here with a named field instead of enqueuing an
    // event the relay would permanently refuse.
    let event = build_team_catalog_event(&scoped_team, members, scoped_team.shared)?
        .custom_created_at(monotonic_created_at(
            existing.as_ref().map(|row| row.created_at),
        ))
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign team catalog event: {e}"))?;
    let retained = RetainedEvent {
        kind: KIND_TEAM_CATALOG,
        pubkey,
        d_tag: team.id.clone(),
        content: event.content.to_string(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: true,
    };
    retain_event(&conn, &retained)?;
    Ok((event, retained, scoped_team))
}

/// Purge a deleted team's retained catalog head and enqueue a NIP-09
/// tombstone for its 30178 coordinate.
///
/// The 30176 team head has its own tombstone (`tombstone_team_pending`); this
/// is the catalog counterpart and both run on delete, because the two kinds
/// are separate coordinates. Same purge-then-tombstone ordering as personas:
/// removing the 30178 row first under the store lock stops an unpublished
/// re-share from resurrecting the entry after the tombstone lands. Best-effort
/// — a failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative delete.
pub(super) fn tombstone_team_catalog_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    d_tag: &str,
) {
    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        tombstone_team_catalog_at(&scope.db_path, &scope.owner_keys, d_tag)
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-catalog-tombstone: {e}");
    }
}

/// Scope-free core of [`tombstone_team_catalog_pending`], so the purge and
/// enqueue can be asserted directly against a retention database.
pub(super) fn tombstone_team_catalog_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    d_tag: &str,
) -> Result<(), String> {
    crate::managed_agents::team_catalog::tombstone_team_catalog_coordinate(db_path, keys, d_tag)
}

/// Refresh or retract the shared 30178 head for `team` after a team edit,
/// resolving members from `personas` first.
///
/// Resolution failure (a member was deleted) is treated as a projection
/// failure: the shared head is tombstoned and the owner is notified via the
/// typed `team-catalog-auto-retracted` Tauri event. Best-effort: a retention
/// hiccup never blocks the team edit from returning.
pub(super) fn refresh_shared_team_catalog_head_resolving<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) {
    let result = (|| -> Result<RefreshOrRetractOutcome, String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        resolve_and_refresh_or_retract_at(&scope.db_path, &scope.owner_keys, team, personas)
    })();
    match result {
        Ok(RefreshOrRetractOutcome::RemovalQueued { ref reason }) => {
            eprintln!(
                "buzz-desktop: team-catalog-refresh: retracting '{}' — {reason}",
                team.name
            );
            emit_team_catalog_auto_retracted(app, &team.name, reason);
        }
        Err(ref e) => {
            eprintln!("buzz-desktop: team-catalog-refresh: '{}' — {e}", team.name);
        }
        _ => {}
    }
}

/// Scope-free single-team core: resolve `team`'s members from `personas`,
/// then run the refresh-or-retract state machine.
///
/// On resolution failure the head may already be shared; the function checks
/// and tombstones if so, returning `RemovalQueued`. This is the ONLY place the
/// "resolution failure → tombstone-if-shared" logic lives — both production
/// and the `#[cfg(test)]` file-based seam call it, so there is no divergence.
pub(super) fn resolve_and_refresh_or_retract_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    personas: &[AgentDefinition],
) -> Result<RefreshOrRetractOutcome, String> {
    use crate::managed_agents::team_catalog::resolve_team_members;

    match resolve_team_members(team, personas) {
        Ok(members) => refresh_or_retract_shared_head_at(db_path, keys, team, &members),
        Err(reason) => {
            // Resolution failed (a required member is missing). Treat this
            // like a projection build failure: tombstone the shared head if
            // one exists, so the stale projection is not left live. Done inline
            // (rather than via `refresh_or_retract_shared_head_at`) so the
            // resolution-error reason is preserved in the payload.
            use crate::managed_agents::retention::{get_retained_event, open_retention_db};
            use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
            use nostr::JsonUtil;

            let pubkey = keys.public_key().to_hex();
            let conn = open_retention_db(db_path)?;
            let Some(existing) = get_retained_event(&conn, KIND_TEAM_CATALOG, &pubkey, &team.id)?
            else {
                return Ok(RefreshOrRetractOutcome::Noop);
            };
            let head_event = nostr::Event::from_json(&existing.raw_event)
                .map_err(|e| format!("failed to parse retained head: {e}"))?;
            if !event_is_shared(&head_event) {
                return Ok(RefreshOrRetractOutcome::Noop);
            }
            // Shared head exists but team is now unresolvable — tombstone it.
            drop(conn);
            crate::managed_agents::team_catalog::tombstone_team_catalog_coordinate(
                db_path, keys, &team.id,
            )?;
            Ok(RefreshOrRetractOutcome::RemovalQueued { reason })
        }
    }
}

/// Core of [`refresh_shared_team_catalog_head_resolving`], scope-free so it is
/// testable without a Tauri `AppHandle`.
pub(super) fn refresh_or_retract_shared_head_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    members: &[AgentDefinition],
) -> Result<RefreshOrRetractOutcome, String> {
    use crate::managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event},
        team_catalog::build_team_catalog_event,
    };
    use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
    use nostr::JsonUtil;

    let pubkey = keys.public_key().to_hex();
    let conn = open_retention_db(db_path)?;

    // Guard: only act when a retained shared head exists — a never-shared team
    // must never produce a 30178 row.
    let Some(existing) = get_retained_event(&conn, KIND_TEAM_CATALOG, &pubkey, &team.id)? else {
        return Ok(RefreshOrRetractOutcome::Noop);
    };
    let head_event = nostr::Event::from_json(&existing.raw_event)
        .map_err(|e| format!("failed to parse retained head: {e}"))?;
    if !event_is_shared(&head_event) {
        return Ok(RefreshOrRetractOutcome::Noop);
    }

    // Rebuild; on failure, purge + tombstone immediately so the stale shared
    // head is not left public.
    let rebuilt = build_team_catalog_event(team, members, true);
    let builder = match rebuilt {
        Ok(b) => b,
        Err(reason) => {
            // Close the read connection before the tombstone opens a write one.
            drop(conn);
            crate::managed_agents::team_catalog::tombstone_team_catalog_coordinate(
                db_path, keys, &team.id,
            )?;
            return Ok(RefreshOrRetractOutcome::RemovalQueued { reason });
        }
    };

    let event = builder
        .custom_created_at(monotonic_created_at(Some(existing.created_at)))
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign team catalog head: {e}"))?;

    // Idempotency across devices: skip the publish when the rebuilt projection
    // is byte-identical to the retained head and still shared. Without this, an
    // owner's edit on device A refreshes A's head AND is re-applied inbound on
    // device B — where B would rebuild the same content and republish, so the
    // two devices churn identical heads at each other. The tag check guards the
    // unshare replay (see the boot reconcile) even though this fn only rebuilds
    // shared heads.
    if existing.content == event.content && event_is_shared(&event) {
        return Ok(RefreshOrRetractOutcome::Noop);
    }

    retain_event(
        &conn,
        &crate::managed_agents::retention::RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey,
            d_tag: team.id.clone(),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: true,
        },
    )?;
    Ok(RefreshOrRetractOutcome::Refreshed)
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
) {
    let result = (|| -> Result<(), String> {
        use crate::managed_agents::{load_personas, load_teams};

        let teams = load_teams(app)?;
        let personas = load_personas(app)?;
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;

        for team in &teams {
            if team.is_builtin || !team.persona_ids.iter().any(|id| id == persona_id) {
                continue;
            }
            // Unified core so resolution-failure → tombstone semantics are
            // identical in production and tests.
            let outcome = resolve_and_refresh_or_retract_at(
                &scope.db_path,
                &scope.owner_keys,
                team,
                &personas,
            );
            match outcome {
                Ok(RefreshOrRetractOutcome::RemovalQueued { ref reason }) => {
                    eprintln!(
                        "buzz-desktop: team-catalog-refresh: retracting '{}' after persona edit — {reason}",
                        team.name
                    );
                    emit_team_catalog_auto_retracted(app, &team.name, reason);
                }
                Err(ref e) => {
                    eprintln!(
                        "buzz-desktop: team-catalog-refresh: '{}' after persona edit — {e}",
                        team.name
                    );
                }
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: team-catalog-refresh-for-persona: {e}");
    }
}

/// Testable seam for [`refresh_shared_team_catalog_heads_for_persona`].
///
/// Reads teams and personas from flat JSON files in `base_dir` rather than
/// through the Tauri store. Calls the SAME `resolve_and_refresh_or_retract_at`
/// that production uses — the seam is a thin file-loading shim with no
/// independent logic. Tests therefore exercise the exact production code path.
#[cfg(test)]
pub(super) fn refresh_for_persona_at(
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
        let _ = resolve_and_refresh_or_retract_at(db_path, keys, team, &personas);
    }
    Ok(())
}

/// Emit a typed Tauri event so the frontend can notify the owner when a shared
/// team is automatically retracted due to a projection failure.
///
/// "Removal queued" is accurate: the tombstone has been enqueued for the flush
/// loop, but the relay head may still be live until the flush succeeds.
/// Best-effort: a failed emit is logged but does not block the operation.
fn emit_team_catalog_auto_retracted<R: tauri::Runtime>(
    app: &AppHandle<R>,
    team_name: &str,
    reason: &str,
) {
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
        eprintln!("buzz-desktop: team-catalog-auto-retracted: failed to emit notice: {e}");
    }
}

#[cfg(test)]
mod tests;
