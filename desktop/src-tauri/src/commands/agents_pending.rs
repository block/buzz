//! Retention-queue helpers for managed-agent lifecycle events: pending
//! upserts, NIP-09 tombstones, and NIP-IA archive requests. Split from
//! `agents.rs` (which mounts this as `mod pending`) purely along the
//! retention seam. Deletion witnesses are prepared here and asynchronously
//! signed by `deletion_witness` before the destructive store phase.

use tauri::AppHandle;

use crate::{app_state::AppState, managed_agents::ManagedAgentRecord};

/// Prepare after a disk-authoritative save, under the store lock. Finish outside
/// the lock; errors remain best-effort and boot reconciliation is the backstop.
pub(crate) fn prepare_managed_agent_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    record: &ManagedAgentRecord,
) -> crate::managed_agents::reconcile::AgentRetentionWork {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    crate::managed_agents::reconcile::prepare_agent_record(
        &scope.db_path,
        &scope.owner_signer(),
        record,
    )
}

pub(crate) async fn finish_managed_agent_pending<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    work: crate::managed_agents::reconcile::AgentRetentionWork,
) {
    let result = match crate::managed_agents::managed_agents_base_dir(app) {
        Ok(base) => {
            crate::managed_agents::reconcile::finish_agent_record(
                work,
                &base,
                &state.managed_agents_store_lock,
            )
            .await
        }
        Err(error) => Err(error),
    };
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-retain: {e}");
    }
}

/// Extract `persona_id` from a retained kind:30177 head's content projection.
/// Absent (definition-less agent) or unparseable content yields `None`, so the
/// archive request falls back to an empty payload — exactly what the record's
/// `None` persona_id produced before this was derived from the head.
pub(super) fn persona_id_from_head(content: &str) -> Option<String> {
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
pub(crate) async fn build_agent_archive_request(
    signer: &crate::active_user_signer::ActiveUserSigner,
    agent_pubkey: &str,
    persona_id: Option<&str>,
) -> Result<nostr::EventBuilder, String> {
    let auth_tag = if signer
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(agent_pubkey)
    {
        None
    } else {
        let agent = nostr::PublicKey::from_hex(agent_pubkey)
            .map_err(|e| format!("invalid agent pubkey: {e}"))?;
        let tag_json = signer
            .authorize_agent(&agent, "")
            .await
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
    )
}

#[cfg(test)]
mod tests {
    use crate::managed_agents::retention::{
        get_pending_sync, get_retained_event, open_retention_db, retain_event, RetainedEvent,
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;

    async fn tombstone_managed_agent_at(
        db: &std::path::Path,
        keys: &nostr::Keys,
        agent: &str,
    ) -> Result<(), String> {
        use super::super::deletion_witness::{commit_agent_deletions, prepare_agent_tombstone};
        let signer = crate::active_user_signer::ActiveUserSigner::local(keys.clone());
        let witness = prepare_agent_tombstone(db, &signer, agent)?
            .sign(&signer)
            .await;
        commit_agent_deletions(vec![Ok(witness)], || Ok(()))
    }

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

    #[tokio::test]
    async fn agent_tombstone_created_at_strictly_dominates_a_future_dated_head() {
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

        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .await
            .unwrap();

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

    #[tokio::test]
    async fn agent_tombstone_rolls_back_head_purge_when_enqueue_fails() {
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

        // Persistence errors are best-effort at the real delete seam.
        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .await
            .unwrap();

        let conn = open_retention_db(&db_path).unwrap();
        assert!(
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, AGENT_PUBKEY)
                .unwrap()
                .is_some(),
            "the 30177 head must survive when the tombstone enqueue fails"
        );
    }

    #[tokio::test]
    async fn agent_tombstone_enqueues_archive_with_persona_id_from_head_atomically() {
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

        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .await
            .unwrap();

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

    #[tokio::test]
    async fn agent_tombstone_rolls_back_kind5_when_archive_enqueue_fails() {
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

        // Persistence errors are best-effort at the real delete seam.
        tombstone_managed_agent_at(&db_path, &keys, AGENT_PUBKEY)
            .await
            .unwrap();

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
