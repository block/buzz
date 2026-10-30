use super::*;
use chrono::Duration;

async fn fixture() -> (Db, CommunityId, Keys, String, ProfilePolicy) {
    let db = Db::new(&crate::DbConfig {
        database_url: crate::test_support::database_url(),
        ..crate::DbConfig::default()
    })
    .await
    .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("profile-{}.test", community.as_uuid()))
        .execute(&db.pool)
        .await
        .unwrap();
    let key = Keys::generate().public_key().to_hex();
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(community.as_uuid())
        .bind(&key)
        .execute(tx.conn())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let policy = ProfilePolicy {
        issuer: "https://issuer.test".into(),
        label: Some("Example".into()),
        refresh_seconds: 3600,
        community_hosts: None,
    };
    db.configure_profile_policies(std::slice::from_ref(&policy), 0)
        .await
        .unwrap();
    (db, community, Keys::generate(), key, policy)
}

#[allow(clippy::too_many_arguments)]
async fn admit(
    db: &Db,
    community: CommunityId,
    keys: &Keys,
    key: &str,
    policy: &ProfilePolicy,
    epoch: ProfileEpoch,
    iat: DateTime<Utc>,
    profile: Option<(&str, &str, Option<&str>)>,
) {
    db.apply_profile_admission(
        ProfileAdmission {
            community,
            issuer: &policy.issuer,
            pubkey: key,
            issued_at: iat,
            admitted_at: Utc::now(),
            profile,
            epoch,
            database_binding: false,
        },
        keys,
    )
    .await
    .unwrap();
}

async fn current(db: &Db, community: CommunityId, key: &str) -> Option<Event> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT event FROM nip_fi_profile_heads WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(key)
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    value.map(|v| Event::from_json(v.to_string()).unwrap())
}

async fn lifecycle() {
    let (db, community, keys, key, policy) = fixture().await;
    let epoch = db
        .profile_epoch(community, &key, &policy)
        .await
        .unwrap()
        .unwrap();
    let now = Utc::now();
    let first_iat = now - Duration::seconds(10);
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat,
        Some(("Alice", "alice", Some("Alice Example"))),
    )
    .await;
    let first = current(&db, community, &key).await.unwrap();
    assert_eq!(first.pubkey, keys.public_key());
    assert_eq!(first.tags.len(), 8);
    assert_eq!(tag(&first, "full_name"), Some("Alice Example"));
    assert_eq!(tag(&first, "display_name"), Some("Alice"));
    assert!(first.verify().is_ok());
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat + Duration::nanoseconds(2),
        Some(("Alice", "alice", Some("Alice Example"))),
    )
    .await;
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        first.id,
        "unchanged admission does not refresh early"
    );
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat + Duration::nanoseconds(3),
        Some(("Alice", "alice", Some("Alice Renamed"))),
    )
    .await;
    let renamed = current(&db, community, &key).await.unwrap();
    assert_ne!(
        renamed.id, first.id,
        "full-name changes publish immediately"
    );
    assert_eq!(tag(&renamed, "full_name"), Some("Alice Renamed"));
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat + Duration::nanoseconds(4),
        Some(("Alice", "alice", None)),
    )
    .await;
    let without_full_name = current(&db, community, &key).await.unwrap();
    assert_ne!(without_full_name.id, renamed.id);
    assert_eq!(tag(&without_full_name, "full_name"), None);
    assert_eq!(without_full_name.tags.len(), 7);
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat + Duration::nanoseconds(1),
        None,
    )
    .await;
    assert!(!db.pending_profile_work(None).await.unwrap().is_empty()); // initial broadcast still queued
    assert_eq!(
        tag(
            &db.process_profile_work(community, &key, &keys)
                .await
                .unwrap()
                .unwrap(),
            "active"
        ),
        Some("true"),
        "older fractional iat cannot retract"
    );
    admit(&db, community, &keys, &key, &policy, epoch, now, None).await;
    let retracted = db
        .process_profile_work(community, &key, &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tag(&retracted, "active"), Some("false"));
    assert_eq!(retracted.tags.len(), 3);
    assert!(retracted.created_at > first.created_at);
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        first_iat,
        Some(("Old", "old", None)),
    )
    .await;
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        retracted.id
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_lifecycle_retains_assertion_ordering() {
    lifecycle().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_profile_lifecycle_retains_assertion_ordering() {
    lifecycle().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn affected_scope_and_configuration_epochs_fence_restored_state() {
    let (db, community, keys, key, policy) = fixture().await;
    let epoch = db
        .profile_epoch(community, &key, &policy)
        .await
        .unwrap()
        .unwrap();
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE relay_members SET access_generation = gen_random_uuid() WHERE community_id = $1 AND pubkey = $2").bind(community.as_uuid()).bind("b".repeat(64)).execute(tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        Utc::now(),
        Some(("Alice", "alice", None)),
    )
    .await;
    assert!(
        current(&db, community, &key).await.is_some(),
        "unrelated key invalidation does not drop Alice"
    );
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE relay_members SET access_generation = gen_random_uuid() WHERE community_id = $1 AND pubkey = $2").bind(community.as_uuid()).bind(&key).execute(tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
    db.retract_profile_admission(
        community,
        &policy.issuer,
        &key,
        Utc::now().timestamp(),
        None,
    )
    .await
    .unwrap();
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        Utc::now(),
        Some(("Stale", "stale", None)),
    )
    .await;
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("false")
    );
    db.configure_profile_policies(&[], 1).await.unwrap();
    assert!(
        db.configure_profile_policies(std::slice::from_ref(&policy), 0)
            .await
            .is_err(),
        "stale pod cannot restore policy"
    );
    assert!(
        db.configure_profile_policies(std::slice::from_ref(&policy), 1)
            .await
            .is_err(),
        "same revision cannot change meaning"
    );
    db.configure_profile_policies(std::slice::from_ref(&policy), 2)
        .await
        .unwrap();
    admit(
        &db,
        community,
        &keys,
        &key,
        &policy,
        epoch,
        Utc::now(),
        Some(("Stale", "stale", None)),
    )
    .await;
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("false")
    );
}

async fn ordering_and_access_races() {
    let (db, community, keys, key, policy) = fixture().await;
    let epoch = db
        .profile_epoch(community, &key, &policy)
        .await
        .unwrap()
        .unwrap();
    let now = Utc::now() - Duration::seconds(30);
    let input = |admitted_at, epoch| ProfileAdmission {
        community,
        issuer: &policy.issuer,
        pubkey: &key,
        issued_at: now,
        admitted_at,
        profile: Some(("Alice", "alice", None)),
        epoch,
        database_binding: false,
    };
    db.apply_profile_admission(input(now, epoch), &keys)
        .await
        .unwrap()
        .unwrap();
    let first = current(&db, community, &key).await.unwrap();
    let trigger = now.timestamp();
    db.retract_profile_admission(community, &policy.issuer, &key, trigger, None)
        .await
        .unwrap();
    db.apply_profile_admission(input(now + Duration::seconds(1), epoch), &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        first.id,
        "unchanged values must not refresh early or erase pending cleanup"
    );
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("false")
    );
    // A still-valid pre-disconnect assertion can republish. Its public
    // verification time is now later than the original cleanup trigger.
    db.apply_profile_admission(input(now + Duration::seconds(2), epoch), &keys)
        .await
        .unwrap()
        .unwrap();
    let later_head = current(&db, community, &key).await.unwrap();
    // A delayed deny-only cancellation has no original command time. It may
    // cancel its observed event, never a later replacement even with a broad T.
    db.retract_profile_admission(
        community,
        &policy.issuer,
        &key,
        trigger + 99,
        Some(&first.id.to_hex()),
    )
    .await
    .unwrap();
    db.retract_profile_admission(community, &policy.issuer, &key, trigger, None)
        .await
        .unwrap();
    db.retract_profile_admission(community, "another-issuer", &key, trigger + 99, None)
        .await
        .unwrap();
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    assert_eq!(
        current(&db, community, &key).await.unwrap().id,
        later_head.id
    );
    assert!(
        db.apply_profile_admission(input(now, epoch), &keys)
            .await
            .unwrap()
            .is_none(),
        "public verified_at cannot move backwards"
    );
    let later = Utc::now();

    // Holding a projection lock must not hold access revocation open.
    let mut lock = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    lock_profile(&mut lock, &key).await.unwrap();
    let bytes = hex::decode(&key).unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        crate::moderation::ban_member(&db.pool, community, &bytes, &bytes, None, None),
    )
    .await
    .unwrap()
    .unwrap();
    crate::moderation::unban_member(&db.pool, community, &bytes, &bytes)
        .await
        .unwrap();
    lock.commit().await.unwrap();
    assert!(
        db.apply_profile_admission(input(later + Duration::seconds(2), epoch), &keys)
            .await
            .unwrap()
            .is_none(),
        "ban/unban fences work from the previous incarnation"
    );
    let restored = db
        .profile_epoch(community, &key, &policy)
        .await
        .unwrap()
        .unwrap();
    db.apply_profile_admission(
        ProfileAdmission {
            profile: Some(("Restored Alice", "alice", None)),
            ..input(later + Duration::seconds(3), restored)
        },
        &keys,
    )
    .await
    .unwrap()
    .unwrap();
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("true"),
        "late cleanup does not retract the independently eligible restored admission"
    );

    // Reinsertions get new membership incarnations even from old writers.
    let removed = crate::relay_members::remove_relay_member(&db.pool, community, &key, &keys)
        .await
        .unwrap();
    assert!(matches!(
        removed.outcome,
        crate::relay_members::RemoveResult::Removed
    ));
    assert!(
        db.apply_profile_admission(input(later + Duration::seconds(4), restored), &keys)
            .await
            .unwrap()
            .is_none(),
        "open relay admission is not explicit membership"
    );
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(community.as_uuid())
        .bind(&key)
        .execute(tx.conn())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        db.apply_profile_admission(input(later + Duration::seconds(5), restored), &keys)
            .await
            .unwrap()
            .is_none(),
        "remove/restore fences the old membership"
    );
    let restored = db
        .profile_epoch(community, &key, &policy)
        .await
        .unwrap()
        .unwrap();
    db.apply_profile_admission(
        ProfileAdmission {
            profile: Some(("Restored again", "alice", None)),
            ..input(later + Duration::seconds(6), restored)
        },
        &keys,
    )
    .await
    .unwrap()
    .unwrap();
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("true")
    );
    // Environment authorization ignores unrelated DB binding backfills.
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO community_nip_fi_issuers (community_id, issuer) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(&policy.issuer)
        .execute(tx.conn())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    db.process_profile_work(community, &key, &keys)
        .await
        .unwrap();
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("true")
    );
    assert!(db
        .apply_profile_admission(input(later + Duration::seconds(7), restored), &keys)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cleanup_versions_preserve_later_admissions_and_access_does_not_wait() {
    ordering_and_access_races().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_cleanup_versions_preserve_later_admissions_and_access_does_not_wait() {
    ordering_and_access_races().await;
}

async fn environment_removal_cleanup() {
    let (db, community, keys, key, mut policy) = fixture().await;
    let other = CommunityId::from_uuid(uuid::Uuid::new_v4());
    let host = format!("profile-{}.test", community.as_uuid());
    let other_host = format!("profile-{}.test", other.as_uuid());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(other.as_uuid())
        .bind(&other_host)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        other,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')")
        .bind(other.as_uuid())
        .bind(&key)
        .execute(tx.conn())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    policy.community_hosts = Some(vec![host.clone(), other_host.clone()]);
    db.configure_profile_policies(std::slice::from_ref(&policy), 1)
        .await
        .unwrap();
    for id in [community, other] {
        let epoch = db.profile_epoch(id, &key, &policy).await.unwrap().unwrap();
        admit(
            &db,
            id,
            &keys,
            &key,
            &policy,
            epoch,
            Utc::now(),
            Some(("Alice", "alice", None)),
        )
        .await;
    }
    let unaffected = current(&db, other, &key).await.unwrap();
    policy.community_hosts = Some(vec![other_host]);
    db.configure_profile_policies(std::slice::from_ref(&policy), 2)
        .await
        .unwrap();
    // Wait on the production asynchronous scheduler, not a test-only marker.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let marked: bool = sqlx::query_scalar("SELECT retract_before IS NOT NULL FROM nip_fi_profile_heads WHERE community_id = $1 AND pubkey = $2")
                .bind(community.as_uuid()).bind(&key).fetch_one(&db.pool).await.unwrap();
            if marked { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    // Restoration before housekeeping must not erase already-triggered cleanup.
    policy.community_hosts.as_mut().unwrap().push(host);
    db.configure_profile_policies(std::slice::from_ref(&policy), 3)
        .await
        .unwrap();
    for id in [community, other] {
        db.process_profile_work(id, &key, &keys).await.unwrap();
    }
    assert_eq!(
        tag(&current(&db, community, &key).await.unwrap(), "active"),
        Some("false")
    );
    assert_eq!(current(&db, other, &key).await.unwrap().id, unaffected.id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn environment_binding_removal_only_cleans_its_community() {
    environment_removal_cleanup().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_environment_binding_removal_only_cleans_its_community() {
    environment_removal_cleanup().await;
}
