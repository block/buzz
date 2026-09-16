//! Real login/adapter through the shared create/import preparation and deletion pipeline.
use super::*;
use crate::{
    active_user_signer::ActiveUserSigner,
    app_state::build_app_state_for_mode,
    commands::deletion_witness::{commit_agent_deletions, prepare_agent_tombstone},
    managed_agents::retention::{
        get_pending_sync, get_retained_event, open_retention_db, retain_event, RetainedEvent,
    },
    native_identity::SignerMode,
    owner_authorization::{prepare_agent, OwnerAuthorizationScope},
};
use std::sync::atomic::{AtomicBool, Ordering};

const OA: &str = "v1/buzz/identity/authorize-agent";

fn seed(db: &std::path::Path, owner: &str, agent: &str) {
    retain_event(
        &open_retention_db(db).unwrap(),
        &RetainedEvent {
            kind: 30177,
            pubkey: owner.into(),
            d_tag: agent.into(),
            content: r#"{"persona_id":"persona-proof"}"#.into(),
            created_at: 1,
            raw_event: "original-head".into(),
            pending_sync: false,
        },
    )
    .unwrap();
}

fn unchanged(db: &std::path::Path, owner: &str, agent: &str) {
    let conn = open_retention_db(db).unwrap();
    assert_eq!(
        get_retained_event(&conn, 30177, owner, agent)
            .unwrap()
            .unwrap()
            .raw_event,
        "original-head"
    );
    assert!(get_pending_sync(&conn).unwrap().is_empty());
}

#[tokio::test]
async fn local_proof_and_shared_create_import_mint_keep_sdk_wire_format() {
    let keys = Keys::generate();
    let owner = ActiveUserSigner::local(keys.clone());
    let agent = Keys::generate().public_key();
    for conditions in ["", "kind=1&created_at<4294967295"] {
        let proof = owner.authorize_agent(&agent, conditions).await.unwrap();
        let sdk = buzz_sdk_pkg::nip_oa::compute_auth_tag(&keys, &agent, conditions).unwrap();
        // SDK Schnorr signing uses randomized auxiliary entropy; compare shape,
        // identity/conditions and verification rather than randomized signature bytes.
        let proof_parts: [String; 4] = serde_json::from_str(&proof).unwrap();
        let sdk_parts: [String; 4] = serde_json::from_str(&sdk).unwrap();
        assert_eq!(proof_parts[..3], sdk_parts[..3]);
        assert_eq!(
            buzz_sdk_pkg::nip_oa::verify_auth_tag(&proof, &agent).unwrap(),
            keys.public_key()
        );
    }
    let minted = prepare_agent(&owner).await.unwrap();
    assert_ne!(minted.keys.public_key(), owner.public_key());
    assert_eq!(
        Keys::parse(&minted.private_key_nsec)
            .unwrap()
            .public_key()
            .to_hex(),
        minted.pubkey
    );
    assert_eq!(
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&minted.auth_tag.unwrap(), &minted.keys.public_key())
            .unwrap(),
        keys.public_key()
    );
}

#[tokio::test]
async fn remote_shared_create_import_and_repair_use_installed_owner_backend() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let scope = OwnerAuthorizationScope::capture(&state).unwrap();
    let minted = prepare_agent(&scope.signer).await.unwrap();
    scope.check_current(&state).unwrap();
    let proof = minted.auth_tag.unwrap();
    assert_eq!(
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&proof, &minted.keys.public_key()).unwrap(),
        api.state.keys.public_key()
    );
    let repair =
        crate::commands::legacy_managed_agent_auth_tag(&scope.signer, &minted.keys.public_key())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&repair, &minted.keys.public_key()).unwrap(),
        api.state.keys.public_key()
    );
    assert!(crate::commands::legacy_managed_agent_auth_tag(
        &scope.signer,
        &scope.signer.public_key()
    )
    .await
    .unwrap()
    .is_none());
    assert!(state.local_identity_keys().is_err());
    assert_eq!(
        serde_json::to_value(state.native_identity_status().unwrap()).unwrap()["workspaceActive"],
        false
    );
    let requests = api.state.requests.lock().unwrap();
    let proofs: Vec<_> = requests.iter().filter(|r| r.0 == OA).collect();
    assert_eq!(proofs.len(), 2);
    for request in proofs {
        assert_eq!(request.1.as_deref(), Some("credential-A"));
        assert_eq!(
            request.2,
            serde_json::json!({"agent_pubkey": minted.pubkey, "conditions":""})
        );
    }
}

#[tokio::test]
async fn remote_archive_pipeline_authorizes_then_signs_and_commits_atomic_pair() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let agent = Keys::generate().public_key().to_hex();
    let owner = signer.public_key().to_hex();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("retention.db");
    seed(&db, &owner, &agent);
    let plan = prepare_agent_tombstone(&db, &signer, &agent).unwrap();
    api.hold(OA);
    let work = plan.sign(&signer);
    tokio::pin!(work);
    tokio::select! { _ = &mut work => panic!("proof did not suspend"), _ = api.arrived() => {} }
    // A writer can acquire SQLite while the actual proof pipeline awaits HTTP.
    let conn = open_retention_db(&db).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").unwrap();
    drop(conn);
    unchanged(&db, &owner, &agent);
    api.release();
    let witness = work.await;
    let deleted = AtomicBool::new(false);
    commit_agent_deletions(vec![Ok(witness)], || {
        deleted.store(true, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    assert!(deleted.load(Ordering::SeqCst));
    let conn = open_retention_db(&db).unwrap();
    assert!(get_retained_event(&conn, 30177, &owner, &agent)
        .unwrap()
        .is_none());
    let rows = get_pending_sync(&conn).unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let event = nostr::Event::from_json(row.raw_event).unwrap();
        event.verify().unwrap();
        assert_eq!(event.pubkey, signer.public_key());
        if event.kind.as_u16() == 9035 {
            assert_eq!(event.content, r#"{"persona_id":"persona-proof"}"#);
            let tag = event
                .tags
                .iter()
                .find(|t| t.as_slice()[0] == "auth")
                .unwrap();
            let proof = serde_json::to_string(tag.as_slice()).unwrap();
            assert_eq!(
                buzz_sdk_pkg::nip_oa::verify_auth_tag(
                    &proof,
                    &nostr::PublicKey::from_hex(&agent).unwrap()
                )
                .unwrap(),
                signer.public_key()
            );
        }
    }
    let paths: Vec<_> = api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .skip(3)
        .map(|r| r.0.clone())
        .collect();
    assert_eq!(
        paths,
        [OA, "v1/buzz/identity/sign", "v1/buzz/identity/sign"]
    );
}

#[tokio::test]
async fn invalid_owner_agent_conditions_signature_never_mint_or_delete() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let agent = Keys::generate().public_key().to_hex();
    let owner = signer.public_key().to_hex();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("retention.db");
    seed(&db, &owner, &agent);
    for fault in ["owner", "agent", "conditions", "signature"] {
        *api.state.oa_fault.lock().unwrap() = Some(fault);
        assert!(prepare_agent(&signer).await.is_err(), "{fault}");
        let witness = prepare_agent_tombstone(&db, &signer, &agent)
            .unwrap()
            .sign(&signer)
            .await;
        let deleted = AtomicBool::new(false);
        assert!(commit_agent_deletions(vec![Ok(witness)], || {
            deleted.store(true, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
        assert!(!deleted.load(Ordering::SeqCst));
        unchanged(&db, &owner, &agent);
    }
    assert!(api
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|r| r.0 != "v1/buzz/identity/sign" && r.0 != "events"));
    // A malformed proof is not evidence that the otherwise valid session expired.
    signer.check_valid().unwrap();
}

#[tokio::test]
async fn logout_replacement_expiry_and_dropped_proof_leave_deletion_preparation_inert() {
    for action in [
        "logout",
        "replace",
        "expire",
        "drop",
        "rejected",
        "transient",
    ] {
        let api = MockApi::new().await;
        if action == "expire" {
            *api.state.expiry.lock().unwrap() =
                (Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339();
        }
        let state = build_app_state_for_mode(SignerMode::Remote);
        api.login(&state.native_auth, "A").await.unwrap();
        let signer = state.active_signer().unwrap();
        let agent = Keys::generate().public_key().to_hex();
        let owner = signer.public_key().to_hex();
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("retention.db");
        seed(&db, &owner, &agent);
        let plan = prepare_agent_tombstone(&db, &signer, &agent).unwrap();
        api.hold(OA);
        let mut work = Box::pin(plan.sign(&signer));
        tokio::select! { _ = &mut work => panic!("proof did not suspend"), _ = api.arrived() => {} }
        match action {
            "logout" => state.native_auth.clear().unwrap(),
            "replace" => {
                api.login(&state.native_auth, "B").await.unwrap();
            }
            "expire" => tokio::time::sleep(Duration::from_millis(750)).await,
            "drop" => {
                drop(work);
                api.release();
                unchanged(&db, &owner, &agent);
                continue;
            }
            "rejected" => {
                *api.state.fail.lock().unwrap() = Some((OA.into(), 401));
                api.release();
            }
            "transient" => {
                *api.state.fail.lock().unwrap() = Some((OA.into(), 503));
                api.release();
            }
            _ => unreachable!(),
        }
        let witness = tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .unwrap();
        api.release();
        let deleted = AtomicBool::new(false);
        assert!(
            commit_agent_deletions(vec![Ok(witness)], || {
                deleted.store(true, Ordering::SeqCst);
                Ok(())
            })
            .is_err(),
            "{action}"
        );
        assert!(!deleted.load(Ordering::SeqCst));
        unchanged(&db, &owner, &agent);
        assert!(api
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.0 != "v1/buzz/identity/sign" && r.0 != "events"));
        if action == "replace" {
            state.active_signer().unwrap().check_valid().unwrap();
        }
        if action == "transient" {
            signer.check_valid().unwrap();
        }
    }
}

#[tokio::test]
async fn successful_proof_cannot_commit_after_logout_or_retarget_relay() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let scope = OwnerAuthorizationScope::capture(&state).unwrap();
    let minted = prepare_agent(&scope.signer).await.unwrap();
    *state.relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:9".into());
    assert!(scope.check_current(&state).is_err());
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("retention.db");
    let owner = scope.signer.public_key().to_hex();
    seed(&db, &owner, &minted.pubkey);
    let witness = prepare_agent_tombstone(&db, &scope.signer, &minted.pubkey)
        .unwrap()
        .sign(&scope.signer)
        .await;
    state.native_auth.clear().unwrap();
    assert!(
        commit_agent_deletions(vec![Ok(witness)], || -> Result<(), String> {
            panic!("stale deletion committed")
        })
        .is_err()
    );
    unchanged(&db, &owner, &minted.pubkey);
}

#[path = "auth_owner_repair_tests.rs"]
mod repair;

#[tokio::test]
async fn logout_during_sqlite_busy_wait_is_rechecked_before_destructive_body() {
    let api = MockApi::new().await;
    let state = build_app_state_for_mode(SignerMode::Remote);
    api.login(&state.native_auth, "A").await.unwrap();
    let signer = state.active_signer().unwrap();
    let agent = Keys::generate().public_key().to_hex();
    let owner = signer.public_key().to_hex();
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("retention.db");
    seed(&db, &owner, &agent);
    let witness = prepare_agent_tombstone(&db, &signer, &agent)
        .unwrap()
        .sign(&signer)
        .await;
    let conn = open_retention_db(&db).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        commit_agent_deletions(vec![Ok(witness)], || -> Result<(), String> {
            panic!("canceled owner crossed SQLite wait into deletion")
        })
    });
    waiting.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !task.is_finished(),
        "the external SQLite writer must suspend commit"
    );
    state.native_auth.clear().unwrap();
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    unchanged(&db, &owner, &agent);
}
