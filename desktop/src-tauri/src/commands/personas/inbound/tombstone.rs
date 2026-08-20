//! Inbound NIP-09 tombstone reconciliation, extracted from `inbound.rs` to keep
//! that file under the file-size cap. Mirrors the upsert spine but removes
//! rather than patches.

use tauri::{AppHandle, Emitter};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_agent_definitions, load_personas, persona_events::persona_d_tag, save_personas,
        try_regenerate_nest, MutationRoute,
    },
};

/// Parse a NIP-09 `a`-tag coordinate `<kind>:<owner_pubkey>:<d_tag>` into its
/// target kind and d-tag. Returns `None` if the tag is absent or malformed, so
/// the caller no-ops on a tombstone it can't route.
pub(super) fn parse_deletion_coordinate(event: &nostr::Event) -> Option<(u32, String)> {
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
pub(super) fn reconcile_inbound_tombstone(
    event: &nostr::Event,
    arrival_relay_url: &str,
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    use crate::managed_agents::{
        load_managed_agents, load_teams,
        retention::{
            open_retention_db, retain_inbound_event, tombstone_retention_d_tag, InboundOutcome,
            RetainedEvent,
        },
        save_managed_agents, save_teams,
    };
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PERSONA, KIND_TEAM};
    use nostr::JsonUtil;

    let Some((target_kind, target_d_tag)) = parse_deletion_coordinate(event) else {
        return Ok(()); // no routable coordinate — nothing to delete
    };
    if !matches!(target_kind, KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT) {
        return Ok(()); // deletion for a kind we don't track locally
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve against the retained tombstone row (keyed by the target
    // coordinate, F2c) so a re-received tombstone or one older than a pending
    // local edit is a no-op. Scoped to the arrival community + owner, so a
    // workspace switch since arrival drops the tombstone instead of retaining
    // it — and deleting a record — in the wrong community's or owner's store.
    let tombstone_owner_pubkey = event.pubkey.to_hex();
    let Some(scope) = crate::managed_agents::retention::arrival_retention_scope(
        app,
        state,
        arrival_relay_url,
        &tombstone_owner_pubkey,
    )?
    else {
        return Ok(());
    };

    // Library-projection preflight (§2.7): a projected persona is
    // library-authoritative, so an inbound tombstone targeting it must NOT delete
    // the local record OR advance the retention head — the future library-aware
    // handler owns removing that coordinate (as a §3.4 workspace-remove). Return
    // WITHOUT retaining (Ok, unretained) so the tombstone is reprocessed once that
    // handler lands. Routes on the tombstone's `target_d_tag` — the same
    // `persona_d_tag`-derived key the KIND_PERSONA `retain` below matches on —
    // against the RAW keyless record. Only KIND_PERSONA tombstones touch the
    // persona store; team/agent removals are out of scope here.
    if target_kind == KIND_PERSONA {
        let raw_definitions = load_agent_definitions(app)?;
        if MutationRoute::for_persona_d_tag(&raw_definitions, &target_d_tag)
            == MutationRoute::LibraryProjected
        {
            return Ok(());
        }
    }

    let conn = open_retention_db(&scope.db_path)?;
    let outcome = retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_DELETION,
            pubkey: event.pubkey.to_hex(),
            d_tag: tombstone_retention_d_tag(target_kind, &target_d_tag),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )?;
    if outcome == InboundOutcome::Skipped {
        return Ok(());
    }

    // Remove the local record using the SAME per-kind match rule the apply fns
    // use: persona by `persona_d_tag`, team by `id`, managed-agent by `pubkey`.
    match target_kind {
        KIND_PERSONA => {
            let mut personas = load_personas(app)?;
            personas.retain(|record| persona_d_tag(record) != target_d_tag);
            save_personas(app, &personas)?;
        }
        KIND_TEAM => {
            let mut teams = load_teams(app)?;
            teams.retain(|record| record.id != target_d_tag);
            save_teams(app, &teams)?;
        }
        KIND_MANAGED_AGENT => {
            let mut agents = load_managed_agents(app)?;
            agents.retain(|record| record.pubkey != target_d_tag);
            save_managed_agents(app, &agents)?;
        }
        _ => unreachable!("target kind gated above"),
    }
    try_regenerate_nest(app);

    // Refresh the live UI on inbound deletion — a removal is as user-visible as
    // an upsert and the Agents tab must drop the tombstoned record without restart.
    let _ = app.emit("agents-data-changed", ());

    Ok(())
}
