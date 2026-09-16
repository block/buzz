//! Installed auth and loopback NIP44 against production archive plan/prepare/
//! commit. These do NOT claim the still-unwired sync retry/lifecycle is ready.
use super::*;
use crate::active_user_signer::ActiveUserSigner;
use crate::archive::{
    ingress_test_support::*, ArchiveCandidate, ArchiveDb, MatchedScope, ScopeType,
};
use nostr::{Event, Kind, Tag};

const RELAY: &str = "wss://archive.example";

pub(super) fn event(owner: &Keys, kind: u16, body: &str) -> Event {
    let agent = Keys::generate();
    let ciphertext = nostr::nips::nip44::encrypt(
        agent.secret_key(),
        &owner.public_key(),
        body,
        nostr::nips::nip44::Version::V2,
    )
    .unwrap();
    EventBuilder::new(Kind::Custom(kind), ciphertext)
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
            Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", "telemetry"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap()
}

fn candidate(event: &Event, owner: &str) -> ArchiveCandidate {
    ArchiveCandidate {
        raw_event_json: event.as_json(),
        matched_scope: MatchedScope {
            scope_type: ScopeType::OwnerP,
            scope_value: owner.into(),
        },
    }
}

async fn database(owner: &str) -> (tempfile::TempDir, Arc<ArchiveDb>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(ArchiveDb::with_test_path(dir.path().join("archive.db")));
    let owner = owner.to_owned();
    db.with_conn(move |conn| {
        crate::archive::store::upsert_save_subscription(
            conn,
            &owner,
            RELAY,
            "owner_p",
            &owner,
            "[24200,44200]",
            0,
        )
    })
    .await
    .unwrap();
    (dir, db)
}

async fn prepare(
    db: &ArchiveDb,
    signer: &ActiveUserSigner,
    candidates: Vec<ArchiveCandidate>,
    relay_failed: bool,
) -> PreparedBatch {
    let owner = signer.public_key().to_hex();
    let plan = db
        .with_conn(move |conn| plan_archive(candidates, &owner, RELAY, conn))
        .await
        .unwrap();
    // Inject only the scoped relay witness; crypto, database, installed auth,
    // and all body/processed-marker decisions are production implementations.
    let buckets = plan
        .buckets
        .into_iter()
        .map(|b| BucketWithResult {
            returned_ids: b.group.iter().map(|p| p.event.id.to_hex()).collect(),
            scope_type_str: b.scope_type_str,
            scope_value: b.scope_value,
            allowed_kinds: b.allowed_kinds,
            group: b.group,
            relay_failed,
        })
        .collect();
    prepare_archive(buckets, plan.ephemeral, plan.pre_dropped, signer).await
}

async fn counts(db: &ArchiveDb) -> (i64, i64, i64) {
    db.with_conn(|conn| {
        Ok((
            conn.query_row("SELECT COUNT(*) FROM archived_events", [], |r| r.get(0))
                .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM observer_channel_index", [], |r| {
                r.get(0)
            })
            .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM agent_metric_index", [], |r| r.get(0))
                .unwrap(),
        ))
    })
    .await
    .unwrap()
}

async fn commit(
    db: &ArchiveDb,
    signer: ActiveUserSigner,
    batch: PreparedBatch,
) -> crate::archive::ArchiveBatchResult {
    let owner = signer.public_key().to_hex();
    db.with_conn(move |conn| commit_ready(&batch, &owner, RELAY, 10, conn, || signer.check_valid()))
        .await
        .unwrap()
}

pub(super) const METRIC: &str = r#"{"harness":"test","timestamp":"2026-07-01T00:00:00Z","turn":{"inputTokens":7},"deltaReliable":true}"#;

#[tokio::test]
async fn archive_ingress_mixed_ready_subset_remote_failure_retry_and_local_equivalence() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let owner = signer.public_key().to_hex();
    let (_dir, db) = database(&owner).await;
    let observer = event(&api.state.keys, 24200, r#"{"channelId":"channel"}"#);
    let metric = event(&api.state.keys, 44200, METRIC);
    let retry_metric = event(&api.state.keys, 44200, METRIC);
    let bad_observer = event(&api.state.keys, 24200, "not-json");
    let bad_metric = event(&api.state.keys, 44200, "{}");
    let events = [
        &observer,
        &metric,
        &retry_metric,
        &bad_observer,
        &bad_metric,
    ];
    *api.state.decrypt_fail_ciphertext.lock().unwrap() = Some(retry_metric.content.clone());
    let mut batch = prepare(
        &db,
        &signer,
        events.iter().map(|e| candidate(e, &owner)).collect(),
        false,
    )
    .await;
    assert_eq!(batch.retry.len(), 1);
    assert_eq!(batch.retry[0].raw_event_json, retry_metric.as_json());
    let pending = std::mem::take(&mut batch.retry);
    let result = commit(&db, signer.clone(), batch).await;
    assert_eq!(
        (
            result.persisted,
            result.persisted_agent_metrics,
            result.dropped
        ),
        (3, 1, 1)
    );
    assert_eq!(counts(&db).await, (3, 2, 1));
    // Transient decrypt failure keeps the full installed session, no re-login.
    assert_eq!(
        state.active_signer().unwrap().generation(),
        signer.generation()
    );
    *api.state.decrypt_fail_ciphertext.lock().unwrap() = None;
    let batch = prepare(&db, &signer, pending, false).await;
    assert!(batch.retry.is_empty());
    assert_eq!(
        commit(&db, signer.clone(), batch)
            .await
            .persisted_agent_metrics,
        1
    );
    let batch = prepare(&db, &signer, vec![candidate(&retry_metric, &owner)], false).await;
    assert_eq!(commit(&db, signer, batch).await.persisted_agent_metrics, 0);
    assert_eq!(counts(&db).await, (4, 2, 2));

    let (_local_dir, local_db) = database(&owner).await;
    let local = ActiveUserSigner::local(api.state.keys.clone());
    let batch = prepare(
        &local_db,
        &local,
        events.iter().map(|e| candidate(e, &owner)).collect(),
        false,
    )
    .await;
    commit(&local_db, local, batch).await;
    assert_eq!(counts(&local_db).await, counts(&db).await);
    for db in [&local_db, &db] {
        db.with_conn(|conn| {
            let metric: String = conn
                .query_row(
                    "SELECT raw_json FROM archived_events WHERE kind=44200 LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&metric).unwrap()["turn"]["inputTokens"],
                7
            );
            let channel: String = conn
                .query_row(
                    "SELECT channel_id FROM observer_channel_index WHERE channel_id IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(channel, "channel");
            Ok(())
        })
        .await
        .unwrap();
    }
    assert_eq!(
        api.state
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.0 == "v1/auth/login/exchange")
            .count(),
        1
    );
}

#[tokio::test]
async fn archive_ingress_http_wait_releases_writer_and_maintenance_then_cancel_retains_ciphertext()
{
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let owner = signer.public_key().to_hex();
    let (_dir, db) = database(&owner).await;
    let observer = event(&api.state.keys, 24200, r#"{"channelId":"secret"}"#);
    let input = vec![candidate(&observer, &owner)];
    api.hold("v1/buzz/identity/decrypt");
    let work_db = db.clone();
    let task = tokio::spawn(async move { prepare(&work_db, &signer, input, false).await });
    api.arrived().await;
    assert!(db.maintenance_write_available());
    db.with_conn(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE writer_probe(x); COMMIT;")
            .unwrap();
        Ok(())
    })
    .await
    .unwrap();
    state.native_auth.clear().unwrap();
    let batch = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(batch.retry.len(), 1);
    assert_eq!(batch.retry[0].raw_event_json, observer.as_json());
    assert_eq!(counts(&db).await, (0, 0, 0));
    api.release();
}

#[tokio::test]
async fn archive_ingress_stale_admission_rolls_back_and_removed_subscription_is_rechecked() {
    let api = MockApi::new().await;
    let state =
        crate::app_state::build_app_state_for_mode(crate::native_identity::SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let owner = signer.public_key().to_hex();
    let (_dir, db) = database(&owner).await;
    let observer = event(&api.state.keys, 24200, r#"{"channelId":"channel"}"#);
    let batch = prepare(&db, &signer, vec![candidate(&observer, &owner)], false).await;
    api.login(&state.native_auth, "B").await.unwrap();
    let old_owner = owner.clone();
    let result = db
        .with_conn(move |conn| {
            commit_ready(&batch, &old_owner, RELAY, 10, conn, || signer.check_valid())
        })
        .await;
    assert!(result.is_err());
    assert_eq!(counts(&db).await, (0, 0, 0));
    let signer = state.active_signer().unwrap();
    let batch = prepare(&db, &signer, vec![candidate(&observer, &owner)], false).await;
    db.with_conn(|conn| {
        conn.execute("DELETE FROM save_subscriptions", []).unwrap();
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(commit(&db, signer, batch).await.persisted, 0);
    assert_eq!(counts(&db).await, (0, 0, 0));
}
