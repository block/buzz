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

async fn accepted(db: &Db, request: &UpgradeRequest) -> UpgradeRecord {
    let UpgradeOutcome::Accepted(record) =
        db.enable_community_nip_fi(request, Ok(())).await.unwrap()
    else {
        panic!("acceptance refused")
    };
    record
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
        db.community_auth_upgrade_status(request.operation_id, &request.operator_pubkey)
            .await
            .unwrap(),
        Some(first.clone())
    );
    assert_eq!(accepted(&db, &request).await, first);
    assert!(db
        .community_auth_upgrade_status(request.operation_id, "another operator")
        .await
        .unwrap()
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
            db.enable_community_nip_fi(&changed, Ok(())).await.unwrap(),
            UpgradeOutcome::Refused("idempotency_conflict")
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
    let mut second = request.clone();
    second.operation_id = Uuid::new_v4();
    let (a, b, c) = tokio::join!(
        accepted(&db, &request),
        accepted(&db, &request),
        accepted(&db, &second)
    );
    assert_eq!(a, b);
    assert_eq!(a.policy_generation, c.policy_generation);
    assert_eq!(policy(&db, &request).await.policy_generation, 1);
    let (_, mut other, _) = fixture().await;
    other.operation_id = request.operation_id;
    assert_eq!(
        db.enable_community_nip_fi(&other, Ok(())).await.unwrap(),
        UpgradeOutcome::Refused("idempotency_conflict")
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
            db.enable_community_nip_fi(&changed, Ok(())).await.unwrap(),
            UpgradeOutcome::Refused(code)
        );
    }
    assert_eq!(
        db.enable_community_nip_fi(&request, Err("authorization_unavailable"))
            .await
            .unwrap(),
        UpgradeOutcome::Refused("authorization_unavailable")
    );
    db.unbind_nip_fi_issuer(CommunityId::from_uuid(request.community_id), ISSUER, true)
        .await
        .unwrap();
    assert_eq!(
        db.enable_community_nip_fi(&request, Ok(())).await.unwrap(),
        UpgradeOutcome::Refused("issuer_conflict")
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
        .community_auth_upgrade_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
        .is_none());
}

// Observe the actual PostgreSQL blocker and statement, rather than treating
// elapsed time or either possible race outcome as evidence of serialization.
async fn blocked_upgrade(
    db: &Db,
    request: &UpgradeRequest,
    locker_pid: i32,
    statement: &str,
) -> tokio::task::JoinHandle<Result<UpgradeOutcome>> {
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
    let task =
        tokio::spawn(async move { upgrade_db.enable_community_nip_fi(&request, Ok(())).await });
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
    task: tokio::task::JoinHandle<Result<UpgradeOutcome>>,
    code: &'static str,
) {
    assert_eq!(task.await.unwrap().unwrap(), UpgradeOutcome::Refused(code));
    assert!(db
        .community_auth_upgrade_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
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
        db.enable_community_nip_fi(&fresh, Ok(())).await.unwrap(),
        UpgradeOutcome::Refused("owner_conflict")
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
    assert!(db.enable_community_nip_fi(&request, Ok(())).await.is_err());
    assert!(db
        .community_auth_upgrade_status(request.operation_id, &request.operator_pubkey)
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
        db.enable_community_nip_fi(&request, Ok(())).await.unwrap(),
        UpgradeOutcome::Refused("lifecycle_conflict")
    );
    assert!(!policy(&db, &request).await.requires_nip_fi);
    assert!(db
        .community_auth_upgrade_status(request.operation_id, &request.operator_pubkey)
        .await
        .unwrap()
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
    assert_eq!(accepted(&db, &request).await.policy_generation, 1);
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
    assert_eq!(policy(&db, &request).await.policy_generation, 1);
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
