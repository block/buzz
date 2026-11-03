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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn transfer_and_enable_have_a_serial_order_and_receipt_survives_transfer() {
    let (db, request, keys) = fixture().await;
    let new_owner = Keys::generate().public_key().to_hex();
    let community = CommunityId::from_uuid(request.community_id);
    let (upgrade, transfer) = tokio::join!(
        db.enable_community_nip_fi(&request, Ok(())),
        db.transfer_ownership(community, &new_owner, &request.expected_owner_pubkey, &keys)
    );
    let _ = transfer.unwrap();
    match upgrade.unwrap() {
        UpgradeOutcome::Accepted(record) => {
            assert_eq!(accepted(&db, &request).await, record);
            let mut fresh = request.clone();
            fresh.operation_id = Uuid::new_v4();
            assert_eq!(
                db.enable_community_nip_fi(&fresh, Ok(())).await.unwrap(),
                UpgradeOutcome::Refused("owner_conflict")
            );
        }
        refused => assert_eq!(refused, UpgradeOutcome::Refused("owner_conflict")),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_race_and_lifecycle_readback_never_claim_active() {
    let (db, request, _) = fixture().await;
    let (upgrade, archive) = tokio::join!(
        db.enable_community_nip_fi(&request, Ok(())),
        db.archive_community_owned_by(
            &request.expected_host,
            &request.expected_owner_pubkey,
            "reserved.example"
        )
    );
    assert!(archive.unwrap().is_some());
    match upgrade.unwrap() {
        UpgradeOutcome::Accepted(_) => {
            let recovered = accepted(&db, &request).await;
            assert_eq!(recovered.state, "unavailable");
            assert!(recovered.requires_nip_fi);
        }
        refused => assert_eq!(refused, UpgradeOutcome::Refused("lifecycle_conflict")),
    }
    let mut fresh = request.clone();
    fresh.operation_id = Uuid::new_v4();
    assert_eq!(
        db.enable_community_nip_fi(&fresh, Ok(())).await.unwrap(),
        UpgradeOutcome::Refused("lifecycle_conflict")
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn issuer_unbind_race_serializes_with_acceptance_and_never_relaxes_policy() {
    let (db, request, _) = fixture().await;
    let (upgrade, unbind) = tokio::join!(
        db.enable_community_nip_fi(&request, Ok(())),
        db.unbind_nip_fi_issuer(CommunityId::from_uuid(request.community_id), ISSUER, true)
    );
    unbind.unwrap();
    match upgrade.unwrap() {
        UpgradeOutcome::Accepted(_) => {
            assert_eq!(accepted(&db, &request).await.state, "unavailable");
            assert!(policy(&db, &request).await.requires_nip_fi);
        }
        refused => {
            assert_eq!(refused, UpgradeOutcome::Refused("issuer_conflict"));
            assert!(!policy(&db, &request).await.requires_nip_fi);
        }
    }
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
    crate::migration::run_migrations_through(&pool, 80)
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
