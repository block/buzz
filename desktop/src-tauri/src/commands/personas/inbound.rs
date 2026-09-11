//! Inbound relay → local store reconciliation for persona/team/managed-agent
//! projections and their NIP-09 tombstones. Extracted from the parent module to
//! keep it under the file-size cap.

use tauri::{AppHandle, Emitter, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::ManagedAgentEventContent, load_personas, persona_events::persona_d_tag,
        save_personas, team_events::TeamEventContent, try_regenerate_nest, AgentDefinition,
        ManagedAgentRecord, TeamRecord,
    },
    util::now_iso,
};

#[cfg(test)]
mod inbound_tests;
// Gated off Windows: the F1 seam test builds a real `AppState` via
// `build_app_state()`, which pulls native DLLs unavailable on the Windows CI
// runner (same constraint as `persona_events::tests::flush_barrier`).
#[cfg(all(test, not(target_os = "windows")))]
mod catalog_reconcile_tests;

#[derive(Debug)]
enum InboundRuntimeRefresh {
    Local {
        pubkey: String,
        relay_urls: Vec<String>,
    },
    Provider {
        pubkey: String,
        provider_id: String,
        config: serde_json::Value,
        cached_binary_path: Option<String>,
        agent_json: Result<serde_json::Value, String>,
    },
}

/// Apply an inbound kind:30175 persona event from the relay onto the local
/// store. The frontend's live subscription invokes this per event for our own
/// authored coordinate so Device B inherits Device A's edits.
///
/// Retention is a sync channel that writes INTO `personas.json`, never an
/// authoritative read source — `load_personas` is untouched, so every agent
/// keeps resolving its persona by UUID and keeps its provider keys.
///
/// MATCH KEY (single source of truth, both directions): an inbound event
/// matches the local record whose `persona_d_tag(record)` equals the event's
/// d-tag. Reusing the same derivation the outbound path uses guarantees the
/// inbound key can never drift from the outbound key — in particular, an
/// in-app persona (`source_team_persona_slug == None`) whose d-tag IS its
/// `id` matches its existing UUID row instead of minting a duplicate.
///
/// On match: patch ONLY the projected fields; preserve local `id`, `env_vars`,
/// `source_team`, and `created_at`. On no match: insert the parsed record as-is
/// — `persona_from_event` already sets `id = d_tag`, so an in-app persona reuses
/// its d-tag as the id and a re-received event stays idempotent (no duplicate).
///
/// The retention store decides whether the inbound event wins over a pending
/// local edit (`retain_inbound_event`): `personas.json` is only patched when the
/// retain reports [`InboundOutcome::Applied`], so an equal-second collision with
/// a pending local edit leaves the local record — and its queued publish —
/// untouched.
///
/// `arrival_relay_url` is the relay the calling subscription is bound to. The
/// retention store this event belongs to is decided by the community that
/// DELIVERED it, not by whichever community happens to be active when the
/// reconcile runs — a workspace switch in flight would otherwise file community
/// A's event into community B's scoped database. An event whose arrival relay is
/// no longer the active scope is dropped: it was already durable in its own
/// community's store when it arrived there, and that community's next boot
/// reconcile refetches it.
#[tauri::command]
pub async fn reconcile_inbound_persona_event(
    event_json: String,
    arrival_relay_url: String,
    app: AppHandle,
) -> Result<(), String> {
    let blocking_app = app.clone();
    let restart = tokio::task::spawn_blocking(move || {
        reconcile_inbound_persona_event_blocking(event_json, arrival_relay_url, blocking_app)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    match restart {
        Some(InboundRuntimeRefresh::Local { pubkey, relay_urls }) => {
            let state = app.state::<AppState>();
            super::super::agents::start_local_agent_pairs_with_preflight(
                &app,
                &state,
                &pubkey,
                &relay_urls,
            )
            .await
            .map_err(|error| {
                format!(
                    "Inbound agent access was saved, but its runtime failed to restart with the new policy: {error}"
                )
            })?;
        }
        Some(InboundRuntimeRefresh::Provider {
            pubkey,
            provider_id,
            config,
            cached_binary_path,
            agent_json,
        }) => {
            let state = app.state::<AppState>();
            let agent_json = match agent_json {
                Ok(agent_json) => agent_json,
                Err(error) => {
                    let message = format!(
                        "Inbound agent access was saved, but its provider deployment could not be refreshed safely: {error}"
                    );
                    super::super::agents::provider_access::persist_failure(
                        &app, &state, &pubkey, &message,
                    )?;
                    let _ = app.emit("agents-data-changed", ());
                    return Err(message);
                }
            };
            super::super::agents::deploy_to_provider(
                &app,
                &state,
                &pubkey,
                &provider_id,
                &config,
                agent_json,
                cached_binary_path.as_deref(),
                None,
                None,
                None,
            )
            .await
            .map_err(|error| {
                format!(
                    "Inbound agent access was saved, but its provider deployment failed to refresh with the new policy: {error}"
                )
            })?;
        }
        None => {}
    }
    Ok(())
}

fn reconcile_inbound_persona_event_blocking<R: tauri::Runtime>(
    event_json: String,
    arrival_relay_url: String,
    app: AppHandle<R>,
) -> Result<Option<InboundRuntimeRefresh>, String> {
    use crate::managed_agents::{
        agent_events::managed_agent_content_from_event,
        load_managed_agents, load_teams,
        persona_events::persona_from_event,
        retention::{
            commit_inbound_with_store, inbound_event_outcome, open_retention_db,
            retain_inbound_event, InboundOutcome, RetainedEvent,
        },
        save_managed_agents, save_teams,
        team_events::team_content_from_event,
    };
    use buzz_core_pkg::kind::{
        KIND_DELETION, KIND_MANAGED_AGENT, KIND_PERSONA, KIND_TEAM, KIND_TEAM_CATALOG,
    };
    use nostr::JsonUtil;

    let state = app.state::<AppState>();
    let event = parse_verified_inbound_event(&event_json)?;

    // The live filter subscribes to 30175/30176/30177/30178 (upserts) plus
    // kind:5 (NIP-09 deletions). d-tags are NOT unique across kinds, so every
    // path below dispatches on kind FIRST and only ever touches its own store —
    // a cross-kind d-tag collision can never link a team to a persona or agent.
    let kind = event.kind.as_u16() as u32;

    // kind:5 deletion: a tombstone removes the local record at the coordinate
    // in its `a` tag (`<target_kind>:<owner>:<d_tag>`). Handled before the
    // upsert dispatch because its coordinate and retention key differ.
    if kind == KIND_DELETION {
        reconcile_inbound_tombstone(&event, &arrival_relay_url, &app, &state)?;
        return Ok(None);
    }

    // Non-deletion upserts (30175/76/77) and the owner's own 30178 catalog head
    // share one scope + connection resolved below. A 30178 head carries no
    // local record, so it routes to witness retention through the shared
    // dispatcher; the store-bearing kinds fall through to their spine.
    if !matches!(
        kind,
        KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT | KIND_TEAM_CATALOG
    ) {
        return Ok(None);
    }

    // The d-tag identifies the record within its kind. Persona derives it from
    // the parsed record (`persona_d_tag`); team/agent carry it as the event's
    // d-tag directly. Definition-bearing content is parsed and validated once
    // here, before retention, then reused in the apply branch below. This keeps
    // an unsafe event out of both the retention database and the local store.
    let inbound_persona = (kind == KIND_PERSONA)
        .then(|| persona_from_event(&event))
        .transpose()?;
    if let Some(persona) = &inbound_persona {
        validate_inbound_persona_definition(persona)?;
    }
    let inbound_managed_agent = (kind == KIND_MANAGED_AGENT)
        .then(|| managed_agent_content_from_event(&event))
        .transpose()?;
    if let Some(managed_agent) = &inbound_managed_agent {
        validate_inbound_managed_agent_definition(managed_agent)?;
    }
    let d_tag = match &inbound_persona {
        Some(persona) => persona_d_tag(persona),
        None => event_d_tag(&event)?,
    };

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve inbound vs. any pending local edit before touching the store, in
    // the scope the event ARRIVED on. A workspace switch since arrival leaves
    // this event to its own community's store — dropping it here is what keeps
    // community A's head out of community B's database.
    let Some(scope) = crate::managed_agents::retention::arrival_retention_scope(
        &app,
        &state,
        &arrival_relay_url,
    )?
    else {
        return Ok(None);
    };
    let conn = open_retention_db(&scope.db_path)?;
    let inbound_retained_event = RetainedEvent {
        kind,
        pubkey: event.pubkey.to_hex(),
        d_tag: d_tag.clone(),
        content: event.content.to_string(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: false,
    };
    // kind:30178 catalog head: retain the owner's own publication witness and
    // stop. Retention-only — no local JSON store, no refresh, and no publish
    // (two devices would otherwise ping-pong identical heads). This is the
    // SINGLE production routing decision for a catalog arrival, resolved on the
    // shared arrival scope + connection above. `catalog_reconcile_tests.rs`
    // drives this decision through the real entrypoint, so removing this
    // invocation turns that regression RED.
    if retain_inbound_catalog_witness(&conn, &inbound_retained_event)? {
        return Ok(None);
    }

    // Advance the durable retention head only AFTER the fallible local-store
    // save succeeds (`commit_inbound_with_store`). If the head advanced first
    // and the save then failed, replay of the identical relay event would read
    // the head as already consumed (equal `created_at` reads as stale,
    // `retention.rs`) and the projection would be lost forever. The
    // managed-agent arm keeps its own preflight so a runtime transition is
    // never attempted for a skipped event.
    let mut runtime_refresh = None;
    match kind {
        KIND_PERSONA => {
            let outcome = commit_inbound_with_store(&conn, &inbound_retained_event, || {
                let mut personas = load_personas(&app)?;
                // `inbound_persona` is `Some` for KIND_PERSONA (set above).
                apply_inbound_persona(
                    &mut personas,
                    inbound_persona.expect("persona parsed above"),
                );
                save_personas(&app, &personas)
            })?;
            if outcome == InboundOutcome::Skipped {
                return Ok(None);
            }
            // A persona edit changes every shared catalog head it is a member
            // of. Refresh those heads on THIS device so the projection tracks
            // the inbound edit — matching the local `update_persona` path.
            // Idempotent: the refresh skips a republish when the rebuilt head
            // is byte-identical to the retained one, so the editing device's
            // own published head does not trigger a churn republish here. The
            // team-membership match keys off the local persona `id`, so resolve
            // it from the just-saved store by d-tag.
            let personas = load_personas(&app)?;
            if let Some(persona_id) = personas
                .iter()
                .find(|record| persona_d_tag(record) == d_tag)
                .map(|record| record.id.clone())
            {
                drop(personas);
                super::super::teams::refresh_team_catalog_heads_for_persona(
                    &app,
                    &state,
                    &persona_id,
                );
            }
        }
        KIND_TEAM => {
            let team_id = d_tag.clone();
            let outcome = commit_inbound_with_store(&conn, &inbound_retained_event, || {
                let mut teams = load_teams(&app)?;
                commit_inbound_team(
                    &mut teams,
                    d_tag,
                    team_content_from_event(&event)?,
                    |teams| save_teams(&app, teams),
                    || load_managed_agents(&app),
                    |records| save_managed_agents(&app, records),
                )
            })?;
            if outcome == InboundOutcome::Skipped {
                return Ok(None);
            }
            // A team edit changes its shared catalog projection. Refresh (or
            // retract, if a member is now missing) THIS device's retained head
            // so the community catalog tracks the inbound edit. Idempotent — a
            // rebuild byte-identical to the retained head does not republish,
            // so the editing device's own published head causes no churn.
            let teams = load_teams(&app)?;
            let personas = load_personas(&app)?;
            if let Some(team) = teams.iter().find(|record| record.id == team_id) {
                super::super::teams::refresh_team_catalog_head(&app, &state, team, &personas);
            }
        }
        KIND_MANAGED_AGENT => {
            // Preflight before the runtime transition: a skipped event must not
            // stop a running agent. The durable head is still advanced only
            // after `save_managed_agents` below.
            if inbound_event_outcome(&conn, &inbound_retained_event)? == InboundOutcome::Skipped {
                return Ok(None);
            }
            let mut agents = load_managed_agents(&app)?;
            let managed_agent = inbound_managed_agent.ok_or_else(|| {
                "managed-agent content was not parsed before retention".to_string()
            })?;
            let access_changed = apply_inbound_managed_agent(&mut agents, &d_tag, managed_agent);
            if access_changed {
                let record = agents
                    .iter_mut()
                    .find(|record| record.pubkey == d_tag)
                    .ok_or_else(|| format!("agent {d_tag} disappeared during inbound apply"))?;
                match &record.backend {
                    crate::managed_agents::BackendKind::Local => {
                        let mut runtimes = state
                            .managed_agent_processes
                            .lock()
                            .map_err(|error| error.to_string())?;
                        let mut relay_urls =
                            crate::managed_agents::managed_agent_runtime_keys(&runtimes, &d_tag)
                                .into_iter()
                                .map(|key| key.relay_url)
                                .collect::<Vec<_>>();
                        if relay_urls.is_empty() && record.runtime_pid.is_some() {
                            relay_urls.push(crate::relay::effective_agent_relay_url(
                                &record.relay_url,
                                &crate::relay::relay_ws_url_with_override(&state),
                            ));
                        }
                        if !relay_urls.is_empty() {
                            crate::managed_agents::stop_managed_agent_process(
                                &app,
                                record,
                                &mut runtimes,
                            )?;
                            runtime_refresh = Some(InboundRuntimeRefresh::Local {
                                pubkey: d_tag.clone(),
                                relay_urls,
                            });
                        }
                    }
                    crate::managed_agents::BackendKind::Provider { id, config }
                        if record.backend_agent_id.is_some() =>
                    {
                        // Persist the unacknowledged policy transition in the
                        // same write as the narrowed policy. If the process
                        // exits before or during deployment, workspace apply
                        // can still recover it in every build.
                        record.provider_policy_pending = true;
                        runtime_refresh = Some(InboundRuntimeRefresh::Provider {
                            pubkey: d_tag.clone(),
                            provider_id: id.clone(),
                            config: config.clone(),
                            cached_binary_path: record.provider_binary_path.clone(),
                            agent_json: super::super::agents::build_deploy_payload(
                                &app, &state, record,
                            ),
                        });
                    }
                    crate::managed_agents::BackendKind::Provider { .. } => {}
                }
            }
            save_managed_agents(&app, &agents)?;
            let outcome = retain_inbound_event(&conn, &inbound_retained_event)?;
            debug_assert_eq!(outcome, InboundOutcome::Applied);
        }
        _ => unreachable!("kind gated above"),
    }
    try_regenerate_nest(&app);

    // Signal the live UI to refetch agents data — inbound relay events otherwise
    // land on disk silently, leaving the Agents tab stale until restart.
    let _ = app.emit("agents-data-changed", ());

    Ok(runtime_refresh)
}

/// Retain an inbound kind:30178 catalog head as this device's publication
/// witness — retention-only, never a local store write or a republish. Returns
/// `true` when the event was a catalog head this fn handled (so the caller
/// stops), `false` for any other kind (the caller falls through to its spine).
///
/// This is the single production routing decision for a catalog arrival: the
/// blocking reconcile calls it on the shared arrival connection, and the
/// `pending/tests.rs` cross-device regressions drive the SAME fn — so disabling
/// the retention here (the `KIND_TEAM_CATALOG` arm) turns those tests RED. A
/// test that retained through `retain_inbound_event` directly could not witness
/// a regression in this routing.
///
/// The owner's own catalog heads are the worklist for two recovery paths on a
/// second device: the boot reconcile (`event_sync::reconcile_team_catalog_heads`)
/// enumerates retained 30178 rows, and the interactive
/// `refresh_or_retract_shared_head_at` guard-returns `Noop` without one. Device
/// B therefore never retains Device A's publication and both paths stay blind,
/// so B's later edit or delete cannot supersede A's discoverable head.
///
/// Deliberately NOT symmetric with the persona/team upsert spine:
/// - No local JSON store — a 30178 head is a pure relay projection with no
///   `TeamRecord`/`AgentDefinition` counterpart on disk.
/// - No refresh or publish triggered by the arrival. A 30178 arrival is either
///   this device's own echo or the other device's publication; rebuilding and
///   republishing on either would make two devices ping-pong identical heads.
///   Retention advances the witness and stops.
///
/// Newest-wins resolution matches the other inbound arms: `retain_inbound_event`
/// skips an event no newer than the retained row.
pub(crate) fn retain_inbound_catalog_witness(
    conn: &rusqlite::Connection,
    inbound: &crate::managed_agents::retention::RetainedEvent,
) -> Result<bool, String> {
    use buzz_core_pkg::kind::KIND_TEAM_CATALOG;
    if inbound.kind != KIND_TEAM_CATALOG {
        return Ok(false);
    }
    crate::managed_agents::retention::retain_inbound_event(conn, inbound)?;
    Ok(true)
}

fn validate_inbound_persona_definition(persona: &AgentDefinition) -> Result<(), String> {
    crate::managed_agents::validate_agent_definition_text(
        &persona.display_name,
        &persona.system_prompt,
    )
    .map_err(|error| format!("Inbound persona definition is unsafe: {error}"))?;
    crate::managed_agents::validate_agent_description_text(persona.description.as_deref())
        .map_err(|error| format!("Inbound persona definition is unsafe: {error}"))
}

fn validate_inbound_managed_agent_definition(
    managed_agent: &ManagedAgentEventContent,
) -> Result<(), String> {
    crate::managed_agents::validate_managed_agent_definition_text(
        &managed_agent.name,
        managed_agent.persona_id.as_deref(),
        managed_agent.system_prompt.as_deref(),
    )
    .map_err(|error| format!("Inbound managed-agent definition is unsafe: {error}"))
}

/// Parse an inbound wire event and enforce the signature gate. Everything
/// downstream trusts `event.pubkey` (ownership routing, tombstone scoping,
/// behavioral-quad application), so a forged pubkey must die here — the
/// TS-side owner filter reads the same attacker-controlled field and is no
/// defense.
fn parse_verified_inbound_event(event_json: &str) -> Result<nostr::Event, String> {
    use nostr::JsonUtil;
    let event = nostr::Event::from_json(event_json)
        .map_err(|e| format!("failed to parse inbound event: {e}"))?;
    event
        .verify()
        .map_err(|e| format!("inbound event failed signature verification: {e}"))?;
    Ok(event)
}

/// Parse a NIP-09 `a`-tag coordinate `<kind>:<owner_pubkey>:<d_tag>` into its
/// target kind and d-tag. Returns `None` if the tag is absent or malformed, so
/// the caller no-ops on a tombstone it can't route.
fn parse_deletion_coordinate(event: &nostr::Event) -> Option<(u32, String)> {
    event.tags.iter().find_map(|tag| {
        let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
        if values.first() != Some(&"a") {
            return None;
        }
        let coord = values.get(1)?;
        // `<kind>:<owner>:<d_tag>` — d_tag may itself contain ':' so split at
        // most twice and keep the remainder as the d_tag.
        let mut parts = coord.splitn(3, ':');
        let kind: u32 = parts.next()?.parse().ok()?;
        let owner = parts.next()?;
        // NIP-09 scoping: only the record's author may tombstone it. The
        // signature gate upstream proves `event.pubkey`; requiring the
        // coordinate owner to match closes the other half — a validly
        // signed kind:5 naming ANOTHER owner's coordinate must no-op.
        if owner != event.pubkey.to_hex() {
            return None;
        }
        let d_tag = parts.next()?;
        Some((kind, d_tag.to_string()))
    })
}

/// Apply an inbound kind:5 NIP-09 deletion: remove the local record at the
/// tombstone's target coordinate, scoped per-kind. Mirrors the upsert spine —
/// arrival-scoped retention resolution under the store lock, then a per-kind
/// store mutation — but removes rather than patches. Unknown/malformed
/// coordinates no-op, as does a tombstone whose arrival community is no longer
/// active.
fn reconcile_inbound_tombstone<R: tauri::Runtime>(
    event: &nostr::Event,
    arrival_relay_url: &str,
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<(), String> {
    use crate::managed_agents::{
        load_managed_agents, load_teams,
        retention::{
            commit_inbound_tombstone_with_store, open_retention_db, tombstone_retention_d_tag,
            InboundOutcome, RetainedEvent,
        },
        save_managed_agents, save_teams,
    };
    use buzz_core_pkg::kind::{
        KIND_DELETION, KIND_MANAGED_AGENT, KIND_PERSONA, KIND_TEAM, KIND_TEAM_CATALOG,
    };
    use nostr::JsonUtil;

    let Some((target_kind, target_d_tag)) = parse_deletion_coordinate(event) else {
        return Ok(()); // no routable coordinate — nothing to delete
    };
    if !matches!(
        target_kind,
        KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT | KIND_TEAM_CATALOG
    ) {
        return Ok(()); // deletion for a kind we don't track locally
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve against the retained tombstone row (keyed by the target
    // coordinate, F2c) so a re-received tombstone or one older than a pending
    // local edit is a no-op. Scoped to the arrival community, so a workspace
    // switch since arrival drops the tombstone instead of retaining it — and
    // deleting a record — in the wrong community's store.
    let Some(scope) =
        crate::managed_agents::retention::arrival_retention_scope(app, state, arrival_relay_url)?
    else {
        return Ok(());
    };
    let conn = open_retention_db(&scope.db_path)?;
    let owner_hex = event.pubkey.to_hex();
    let inbound_tombstone = RetainedEvent {
        kind: KIND_DELETION,
        pubkey: owner_hex.clone(),
        d_tag: tombstone_retention_d_tag(target_kind, &target_d_tag),
        content: event.content.to_string(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: false,
    };

    // Teams reference a member by its local persona `id`, which differs from
    // the d-tag for pack personas. Capture the id before the removal so the
    // post-tombstone member-loss refresh can find the affected teams — after
    // the closure runs, the persona is gone from the store.
    let deleted_persona_id = (target_kind == KIND_PERSONA)
        .then(|| load_personas(app))
        .transpose()?
        .and_then(|personas| {
            personas
                .iter()
                .find(|record| persona_d_tag(record) == target_d_tag)
                .map(|record| record.id.clone())
        });

    // Resolve the tombstone against BOTH its own kind:5 row AND the covered
    // `(target_kind, owner, d_tag)` head, purging the head atomically with the
    // tombstone commit only after the fallible JSON save — the relay's
    // coordinate-deletion contract (see `commit_inbound_tombstone_with_store`).
    // The removal uses the SAME per-kind match rule the apply fns use: persona
    // by `persona_d_tag`, team by `id`, managed-agent by `pubkey`.
    let outcome = commit_inbound_tombstone_with_store(
        &conn,
        &inbound_tombstone,
        target_kind,
        &owner_hex,
        &target_d_tag,
        || match target_kind {
            KIND_PERSONA => {
                let mut personas = load_personas(app)?;
                personas.retain(|record| persona_d_tag(record) != target_d_tag);
                save_personas(app, &personas)
            }
            KIND_TEAM => {
                let mut teams = load_teams(app)?;
                teams.retain(|record| record.id != target_d_tag);
                save_teams(app, &teams)
            }
            KIND_MANAGED_AGENT => {
                let mut agents = load_managed_agents(app)?;
                agents.retain(|record| record.pubkey != target_d_tag);
                save_managed_agents(app, &agents)
            }
            // A 30178 catalog head has no local JSON record — it lives only in
            // the retention store as this device's publication witness. The
            // covered-head purge inside `commit_inbound_tombstone_with_store`
            // removes the retained row; there is nothing else to delete.
            KIND_TEAM_CATALOG => Ok(()),
            _ => unreachable!("target kind gated above"),
        },
    )?;
    if outcome == InboundOutcome::Skipped {
        return Ok(());
    }

    // Converge the catalog after a tracked removal, matching the local delete
    // paths. A team tombstone must also retract its separate 30178 catalog
    // coordinate (the 30176 tombstone does not cover it). A persona tombstone
    // triggers the member-loss → supersede-or-retract path on every team that
    // listed it. A 30178 tombstone already purged the retained head above, so
    // it needs no further catalog work. Best-effort — each helper logs and
    // swallows so a retention hiccup never blocks the disk-authoritative delete.
    match target_kind {
        KIND_TEAM => {
            super::super::teams::tombstone_team_catalog_head(app, state, &target_d_tag);
        }
        KIND_PERSONA => {
            if let Some(persona_id) = &deleted_persona_id {
                super::super::teams::refresh_team_catalog_heads_for_persona(app, state, persona_id);
            }
        }
        _ => {}
    }

    try_regenerate_nest(app);

    // Refresh the live UI on inbound deletion — a removal is as user-visible as
    // an upsert and the Agents tab must drop the tombstoned record without restart.
    let _ = app.emit("agents-data-changed", ());

    Ok(())
}

/// Extract the `d` tag value from an event, the match key for team (= team id)
/// and managed-agent (= agent pubkey) inbound reconcile.
fn event_d_tag(event: &nostr::Event) -> Result<String, String> {
    event
        .tags
        .iter()
        .find_map(|tag| {
            let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
            (values.first() == Some(&"d"))
                .then(|| values.get(1).map(|s| s.to_string()))
                .flatten()
        })
        .ok_or_else(|| "inbound event missing d-tag".to_string())
}

/// Merge a parsed inbound persona into the local set: patch the matching record
/// in place, or push it when none matches.
///
/// The match key is `persona_d_tag` — the same derivation the outbound path
/// uses — so the inbound and outbound keys can never drift. On match, only the
/// projected fields are overwritten; local `id`, `env_vars`, `source_team`, and
/// `created_at` survive. On no match, the parsed record is inserted as-is; since
/// `persona_from_event` sets `id = d_tag`, an in-app persona reuses its d-tag as
/// the id and a re-received event stays idempotent (no duplicate row).
fn apply_inbound_persona(personas: &mut Vec<AgentDefinition>, inbound: AgentDefinition) {
    let d_tag = persona_d_tag(&inbound);
    match personas
        .iter_mut()
        .find(|record| persona_d_tag(record) == d_tag)
    {
        Some(local) => {
            local.display_name = inbound.display_name;
            local.avatar_url = inbound.avatar_url;
            local.description = inbound.description;
            local.system_prompt = inbound.system_prompt;
            local.runtime = inbound.runtime;
            local.model = inbound.model;
            local.provider = inbound.provider;
            local.name_pool = inbound.name_pool;
            local.respond_to = inbound.respond_to;
            local.respond_to_allowlist = inbound.respond_to_allowlist;
            local.parallelism = inbound.parallelism;
            local.shared = inbound.shared;
            local.updated_at = inbound.updated_at;
        }
        None => personas.push(inbound),
    }
}

/// Merge an inbound kind:30177 managed-agent projection into the local set.
///
/// Matches the local record whose `pubkey` equals the event's d-tag (the d-tag
/// IS the agent pubkey — see `build_agent_event`). On match, overwrite ONLY the
/// 10 projected fields; every secret (`private_key_nsec`, `auth_tag`,
/// `env_vars`, `backend`), the harness pins (`agent_command`,
/// `agent_command_override`), and all runtime/local fields are preserved
/// untouched. The projection type carries none of them, so they cannot be
/// reached here even if a foreign event tried to inject them.
///
/// No match is a no-op: managed agents carry device-local secrets and are never
/// minted from a relay event — an agent that does not already exist locally has
/// no secret key to run with, so inserting a secretless shell would be useless
/// and misleading. This diverges from the persona path, which DOES insert on no
/// match (personas are secretless definitions). Flagged in the reconcile docs.
fn apply_inbound_managed_agent(
    agents: &mut [ManagedAgentRecord],
    d_tag: &str,
    inbound: ManagedAgentEventContent,
) -> bool {
    if let Some(local) = agents.iter_mut().find(|record| record.pubkey == d_tag) {
        let previous_mode = local.respond_to;
        let previous_allowlist = local.respond_to_allowlist.clone();
        local.name = inbound.name;
        // Mirror of the slimmed writer (agent_event_content): a
        // definition-linked event omits the definition quad because those
        // fields resolve through the kind:30175 definition — absent means
        // "not carried", never "clear". Definition-less events still carry
        // the quad and apply it unconditionally (including clears).
        let definition_linked = inbound.persona_id.is_some();
        local.persona_id = inbound.persona_id;
        if !definition_linked {
            local.system_prompt = inbound.system_prompt;
            local.model = inbound.model;
            local.provider = inbound.provider;
            local.persona_source_version = inbound.persona_source_version;
        }
        local.parallelism = inbound.parallelism;
        local.respond_to = inbound.respond_to;
        local.respond_to_allowlist = inbound.respond_to_allowlist;
        return super::super::agent_models::managed_agent_access_policy_changed(
            previous_mode,
            &previous_allowlist,
            local.respond_to,
            &local.respond_to_allowlist,
            crate::managed_agents::owner_only_access_build(),
        );
    }
    false
}

/// In-memory core of the inbound `KIND_TEAM` reconcile: capture the matched
/// team's roster *before* applying the inbound projection, apply it, persist
/// teams authoritatively, then propagate the prior→current membership delta to
/// live instances best-effort — the same binding semantics the local
/// create/update commands use. Without this, a 30176 team edit from another
/// device lands on `teams.json` but never touches `ManagedAgentRecord.team_id`:
/// an added persona's running instances stay unbound (member in roster, not in
/// behavior) and a removed persona's instances keep drawing the old team's
/// instructions at spawn until restart.
///
/// A no-match insert has no prior roster, so its whole roster is the added
/// delta — symmetric with `commit_team_create`. Injected persistence keeps it
/// `AppHandle`-free so the prior-roster capture and delta direction are
/// unit-testable; a `persist_teams` error propagates, agent IO is best-effort
/// (mirrors the local command path: the authoritative team write already
/// landed, and boot repair is the designed retry for a stale binding).
fn commit_inbound_team(
    teams: &mut Vec<TeamRecord>,
    d_tag: String,
    inbound: TeamEventContent,
    persist_teams: impl FnOnce(&[TeamRecord]) -> Result<(), String>,
    load_agents: impl FnOnce() -> Result<Vec<ManagedAgentRecord>, String>,
    save_agents: impl FnOnce(&[ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), String> {
    let team_id = d_tag.clone();
    let previous_persona_ids = teams
        .iter()
        .find(|record| record.id == team_id)
        .map(|record| record.persona_ids.clone())
        .unwrap_or_default();
    apply_inbound_team(teams, d_tag, inbound);
    let current_persona_ids = teams
        .iter()
        .find(|record| record.id == team_id)
        .map(|record| record.persona_ids.clone())
        .unwrap_or_default();
    persist_teams(teams)?;
    crate::commands::teams::propagate_membership_best_effort(
        &team_id,
        &previous_persona_ids,
        &current_persona_ids,
        load_agents,
        save_agents,
    );
    Ok(())
}

/// Merge an inbound kind:30176 team projection into the local set.
///
/// Matches the local record whose `id` equals the event's d-tag (the d-tag IS
/// the team id — see `build_team_event`). On match, overwrite ONLY the three
/// shared fields (`name`, `description`, `persona_ids`); install-specific local
/// fields (`source_dir`, `is_symlink`, `symlink_target`, `is_builtin`,
/// `version`, `created_at`) are preserved. On no match, insert a fresh record
/// reusing the d-tag as the id so a re-received event stays idempotent —
/// symmetric to the persona path, since a team (like a persona) is a secretless
/// definition that another device may legitimately learn about from the relay.
fn apply_inbound_team(teams: &mut Vec<TeamRecord>, d_tag: String, inbound: TeamEventContent) {
    match teams.iter_mut().find(|record| record.id == d_tag) {
        Some(local) => {
            local.name = inbound.name;
            local.description = inbound.description;
            // `None` means the event came from a client that predates
            // always-publish — its true value is unknown, so preserve
            // local. Only `Some` (including the explicit-clear variants)
            // overwrites. See `TeamEventContent` for the wire rules.
            if let Some(instructions) = inbound.instructions {
                local.instructions = instructions;
            }
            if let Some(persona_ids) = inbound.persona_ids {
                local.persona_ids = persona_ids;
            }
        }
        None => teams.push(TeamRecord {
            id: d_tag,
            name: inbound.name,
            description: inbound.description,
            // Fresh insert has no local value to preserve; `None` from a
            // pre-fix client simply means no known value.
            instructions: inbound.instructions.unwrap_or_default(),
            persona_ids: inbound.persona_ids.unwrap_or_default(),
            is_builtin: false,
            // Catalog share state is scoped and never inbound-authoritative.
            shared: false,
            // Owner-device sync, not a catalog add: the team is this owner's
            // own, so it has no foreign publication to attribute.
            catalog_source: None,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: now_iso(),
            updated_at: now_iso(),
        }),
    }
}
