//! Managed-agent retention / tombstone / archive helpers.
//! Split from `agents.rs` for the desktop file-size ratchet.

use tauri::AppHandle;

use crate::{app_state::AppState, managed_agents::ManagedAgentRecord};

/// Retain a freshly authored managed-agent event in the local store, flagged
/// for relay sync. MUST be called inside the `managed_agents_store_lock`-held
/// body after `save_managed_agents`, NEVER across an `.await`: it acquires
/// `state.keys` and a retention-db connection, both `std::sync` guards, and
/// drops them before returning.
///
/// Owner-authored, mirroring `commands::personas::retain_persona_pending`: the
/// owner keys sign, the d_tag is the agent's pubkey, so the coordinate is
/// `30177:<owner>:<agent_pubkey>`. The event content is the opt-IN
/// [`agent_event_content`] projection — the retention upsert's content-equality
/// guard compares this projection, so an operational start/stop that mutates
/// only runtime fields produces an identical row and never re-enqueues a
/// publish. Best-effort: a failure here is logged and swallowed so a retention
/// hiccup never blocks the disk-authoritative write.
pub(crate) fn retain_managed_agent_pending(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) {
    use crate::managed_agents::{reconcile::retain_agent_record, retention::open_retention_db};

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let conn = open_retention_db(&scope.db_path)?;
        // Shared engine with the boot-time reconcile: projection content diff
        // (no republish for runtime-only churn) + monotonic created_at bump
        // past the retained head (NIP-AP step 3).
        retain_agent_record(&conn, &scope.owner_keys, record).map(|_| ())
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-retain: {e}");
    }
}

/// Purge a deleted agent's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body and NEVER across an
/// `.await`.
///
/// Mirrors `commands::personas::tombstone_persona_pending`: the agent row at
/// `(30177, owner, agent_pubkey)` is purged first so an unpublished edit can
/// never resurrect it after the tombstone publishes, then the kind:5 tombstone
/// is retained at its own `(5, owner, agent_pubkey)` coordinate with
/// `pending_sync = 1`. The `d_tag` is the agent's pubkey. Best-effort: a
/// failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative delete.
pub(crate) fn tombstone_managed_agent_pending(
    app: &AppHandle,
    state: &AppState,
    agent_pubkey: &str,
) {
    use crate::managed_agents::{
        agent_events::build_agent_delete,
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let owner_pubkey = scope.owner_keys.public_key().to_hex();
        let event = build_agent_delete(agent_pubkey, &owner_pubkey)?
            .sign_with_keys(&scope.owner_keys)
            .map_err(|e| format!("failed to sign managed-agent tombstone: {e}"))?;
        let conn = open_retention_db(&scope.db_path)?;
        delete_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, agent_pubkey)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey: owner_pubkey,
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_MANAGED_AGENT, agent_pubkey),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-tombstone: {e}");
    }
}

/// Build and sign the NIP-IA `kind:9035` archive request enqueued when an
/// agent is deleted. Pure given the keys — unit-testable without an
/// `AppHandle`. Reuses the same wire builder as the GUI's Archive action
/// (`events::build_archive_identity_request`); the machine-readable reason is
/// `retired` (NIP-IA suggested code for a deliberately decommissioned key).
///
/// The owner auth tag is minted locally from the same keys used to sign the
/// request, avoiding a network fetch while the managed-agent store lock is
/// held. The relay still independently verifies it against the agent's live
/// kind:0.
pub(crate) fn build_agent_archive_request(
    keys: &nostr::Keys,
    agent_pubkey: &str,
) -> Result<nostr::Event, String> {
    let auth_tag = if keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(agent_pubkey)
    {
        None
    } else {
        let agent = nostr::PublicKey::from_hex(agent_pubkey)
            .map_err(|e| format!("invalid agent pubkey: {e}"))?;
        let tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(keys, &agent, "")
            .map_err(|e| format!("failed to build owner auth tag: {e}"))?;
        let parts: Vec<String> = serde_json::from_str(&tag_json)
            .map_err(|e| format!("failed to parse owner auth tag: {e}"))?;
        Some(
            <[String; 4]>::try_from(parts)
                .map_err(|_| "owner auth tag must have four elements".to_string())?,
        )
    };
    crate::events::build_archive_identity_request(
        agent_pubkey,
        "",
        Some("retired"),
        None,
        auth_tag.as_ref(),
    )?
    .sign_with_keys(keys)
    .map_err(|e| format!("failed to sign archive request: {e}"))
}

/// Enqueue a NIP-IA `kind:9035` archive request for a deleted agent, retained
/// next to its kind:5 tombstone with `pending_sync = 1`.
///
/// The tombstone removes the agent's 30177 record cross-device, but the
/// agent's `kind:0` and channel membership keep populating member pickers and
/// autocomplete on the relay until the identity is archived. Retaining the
/// request here gives archival the same offline durability as the tombstone;
/// the flush loop is the sole publisher and re-signs the request with a fresh
/// `created_at` at publish time, because the relay enforces a ±120s freshness
/// window on 9035s.
///
/// Same contract as `tombstone_managed_agent_pending`: called inside the
/// `managed_agents_store_lock`-held delete body, never across an `.await`,
/// best-effort — a failure is logged and swallowed so it never blocks the
/// disk-authoritative delete.
pub(crate) fn archive_managed_agent_pending(app: &AppHandle, state: &AppState, agent_pubkey: &str) {
    use crate::managed_agents::retention::{open_retention_db, retain_event, RetainedEvent};
    use buzz_core_pkg::kind::KIND_IA_ARCHIVE_REQUEST;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let owner_pubkey = scope.owner_keys.public_key().to_hex();
        let event = build_agent_archive_request(&scope.owner_keys, agent_pubkey)?;
        let conn = open_retention_db(&scope.db_path)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_IA_ARCHIVE_REQUEST,
                pubkey: owner_pubkey,
                d_tag: agent_pubkey.to_string(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-archive: {e}");
    }
}
