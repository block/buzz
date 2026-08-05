use super::*;

use crate::identity_binding::{
    get_active_identity_binding_by_pubkey, resolve_identity_binding, test_lock_schedule,
    BindingDenial, ResolveBindingInput, ResolveBindingResult,
};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
const ISSUER: &str = "https://idp.example";
const SUBJECT: &str = "deterministic-subject";
const OLD_KEY: [u8; 32] = [31; 32];
const ENROLL_KEY: [u8; 32] = [32; 32];
const ROTATE_KEY: [u8; 32] = [33; 32];
const RECOVER_KEY: [u8; 32] = [34; 32];
const ENABLE_KEY: [u8; 32] = [35; 32];
const DOMAIN_B_KEY: [u8; 32] = [201; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Enroll,
    Retire,
    Rotate,
    Recover,
    Disable,
    Revoke,
    Enable,
    Archive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Applied,
    Existing,
    Denied,
    Error,
}

#[derive(Clone)]
struct Fixture {
    community_id: CommunityId,
    expected_pending: Option<PendingLineage>,
    enrollment_key: [u8; 32],
    archive_binding_id: Option<Uuid>,
}

async fn setup_pool() -> PgPool {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| TEST_DB_URL.to_owned());
    let pool = PgPool::connect(&database_url)
        .await
        .expect("connect deterministic test DB");
    crate::migration::run_migrations(&pool)
        .await
        .expect("run migrations");
    pool
}

async fn make_community(pool: &PgPool, label: &str) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
        .bind(id)
        .bind(format!("{label}-{}.example", id.simple()))
        .execute(pool)
        .await
        .expect("insert deterministic community");
    CommunityId::from_uuid(id)
}

fn principal() -> IdentityPrincipal<'static> {
    IdentityPrincipal {
        issuer: ISSUER,
        subject: SUBJECT,
    }
}

fn context(operation_id: Uuid, reason: &'static str) -> LifecycleContext<'static> {
    const ACTOR: [u8; 32] = [0xA1; 32];
    LifecycleContext {
        operation_id: LifecycleOperationId::from_uuid_for_test(operation_id),
        actor: &ACTOR,
        reason,
    }
}

fn replacement(pubkey: &'static [u8; 32]) -> VerifiedReplacementKey<'static> {
    VerifiedReplacementKey::after_verified_proof(
        pubkey,
        None,
        BindingProvenance::AttestedKey,
        "deterministic-policy-v1",
    )
    .expect("construct deterministic replacement")
}

async fn enroll_key(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> crate::identity_binding::BindingEvidence {
    let result = resolve_identity_binding(
        pool,
        &ResolveBindingInput {
            authorization_domain: community_id,
            issuer: ISSUER,
            subject: SUBJECT,
            pubkey,
            display_name: None,
            enrollment_mode: EnrollmentMode::AttestedKey,
            key_attested: true,
            policy_version: "deterministic-policy-v1",
            evidence_valid_from: 0,
            evidence_valid_until: i64::MAX as u64,
        },
    )
    .await
    .expect("seed enrollment");
    match result {
        ResolveBindingResult::Enrolled(evidence) => evidence,
        other => panic!("expected deterministic enrollment, got {other:?}"),
    }
}

fn pair_contains(pair: (Action, Action), action: Action) -> bool {
    pair.0 == action || pair.1 == action
}

async fn setup_fixture(pool: &PgPool, pair: (Action, Action), label: &str) -> Fixture {
    let community_id = make_community(pool, label).await;
    let mut expected_pending = None;
    let mut enrollment_key = ENROLL_KEY;
    let mut archive_binding_id = None;

    if pair_contains(pair, Action::Archive) {
        let evidence = enroll_key(pool, community_id, &OLD_KEY).await;
        retire_identity_pair(
            pool,
            community_id,
            context(Uuid::from_u128(9), "prepare archive recovery"),
            principal(),
            &OLD_KEY,
        )
        .await
        .expect("prepare archivable pending recovery");
        expected_pending = get_pending_lineage(pool, community_id, principal())
            .await
            .expect("read archive recovery selector");
        archive_binding_id = Some(evidence.binding_id());
    } else if pair_contains(pair, Action::Enroll) && pair_contains(pair, Action::Rotate) {
        enrollment_key = OLD_KEY;
    } else if pair_contains(pair, Action::Enroll) && pair_contains(pair, Action::Recover) {
        enroll_key(pool, community_id, &OLD_KEY).await;
        retire_identity_pair(
            pool,
            community_id,
            context(Uuid::from_u128(10), "prepare enrollment recovery"),
            principal(),
            &OLD_KEY,
        )
        .await
        .expect("prepare pending recovery");
        expected_pending = get_pending_lineage(pool, community_id, principal())
            .await
            .expect("read recovery selector");
        enrollment_key = RECOVER_KEY;
    } else {
        enroll_key(pool, community_id, &OLD_KEY).await;
        let needs_enabled_pending =
            pair_contains(pair, Action::Recover) && !pair_contains(pair, Action::Enable);
        let needs_disabled_pending = pair_contains(pair, Action::Enable);
        if needs_enabled_pending {
            retire_identity_pair(
                pool,
                community_id,
                context(Uuid::from_u128(11), "prepare enabled pending"),
                principal(),
                &OLD_KEY,
            )
            .await
            .expect("prepare enabled pending");
        } else if needs_disabled_pending {
            disable_identity_principal(
                pool,
                community_id,
                context(Uuid::from_u128(12), "prepare disabled pending"),
                principal(),
            )
            .await
            .expect("prepare disabled pending");
        }
        expected_pending = get_pending_lineage(pool, community_id, principal())
            .await
            .expect("read prepared selector");
    }

    Fixture {
        community_id,
        expected_pending,
        enrollment_key,
        archive_binding_id,
    }
}

async fn run_action(
    pool: &PgPool,
    fixture: &Fixture,
    action: Action,
    operation_id: Uuid,
) -> Outcome {
    match action {
        Action::Enroll => match resolve_identity_binding(
            pool,
            &ResolveBindingInput {
                authorization_domain: fixture.community_id,
                issuer: ISSUER,
                subject: SUBJECT,
                pubkey: &fixture.enrollment_key,
                display_name: None,
                enrollment_mode: EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "deterministic-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        {
            Ok(ResolveBindingResult::Enrolled(_)) => Outcome::Applied,
            Ok(ResolveBindingResult::Existing(_)) => Outcome::Existing,
            Ok(ResolveBindingResult::Denied(
                BindingDenial::Conflict
                | BindingDenial::Revoked
                | BindingDenial::BindingRequired
                | BindingDenial::KeyAttestationRequired
                | BindingDenial::BindingExpired
                | BindingDenial::StaleEvidence,
            )) => Outcome::Denied,
            Err(_) => Outcome::Error,
        },
        Action::Rotate => rotate_identity_binding(
            pool,
            fixture.community_id,
            context(operation_id, "deterministic rotate"),
            principal(),
            &OLD_KEY,
            replacement(&ROTATE_KEY),
        )
        .await
        .map_or(Outcome::Error, |_| Outcome::Applied),
        Action::Retire => retire_identity_pair(
            pool,
            fixture.community_id,
            context(operation_id, "deterministic retire"),
            principal(),
            &OLD_KEY,
        )
        .await
        .map_or(Outcome::Error, |_| Outcome::Applied),
        Action::Recover => {
            let Some(expected) = fixture.expected_pending.as_ref() else {
                return Outcome::Error;
            };
            recover_identity_binding(
                pool,
                fixture.community_id,
                context(operation_id, "deterministic recover"),
                principal(),
                expected,
                replacement(&RECOVER_KEY),
            )
            .await
            .map_or(Outcome::Error, |_| Outcome::Applied)
        }
        Action::Disable => disable_identity_principal(
            pool,
            fixture.community_id,
            context(operation_id, "deterministic disable"),
            principal(),
        )
        .await
        .map_or(Outcome::Error, |_| Outcome::Applied),
        Action::Revoke => revoke_identity_key(
            pool,
            fixture.community_id,
            context(operation_id, "deterministic revoke"),
            &OLD_KEY,
        )
        .await
        .map_or(Outcome::Error, |_| Outcome::Applied),
        Action::Enable => enable_identity_principal(
            pool,
            fixture.community_id,
            context(operation_id, "deterministic enable"),
            principal(),
            fixture.expected_pending.as_ref(),
            replacement(&ENABLE_KEY),
        )
        .await
        .map_or(Outcome::Error, |_| Outcome::Applied),
        Action::Archive => {
            let Some(binding_id) = fixture.archive_binding_id else {
                return Outcome::Error;
            };
            archive_identity_binding(
                pool,
                fixture.community_id,
                context(operation_id, "deterministic archive"),
                binding_id,
            )
            .await
            .map_or(Outcome::Error, |_| Outcome::Applied)
        }
    }
}

async fn normalized_rows(
    pool: &PgPool,
    community_id: CommunityId,
    query: &'static str,
) -> Vec<String> {
    let mut rows = sqlx::query_scalar::<_, Value>(query)
        .bind(community_id.as_uuid())
        .fetch_all(pool)
        .await
        .expect("read normalized identity projection")
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

async fn logical_snapshot(pool: &PgPool, community_id: CommunityId) -> Vec<String> {
    let queries = [
        "SELECT jsonb_build_object('t','binding','v',to_jsonb(b)-ARRAY['community_id','binding_id','replacement_binding_id','created_at','updated_at','last_seen_at','revoked_at','revoked_by','rotation_completed_at','rotation_by','archived_at']::text[]) FROM identity_bindings b WHERE community_id=$1",
        "SELECT jsonb_build_object('t','principal','v',jsonb_build_object('issuer',issuer,'subject',uid,'disabled',disabled_at IS NOT NULL,'reason',disabled_reason)) FROM identity_principals WHERE community_id=$1",
        "SELECT jsonb_build_object('t','revoked_key','v',jsonb_build_object('pubkey',encode(pubkey,'hex'),'reason',reason)) FROM identity_revoked_keys WHERE community_id=$1",
        "SELECT jsonb_build_object('t','retired','v',to_jsonb(r)-ARRAY['community_id','retired_binding_id','retired_at','retired_by']::text[]) FROM identity_retired_pairs r WHERE community_id=$1",
        "SELECT jsonb_build_object('t','pending','v',to_jsonb(p)-ARRAY['community_id','retired_binding_id','created_at','created_operation_id','cleared_at','cleared_operation_id']::text[] || jsonb_build_object('cleared',cleared_at IS NOT NULL)) FROM identity_pending_replacements p WHERE community_id=$1",
        "SELECT jsonb_build_object('t','history','v',to_jsonb(h)-ARRAY['community_id','history_id','binding_id','replacement_binding_id','operation_id','recorded_at']::text[]) FROM identity_binding_history h WHERE community_id=$1",
        "SELECT jsonb_build_object('t','operation','v',to_jsonb(o)-ARRAY['community_id','operation_id','request_fingerprint','binding_id','replacement_binding_id','created_at']::text[]) FROM identity_lifecycle_operations o WHERE community_id=$1",
        "SELECT jsonb_build_object('t','lineage','v',jsonb_build_object('predecessor',encode(p.pubkey,'hex'),'successor',encode(s.pubkey,'hex'))) FROM identity_binding_lineage l JOIN identity_bindings p ON p.community_id=l.community_id AND p.binding_id=l.predecessor_binding_id JOIN identity_bindings s ON s.community_id=l.community_id AND s.binding_id=l.successor_binding_id WHERE l.community_id=$1",
    ];
    let mut snapshot = Vec::new();
    for query in queries {
        snapshot.extend(normalized_rows(pool, community_id, query).await);
    }
    snapshot.sort();
    snapshot
}

async fn raw_domain_snapshot(pool: &PgPool, community_id: CommunityId) -> Vec<String> {
    let community: String = sqlx::query_scalar(
        "SELECT to_jsonb(community)::text FROM communities community WHERE id=$1",
    )
    .bind(community_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read exact domain community sentinel");
    let tables = [
        "identity_bindings",
        "identity_principals",
        "identity_revoked_keys",
        "identity_migration_denials",
        "identity_migration_denied_keys",
        "identity_binding_lineage",
        "identity_retired_pairs",
        "identity_pending_replacements",
        "identity_binding_history",
        "identity_lifecycle_operations",
        "audit_log",
    ];
    let mut snapshot = vec![format!("communities:{community}")];
    for table in tables {
        let query = format!(
            "SELECT to_jsonb(row_value)::text FROM {table} row_value WHERE community_id=$1 ORDER BY 1"
        );
        let rows = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(query))
            .bind(community_id.as_uuid())
            .fetch_all(pool)
            .await
            .expect("read exact domain sentinel");
        snapshot.extend(rows.into_iter().map(|row| format!("{table}:{row}")));
    }
    snapshot
}

async fn authorization_sentinel(pool: &PgPool, community_id: CommunityId) -> (Uuid, u64) {
    let binding = get_active_identity_binding_by_pubkey(pool, community_id, &DOMAIN_B_KEY)
        .await
        .expect("read domain-B authorization")
        .expect("domain-B binding remains active");
    (binding.binding_id, binding.binding_version)
}

async fn wait_for_advisory_waiter(
    pool: &PgPool,
    database_oid: u32,
    holder_pid: i32,
    waiter_pid: i32,
    lock_keys: &[test_lock_schedule::AdvisoryLockKey],
) {
    assert_ne!(holder_pid, waiter_pid);
    assert!(!lock_keys.is_empty(), "actors must share an identity lock");
    for _ in 0..10_000 {
        for lock_key in lock_keys {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(\
                   SELECT 1 FROM pg_locks held JOIN pg_locks waiting \
                     ON waiting.locktype=held.locktype \
                    AND waiting.database=held.database \
                    AND waiting.classid=held.classid \
                    AND waiting.objid=held.objid \
                    AND waiting.objsubid=held.objsubid \
                  WHERE held.locktype='advisory' \
                    AND held.database::BIGINT=$1 \
                    AND held.classid::BIGINT=$2 \
                    AND held.objid::BIGINT=$3 \
                    AND held.pid=$4 AND held.granted \
                    AND waiting.pid=$5 AND NOT waiting.granted\
                 )",
            )
            .bind(i64::from(database_oid))
            .bind(i64::from(lock_key.class_id()))
            .bind(i64::from(lock_key.object_id()))
            .bind(holder_pid)
            .bind(waiter_pid)
            .fetch_one(pool)
            .await
            .expect("inspect exact identity advisory waiter");
            if waiting {
                return;
            }
        }
        tokio::task::yield_now().await;
    }
    panic!("second actor never blocked on the shared identity advisory lock");
}

async fn wait_for_transaction_waiter(
    pool: &PgPool,
    database_oid: u32,
    holder_pid: i32,
    holder_transaction_id: i64,
    waiter_pid: i32,
) {
    assert_ne!(holder_pid, waiter_pid);
    for _ in 0..10_000 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
               SELECT 1 FROM pg_locks held JOIN pg_locks waiting \
                 ON waiting.locktype=held.locktype \
                AND waiting.transactionid=held.transactionid \
              WHERE held.locktype='transactionid' \
                AND held.database IS NULL AND waiting.database IS NULL \
                AND held.transactionid::text::BIGINT=$1 \
                AND held.pid=$2 AND held.mode='ExclusiveLock' AND held.granted \
                AND waiting.pid=$3 AND waiting.mode='ShareLock' AND NOT waiting.granted \
                AND EXISTS(SELECT 1 FROM pg_stat_activity activity \
                           WHERE activity.pid=waiting.pid \
                             AND activity.datid::BIGINT=$4 \
                             AND activity.wait_event_type='Lock' \
                             AND activity.wait_event='transactionid')\
             )",
        )
        .bind(holder_transaction_id)
        .bind(holder_pid)
        .bind(waiter_pid)
        .bind(i64::from(database_oid))
        .fetch_one(pool)
        .await
        .expect("inspect exact identity row-lock waiter");
        if waiting {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("second actor never blocked on the first actor's exact row transaction lock");
}

fn serializes_on_active_binding_row(first: Action, second: Action) -> bool {
    matches!(
        (first, second),
        (Action::Disable, Action::Revoke) | (Action::Revoke, Action::Disable)
    )
}

async fn force_order(
    pool: &PgPool,
    fixture: &Fixture,
    first: Action,
    second: Action,
    domain_b: CommunityId,
    domain_b_auth: (Uuid, u64),
) -> (Outcome, Outcome) {
    let (mut events, _controller) = test_lock_schedule::install();
    let row_schedule = serializes_on_active_binding_row(first, second);
    let (mut row_events, _row_controller) = row_schedule
        .then(test_lock_schedule::install_row)
        .map_or((None, None), |(events, guard)| (Some(events), Some(guard)));
    let first_pool = pool.clone();
    let first_fixture = fixture.clone();
    let first_task = tokio::spawn(test_lock_schedule::actor_scope("first", async move {
        run_action(&first_pool, &first_fixture, first, Uuid::from_u128(100)).await
    }));
    let event = events.recv().await.expect("first lock request trace");
    assert_eq!(
        (event.actor(), event.phase()),
        ("first", test_lock_schedule::LockPhase::Request)
    );
    let first_pid = event.backend_pid();
    let database_oid = event.database_oid();
    let first_lock_keys = event.lock_keys().to_vec();
    event.resume();
    let first_acquired = events.recv().await.expect("first lock acquired trace");
    assert_eq!(
        (first_acquired.actor(), first_acquired.phase()),
        ("first", test_lock_schedule::LockPhase::Acquired)
    );
    assert_eq!(first_acquired.isolation(), Some("read committed"));
    assert!(first_acquired.transaction_id().is_some());
    assert!(first_acquired.coordinate_count() >= 2);
    assert_eq!(first_acquired.backend_pid(), first_pid);
    assert_eq!(first_acquired.database_oid(), database_oid);
    assert_eq!(first_acquired.lock_keys(), first_lock_keys);

    let second_pool = pool.clone();
    let second_fixture = fixture.clone();
    let second_task = tokio::spawn(test_lock_schedule::actor_scope("second", async move {
        run_action(&second_pool, &second_fixture, second, Uuid::from_u128(101)).await
    }));
    let event = events.recv().await.expect("second lock request trace");
    assert_eq!(
        (event.actor(), event.phase()),
        ("second", test_lock_schedule::LockPhase::Request)
    );
    let second_pid = event.backend_pid();
    assert_eq!(event.database_oid(), database_oid);
    let shared_lock_keys = first_lock_keys
        .iter()
        .copied()
        .filter(|lock_key| event.lock_keys().contains(lock_key))
        .collect::<Vec<_>>();
    event.resume();
    if row_schedule {
        assert!(
            shared_lock_keys.is_empty(),
            "disable/revoke must prove its actual row-lock serialization point"
        );
        let second_acquired = events
            .recv()
            .await
            .expect("second independent advisory lock acquired trace");
        assert_eq!(
            (second_acquired.actor(), second_acquired.phase()),
            ("second", test_lock_schedule::LockPhase::Acquired)
        );
        assert_eq!(second_acquired.isolation(), Some("read committed"));
        assert_eq!(second_acquired.backend_pid(), second_pid);
        assert_eq!(second_acquired.database_oid(), database_oid);
        let first_advisory_transaction_id = first_acquired.transaction_id();
        let second_advisory_transaction_id = second_acquired.transaction_id();

        first_acquired.resume();
        let row_events = row_events.as_mut().expect("installed row-lock schedule");
        let first_row_request = row_events
            .recv()
            .await
            .expect("first row-lock request trace");
        assert_eq!(
            (first_row_request.actor(), first_row_request.phase()),
            ("first", test_lock_schedule::RowLockPhase::Request)
        );
        assert_eq!(first_row_request.backend_pid(), first_pid);
        assert_eq!(first_row_request.database_oid(), database_oid);
        assert_eq!(
            Some(first_row_request.transaction_id()),
            first_advisory_transaction_id
        );
        first_row_request.resume();
        let first_row_acquired = row_events
            .recv()
            .await
            .expect("first row-lock acquired trace");
        assert_eq!(
            (first_row_acquired.actor(), first_row_acquired.phase()),
            ("first", test_lock_schedule::RowLockPhase::Acquired)
        );
        assert_eq!(first_row_acquired.backend_pid(), first_pid);
        assert_eq!(first_row_acquired.database_oid(), database_oid);
        let first_transaction_id = first_row_acquired.transaction_id();
        assert_eq!(Some(first_transaction_id), first_advisory_transaction_id);

        second_acquired.resume();
        let second_row_request = row_events
            .recv()
            .await
            .expect("second row-lock request trace");
        assert_eq!(
            (second_row_request.actor(), second_row_request.phase()),
            ("second", test_lock_schedule::RowLockPhase::Request)
        );
        assert_eq!(second_row_request.backend_pid(), second_pid);
        assert_eq!(second_row_request.database_oid(), database_oid);
        assert_eq!(
            Some(second_row_request.transaction_id()),
            second_advisory_transaction_id
        );
        second_row_request.resume();
        wait_for_transaction_waiter(
            pool,
            database_oid,
            first_pid,
            first_transaction_id,
            second_pid,
        )
        .await;
        assert_eq!(authorization_sentinel(pool, domain_b).await, domain_b_auth);

        first_row_acquired.resume();
        let second_row_acquired = row_events
            .recv()
            .await
            .expect("second row-lock acquired trace");
        assert_eq!(
            (second_row_acquired.actor(), second_row_acquired.phase()),
            ("second", test_lock_schedule::RowLockPhase::Acquired)
        );
        assert_eq!(second_row_acquired.backend_pid(), second_pid);
        assert_eq!(second_row_acquired.database_oid(), database_oid);
        assert_eq!(
            Some(second_row_acquired.transaction_id()),
            second_advisory_transaction_id
        );
        second_row_acquired.resume();
    } else {
        wait_for_advisory_waiter(pool, database_oid, first_pid, second_pid, &shared_lock_keys)
            .await;
        assert_eq!(authorization_sentinel(pool, domain_b).await, domain_b_auth);

        first_acquired.resume();
        let second_acquired = events.recv().await.expect("second lock acquired trace");
        assert_eq!(
            (second_acquired.actor(), second_acquired.phase()),
            ("second", test_lock_schedule::LockPhase::Acquired)
        );
        assert_eq!(second_acquired.isolation(), Some("read committed"));
        assert_eq!(second_acquired.backend_pid(), second_pid);
        assert_eq!(second_acquired.database_oid(), database_oid);
        second_acquired.resume();
    }

    (
        first_task.await.expect("join first scheduled actor"),
        second_task.await.expect("join second scheduled actor"),
    )
}

async fn exercise_ordered_case(pool: &PgPool, first: Action, second: Action) {
    let pair = (first, second);
    let reference = setup_fixture(pool, pair, "identity-reference").await;
    let scheduled = setup_fixture(pool, pair, "identity-scheduled").await;
    let domain_b = make_community(pool, "identity-domain-b").await;
    enroll_key(pool, domain_b, &DOMAIN_B_KEY).await;
    let domain_b_bytes = raw_domain_snapshot(pool, domain_b).await;
    let domain_b_auth = authorization_sentinel(pool, domain_b).await;

    let expected_first = run_action(pool, &reference, first, Uuid::from_u128(100)).await;
    let expected_second = run_action(pool, &reference, second, Uuid::from_u128(101)).await;
    let actual = force_order(pool, &scheduled, first, second, domain_b, domain_b_auth).await;

    assert_eq!(actual, (expected_first, expected_second));
    assert_eq!(
        logical_snapshot(pool, scheduled.community_id).await,
        logical_snapshot(pool, reference.community_id).await,
        "forced schedule must match its independently executed sequential reference: {first:?} then {second:?}"
    );
    assert_eq!(raw_domain_snapshot(pool, domain_b).await, domain_b_bytes);
    assert_eq!(authorization_sentinel(pool, domain_b).await, domain_b_auth);
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn forced_lock_schedules_match_every_ordered_lifecycle_reference() {
    let pool = setup_pool().await;
    let actions = [
        Action::Retire,
        Action::Disable,
        Action::Revoke,
        Action::Rotate,
        Action::Recover,
        Action::Enable,
    ];
    for left in 0..actions.len() {
        for right in (left + 1)..actions.len() {
            exercise_ordered_case(&pool, actions[left], actions[right]).await;
            exercise_ordered_case(&pool, actions[right], actions[left]).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn enrollment_rotate_and_recover_have_forced_orders_and_references() {
    let pool = setup_pool().await;
    for lifecycle in [Action::Rotate, Action::Recover] {
        exercise_ordered_case(&pool, Action::Enroll, lifecycle).await;
        exercise_ordered_case(&pool, lifecycle, Action::Enroll).await;
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn archive_and_recovery_have_forced_orders_and_references() {
    let pool = setup_pool().await;
    exercise_ordered_case(&pool, Action::Archive, Action::Recover).await;
    exercise_ordered_case(&pool, Action::Recover, Action::Archive).await;
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn compare_and_clear_rejects_aba_recreation_and_preserves_domain_b() {
    let pool = setup_pool().await;
    let fixture = setup_fixture(&pool, (Action::Recover, Action::Disable), "identity-aba").await;
    let expected = fixture.expected_pending.expect("pending selector");
    let independently_observed = get_pending_lineage(&pool, fixture.community_id, principal())
        .await
        .expect("read second stale G1 observation")
        .expect("G1 remains active before either actor");
    assert!(expected == independently_observed);
    let domain_b = make_community(&pool, "identity-aba-domain-b").await;
    enroll_key(&pool, domain_b, &DOMAIN_B_KEY).await;
    let domain_b_bytes = raw_domain_snapshot(&pool, domain_b).await;
    let domain_b_auth = authorization_sentinel(&pool, domain_b).await;
    let history_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1")
            .bind(fixture.community_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count pre-ABA history");
    let operations_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1",
    )
    .bind(fixture.community_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count pre-ABA operations");

    let (mut events, _controller) = test_lock_schedule::install();
    let winner_pool = pool.clone();
    let winner_expected = expected.clone();
    let winner_community = fixture.community_id;
    let winner_task = tokio::spawn(test_lock_schedule::actor_scope("aba-winner", async move {
        let winner_context = context(Uuid::from_u128(200), "committed ABA recreation");
        let coordinates = lifecycle_coordinates(
            winner_community,
            winner_context,
            Some(principal()),
            &[],
            None,
        );
        let mut tx = begin_locked(&winner_pool, coordinates)
            .await
            .expect("begin locked ABA winner");
        let cleared = sqlx::query(
            "UPDATE identity_pending_replacements \
             SET cleared_at=NOW(),cleared_operation_id=$8 \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 \
               AND retired_pubkey=$4 AND retired_binding_id=$5 \
               AND retired_binding_version=$6 AND selector_version=$7 \
               AND cleared_at IS NULL",
        )
        .bind(winner_community.as_uuid())
        .bind(ISSUER)
        .bind(SUBJECT)
        .bind(&winner_expected.retired_pubkey)
        .bind(winner_expected.retired_binding_id)
        .bind(i64::try_from(winner_expected.retired_binding_version).unwrap())
        .bind(i64::try_from(winner_expected.selector_version).unwrap())
        .bind(winner_context.operation_id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("clear committed G1");
        assert_eq!(cleared.rows_affected(), 1);
        sqlx::query(
            "INSERT INTO identity_pending_replacements \
             (community_id,issuer,subject,selector_version,retired_pubkey, \
              retired_binding_id,retired_binding_version,created_operation_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(winner_community.as_uuid())
        .bind(ISSUER)
        .bind(SUBJECT)
        .bind(i64::try_from(winner_expected.selector_version + 1).unwrap())
        .bind(&winner_expected.retired_pubkey)
        .bind(winner_expected.retired_binding_id)
        .bind(i64::try_from(winner_expected.retired_binding_version).unwrap())
        .bind(Uuid::from_u128(201))
        .execute(&mut *tx)
        .await
        .expect("create semantically equal G2");
        tx.commit().await.expect("durably commit G1-to-G2 ABA");
    }));
    let winner_request = events.recv().await.expect("winner lock request");
    assert_eq!(
        (winner_request.actor(), winner_request.phase()),
        ("aba-winner", test_lock_schedule::LockPhase::Request)
    );
    let winner_pid = winner_request.backend_pid();
    let database_oid = winner_request.database_oid();
    let winner_lock_keys = winner_request.lock_keys().to_vec();
    winner_request.resume();
    let winner_acquired = events.recv().await.expect("winner lock acquired");
    assert_eq!(
        (winner_acquired.actor(), winner_acquired.phase()),
        ("aba-winner", test_lock_schedule::LockPhase::Acquired)
    );
    let winner_transaction_id = winner_acquired
        .transaction_id()
        .expect("winner transaction assigned");

    let loser_pool = pool.clone();
    let loser_expected = independently_observed.clone();
    let loser_community = fixture.community_id;
    let loser_task = tokio::spawn(test_lock_schedule::actor_scope("aba-loser", async move {
        let loser_context = context(Uuid::from_u128(202), "stale committed ABA compare");
        let coordinates =
            lifecycle_coordinates(loser_community, loser_context, Some(principal()), &[], None);
        let mut tx = begin_locked(&loser_pool, coordinates)
            .await
            .expect("begin locked ABA loser");
        let g2_before: String = sqlx::query_scalar(
            "SELECT to_jsonb(row_value)::TEXT FROM identity_pending_replacements row_value \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL",
        )
        .bind(loser_community.as_uuid())
        .bind(ISSUER)
        .bind(SUBJECT)
        .fetch_one(&mut *tx)
        .await
        .expect("read committed G2 before stale compare");
        let exact_stale_match: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_pending_replacements \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 \
               AND retired_pubkey=$4 AND retired_binding_id=$5 \
               AND retired_binding_version=$6 AND selector_version=$7 \
               AND cleared_at IS NULL",
        )
        .bind(loser_community.as_uuid())
        .bind(ISSUER)
        .bind(SUBJECT)
        .bind(&loser_expected.retired_pubkey)
        .bind(loser_expected.retired_binding_id)
        .bind(i64::try_from(loser_expected.retired_binding_version).unwrap())
        .bind(i64::try_from(loser_expected.selector_version).unwrap())
        .fetch_one(&mut *tx)
        .await
        .expect("count stale G1 tuple at compare point");
        assert_eq!(exact_stale_match, 0, "stale compare must affect zero rows");
        let error = compare_and_clear_pending_tx(
            &mut tx,
            loser_community,
            loser_context,
            principal(),
            &loser_expected,
        )
        .await
        .expect_err("committed G2 must reject stale G1 compare");
        assert!(error
            .to_string()
            .contains("pending identity lineage changed concurrently"));
        let g2_after: String = sqlx::query_scalar(
            "SELECT to_jsonb(row_value)::TEXT FROM identity_pending_replacements row_value \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL",
        )
        .bind(loser_community.as_uuid())
        .bind(ISSUER)
        .bind(SUBJECT)
        .fetch_one(&mut *tx)
        .await
        .expect("read G2 after stale compare");
        assert_eq!(g2_after, g2_before);
        tx.rollback().await.expect("rollback stale ABA actor");
        g2_after
    }));
    let loser_request = events.recv().await.expect("loser lock request");
    assert_eq!(
        (loser_request.actor(), loser_request.phase()),
        ("aba-loser", test_lock_schedule::LockPhase::Request)
    );
    let loser_pid = loser_request.backend_pid();
    let shared_keys = winner_lock_keys
        .iter()
        .copied()
        .filter(|key| loser_request.lock_keys().contains(key))
        .collect::<Vec<_>>();
    assert!(!shared_keys.is_empty());
    loser_request.resume();
    wait_for_advisory_waiter(&pool, database_oid, winner_pid, loser_pid, &shared_keys).await;
    assert_eq!(raw_domain_snapshot(&pool, domain_b).await, domain_b_bytes);
    assert_eq!(authorization_sentinel(&pool, domain_b).await, domain_b_auth);

    winner_acquired.resume();
    winner_task.await.expect("join committed ABA winner");
    let loser_acquired = events
        .recv()
        .await
        .expect("loser lock acquired after commit");
    assert_eq!(
        (loser_acquired.actor(), loser_acquired.phase()),
        ("aba-loser", test_lock_schedule::LockPhase::Acquired)
    );
    assert_ne!(
        loser_acquired.transaction_id(),
        Some(winner_transaction_id),
        "ABA actors must use distinct transactions"
    );
    loser_acquired.resume();
    let g2_from_loser = loser_task.await.expect("join stale ABA loser");

    let final_g2: String = sqlx::query_scalar(
        "SELECT to_jsonb(row_value)::TEXT FROM identity_pending_replacements row_value \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL",
    )
    .bind(fixture.community_id.as_uuid())
    .bind(ISSUER)
    .bind(SUBJECT)
    .fetch_one(&pool)
    .await
    .expect("read final committed G2");
    assert_eq!(final_g2, g2_from_loser);
    let final_lineage = get_pending_lineage(&pool, fixture.community_id, principal())
        .await
        .expect("read final G2")
        .expect("G2 remains active");
    assert_eq!(
        final_lineage.selector_version,
        expected.selector_version + 1
    );
    assert_eq!(final_lineage.retired_pubkey, expected.retired_pubkey);
    assert_eq!(
        final_lineage.retired_binding_id,
        expected.retired_binding_id
    );
    assert_eq!(
        final_lineage.retired_binding_version,
        expected.retired_binding_version
    );
    let active_pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_pending_replacements \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL",
    )
    .bind(fixture.community_id.as_uuid())
    .bind(ISSUER)
    .bind(SUBJECT)
    .fetch_one(&pool)
    .await
    .expect("count final active G2");
    assert_eq!(active_pending, 1);
    let cleared_g1: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_pending_replacements \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 \
           AND selector_version=$4 AND cleared_at IS NOT NULL",
    )
    .bind(fixture.community_id.as_uuid())
    .bind(ISSUER)
    .bind(SUBJECT)
    .bind(i64::try_from(expected.selector_version).unwrap())
    .fetch_one(&pool)
    .await
    .expect("count durably cleared G1");
    assert_eq!(cleared_g1, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1",
        )
        .bind(fixture.community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count post-ABA history"),
        history_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1",
        )
        .bind(fixture.community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count post-ABA operations"),
        operations_before
    );
    let fresh_attempt = resolve_identity_binding(
        &pool,
        &ResolveBindingInput {
            authorization_domain: fixture.community_id,
            issuer: ISSUER,
            subject: SUBJECT,
            pubkey: &ENABLE_KEY,
            display_name: None,
            enrollment_mode: EnrollmentMode::AttestedKey,
            key_attested: true,
            policy_version: "deterministic-policy-v1",
            evidence_valid_from: 0,
            evidence_valid_until: i64::MAX as u64,
        },
    )
    .await
    .expect("pending G2 denies routine enrollment");
    assert_eq!(
        fresh_attempt,
        ResolveBindingResult::Denied(BindingDenial::Revoked)
    );

    assert_eq!(raw_domain_snapshot(&pool, domain_b).await, domain_b_bytes);
    assert_eq!(authorization_sentinel(&pool, domain_b).await, domain_b_auth);
}
