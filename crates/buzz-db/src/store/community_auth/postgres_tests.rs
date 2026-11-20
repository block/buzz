use super::*;
use crate::CreateCommunityWithOwnerResult;
use nostr::Keys;

const ISSUER: &str = "https://identity.example/api/goose";

async fn fixture() -> (Db, UpgradeRequest, Keys) {
    let db = Db::from_pool(
        sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap(),
    );
    let keys = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let host = format!("auth-{}.example", Uuid::new_v4().simple());
    let write = db
        .create_community_with_owner_and_nip_fi_issuers(&host, &owner, &[ISSUER.to_owned()], &keys)
        .await
        .unwrap();
    let CreateCommunityWithOwnerResult::Created(record) = write.outcome else {
        panic!("create failed")
    };
    let request = UpgradeRequest {
        operation_id: Uuid::new_v4(),
        community_id: *record.id.as_uuid(),
        expected_host: host,
        actor_pubkey: owner.clone(),
        expected_owner_pubkey: owner,
        expected_issuer: ISSUER.to_owned(),
        operator_pubkey: keys.public_key().to_hex(),
    };
    (db, request, keys)
}

fn initial_enable(tuple: &UpgradeRequest) -> PolicyRequest {
    PolicyRequest {
        tuple: tuple.clone(),
        requires_nip_fi: true,
        expected_policy_generation: 0,
        expected_requires_nip_fi: false,
    }
}

async fn accepted(db: &Db, request: &UpgradeRequest) -> UpgradeRecord {
    let PolicyOutcome::Accepted(record) = db
        .set_community_nip_fi_policy(&initial_enable(request), Ok(()))
        .await
        .unwrap()
    else {
        panic!("acceptance refused")
    };
    record.acceptance
}

async fn policy(db: &Db, request: &UpgradeRequest) -> CommunityAuthPolicy {
    db.community_auth_policy(CommunityId::from_uuid(request.community_id))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn response_loss_recovery_and_payload_binding() {
    let (db, request, _) = fixture().await;
    assert!(!policy(&db, &request).await.requires_nip_fi);
    let first = accepted(&db, &request).await;
    assert_eq!(first.state, "pending");
    assert_eq!(first.policy_generation, 1);
    // Discard the POST result and recover by original operator/UUID.
    assert_eq!(
        db.community_auth_policy_status(request.operation_id, &request.operator_pubkey)
            .await
            .unwrap()
            .map(|record| record.acceptance),
        Some(first.clone())
    );
    assert_eq!(accepted(&db, &request).await, first);
    assert!(db
        .community_auth_policy_status(request.operation_id, "another operator")
        .await
        .unwrap()
        .map(|record| record.acceptance)
        .is_none());
    for field in 0..6 {
        let mut changed = request.clone();
        match field {
            0 => changed.community_id = Uuid::new_v4(),
            1 => changed.expected_host = "another.example".to_owned(),
            2 => changed.actor_pubkey = "b".repeat(64),
            3 => changed.expected_owner_pubkey = "c".repeat(64),
            4 => changed.expected_issuer = "https://another.example".to_owned(),
            _ => changed.operator_pubkey = "d".repeat(64),
        }
        assert_eq!(
            db.set_community_nip_fi_policy(&initial_enable(&changed), Ok(()))
                .await
                .unwrap(),
            PolicyOutcome::Refused("idempotency_conflict")
        );
    }
    assert_eq!(policy(&db, &request).await.policy_generation, 1);
    let community = CommunityId::from_uuid(request.community_id);
    for allow_last in [true, false] {
        assert_eq!(
            db.unbind_nip_fi_issuer(community, ISSUER, allow_last)
                .await
                .unwrap(),
            crate::NipFiUnbind::RequiredIssuer
        );
    }
    assert_eq!(accepted(&db, &request).await, first);
    // Rotation can remove the accepted issuer only after a replacement is bound.
    db.bind_nip_fi_issuers(community, &["https://replacement.example".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        db.unbind_nip_fi_issuer(community, ISSUER, true)
            .await
            .unwrap(),
        crate::NipFiUnbind::Removed
    );
    assert_eq!(accepted(&db, &request).await.state, "unavailable");
    assert_eq!(
        db.unbind_nip_fi_issuer(community, "https://replacement.example", true)
            .await
            .unwrap(),
        crate::NipFiUnbind::RequiredIssuer
    );
    assert!(policy(&db, &request).await.requires_nip_fi);
    // Out-of-band trust loss requires an explicit repair, never a bulk rebind.
    sqlx::query("DELETE FROM community_nip_fi_issuers WHERE community_id = $1")
        .bind(request.community_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        db.bind_nip_fi_issuers_to_active(&[community], &[ISSUER.to_owned()])
            .await
            .unwrap(),
        0
    );
    assert_eq!(accepted(&db, &request).await.state, "unavailable");
    db.bind_nip_fi_issuers(community, &[ISSUER.to_owned()])
        .await
        .unwrap();
    assert_eq!(accepted(&db, &request).await, first);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_ids_and_duplicate_requests_converge() {
    let (db, request, _) = fixture().await;
    let first = initial_enable(&request);
    let second = intent(&request, true, 0, false);
    let (a, b) = tokio::join!(
        db.set_community_nip_fi_policy(&first, Ok(())),
        db.set_community_nip_fi_policy(&second, Ok(()))
    );
    let (winner, loser) = match (a.unwrap(), b.unwrap()) {
        (PolicyOutcome::Accepted(record), other) => (&first, (record, other)),
        (other, PolicyOutcome::Accepted(record)) => (&second, (record, other)),
        outcomes => panic!("no CAS winner: {outcomes:?}"),
    };
    assert_eq!(loser.1, PolicyOutcome::Refused("policy_conflict"));
    assert_eq!(apply(&db, winner).await, *loser.0);
    let (replay_a, replay_b) = tokio::join!(apply(&db, winner), apply(&db, winner));
    assert_eq!(replay_a, replay_b);
    assert_eq!(policy(&db, &request).await.policy_generation, 1);
    let (_, mut other, _) = fixture().await;
    other.operation_id = winner.tuple.operation_id;
    assert_eq!(
        db.set_community_nip_fi_policy(&initial_enable(&other), Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("idempotency_conflict")
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_preconditions_leave_free_policy_and_no_receipt() {
    let (db, request, _) = fixture().await;
    for (mut changed, code) in [
        (request.clone(), "host_conflict"),
        (request.clone(), "owner_conflict"),
        (request.clone(), "issuer_conflict"),
        (request.clone(), "community_not_found"),
    ] {
        match code {
            "host_conflict" => changed.expected_host = "wrong.example".to_owned(),
            "owner_conflict" => changed.actor_pubkey = "b".repeat(64),
            "issuer_conflict" => changed.expected_issuer = "https://wrong.example".to_owned(),
            _ => changed.community_id = Uuid::new_v4(),
        }
        assert_eq!(
            db.set_community_nip_fi_policy(&initial_enable(&changed), Ok(()))
                .await
                .unwrap(),
            PolicyOutcome::Refused(code)
        );
    }
    assert_eq!(
        db.set_community_nip_fi_policy(&initial_enable(&request), Err("authorization_unavailable"))
            .await
            .unwrap(),
        PolicyOutcome::Refused("authorization_unavailable")
    );
    db.unbind_nip_fi_issuer(CommunityId::from_uuid(request.community_id), ISSUER, true)
        .await
        .unwrap();
    assert_eq!(
        db.set_community_nip_fi_policy(&initial_enable(&request), Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("issuer_conflict")
    );
    assert_eq!(
        policy(&db, &request).await,
        CommunityAuthPolicy {
            community_id: request.community_id,
            requires_nip_fi: false,
            policy_generation: 0
        }
    );
    assert!(db
        .community_auth_policy_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
        .map(|record| record.acceptance)
        .is_none());
}

// Observe the actual PostgreSQL blocker and statement, rather than treating
// elapsed time or either possible race outcome as evidence of serialization.
async fn blocked_upgrade(
    db: &Db,
    request: &UpgradeRequest,
    locker_pid: i32,
    statement: &str,
) -> tokio::task::JoinHandle<Result<PolicyOutcome>> {
    use std::str::FromStr;
    let application = format!("upgrade-lock-{}", request.operation_id.simple());
    let options = sqlx::postgres::PgConnectOptions::from_str(&crate::test_support::database_url())
        .unwrap()
        .application_name(&application);
    let upgrade_db = Db::from_pool(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap(),
    );
    let request = request.clone();
    let task = tokio::spawn(async move {
        upgrade_db
            .set_community_nip_fi_policy(&initial_enable(&request), Ok(()))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            assert!(
                !task.is_finished(),
                "enable escaped before taking its precondition lock"
            );
            let row: Option<(String, Vec<i32>)> = sqlx::query_as(
                "SELECT query, pg_blocking_pids(pid) FROM pg_stat_activity
                 WHERE application_name = $1 AND wait_event_type = 'Lock'",
            )
            .bind(&application)
            .fetch_optional(&db.pool)
            .await
            .unwrap();
            if let Some((query, blockers)) = row {
                if blockers.contains(&locker_pid) {
                    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
                    assert!(
                        query.starts_with(statement),
                        "enable reached a later statement without the precondition lock: {query}"
                    );
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("observe the enable statement blocked by the held transaction");
    task
}

async fn assert_refused_without_mutation(
    db: &Db,
    request: &UpgradeRequest,
    task: tokio::task::JoinHandle<Result<PolicyOutcome>>,
    code: &'static str,
) {
    assert_eq!(task.await.unwrap().unwrap(), PolicyOutcome::Refused(code));
    assert!(db
        .community_auth_policy_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
        .map(|record| record.acceptance)
        .is_none());
    let policy: (bool, i64) =
        sqlx::query_as("SELECT requires_nip_fi, policy_generation FROM communities WHERE id = $1")
            .bind(request.community_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(policy, (false, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_lock_blocks_uncommitted_owner_change_then_refuses_enable() {
    let (db, request, _) = fixture().await;
    let mut locker = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *locker)
        .await
        .unwrap();
    sqlx::query("UPDATE relay_members SET role = 'member' WHERE community_id = $1 AND pubkey = $2")
        .bind(request.community_id)
        .bind(&request.expected_owner_pubkey)
        .execute(&mut *locker)
        .await
        .unwrap();
    let task = blocked_upgrade(&db, &request, pid, "SELECT pubkey FROM relay_members").await;
    locker.commit().await.unwrap();
    assert_refused_without_mutation(&db, &request, task, "owner_conflict").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_lock_blocks_uncommitted_archive_then_refuses_enable() {
    let (db, request, _) = fixture().await;
    let mut locker = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *locker)
        .await
        .unwrap();
    sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
        .bind(request.community_id)
        .execute(&mut *locker)
        .await
        .unwrap();
    let task = blocked_upgrade(&db, &request, pid, "SELECT host, archived_at").await;
    locker.commit().await.unwrap();
    assert_refused_without_mutation(&db, &request, task, "lifecycle_conflict").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn issuer_lock_blocks_uncommitted_unbind_then_refuses_enable() {
    let (db, request, _) = fixture().await;
    let mut locker = db.pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *locker)
        .await
        .unwrap();
    sqlx::query("DELETE FROM community_nip_fi_issuers WHERE community_id = $1 AND issuer = $2")
        .bind(request.community_id)
        .bind(ISSUER)
        .execute(&mut *locker)
        .await
        .unwrap();
    let task = blocked_upgrade(
        &db,
        &request,
        pid,
        "SELECT issuer FROM community_nip_fi_issuers",
    )
    .await;
    locker.commit().await.unwrap();
    assert_refused_without_mutation(&db, &request, task, "issuer_conflict").await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn accepted_receipt_survives_transfer_without_granting_fresh_owner_authority() {
    let (db, request, keys) = fixture().await;
    let first = accepted(&db, &request).await;
    let new_owner = Keys::generate().public_key().to_hex();
    let _ = db
        .transfer_ownership(
            CommunityId::from_uuid(request.community_id),
            &new_owner,
            &request.expected_owner_pubkey,
            &keys,
        )
        .await
        .unwrap();
    assert_eq!(accepted(&db, &request).await, first);
    let mut fresh = request.clone();
    fresh.operation_id = Uuid::new_v4();
    assert_eq!(
        db.set_community_nip_fi_policy(
            &PolicyRequest {
                tuple: fresh.clone(),
                requires_nip_fi: true,
                expected_policy_generation: 1,
                expected_requires_nip_fi: true
            },
            Ok(())
        )
        .await
        .unwrap(),
        PolicyOutcome::Refused("owner_conflict")
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn authoritative_lookup_failure_is_error_not_fabricated_acceptance() {
    let db = Db::from_pool(
        sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(100))
            .connect_lazy("postgres://127.0.0.1:1/unavailable")
            .unwrap(),
    );
    let (_, request, _) = fixture().await;
    assert!(db
        .set_community_nip_fi_policy(&initial_enable(&request), Ok(()))
        .await
        .is_err());
    assert!(db
        .community_auth_policy_status(request.operation_id, &request.operator_pubkey)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deletion_intent_refuses_enable_before_quiescing() {
    let (db, request, _) = fixture().await;
    db.deletion_store()
        .submit(
            Some(&request.expected_host),
            "operator",
            None,
            Some(CommunityId::from_uuid(request.community_id)),
        )
        .await
        .unwrap();
    assert_eq!(
        db.set_community_nip_fi_policy(&initial_enable(&request), Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("lifecycle_conflict")
    );
    assert!(!policy(&db, &request).await.requires_nip_fi);
    assert!(db
        .community_auth_policy_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
        .map(|record| record.acceptance)
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_expansion_preserves_existing_optional_rows_with_and_without_trust() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    crate::migration::run_migrations_through(&pool, 81)
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let free = db
        .ensure_configured_community("free-before.example")
        .await
        .unwrap()
        .id;
    let bound = db
        .ensure_configured_community_with_nip_fi_issuers(
            "bound-before.example",
            &[ISSUER.to_owned()],
        )
        .await
        .unwrap()
        .id;
    crate::migration::run_migrations(&pool).await.unwrap();
    for community in [free, bound] {
        assert_eq!(
            db.community_auth_policy(community).await.unwrap(),
            Some(CommunityAuthPolicy {
                community_id: *community.as_uuid(),
                requires_nip_fi: false,
                policy_generation: 0,
            })
        );
    }
    assert_eq!(
        db.lookup_nip_fi_binding_by_host("bound-before.example")
            .await
            .unwrap()
            .unwrap()
            .issuers,
        vec![ISSUER.to_owned()]
    );
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM community_auth_upgrades")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(receipts, 0);
    let global: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _operator_global_tables WHERE table_name = 'community_auth_upgrades')")
        .fetch_one(&pool).await.unwrap();
    assert!(global);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn supported_writer_maintains_generation_while_direct_sql_is_not_constrained() {
    let (db, mut request, _) = fixture().await;
    assert_eq!(policy(&db, &request).await.policy_generation, 0);
    sqlx::query("UPDATE communities SET policy_generation = -1 WHERE id = $1")
        .bind(request.community_id)
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(policy(&db, &request).await.policy_generation, -1);
    sqlx::query("UPDATE communities SET policy_generation = DEFAULT WHERE id = $1")
        .bind(request.community_id)
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(policy(&db, &request).await.policy_generation, 0);
    let first = accepted(&db, &request).await;
    assert_eq!(first.policy_generation, 1);
    request.operation_id = Uuid::new_v4();
    assert_eq!(
        apply(
            &db,
            &PolicyRequest {
                tuple: request.clone(),
                requires_nip_fi: true,
                expected_policy_generation: 1,
                expected_requires_nip_fi: true
            }
        )
        .await
        .acceptance
        .policy_generation,
        2
    );
    let invalid_receipt = sqlx::query(
        "UPDATE community_auth_upgrades SET policy_generation = 0 WHERE operation_id = $1",
    )
    .bind(request.operation_id)
    .execute(db.pool())
    .await
    .unwrap_err();
    assert_eq!(
        invalid_receipt
            .as_database_error()
            .unwrap()
            .code()
            .as_deref(),
        Some("23514")
    );
    assert_eq!(policy(&db, &request).await.policy_generation, 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn host_policy_tracks_primary_requirement_and_serving_lifecycle() {
    let (db, request, _) = fixture().await;
    let host = &request.expected_host;
    assert_eq!(
        db.community_auth_policy_for_host(host).await.unwrap(),
        Some(CommunityAuthPolicy {
            community_id: request.community_id,
            requires_nip_fi: false,
            policy_generation: 0
        })
    );
    let upgraded = accepted(&db, &request).await;
    assert_eq!(
        db.community_auth_policy_for_host(host).await.unwrap(),
        Some(CommunityAuthPolicy {
            community_id: request.community_id,
            requires_nip_fi: true,
            policy_generation: upgraded.policy_generation
        })
    );
    assert!(db
        .community_auth_policy_for_host("unknown.example")
        .await
        .unwrap()
        .is_none());
    sqlx::query("INSERT INTO community_deletion_requests (community_id, community_host, requested_by) VALUES ($1, $2, 'policy-test')")
        .bind(request.community_id).bind(host).execute(db.pool()).await.unwrap();
    for stage in ["submitted", "inventoried", "approved"] {
        sqlx::query("UPDATE community_deletion_requests SET stage = $2 WHERE community_id = $1")
            .bind(request.community_id)
            .bind(stage)
            .execute(db.pool())
            .await
            .unwrap();
        assert!(db.lookup_community_by_host(host).await.unwrap().is_some());
        assert!(
            db.community_auth_policy_for_host(host)
                .await
                .unwrap()
                .unwrap()
                .requires_nip_fi,
            "{stage}"
        );
    }
    sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
        .bind(request.community_id)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(db
        .community_auth_policy_for_host(host)
        .await
        .unwrap()
        .is_none());
}

fn intent(tuple: &UpgradeRequest, desired: bool, generation: i64, required: bool) -> PolicyRequest {
    let mut tuple = tuple.clone();
    tuple.operation_id = Uuid::new_v4();
    if !desired {
        tuple.expected_issuer.clear();
    }
    PolicyRequest {
        tuple,
        requires_nip_fi: desired,
        expected_policy_generation: generation,
        expected_requires_nip_fi: required,
    }
}

async fn apply(db: &Db, request: &PolicyRequest) -> PolicyRecord {
    let PolicyOutcome::Accepted(record) = db
        .set_community_nip_fi_policy(request, Ok(()))
        .await
        .unwrap()
    else {
        panic!("policy refused")
    };
    *record
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn policy_enable_disable_reenable_history_and_identity() {
    let (db, tuple, _) = fixture().await;
    let enable = intent(&tuple, true, 0, false);
    let first = apply(&db, &enable).await;
    assert_eq!(first.acceptance.policy_generation, 1);
    assert_eq!(first.acceptance.state, "pending");
    let disable = intent(&tuple, false, 1, true);
    let second = apply(&db, &disable).await;
    assert_eq!(second.acceptance.policy_generation, 2);
    assert_eq!(second.acceptance.state, "applied");
    let old = db
        .community_auth_policy_status(enable.tuple.operation_id, &tuple.operator_pubkey)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.acceptance.state, "superseded");
    assert_eq!(old.current_policy_generation, Some(2));
    assert_eq!(old.current_requires_nip_fi, Some(false));
    // A committed retry is history even with the mutation gate disabled.
    assert_eq!(
        db.set_community_nip_fi_policy(&enable, Err("upgrades_disabled"))
            .await
            .unwrap(),
        PolicyOutcome::Accepted(Box::new(old))
    );
    let reenable = intent(&tuple, true, 2, false);
    let third = apply(&db, &reenable).await;
    assert_eq!(third.acceptance.policy_generation, 3);
    assert_eq!(third.current_requires_nip_fi, Some(true));
    for request in [&enable, &disable, &reenable] {
        let record = db
            .community_auth_policy_status(request.tuple.operation_id, &tuple.operator_pubkey)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.current_policy_generation, Some(3));
        assert_eq!(
            record.acceptance.state,
            if request == &reenable {
                "pending"
            } else {
                "superseded"
            }
        );
    }
    for change in 0..5 {
        let mut changed = enable.clone();
        match change {
            0 => changed.requires_nip_fi = false,
            1 => changed.expected_policy_generation = 2,
            2 => changed.expected_requires_nip_fi = true,
            3 => changed.tuple.operator_pubkey = "f".repeat(64),
            _ => changed.tuple.expected_issuer = "https://different.example".into(),
        }
        assert_eq!(
            db.set_community_nip_fi_policy(&changed, Ok(()))
                .await
                .unwrap(),
            PolicyOutcome::Refused("idempotency_conflict")
        );
    }
    assert_eq!(policy(&db, &tuple).await.policy_generation, 3);
    // Reasserting required policy fences an earlier uncommitted disable too.
    let stale_disable = intent(&tuple, false, 3, true);
    let reassert = intent(&tuple, true, 3, true);
    assert_eq!(apply(&db, &reassert).await.acceptance.policy_generation, 4);
    assert_eq!(
        db.set_community_nip_fi_policy(&stale_disable, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("policy_conflict")
    );
    assert!(policy(&db, &tuple).await.requires_nip_fi);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn policy_same_state_barrier_and_retained_legacy_receipt_identity() {
    let (db, tuple, _) = fixture().await;
    let stale = intent(&tuple, true, 0, false);
    let barrier = intent(&tuple, false, 0, false);
    let record = apply(&db, &barrier).await;
    assert_eq!(record.acceptance.policy_generation, 1);
    assert!(!record.acceptance.requires_nip_fi);
    assert_eq!(
        db.set_community_nip_fi_policy(&stale, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("policy_conflict")
    );
    let wrong_state = intent(&tuple, true, 1, true);
    assert_eq!(
        db.set_community_nip_fi_policy(&wrong_state, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("policy_conflict")
    );
    assert_eq!(apply(&db, &barrier).await, record);
    let enable = intent(&tuple, true, 1, false);
    apply(&db, &enable).await;
    // Emulate an acceptance written before migration 0085, without restoring
    // an unfenced writer just to create test data. Old INSERTs remain valid.
    let legacy_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO community_auth_upgrades
        (operation_id,target_community_id,expected_host,actor_pubkey,expected_owner_pubkey,
         expected_issuer,operator_pubkey,policy_generation) VALUES ($1,$2,$3,$4,$4,$5,$6,2)",
    )
    .bind(legacy_id)
    .bind(tuple.community_id)
    .bind(&tuple.expected_host)
    .bind(&tuple.actor_pubkey)
    .bind(&tuple.expected_issuer)
    .bind(&tuple.operator_pubkey)
    .execute(db.pool())
    .await
    .unwrap();
    let retained = db
        .community_auth_policy_status(legacy_id, &tuple.operator_pubkey)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.expected_policy_generation, None);
    assert_eq!(retained.expected_requires_nip_fi, None);
    assert_eq!(retained.acceptance.state, "pending");
    let mut reuse = intent(&tuple, true, 2, true);
    reuse.tuple.operation_id = legacy_id;
    assert_eq!(
        db.set_community_nip_fi_policy(&reuse, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("idempotency_conflict")
    );
    apply(&db, &intent(&tuple, false, 2, true)).await;
    let retained = db
        .community_auth_policy_status(legacy_id, &tuple.operator_pubkey)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.acceptance.state, "superseded");
    assert_eq!(retained.current_policy_generation, Some(3));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn policy_disable_guards_and_issuer_removal_then_reenable() {
    let (db, tuple, _) = fixture().await;
    accepted(&db, &tuple).await;
    let disable = intent(&tuple, false, 1, true);
    for kind in 0..4 {
        let mut changed = disable.clone();
        let code = match kind {
            0 => {
                changed.tuple.actor_pubkey = "a".repeat(64);
                "owner_conflict"
            }
            1 => {
                changed.tuple.expected_host = "wrong.example".into();
                "host_conflict"
            }
            2 => {
                changed.tuple.expected_owner_pubkey = "b".repeat(64);
                "owner_conflict"
            }
            _ => {
                changed.tuple.community_id = Uuid::new_v4();
                "community_not_found"
            }
        };
        assert_eq!(
            db.set_community_nip_fi_policy(&changed, Ok(()))
                .await
                .unwrap(),
            PolicyOutcome::Refused(code)
        );
    }
    assert_eq!(
        db.set_community_nip_fi_policy(&disable, Err("upgrades_disabled"))
            .await
            .unwrap(),
        PolicyOutcome::Refused("upgrades_disabled")
    );
    apply(&db, &disable).await;
    let community = CommunityId::from_uuid(tuple.community_id);
    assert_eq!(
        db.unbind_nip_fi_issuer(community, ISSUER, true)
            .await
            .unwrap(),
        crate::NipFiUnbind::Removed
    );
    let enable = intent(&tuple, true, 2, false);
    assert_eq!(
        db.set_community_nip_fi_policy(&enable, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("issuer_conflict")
    );
    // Disable works without a trusted/bound issuer and still advances its barrier.
    let next = apply(&db, &intent(&tuple, false, 2, false)).await;
    assert_eq!(next.acceptance.policy_generation, 3);
    sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
        .bind(tuple.community_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        db.set_community_nip_fi_policy(&intent(&tuple, false, 3, false), Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("lifecycle_conflict")
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn policy_disable_serializes_behind_enable_drain() {
    use std::str::FromStr;
    let (db, tuple, _) = fixture().await;
    let application = format!("policy-drain-{}", Uuid::new_v4().simple());
    let options = sqlx::postgres::PgConnectOptions::from_str(&crate::test_support::database_url())
        .unwrap()
        .application_name(&application);
    let writer = Db::from_pool(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap(),
    );
    let enable = intent(&tuple, true, 0, false);
    let task = tokio::spawn(async move { apply(&writer, &enable).await });
    // The lease-lock statement followed by idle-in-transaction identifies the
    // production drain while the writer still owns its community row lock.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            assert!(!task.is_finished(), "missed the production lease drain");
            let draining: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name = $1 AND state = 'idle in transaction' AND query LIKE 'SELECT pg_advisory_xact_lock($1,%')")
                .bind(&application).fetch_one(&db.pool).await.unwrap();
            if draining { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let disable = intent(&tuple, false, 0, false);
    assert_eq!(
        db.set_community_nip_fi_policy(&disable, Ok(()))
            .await
            .unwrap(),
        PolicyOutcome::Refused("policy_conflict")
    );
    assert_eq!(task.await.unwrap().acceptance.policy_generation, 1);
    assert!(policy(&db, &tuple).await.requires_nip_fi);
    assert_eq!(
        apply(&db, &intent(&tuple, false, 1, true))
            .await
            .acceptance
            .policy_generation,
        2
    );
}
