use tauri::AppHandle;

/// Apply only private agent authority and managed-agent tombstones during
/// native boot history recovery. No public-policy restart or catalog mutation.
pub(crate) fn reconcile_managed_agent_bootstrap_event<R: tauri::Runtime>(
    event: &nostr::Event,
    arrival_relay_url: &str,
    app: &AppHandle<R>,
) -> Result<(), String> {
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT};
    use nostr::JsonUtil;

    let managed_deletion = event.kind.as_u16() as u32 == KIND_DELETION
        && super::inbound::parse_deletion_coordinate(event).is_some_and(|(kind, _)| {
            matches!(kind, KIND_MANAGED_AGENT | KIND_PRIVATE_MANAGED_AGENT)
        });
    if event.kind.as_u16() as u32 == KIND_PRIVATE_MANAGED_AGENT || managed_deletion {
        super::inbound::reconcile_inbound_persona_event_blocking(
            event.as_json(),
            arrival_relay_url.to_string(),
            app.clone(),
        )?;
    }
    Ok(())
}

/// A newer public-only recreation is enough to preserve an existing local
/// identity, not enough to reconstruct private config or consume public policy.
/// Retain the historical deletion watermark without destructive cleanup.
/// `public_heads` contains signature/owner-verified history from this bootstrap.
pub(crate) fn retain_bootstrap_deletion_with_public_witness<R: tauri::Runtime>(
    event: &nostr::Event,
    public_heads: &std::collections::HashMap<String, u64>,
    arrival_relay_url: &str,
    app: &AppHandle<R>,
) -> Result<bool, String> {
    use crate::managed_agents::retention::*;
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT};
    use nostr::JsonUtil;
    use tauri::Manager;

    if event.kind.as_u16() as u32 != KIND_DELETION {
        return Ok(false);
    }
    let Some((kind, agent)) = super::inbound::parse_deletion_coordinate(event) else {
        return Ok(false);
    };
    if !matches!(kind, KIND_MANAGED_AGENT | KIND_PRIVATE_MANAGED_AGENT) {
        return Ok(false);
    }
    if public_heads
        .get(&agent)
        .is_none_or(|timestamp| *timestamp <= event.created_at.as_secs())
    {
        return Ok(false);
    }
    let state = app.state::<crate::app_state::AppState>();
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let scope = arrival_retention_scope(app, &state, arrival_relay_url)?
        .ok_or("managed-agent bootstrap scope changed")?;
    if scope.owner_keys.public_key() != event.pubkey {
        return Err("bootstrap deletion owner changed".into());
    }
    let conn = open_retention_db(&scope.db_path)?;
    // An already-prepared exact deletion must finish; a later head cannot
    // cancel local cleanup that was committed before this bootstrap began.
    if deletion_intent::pending(&conn, &event.pubkey.to_hex(), &agent)? {
        return Ok(false);
    }
    let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    retain_inbound_event(
        &transaction,
        &RetainedEvent {
            kind: KIND_DELETION,
            pubkey: event.pubkey.to_hex(),
            d_tag: tombstone_retention_d_tag(kind, &agent),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )?;
    // Preserve the recreated identity, not an older deleted private config.
    // The public witness must not let a covered private patch survive hydration.
    let private_is_deleted = get_retained_event(
        &transaction,
        KIND_PRIVATE_MANAGED_AGENT,
        &event.pubkey.to_hex(),
        &agent,
    )?
    .is_some_and(|head| head.created_at <= event.created_at.as_secs() as i64);
    if private_is_deleted {
        delete_retained_event(
            &transaction,
            KIND_PRIVATE_MANAGED_AGENT,
            &event.pubkey.to_hex(),
            &agent,
        )?;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    state
        .private_managed_agent_overlay
        .lock()
        .map_err(|e| e.to_string())?
        .refresh_config_authority(&conn, &scope.owner_keys, &agent)?;
    Ok(true)
}
