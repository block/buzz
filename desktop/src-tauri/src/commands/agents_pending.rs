//! Retention-queue helpers for managed-agent lifecycle events: pending
//! upserts, NIP-09 tombstones, and NIP-IA archive requests. Split from
//! `agents.rs` (which mounts this as `mod pending`) purely along the
//! retention seam; every function runs inside the
//! `managed_agents_store_lock`-held body and NEVER across an `.await`.

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
/// publish. Retention failures propagate: disk persistence alone is not a
/// successful relay-primary configuration write.
///
/// Also writes the just-retained kind:30179 head back through to the in-memory
/// overlay. This is the ONLY path by which the overlay learns config this
/// device authored — inbound `insert_patch` never fires for our own event
/// (the relay echo dedupes to `Skipped`) and boot hydration runs once per
/// launch — so without it a second edit in the same session resolves the
/// stale patch onto the fresher disk record and publishes a silent revert of
/// the first edit. Overlay lock is taken UNDER `managed_agents_store_lock`,
/// which every caller already holds: the established order, same as
/// `resolved_local_record`.
pub(crate) fn retain_managed_agent_pending(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) -> Result<(), String> {
    use crate::managed_agents::{reconcile::retain_agent_record, retention::open_retention_db};

    (|| -> Result<(), String> {
        crate::managed_agents::private_config_overlay::require_authority_ready(state)?;
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let conn = open_retention_db(&scope.db_path)?;
        // Shared engine with the boot-time reconcile: projection content diff
        // (no republish for runtime-only churn) + monotonic created_at bump
        // past the retained head (NIP-AP step 3).
        retain_agent_record(&conn, &scope.owner_keys, record)?;
        state
            .private_managed_agent_overlay
            .lock()
            .map_err(|error| error.to_string())?
            .absorb_retained_head(&conn, &scope.owner_keys, &record.pubkey)
    })()
}

/// Durably prepare deletion BEFORE removing JSON or key material.
/// Caller holds the store lock. Tombstones, archive, head purge, and exact
/// cleanup intent commit together; cleanup completion clears only the intent.
pub(crate) fn tombstone_managed_agent_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    agent_pubkey: &str,
) -> Result<(), String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    tombstone_managed_agent_at(&scope.db_path, &scope.owner_keys, agent_pubkey)
}

/// Atomic scope-bound tombstones/archive and recoverable cleanup intent.
pub(crate) fn tombstone_managed_agent_at(
    db_path: &std::path::Path,
    keys: &nostr::Keys,
    agent_pubkey: &str,
) -> Result<(), String> {
    use crate::managed_agents::{
        agent_events::{build_agent_delete, build_private_agent_delete},
        persona_events::monotonic_created_at,
        retention::{
            delete_retained_event, get_retained_event, open_retention_db, retain_event,
            tombstone_retention_d_tag, RetainedEvent,
        },
    };
    use buzz_core_pkg::kind::{
        KIND_IA_ARCHIVE_REQUEST, KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT,
    };
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let owner_pubkey = keys.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    if crate::managed_agents::retention::deletion_intent::pending(
        &conn,
        &owner_pubkey,
        agent_pubkey,
    )? {
        return Ok(());
    }
    // Single transaction: a kill between the head purge and the tombstone
    // enqueue would otherwise leave the 30177 head live with no local retry
    // witness. Reading the head's `created_at` inside the same `BEGIN
    // IMMEDIATE` closes both the crash window and the read-then-sign race —
    // and lets the kind:5 be signed strictly past a future-dated head
    // (`retain_agent_record` bumps a same-second re-publish past the prior
    // head) so it cannot survive its own tombstone once the head row is
    // purged. Mirrors the persona/team tombstone helpers.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("failed to begin managed-agent tombstone transaction: {e}"))?;
    let result = (|| -> Result<(), String> {
        let prior_head =
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, agent_pubkey)?;
        let prior_private_head = get_retained_event(
            &conn,
            KIND_PRIVATE_MANAGED_AGENT,
            &owner_pubkey,
            agent_pubkey,
        )?;
        // One deletion instant for both coordinates: strictly past the newer
        // of the two heads, so neither can survive its tombstone.
        let deleted_at = monotonic_created_at(
            prior_head
                .as_ref()
                .map(|row| row.created_at)
                .max(prior_private_head.as_ref().map(|row| row.created_at)),
        );
        let public_delete = build_agent_delete(agent_pubkey, &owner_pubkey)?
            .custom_created_at(deleted_at)
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign managed-agent tombstone: {e}"))?;
        let private_delete = build_private_agent_delete(agent_pubkey, &owner_pubkey)?
            .custom_created_at(deleted_at)
            .sign_with_keys(keys)
            .map_err(|e| format!("failed to sign private managed-agent tombstone: {e}"))?;
        // Recover the archive's `persona_id` from the head that is about to be
        // purged, where it survives as owner-signed historical alias data.
        let persona_id = prior_head
            .as_ref()
            .and_then(|row| persona_id_from_head(&row.content));
        let archive = build_agent_archive_request(keys, agent_pubkey, persona_id.as_deref())?;
        for (target_kind, event) in [
            (KIND_MANAGED_AGENT, public_delete),
            (KIND_PRIVATE_MANAGED_AGENT, private_delete),
        ] {
            delete_retained_event(&conn, target_kind, &owner_pubkey, agent_pubkey)?;
            retain_event(
                &conn,
                &RetainedEvent {
                    kind: KIND_DELETE,
                    pubkey: owner_pubkey.clone(),
                    // Key by the target coordinate so cross-kind d-tag tombstones
                    // occupy distinct rows (F2c).
                    d_tag: tombstone_retention_d_tag(target_kind, agent_pubkey),
                    content: event.content.to_string(),
                    created_at: event.created_at.as_secs() as i64,
                    raw_event: event.as_json(),
                    pending_sync: true,
                },
            )?;
        }
        crate::managed_agents::retention::deletion_intent::record(
            &conn,
            &owner_pubkey,
            agent_pubkey,
            deleted_at.as_secs() as i64,
        )?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_IA_ARCHIVE_REQUEST,
                pubkey: owner_pubkey.clone(),
                d_tag: agent_pubkey.to_string(),
                content: archive.content.to_string(),
                created_at: archive.created_at.as_secs() as i64,
                raw_event: archive.as_json(),
                pending_sync: true,
            },
        )
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("failed to commit managed-agent tombstone transaction: {e}")),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Extract `persona_id` from a retained kind:30177 head's content projection.
/// Absent (definition-less agent) or unparseable content yields `None`, so the
/// archive request falls back to an empty payload — exactly what the record's
/// `None` persona_id produced before this was derived from the head.
fn persona_id_from_head(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("persona_id")?
        .as_str()
        .map(str::to_owned)
}

/// Build an owner-authenticated NIP-IA `kind:9035` archive request for a deleted agent.
/// Definition-linked agents carry the persona id in `content`, where it survives the
/// kind:30177 tombstone as owner-signed historical alias data. The request uses the
/// same builder as the GUI Archive action and the NIP-IA `retired` reason.
pub(crate) fn build_agent_archive_request(
    keys: &nostr::Keys,
    agent_pubkey: &str,
    persona_id: Option<&str>,
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
    let content = persona_id
        .filter(|id| !id.trim().is_empty())
        .map(|id| serde_json::json!({ "persona_id": id }).to_string())
        .unwrap_or_default();
    crate::events::build_archive_identity_request(
        agent_pubkey,
        &content,
        Some("retired"),
        None,
        auth_tag.as_ref(),
    )?
    .sign_with_keys(keys)
    .map_err(|e| format!("failed to sign archive request: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::{
        get_pending_sync, get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;

    // A valid 32-byte x-only pubkey hex — the folded archive request derives an
    // owner auth tag, which parses `agent_pubkey`, so it must be well-formed.
    const AGENT_PUBKEY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// Seed a retained 30177 agent head dated `created_at` seconds since epoch.
    /// The tombstone helper reads only the head's `created_at`, so the content
    /// need not be a full agent projection.
    fn seed_agent_head(db_path: &std::path::Path, owner: &str, created_at: i64) {
        seed_agent_head_content(db_path, owner, created_at, r#"{"name":"Agent"}"#);
    }

    /// Like [`seed_agent_head`] but with explicit head `content`, so the
    /// archive-payload derivation from the head can be asserted.
    fn seed_agent_head_content(
        db_path: &std::path::Path,
        owner: &str,
        created_at: i64,
        content: &str,
    ) {
        let conn = open_retention_db(db_path).unwrap();
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: owner.to_string(),
                d_tag: AGENT_PUBKEY.to_string(),
                content: content.to_string(),
                created_at,
                raw_event: r#"{"id":"seed"}"#.to_string(),
                pending_sync: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn local_tombstone_prepares_exact_cleanup_and_retry_does_not_reauthor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        seed_agent_head(&path, &owner, 10);
        tombstone_managed_agent_at(&path, &keys, AGENT_PUBKEY).unwrap();
        let conn = open_retention_db(&path).unwrap();
        assert_eq!(
            crate::managed_agents::retention::deletion_intent::agents(&conn, &owner).unwrap(),
            vec![AGENT_PUBKEY]
        );
        let before: Vec<_> = get_pending_sync(&conn)
            .unwrap()
            .into_iter()
            .map(|row| row.raw_event)
            .collect();
        tombstone_managed_agent_at(&path, &keys, AGENT_PUBKEY).unwrap();
        let after: Vec<_> = get_pending_sync(&conn)
            .unwrap()
            .into_iter()
            .map(|row| row.raw_event)
            .collect();
        assert_eq!(before, after, "retry must not mint a newer deletion");
    }

    #[test]
    fn agent_tombstone_created_at_strictly_dominates_a_future_dated_head() {
        // The retained 30177 head may be future-dated (retain_agent_record
        // bumps a same-second re-publish past the prior head). The relay only
        // soft-deletes coordinate versions with created_at <= the tombstone's,
        // and the flush loop never re-reads the (purged) head — so a kind:5
        // signed at wall-clock `now` would leave the agent live forever once
        // its local retry witness is gone.
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = dir.path().join("retention.sqlite3");

        let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
        seed_agent_head(&db_path, &owner, future);

        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY).unwrap();

        let conn = open_retention_db(&db_path).unwrap();
        let tombstone = get_pending_sync(&conn)
            .unwrap()
            .into_iter()
            .find(|row| row.kind == 5)
            .expect("a kind:5 agent tombstone is enqueued");
        assert!(
            tombstone.created_at > future,
            "tombstone created_at ({}) must strictly dominate the future-dated head ({future})",
            tombstone.created_at
        );
        assert!(
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, AGENT_PUBKEY)
                .unwrap()
                .is_none(),
            "the 30177 head is purged so no stale edit can republish it"
        );
    }

    #[test]
    fn agent_tombstone_rolls_back_head_purge_when_enqueue_fails() {
        // The head purge and kind:5 enqueue run in one `BEGIN IMMEDIATE`
        // transaction. A `BEFORE INSERT` trigger blocks the enqueue (which
        // follows the head DELETE); the whole transaction must roll back so the
        // 30177 head survives with its local retry witness intact.
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = dir.path().join("retention.sqlite3");

        let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
        seed_agent_head(&db_path, &owner, future);

        let conn = open_retention_db(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER block_all_inserts BEFORE INSERT ON persona_events
             BEGIN
                 SELECT RAISE(ABORT, 'insert blocked by test trigger');
             END;",
        )
        .unwrap();
        drop(conn);

        let err = tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .expect_err("tombstone with INSERT trigger must fail");
        assert!(
            err.contains("insert blocked by test trigger") || err.contains("blocked"),
            "error must name the trigger cause; got: {err}"
        );

        let conn = open_retention_db(&db_path).unwrap();
        assert!(
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, AGENT_PUBKEY)
                .unwrap()
                .is_some(),
            "the 30177 head must survive when the tombstone enqueue fails"
        );
    }

    #[test]
    fn agent_tombstone_enqueues_archive_with_persona_id_from_head_atomically() {
        // FOLD-4: the kind:5 tombstone and the NIP-IA kind:9035 archive request
        // are enqueued in ONE transaction, and the archive's `persona_id`
        // payload is derived from the retained 30177 head's content (not the
        // already-deleted record). Both rows must be present and pending after
        // a successful tombstone.
        use buzz_core_pkg::kind::KIND_IA_ARCHIVE_REQUEST;

        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = dir.path().join("retention.sqlite3");

        let now = nostr::Timestamp::now().as_secs() as i64;
        seed_agent_head_content(
            &db_path,
            &owner,
            now,
            r#"{"name":"Agent","persona_id":"persona-abc"}"#,
        );

        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY).unwrap();

        let conn = open_retention_db(&db_path).unwrap();
        let pending = get_pending_sync(&conn).unwrap();
        assert!(
            pending.iter().any(|row| row.kind == 5),
            "a kind:5 tombstone is enqueued"
        );
        let archive = pending
            .iter()
            .find(|row| row.kind == KIND_IA_ARCHIVE_REQUEST)
            .expect("a kind:9035 archive request is enqueued in the same transaction");
        assert!(
            archive.content.contains("persona-abc"),
            "archive payload derives persona_id from the retained head; got: {}",
            archive.content
        );
    }

    #[test]
    fn agent_tombstone_rolls_back_kind5_when_archive_enqueue_fails() {
        // FOLD-4 atomicity: the kind:5 tombstone and kind:9035 archive share one
        // `BEGIN IMMEDIATE`. A trigger blocks ONLY the 9035 insert (which
        // follows the kind:5 insert); the whole transaction must roll back so
        // NEITHER the tombstone nor a purged head is left behind. Splitting the
        // two enqueues into separate transactions turns this RED — the kind:5
        // would commit and the head would be gone while the archive is lost.
        use buzz_core_pkg::kind::KIND_MANAGED_AGENT;

        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = dir.path().join("retention.sqlite3");

        let now = nostr::Timestamp::now().as_secs() as i64;
        seed_agent_head(&db_path, &owner, now);

        let conn = open_retention_db(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER block_archive_insert BEFORE INSERT ON persona_events
             WHEN NEW.kind = 9035
             BEGIN
                 SELECT RAISE(ABORT, 'archive insert blocked by test trigger');
             END;",
        )
        .unwrap();
        drop(conn);

        let err = tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .expect_err("tombstone must fail when the archive enqueue is blocked");
        assert!(
            err.contains("archive insert blocked") || err.contains("blocked"),
            "error must name the trigger cause; got: {err}"
        );

        let conn = open_retention_db(&db_path).unwrap();
        assert!(
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, AGENT_PUBKEY)
                .unwrap()
                .is_some(),
            "the 30177 head must survive — the whole transaction rolls back"
        );
        assert!(
            get_pending_sync(&conn)
                .unwrap()
                .iter()
                .all(|row| row.kind != 5),
            "no kind:5 tombstone may be committed when the archive enqueue fails"
        );
    }
}
