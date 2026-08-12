use super::*;
use crate::identity_lifecycle::{
    lock_identity_lifecycle_transaction, IdentityLifecycleLockCoordinates,
};
use buzz_core::CommunityId;
use nostr::{Keys, PublicKey};
use sqlx::postgres::PgDatabaseError;
use sqlx::{Executor, PgConnection, Postgres};
use std::time::Duration;
use uuid::Uuid;

const ATTACH_SCHEMA_PARTITIONS_SQL: &str =
    include_str!("../../../../../scripts/attach-schema-partitions.sql");
const IDENTITY_FOUNDATION_SQL: &str =
    include_str!("../../../../../migrations/0029_nip_fi_identity_foundation.sql");
const AUTHORIZATION_FOUNDATION_SQL: &str =
    include_str!("../../../../../migrations/0030_nip_fi_authorization_foundation.sql");

#[test]
fn direct_schema_separates_durable_binding_and_invite_capabilities() {
    assert!(IDENTITY_FOUNDATION_SQL.contains("Federated token `exp` MUST NOT be"));
    assert!(IDENTITY_FOUNDATION_SQL.contains("binding_provenance IN (1, 2, 3)"));
    assert!(
        IDENTITY_FOUNDATION_SQL.contains("UNIQUE (community_id, policy_revision, enrollment_mode)")
    );
    assert!(IDENTITY_FOUNDATION_SQL
        .contains("FOREIGN KEY (community_id, policy_revision, binding_provenance)"));
    assert!(IDENTITY_FOUNDATION_SQL.contains("(community_id, policy_revision, enrollment_mode)"));
    assert!(IDENTITY_FOUNDATION_SQL.contains("expires_at TIMESTAMPTZ"));
    assert!(AUTHORIZATION_FOUNDATION_SQL.contains("28, 29"));
}

#[test]
fn client_status_is_ephemeral_and_retired_codes_stay_unallocated() {
    for forbidden in [
        "CREATE TABLE client_status_revisions",
        "client_status_revisions_current_lookup",
        "client_status_revisions_immutable",
        "client_status_revisions_no_truncate",
        "status published",
        "status withdrawn",
        "7 binding status",
    ] {
        assert!(
            !AUTHORIZATION_FOUNDATION_SQL.contains(forbidden),
            "durable status artifact remains: {forbidden}"
        );
    }
    assert!(IDENTITY_FOUNDATION_SQL
        .contains("operation_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)"));
    assert!(AUTHORIZATION_FOUNDATION_SQL.contains("object_kind IN (1, 2, 3, 4, 5, 6)"));
    assert!(AUTHORIZATION_FOUNDATION_SQL
        .contains("event_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14)"));
    assert!(AUTHORIZATION_FOUNDATION_SQL.contains("component_kind IN (1, 2, 3, 4, 6, 7)"));
    assert!(AUTHORIZATION_FOUNDATION_SQL.contains("Kinds 12 and 13 are retired"));
    assert!(AUTHORIZATION_FOUNDATION_SQL.contains("Kind 5 is retired"));
}

#[test]
fn catalog_closure_is_exact_for_retained_nip_fi_checks() {
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("ensure_exact_check_constraint"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("actual_definition IS DISTINCT FROM"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("ERRCODE = 'check_violation'"));

    let retained_constraints = [
        "authorization_invalidation_floors_check",
        "identity_bindings_check1",
        "identity_lifecycle_history_check",
        "identity_lifecycle_history_check1",
        "identity_lifecycle_history_check5",
        "identity_lifecycle_selectors_check",
        "authorization_event_capacity_check3",
        "authorization_events_check",
        "authorization_events_check1",
        "protected_object_authority_check1",
    ];
    for constraint in retained_constraints {
        let quoted = format!("'{constraint}'");
        assert_eq!(
            ATTACH_SCHEMA_PARTITIONS_SQL.matches(&quoted).count(),
            1,
            "closure must declare {constraint} exactly once"
        );
    }
}

#[test]
fn partition_attach_removes_every_parent_trigger_clone_dynamically() {
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("FROM pg_trigger AS child_trigger"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("JOIN pg_trigger AS parent_trigger"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("parent_trigger.tgrelid = parent_table"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("parent_trigger.tgname = child_trigger.tgname"));
    assert!(ATTACH_SCHEMA_PARTITIONS_SQL.contains("DROP TRIGGER IF EXISTS %I ON %s"));

    for partition in [
        "events_p_past",
        "events_p2026_01",
        "events_p2026_02",
        "events_p2026_03",
        "events_p2026_04",
        "events_p2026_05",
        "events_p2026_06",
        "events_p_future",
    ] {
        let call = format!("drop_cloned_parent_triggers('events', '{partition}')");
        assert!(
            ATTACH_SCHEMA_PARTITIONS_SQL.contains(&call),
            "missing parent-driven cleanup for {partition}"
        );
    }

    for partition in [
        "delivery_log_p_past",
        "delivery_log_p2026_03",
        "delivery_log_p2026_04",
        "delivery_log_p2026_05",
        "delivery_log_p2026_06",
        "delivery_log_p_future",
    ] {
        let call = format!("drop_cloned_parent_triggers('delivery_log', '{partition}')");
        assert!(
            ATTACH_SCHEMA_PARTITIONS_SQL.contains(&call),
            "missing parent-driven cleanup for {partition}"
        );
    }

    // These four current parent triggers motivated C5. They must be handled by
    // the catalog-driven loop, never by another finite hard-coded allow-list.
    for trigger in [
        "trg_events_guard_nip_rs_hard_delete",
        "trg_events_nip_rs_watermark",
        "trg_events_purge_soft_deleted_buzz_mesh_status",
        "trg_events_purge_soft_deleted_nip_rs",
    ] {
        assert!(
            !ATTACH_SCHEMA_PARTITIONS_SQL.contains(&format!("DROP TRIGGER IF EXISTS {trigger}"))
        );
    }

    for obsolete_allow_list_entry in [
        "DROP TRIGGER IF EXISTS events_enqueue_push_match",
        "DROP TRIGGER IF EXISTS events_refresh_channel_ttl",
        "DROP TRIGGER IF EXISTS events_created_at_floor",
    ] {
        assert!(!ATTACH_SCHEMA_PARTITIONS_SQL.contains(obsolete_allow_list_entry));
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn partition_attach_live_reconciles_current_and_future_parent_triggers_idempotently() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply direct-final migrations");

    sqlx::raw_sql(
        r#"
        CREATE FUNCTION c5_future_parent_trigger() RETURNS trigger
        LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
        CREATE TRIGGER trg_events_c5_future_parent_clone
        BEFORE INSERT ON events FOR EACH ROW EXECUTE FUNCTION c5_future_parent_trigger();
        ALTER TABLE events DETACH PARTITION events_p_future;
        "#,
    )
    .execute(&pool)
    .await
    .expect("create a future parent trigger and detach its cloned child");

    for attempt in 1..=2 {
        sqlx::raw_sql(ATTACH_SCHEMA_PARTITIONS_SQL)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("attachment attempt {attempt} failed: {error}"));
    }

    let attached: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_inherits \
         WHERE inhparent='events'::regclass AND inhrelid='events_p_future'::regclass)",
    )
    .fetch_one(&pool)
    .await
    .expect("read attachment state");
    assert!(attached);

    let missing_clones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger parent_trigger \
         WHERE parent_trigger.tgrelid='events'::regclass \
           AND NOT parent_trigger.tgisinternal \
           AND NOT EXISTS (SELECT 1 FROM pg_trigger child_trigger \
             WHERE child_trigger.tgrelid='events_p_future'::regclass \
               AND child_trigger.tgname=parent_trigger.tgname \
               AND NOT child_trigger.tgisinternal)",
    )
    .fetch_one(&pool)
    .await
    .expect("compare inherited trigger names");
    assert_eq!(missing_clones, 0);

    let future_clone_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger WHERE tgrelid='events_p_future'::regclass \
         AND tgname='trg_events_c5_future_parent_clone' AND NOT tgisinternal",
    )
    .fetch_one(&pool)
    .await
    .expect("count dynamically reconciled future trigger");
    assert_eq!(future_clone_count, 1);
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn catalog_closure_restores_exact_checks_and_rejects_wrong_same_name_definition() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply direct-final migrations");

    let retained_constraints = [
        (
            "authorization_invalidation_floors",
            "authorization_invalidation_floors_check",
        ),
        ("identity_bindings", "identity_bindings_check1"),
        (
            "identity_lifecycle_history",
            "identity_lifecycle_history_check",
        ),
        (
            "identity_lifecycle_history",
            "identity_lifecycle_history_check1",
        ),
        (
            "identity_lifecycle_history",
            "identity_lifecycle_history_check5",
        ),
        (
            "identity_lifecycle_selectors",
            "identity_lifecycle_selectors_check",
        ),
        (
            "authorization_event_capacity",
            "authorization_event_capacity_check3",
        ),
        ("authorization_events", "authorization_events_check"),
        ("authorization_events", "authorization_events_check1"),
        (
            "protected_object_authority",
            "protected_object_authority_check1",
        ),
    ];

    // A migration-built catalog already has every constraint; the closure is
    // a read-only exact-definition check in this state.
    sqlx::raw_sql(ATTACH_SCHEMA_PARTITIONS_SQL)
        .execute(&pool)
        .await
        .expect("validate migration-built constraint definitions");

    for (table, constraint) in retained_constraints {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} DROP CONSTRAINT {constraint}"
        )))
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("drop {constraint}: {error}"));
    }

    for attempt in 1..=2 {
        sqlx::raw_sql(ATTACH_SCHEMA_PARTITIONS_SQL)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("catalog closure attempt {attempt}: {error}"));
    }

    let restored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint \
         WHERE connamespace='public'::regnamespace AND conname=ANY($1)",
    )
    .bind(
        retained_constraints
            .iter()
            .map(|(_, constraint)| *constraint)
            .collect::<Vec<_>>(),
    )
    .fetch_one(&pool)
    .await
    .expect("count restored direct-final constraints");
    assert_eq!(restored, retained_constraints.len() as i64);

    let mut wrong_definition = pool.begin().await.expect("begin wrong-definition test");
    sqlx::raw_sql(
        "ALTER TABLE authorization_event_capacity \
         DROP CONSTRAINT authorization_event_capacity_check3; \
         ALTER TABLE authorization_event_capacity \
         ADD CONSTRAINT authorization_event_capacity_check3 \
         CHECK (health_state IN (1, 2))",
    )
    .execute(&mut *wrong_definition)
    .await
    .expect("install same-name wrong definition");
    let error = sqlx::raw_sql(ATTACH_SCHEMA_PARTITIONS_SQL)
        .execute(&mut *wrong_definition)
        .await
        .expect_err("same-name wrong constraint definition must fail closed");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_check3")
    );
    wrong_definition
        .rollback()
        .await
        .expect("rollback wrong-definition test");
}

#[derive(Clone)]
struct BindingFixture {
    id: Uuid,
    version: i64,
    principal: Vec<u8>,
    key: Vec<u8>,
    subject: String,
}

async fn insert_receipt<'executor, E>(
    executor: E,
    community_id: Uuid,
    operation_id: Uuid,
    fingerprint: &[u8],
    operation_kind: i16,
    outcome_code: i16,
) where
    E: Executor<'executor, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, request_fingerprint, operation_kind, \
          actor_fingerprint, outcome_code, result_digest) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(community_id)
    .bind(operation_id)
    .bind(fingerprint)
    .bind(operation_kind)
    .bind(vec![241_u8; 32])
    .bind(outcome_code)
    .bind(vec![242_u8; 32])
    .execute(executor)
    .await
    .expect("insert shared operation receipt");
}

#[allow(clippy::too_many_arguments)]
async fn insert_binding(
    connection: &mut PgConnection,
    community_id: Uuid,
    binding_id: Uuid,
    subject: &str,
    principal: &[u8],
    key: &[u8],
    birth_history_id: Uuid,
    operation_id: Uuid,
    fingerprint: &[u8],
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO identity_bindings \
         (community_id, binding_id, issuer, subject, principal_fingerprint, \
          event_author_pubkey, binding_state, lifecycle_revision, binding_provenance, \
          policy_revision, enrollment_evidence_digest, expires_at, birth_history_id, \
          creation_operation_id, creation_request_fingerprint) \
         VALUES ($1, $2, 'https://issuer.example', $3, $4, $5, 1, 1, 2, 1, $6, \
                 NULL, $7, $8, $9) \
         RETURNING binding_version",
    )
    .bind(community_id)
    .bind(binding_id)
    .bind(subject)
    .bind(principal)
    .bind(key)
    .bind(vec![211_u8; 32])
    .bind(birth_history_id)
    .bind(operation_id)
    .bind(fingerprint)
    .fetch_one(connection)
    .await
    .expect("insert immutable binding generation")
}

async fn insert_lifecycle_event(
    connection: &mut PgConnection,
    community_id: Uuid,
    operation_id: Uuid,
    fingerprint: &[u8],
    outcome_code: i16,
    envelope_byte: u8,
) {
    sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
          actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
          occurred_at,canonical_envelope,envelope_digest) \
         SELECT $1,$2,CASE receipt.operation_kind \
             WHEN 1 THEN 1 WHEN 2 THEN 1 WHEN 3 THEN 6 WHEN 4 THEN 7 \
             WHEN 5 THEN 2 WHEN 6 THEN 3 WHEN 7 THEN 4 WHEN 8 THEN 5 WHEN 9 THEN 8 \
         END,$3,1,1,receipt.actor_fingerprint,$4,$5,$6,$7,transaction_timestamp(),$8,$9 \
         FROM authorization_operation_receipts receipt \
         WHERE receipt.community_id=$1 AND receipt.operation_id=$4 \
           AND receipt.operation_kind BETWEEN 1 AND 9",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(outcome_code)
    .bind(operation_id)
    .bind(fingerprint)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![envelope_byte; 32])
    .bind(vec![214_u8; 32])
    .execute(&mut *connection)
    .await
    .expect("insert lifecycle audit event");
}

#[allow(clippy::too_many_arguments)]
async fn insert_history(
    connection: &mut PgConnection,
    community_id: Uuid,
    history_id: Uuid,
    transition_kind: i16,
    outcome_code: i16,
    old_binding: Option<&BindingFixture>,
    old_prior_revision: Option<i64>,
    old_prior_state: Option<i16>,
    old_resulting_revision: Option<i64>,
    old_resulting_state: Option<i16>,
    successor: Option<&BindingFixture>,
    operation_id: Uuid,
    fingerprint: &[u8],
) {
    sqlx::query(
        "INSERT INTO identity_lifecycle_history \
         (community_id, history_id, transition_kind, outcome_code, old_binding_id, \
          old_binding_version, old_prior_lifecycle_revision, old_prior_state, \
          old_resulting_lifecycle_revision, old_resulting_state, successor_binding_id, \
          successor_binding_version, successor_lifecycle_revision, successor_state, \
          operation_id, request_fingerprint, transition_digest) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
                 CASE WHEN $11::uuid IS NULL THEN NULL ELSE 1 END, \
                 CASE WHEN $11::uuid IS NULL THEN NULL ELSE 1 END, $13, $14, $15)",
    )
    .bind(community_id)
    .bind(history_id)
    .bind(transition_kind)
    .bind(outcome_code)
    .bind(old_binding.map(|binding| binding.id))
    .bind(old_binding.map(|binding| binding.version))
    .bind(old_prior_revision)
    .bind(old_prior_state)
    .bind(old_resulting_revision)
    .bind(old_resulting_state)
    .bind(successor.map(|binding| binding.id))
    .bind(successor.map(|binding| binding.version))
    .bind(operation_id)
    .bind(fingerprint)
    .bind(vec![212_u8; 32])
    .execute(&mut *connection)
    .await
    .expect("insert canonical lifecycle transition");

    insert_lifecycle_event(
        connection,
        community_id,
        operation_id,
        fingerprint,
        outcome_code,
        transition_kind as u8,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn insert_selector(
    connection: &mut PgConnection,
    community_id: Uuid,
    selector_id: Uuid,
    selector_kind: i16,
    fingerprint_byte: u8,
    fact_generation: i64,
    principal: Option<&[u8]>,
    key: Option<&[u8]>,
    binding: Option<&BindingFixture>,
    history_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: &[u8],
) {
    sqlx::query(
        "INSERT INTO identity_lifecycle_selectors \
         (community_id, selector_id, selector_kind, selector_fingerprint, fact_generation, \
          principal_fingerprint, event_author_pubkey, binding_id, binding_version, \
          asserted_history_id, selected_by_operation_id, selected_by_request_fingerprint) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(community_id)
    .bind(selector_id)
    .bind(selector_kind)
    .bind(vec![fingerprint_byte; 32])
    .bind(fact_generation)
    .bind(principal)
    .bind(key)
    .bind(binding.map(|row| row.id))
    .bind(binding.map(|row| row.version))
    .bind(history_id)
    .bind(operation_id)
    .bind(request_fingerprint)
    .execute(connection)
    .await
    .expect("insert immutable lifecycle selector");
}

#[allow(clippy::too_many_arguments)]
async fn insert_consumption(
    connection: &mut PgConnection,
    community_id: Uuid,
    selector_id: Uuid,
    selector_kind: i16,
    history_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: &[u8],
    successor: &BindingFixture,
) {
    sqlx::query(
        "INSERT INTO identity_lifecycle_selector_consumptions \
         (community_id, selector_id, selector_kind, history_id, operation_id, \
          request_fingerprint, successor_binding_id, successor_binding_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(community_id)
    .bind(selector_id)
    .bind(selector_kind)
    .bind(history_id)
    .bind(operation_id)
    .bind(request_fingerprint)
    .bind(successor.id)
    .bind(successor.version)
    .execute(connection)
    .await
    .expect("consume immutable lifecycle selector");
}

async fn seed_binding(
    pool: &PgPool,
    community_id: Uuid,
    subject: &str,
    principal_byte: u8,
    key_byte: u8,
) -> BindingFixture {
    seed_binding_coordinates(
        pool,
        community_id,
        subject,
        vec![principal_byte; 32],
        vec![key_byte; 32],
    )
    .await
}

async fn seed_binding_coordinates(
    pool: &PgPool,
    community_id: Uuid,
    subject: &str,
    principal: Vec<u8>,
    key: Vec<u8>,
) -> BindingFixture {
    let operation_id = Uuid::new_v4();
    let history_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let fingerprint = vec![principal[0].wrapping_add(80); 32];
    let mut transaction = pool.begin().await.expect("begin binding birth");
    insert_receipt(
        &mut *transaction,
        community_id,
        operation_id,
        &fingerprint,
        1,
        1,
    )
    .await;
    let version = insert_binding(
        &mut transaction,
        community_id,
        binding_id,
        subject,
        &principal,
        &key,
        history_id,
        operation_id,
        &fingerprint,
    )
    .await;
    let binding = BindingFixture {
        id: binding_id,
        version,
        principal,
        key,
        subject: subject.to_owned(),
    };
    insert_history(
        &mut transaction,
        community_id,
        history_id,
        1,
        1,
        None,
        None,
        None,
        None,
        None,
        Some(&binding),
        operation_id,
        &fingerprint,
    )
    .await;
    (&mut *transaction)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("binding birth satisfies deferred constraints");
    transaction.commit().await.expect("commit binding birth");
    binding
}

fn constraint(error: &sqlx::Error) -> Option<&str> {
    error
        .as_database_error()
        .and_then(|database_error| database_error.try_downcast_ref::<PgDatabaseError>())
        .and_then(PgDatabaseError::constraint)
}

#[derive(Clone, Copy, Debug)]
enum SelectorRaceClass {
    P,
    X,
    Y,
    Q,
}

impl SelectorRaceClass {
    const ALL: [Self; 4] = [Self::P, Self::X, Self::Y, Self::Q];

    const fn seed(self) -> u8 {
        match self {
            Self::P => 160,
            Self::X => 180,
            Self::Y => 200,
            Self::Q => 220,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::P => "P",
            Self::X => "X",
            Self::Y => "Y",
            Self::Q => "Q",
        }
    }

    const fn transition_kind(self) -> i16 {
        match self {
            Self::P | Self::Q => 3,
            Self::X => 4,
            Self::Y => 5,
        }
    }

    const fn requires_old_binding(self) -> bool {
        matches!(self, Self::P | Self::Q)
    }

    fn selector_order(self) -> Vec<i16> {
        match self {
            Self::P => vec![1, 4],
            Self::Q => vec![4, 1],
            Self::X => vec![2],
            Self::Y => vec![3],
        }
    }
}

fn race_principal(principal: &[u8]) -> [u8; 32] {
    principal
        .try_into()
        .expect("race principal fingerprint has canonical length")
}

fn selector_race_lock_coordinates(
    community_id: Uuid,
    class: SelectorRaceClass,
    principal: &[u8],
    event_author_pubkey: PublicKey,
) -> IdentityLifecycleLockCoordinates {
    let principal = matches!(
        class,
        SelectorRaceClass::P | SelectorRaceClass::X | SelectorRaceClass::Q
    )
    .then(|| race_principal(principal));
    let key = matches!(
        class,
        SelectorRaceClass::P | SelectorRaceClass::Y | SelectorRaceClass::Q
    )
    .then_some(event_author_pubkey);
    IdentityLifecycleLockCoordinates::new(
        CommunityId::from_uuid(community_id),
        principal,
        key,
        None,
    )
    .expect("valid selector race lock coordinates")
}

fn birth_race_lock_coordinates(
    community_id: Uuid,
    principal: &[u8],
    event_author_pubkey: PublicKey,
) -> IdentityLifecycleLockCoordinates {
    IdentityLifecycleLockCoordinates::new(
        CommunityId::from_uuid(community_id),
        Some(race_principal(principal)),
        None,
        Some(event_author_pubkey),
    )
    .expect("valid binding birth race lock coordinates")
}

#[derive(Debug)]
struct RaceLoser {
    operation_id: Uuid,
    constraint: Option<String>,
}

async fn wait_for_advisory_lock(pool: &PgPool, backend_pid: i32) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT COALESCE(wait_event = 'advisory', false) \
                 FROM pg_stat_activity WHERE pid = $1",
            )
            .bind(backend_pid)
            .fetch_one(pool)
            .await
            .expect("observe competing backend");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("competing backend must block on the shared advisory scope");
}

async fn assert_race_loser_has_no_residue(pool: &PgPool, community_id: Uuid, operation_id: Uuid) {
    let residue: i64 = sqlx::query_scalar(
        "SELECT \
            (SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2) + \
            (SELECT count(*) FROM identity_lifecycle_history \
             WHERE community_id=$1 AND operation_id=$2) + \
            (SELECT count(*) FROM identity_bindings \
             WHERE community_id=$1 AND creation_operation_id=$2) + \
            (SELECT count(*) FROM identity_lifecycle_selectors \
             WHERE community_id=$1 AND selected_by_operation_id=$2) + \
            (SELECT count(*) FROM identity_lifecycle_selector_consumptions \
             WHERE community_id=$1 AND operation_id=$2) + \
            (SELECT count(*) FROM authorization_events \
             WHERE community_id=$1 AND operation_id=$2)",
    )
    .bind(community_id)
    .bind(operation_id)
    .fetch_one(pool)
    .await
    .expect("count race-loser residue");
    assert_eq!(residue, 0, "failed race transaction must roll back exactly");
}

#[allow(clippy::too_many_arguments)]
async fn insert_race_selectors(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    community_id: Uuid,
    class: SelectorRaceClass,
    selector_fingerprint_seed: u8,
    principal: &[u8],
    key: &[u8],
    old_binding: Option<&BindingFixture>,
    history_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: &[u8],
) {
    for (offset, selector_kind) in class.selector_order().into_iter().enumerate() {
        insert_selector(
            transaction,
            community_id,
            Uuid::new_v4(),
            selector_kind,
            selector_fingerprint_seed.wrapping_add(3 + offset as u8),
            1,
            matches!(selector_kind, 1 | 2 | 4).then_some(principal),
            matches!(selector_kind, 1 | 3 | 4).then_some(key),
            matches!(selector_kind, 1 | 4)
                .then_some(old_binding)
                .flatten(),
            history_id,
            operation_id,
            request_fingerprint,
        )
        .await;
    }
}

async fn assert_selector_first_race(pool: &PgPool, community_id: Uuid, class: SelectorRaceClass) {
    let seed = class.seed();
    let principal = vec![seed; 32];
    let event_author_pubkey = Keys::generate().public_key();
    let key = event_author_pubkey.to_bytes().to_vec();
    let old_binding = if class.requires_old_binding() {
        Some(
            seed_binding_coordinates(
                pool,
                community_id,
                &format!("selector-first-{}", class.label()),
                principal.clone(),
                key.clone(),
            )
            .await,
        )
    } else {
        None
    };
    let winner_operation = Uuid::new_v4();
    let winner_history = Uuid::new_v4();
    let winner_fingerprint = vec![seed.wrapping_add(2); 32];
    let mut winner = pool.begin().await.expect("begin selector-first winner");
    let winner_locks =
        selector_race_lock_coordinates(community_id, class, &principal, event_author_pubkey);
    lock_identity_lifecycle_transaction(&mut winner, &winner_locks)
        .await
        .expect("pre-lock selector-first winner coordinates");
    insert_receipt(
        &mut *winner,
        community_id,
        winner_operation,
        &winner_fingerprint,
        class.transition_kind(),
        1,
    )
    .await;
    insert_history(
        &mut winner,
        community_id,
        winner_history,
        class.transition_kind(),
        1,
        old_binding.as_ref(),
        old_binding.as_ref().map(|_| 1),
        old_binding.as_ref().map(|_| 1),
        old_binding.as_ref().map(|_| 2),
        old_binding.as_ref().map(|_| 2),
        None,
        winner_operation,
        &winner_fingerprint,
    )
    .await;
    insert_race_selectors(
        &mut winner,
        community_id,
        class,
        seed,
        &principal,
        &key,
        old_binding.as_ref(),
        winner_history,
        winner_operation,
        &winner_fingerprint,
    )
    .await;
    if let Some(old_binding) = old_binding.as_ref() {
        sqlx::query(
            "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
             retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
        )
        .bind(winner_history)
        .bind(community_id)
        .bind(old_binding.id)
        .execute(&mut *winner)
        .await
        .expect("complete selector-first retirement");
    }

    let loser_operation = Uuid::new_v4();
    let loser_history = Uuid::new_v4();
    let loser_binding_id = Uuid::new_v4();
    let loser_fingerprint = vec![seed.wrapping_add(7); 32];
    let loser_pool = pool.clone();
    let loser_principal = principal.clone();
    let loser_key = key.clone();
    let loser_event_author_pubkey = event_author_pubkey;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let loser = tokio::spawn(async move {
        let mut transaction = loser_pool.begin().await.expect("begin waiting birth");
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await
            .expect("read waiting birth backend pid");
        ready_tx
            .send(backend_pid)
            .expect("publish birth backend pid");
        let loser_locks =
            birth_race_lock_coordinates(community_id, &loser_principal, loser_event_author_pubkey);
        lock_identity_lifecycle_transaction(&mut transaction, &loser_locks)
            .await
            .expect("resume after selector winner releases birth coordinates");
        insert_receipt(
            &mut *transaction,
            community_id,
            loser_operation,
            &loser_fingerprint,
            1,
            1,
        )
        .await;
        let version = insert_binding(
            &mut transaction,
            community_id,
            loser_binding_id,
            &format!("selector-race-loser-{}", class.label()),
            &loser_principal,
            &loser_key,
            loser_history,
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        let binding = BindingFixture {
            id: loser_binding_id,
            version,
            principal: loser_principal,
            key: loser_key,
            subject: format!("selector-race-loser-{}", class.label()),
        };
        insert_history(
            &mut transaction,
            community_id,
            loser_history,
            1,
            1,
            None,
            None,
            None,
            None,
            None,
            Some(&binding),
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        let error = (&mut *transaction)
            .execute("SET CONSTRAINTS ALL IMMEDIATE")
            .await
            .expect_err("selector winner must reject resumed binding birth");
        let failed_constraint = constraint(&error).map(str::to_owned);
        transaction
            .rollback()
            .await
            .expect("rollback rejected birth");
        RaceLoser {
            operation_id: loser_operation,
            constraint: failed_constraint,
        }
    });
    let backend_pid = tokio::time::timeout(Duration::from_secs(2), ready_rx)
        .await
        .expect("birth waiter must start")
        .expect("birth waiter must publish backend pid");
    wait_for_advisory_lock(pool, backend_pid).await;
    (&mut *winner)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("selector-first winner remains canonical");
    winner.commit().await.expect("commit selector-first winner");
    let loser = tokio::time::timeout(Duration::from_secs(2), loser)
        .await
        .expect("birth waiter must resume")
        .expect("birth waiter task must not panic");
    assert!(
        matches!(
            loser.constraint.as_deref(),
            Some("identity_bindings_birth_eligibility" | "identity_lifecycle_transition_integrity")
        ),
        "unexpected selector-first loser constraint: {:?}",
        loser.constraint
    );
    assert_race_loser_has_no_residue(pool, community_id, loser.operation_id).await;
}

async fn assert_birth_first_race(pool: &PgPool, community_id: Uuid, class: SelectorRaceClass) {
    let seed = class.seed().wrapping_add(10);
    let principal = vec![seed; 32];
    let event_author_pubkey = Keys::generate().public_key();
    let key = event_author_pubkey.to_bytes().to_vec();
    let winner_operation = Uuid::new_v4();
    let winner_history = Uuid::new_v4();
    let winner_binding_id = Uuid::new_v4();
    let winner_fingerprint = vec![seed.wrapping_add(2); 32];
    let mut winner = pool.begin().await.expect("begin birth-first winner");
    let winner_locks = birth_race_lock_coordinates(community_id, &principal, event_author_pubkey);
    lock_identity_lifecycle_transaction(&mut winner, &winner_locks)
        .await
        .expect("pre-lock birth-first winner coordinates");
    insert_receipt(
        &mut *winner,
        community_id,
        winner_operation,
        &winner_fingerprint,
        1,
        1,
    )
    .await;
    let winner_version = insert_binding(
        &mut winner,
        community_id,
        winner_binding_id,
        &format!("birth-first-{}", class.label()),
        &principal,
        &key,
        winner_history,
        winner_operation,
        &winner_fingerprint,
    )
    .await;
    let winner_binding = BindingFixture {
        id: winner_binding_id,
        version: winner_version,
        principal: principal.clone(),
        key: key.clone(),
        subject: format!("birth-first-{}", class.label()),
    };
    insert_history(
        &mut winner,
        community_id,
        winner_history,
        1,
        1,
        None,
        None,
        None,
        None,
        None,
        Some(&winner_binding),
        winner_operation,
        &winner_fingerprint,
    )
    .await;

    let loser_operation = Uuid::new_v4();
    let loser_history = Uuid::new_v4();
    let loser_fingerprint = vec![seed.wrapping_add(7); 32];
    let loser_pool = pool.clone();
    let loser_principal = principal.clone();
    let loser_key = key.clone();
    let loser_binding = winner_binding.clone();
    let loser_event_author_pubkey = event_author_pubkey;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let loser = tokio::spawn(async move {
        let mut transaction = loser_pool.begin().await.expect("begin waiting selector");
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await
            .expect("read waiting selector backend pid");
        ready_tx
            .send(backend_pid)
            .expect("publish selector backend pid");
        let loser_locks = selector_race_lock_coordinates(
            community_id,
            class,
            &loser_principal,
            loser_event_author_pubkey,
        );
        lock_identity_lifecycle_transaction(&mut transaction, &loser_locks)
            .await
            .expect("resume after birth winner releases selector coordinates");
        insert_receipt(
            &mut *transaction,
            community_id,
            loser_operation,
            &loser_fingerprint,
            class.transition_kind(),
            1,
        )
        .await;
        let old_binding = class.requires_old_binding().then_some(&loser_binding);
        insert_history(
            &mut transaction,
            community_id,
            loser_history,
            class.transition_kind(),
            1,
            old_binding,
            old_binding.map(|_| 1),
            old_binding.map(|_| 1),
            old_binding.map(|_| 2),
            old_binding.map(|_| 2),
            None,
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        insert_race_selectors(
            &mut transaction,
            community_id,
            class,
            seed,
            &loser_principal,
            &loser_key,
            old_binding,
            loser_history,
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        let error = (&mut *transaction)
            .execute("SET CONSTRAINTS ALL IMMEDIATE")
            .await
            .expect_err("birth winner must reject resumed selector assertion");
        let failed_constraint = constraint(&error).map(str::to_owned);
        transaction
            .rollback()
            .await
            .expect("rollback rejected selector assertion");
        RaceLoser {
            operation_id: loser_operation,
            constraint: failed_constraint,
        }
    });
    let backend_pid = tokio::time::timeout(Duration::from_secs(2), ready_rx)
        .await
        .expect("selector waiter must start")
        .expect("selector waiter must publish backend pid");
    wait_for_advisory_lock(pool, backend_pid).await;
    (&mut *winner)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("birth-first winner remains canonical");
    winner.commit().await.expect("commit birth-first winner");
    let loser = tokio::time::timeout(Duration::from_secs(2), loser)
        .await
        .expect("selector waiter must resume")
        .expect("selector waiter task must not panic");
    assert_eq!(
        loser.constraint.as_deref(),
        Some("identity_lifecycle_transition_integrity"),
        "unexpected birth-first loser constraint"
    );
    assert_race_loser_has_no_residue(pool, community_id, loser.operation_id).await;
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn nip_fi_direct_final_catalog_and_behavior() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply direct-final NIP-FI migrations");

    let forbidden_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = ANY($1)",
    )
    .bind(vec![
        "identity_lifecycle_operations",
        "identity_pending_replacements",
        "authorization_leases",
        "audio_admission_ledger",
        "authorization_event_deliveries",
        "authorization_event_claims",
        "client_status_revisions",
        "public_projection",
        "trusted_assertion_projections",
    ])
    .fetch_one(&pool)
    .await
    .expect("check removed provider/compatibility tables");
    assert_eq!(forbidden_count, 0);

    let community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community_id)
        .bind(format!("nip-fi-{}.example", community_id.simple()))
        .execute(&pool)
        .await
        .expect("insert test community");
    sqlx::query(
        "INSERT INTO identity_enrollment_policies \
         (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
         VALUES ($1, 1, 2, $2, transaction_timestamp())",
    )
    .bind(community_id)
    .bind(vec![17_u8; 32])
    .execute(&pool)
    .await
    .expect("insert local enrollment policy");

    for (limits, expected_constraint) in [
        (
            (10_001_i64, 16_777_216_i64, 16_384_i32),
            "authorization_event_capacity_max_events",
        ),
        (
            (10_000_i64, 16_777_217_i64, 16_384_i32),
            "authorization_event_capacity_max_bytes",
        ),
        (
            (10_000_i64, 16_777_216_i64, 16_385_i32),
            "authorization_event_capacity_max_envelope",
        ),
    ] {
        let error = sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(community_id)
        .bind(limits.0)
        .bind(limits.1)
        .bind(limits.2)
        .execute(&pool)
        .await
        .expect_err("V1 hard audit-capacity ceiling must reject oversized policy");
        assert_eq!(constraint(&error), Some(expected_constraint));
    }
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
         VALUES ($1,10000,16777216,16384)",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect("install direct-final audit capacity");

    // Both transaction orderings are exercised for every closed selector
    // class. The waiter is observed in PostgreSQL's advisory wait state, the
    // winner commits, the waiter resumes and rejects, and all loser rows roll
    // back. P/Q use the canonical paired Retire shape with the selected class
    // inserted first so each trigger lock path is independently exercised.
    let mut executed_race_cases = 0_u8;
    for class in SelectorRaceClass::ALL {
        assert_selector_first_race(&pool, community_id, class).await;
        executed_race_cases += 1;
        assert_birth_first_race(&pool, community_id, class).await;
        executed_race_cases += 1;
    }
    assert_eq!(
        executed_race_cases, 8,
        "every P/X/Y/Q selector-first and birth-first race must execute"
    );

    // Rotate retires one immutable generation and births a distinct fresh
    // generation under one history row. It asserts P, never Q or Y.
    let rotate_old = seed_binding(&pool, community_id, "rotate", 31, 41).await;
    let rotate_operation = Uuid::new_v4();
    let rotate_history = Uuid::new_v4();
    let rotate_fingerprint = vec![32_u8; 32];
    let mut rotate = pool.begin().await.expect("begin rotate");
    insert_receipt(
        &mut *rotate,
        community_id,
        rotate_operation,
        &rotate_fingerprint,
        6,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3 \
         AND binding_state=1 AND lifecycle_revision=1",
    )
    .bind(rotate_history)
    .bind(community_id)
    .bind(rotate_old.id)
    .execute(&mut *rotate)
    .await
    .expect("retire rotate predecessor");
    let rotate_successor_id = Uuid::new_v4();
    let rotate_successor_version = insert_binding(
        &mut rotate,
        community_id,
        rotate_successor_id,
        &rotate_old.subject,
        &rotate_old.principal,
        &[42_u8; 32],
        rotate_history,
        rotate_operation,
        &rotate_fingerprint,
    )
    .await;
    let rotate_successor = BindingFixture {
        id: rotate_successor_id,
        version: rotate_successor_version,
        principal: rotate_old.principal.clone(),
        key: vec![42_u8; 32],
        subject: rotate_old.subject.clone(),
    };
    insert_history(
        &mut rotate,
        community_id,
        rotate_history,
        6,
        1,
        Some(&rotate_old),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        Some(&rotate_successor),
        rotate_operation,
        &rotate_fingerprint,
    )
    .await;
    insert_selector(
        &mut rotate,
        community_id,
        Uuid::new_v4(),
        1,
        33,
        1,
        Some(&rotate_old.principal),
        Some(&rotate_old.key),
        Some(&rotate_old),
        rotate_history,
        rotate_operation,
        &rotate_fingerprint,
    )
    .await;
    (&mut *rotate)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("rotate exact composites");
    rotate.commit().await.expect("commit rotate");
    let rotate_state: (i64, i16, i64) = sqlx::query_as(
        "SELECT binding_version, binding_state, lifecycle_revision \
         FROM identity_bindings WHERE community_id=$1 AND binding_id=$2",
    )
    .bind(community_id)
    .bind(rotate_old.id)
    .fetch_one(&pool)
    .await
    .expect("read rotated predecessor");
    assert_eq!(rotate_state, (rotate_old.version, 2, 2));
    assert_ne!(rotate_successor.version, rotate_old.version);
    let rotate_forbidden_selectors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_lifecycle_selectors \
         WHERE community_id=$1 AND asserted_history_id=$2 AND selector_kind IN (3,4)",
    )
    .bind(community_id)
    .bind(rotate_history)
    .fetch_one(&pool)
    .await
    .expect("read rotate selectors");
    assert_eq!(rotate_forbidden_selectors, 0);

    // Retire creates permanent P plus open Q. Recover keeps the old generation
    // retired, births a fresh generation, and consumes the exact Q.
    let recover_old = seed_binding(&pool, community_id, "recover", 51, 61).await;
    let retire_operation = Uuid::new_v4();
    let retire_history = Uuid::new_v4();
    let retire_fingerprint = vec![52_u8; 32];
    let q_selector = Uuid::new_v4();
    let mut retire = pool.begin().await.expect("begin retire");
    insert_receipt(
        &mut *retire,
        community_id,
        retire_operation,
        &retire_fingerprint,
        3,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(retire_history)
    .bind(community_id)
    .bind(recover_old.id)
    .execute(&mut *retire)
    .await
    .expect("retire recover predecessor");
    insert_history(
        &mut retire,
        community_id,
        retire_history,
        3,
        1,
        Some(&recover_old),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        retire_operation,
        &retire_fingerprint,
    )
    .await;
    insert_selector(
        &mut retire,
        community_id,
        Uuid::new_v4(),
        1,
        53,
        1,
        Some(&recover_old.principal),
        Some(&recover_old.key),
        Some(&recover_old),
        retire_history,
        retire_operation,
        &retire_fingerprint,
    )
    .await;
    insert_selector(
        &mut retire,
        community_id,
        q_selector,
        4,
        54,
        1,
        Some(&recover_old.principal),
        Some(&recover_old.key),
        Some(&recover_old),
        retire_history,
        retire_operation,
        &retire_fingerprint,
    )
    .await;
    (&mut *retire)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("retire exact composites");
    retire.commit().await.expect("commit retire");

    let recover_operation = Uuid::new_v4();
    let recover_history = Uuid::new_v4();
    let recover_fingerprint = vec![55_u8; 32];
    let mut recover = pool.begin().await.expect("begin recover");
    insert_receipt(
        &mut *recover,
        community_id,
        recover_operation,
        &recover_fingerprint,
        7,
        1,
    )
    .await;
    let recovered_id = Uuid::new_v4();
    let recovered_version = insert_binding(
        &mut recover,
        community_id,
        recovered_id,
        &recover_old.subject,
        &recover_old.principal,
        &[62_u8; 32],
        recover_history,
        recover_operation,
        &recover_fingerprint,
    )
    .await;
    let recovered = BindingFixture {
        id: recovered_id,
        version: recovered_version,
        principal: recover_old.principal.clone(),
        key: vec![62_u8; 32],
        subject: recover_old.subject.clone(),
    };
    insert_history(
        &mut recover,
        community_id,
        recover_history,
        7,
        1,
        Some(&recover_old),
        Some(2),
        Some(2),
        Some(2),
        Some(2),
        Some(&recovered),
        recover_operation,
        &recover_fingerprint,
    )
    .await;
    insert_consumption(
        &mut recover,
        community_id,
        q_selector,
        4,
        recover_history,
        recover_operation,
        &recover_fingerprint,
        &recovered,
    )
    .await;
    (&mut *recover)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("recover exact composites");
    recover.commit().await.expect("commit recover");

    // Disable creates P/X/Q. Enable consumes X and Q into the same successor;
    // there is deliberately no successor uniqueness on consumptions.
    let enable_old = seed_binding(&pool, community_id, "enable", 71, 81).await;
    let disable_operation = Uuid::new_v4();
    let disable_history = Uuid::new_v4();
    let disable_fingerprint = vec![72_u8; 32];
    let x_selector = Uuid::new_v4();
    let enable_q_selector = Uuid::new_v4();
    let mut disable = pool.begin().await.expect("begin disable");
    insert_receipt(
        &mut *disable,
        community_id,
        disable_operation,
        &disable_fingerprint,
        4,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(disable_history)
    .bind(community_id)
    .bind(enable_old.id)
    .execute(&mut *disable)
    .await
    .expect("retire disabled binding");
    insert_history(
        &mut disable,
        community_id,
        disable_history,
        4,
        1,
        Some(&enable_old),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        disable_operation,
        &disable_fingerprint,
    )
    .await;
    for (kind, selector_id, fingerprint_byte, binding) in [
        (1_i16, Uuid::new_v4(), 73_u8, Some(&enable_old)),
        (2_i16, x_selector, 74_u8, None),
        (4_i16, enable_q_selector, 75_u8, Some(&enable_old)),
    ] {
        insert_selector(
            &mut disable,
            community_id,
            selector_id,
            kind,
            fingerprint_byte,
            1,
            Some(&enable_old.principal),
            (kind != 2).then_some(enable_old.key.as_slice()),
            binding,
            disable_history,
            disable_operation,
            &disable_fingerprint,
        )
        .await;
    }
    (&mut *disable)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("disable exact composites");
    disable.commit().await.expect("commit disable");

    let enable_operation = Uuid::new_v4();
    let enable_history = Uuid::new_v4();
    let enable_fingerprint = vec![76_u8; 32];
    let mut enable = pool.begin().await.expect("begin enable");
    insert_receipt(
        &mut *enable,
        community_id,
        enable_operation,
        &enable_fingerprint,
        8,
        1,
    )
    .await;
    let enabled_id = Uuid::new_v4();
    let enabled_version = insert_binding(
        &mut enable,
        community_id,
        enabled_id,
        &enable_old.subject,
        &enable_old.principal,
        &[82_u8; 32],
        enable_history,
        enable_operation,
        &enable_fingerprint,
    )
    .await;
    let enabled = BindingFixture {
        id: enabled_id,
        version: enabled_version,
        principal: enable_old.principal.clone(),
        key: vec![82_u8; 32],
        subject: enable_old.subject.clone(),
    };
    insert_history(
        &mut enable,
        community_id,
        enable_history,
        8,
        1,
        Some(&enable_old),
        Some(2),
        Some(2),
        Some(2),
        Some(2),
        Some(&enabled),
        enable_operation,
        &enable_fingerprint,
    )
    .await;
    insert_consumption(
        &mut enable,
        community_id,
        x_selector,
        2,
        enable_history,
        enable_operation,
        &enable_fingerprint,
        &enabled,
    )
    .await;
    insert_consumption(
        &mut enable,
        community_id,
        enable_q_selector,
        4,
        enable_history,
        enable_operation,
        &enable_fingerprint,
        &enabled,
    )
    .await;
    (&mut *enable)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("one Enable successor consumes exact X and Q");
    enable.commit().await.expect("commit enable");
    let enabled_consumptions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_lifecycle_selector_consumptions \
         WHERE community_id=$1 AND successor_binding_id=$2 AND successor_binding_version=$3",
    )
    .bind(community_id)
    .bind(enabled.id)
    .bind(enabled.version)
    .fetch_one(&pool)
    .await
    .expect("count Enable consumptions");
    assert_eq!(enabled_consumptions, 2);

    let mut late_extra = pool.begin().await.expect("begin late selector append");
    sqlx::query(
        "INSERT INTO identity_lifecycle_selectors \
         (community_id, selector_id, selector_kind, selector_fingerprint, fact_generation, \
          principal_fingerprint, asserted_history_id, selected_by_operation_id, \
          selected_by_request_fingerprint) \
         VALUES ($1, $2, 2, $3, 2, $4, $5, $6, $7)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![77_u8; 32])
    .bind(&enabled.principal)
    .bind(disable_history)
    .bind(disable_operation)
    .bind(&disable_fingerprint)
    .execute(&mut *late_extra)
    .await
    .expect("late X is structurally valid before deferred integrity check");
    let late_extra_error = (&mut *late_extra)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("late selector append must recount the old transition");
    assert_eq!(
        constraint(&late_extra_error),
        Some("identity_lifecycle_transition_integrity")
    );
    late_extra.rollback().await.expect("rollback late selector");

    // After X(g1) was consumed, a later disable may assert X(g2), while a
    // second open X is rejected under the same principal-scope lock.
    let redisable_operation = Uuid::new_v4();
    let redisable_history = Uuid::new_v4();
    let redisable_fingerprint = vec![77_u8; 32];
    let x_generation_two = Uuid::new_v4();
    let mut redisable = pool.begin().await.expect("begin second disable");
    insert_receipt(
        &mut *redisable,
        community_id,
        redisable_operation,
        &redisable_fingerprint,
        4,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(redisable_history)
    .bind(community_id)
    .bind(enabled.id)
    .execute(&mut *redisable)
    .await
    .expect("retire re-disabled generation");
    insert_history(
        &mut redisable,
        community_id,
        redisable_history,
        4,
        1,
        Some(&enabled),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        redisable_operation,
        &redisable_fingerprint,
    )
    .await;
    insert_selector(
        &mut redisable,
        community_id,
        Uuid::new_v4(),
        1,
        78,
        1,
        Some(&enabled.principal),
        Some(&enabled.key),
        Some(&enabled),
        redisable_history,
        redisable_operation,
        &redisable_fingerprint,
    )
    .await;
    insert_selector(
        &mut redisable,
        community_id,
        x_generation_two,
        2,
        79,
        2,
        Some(&enabled.principal),
        None,
        None,
        redisable_history,
        redisable_operation,
        &redisable_fingerprint,
    )
    .await;
    insert_selector(
        &mut redisable,
        community_id,
        Uuid::new_v4(),
        4,
        80,
        2,
        Some(&enabled.principal),
        Some(&enabled.key),
        Some(&enabled),
        redisable_history,
        redisable_operation,
        &redisable_fingerprint,
    )
    .await;
    (&mut *redisable)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("later monotonic X fact");
    redisable.commit().await.expect("commit second disable");
    let open_x_conflict = sqlx::query(
        "INSERT INTO identity_lifecycle_selectors \
         (community_id, selector_id, selector_kind, selector_fingerprint, fact_generation, \
          principal_fingerprint, asserted_history_id, selected_by_operation_id, \
          selected_by_request_fingerprint) \
         VALUES ($1, $2, 2, $3, 3, $4, $5, $6, $7)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![81_u8; 32])
    .bind(&enabled.principal)
    .bind(redisable_history)
    .bind(redisable_operation)
    .bind(&redisable_fingerprint)
    .execute(&pool)
    .await
    .expect_err("a second open X fact must fail");
    assert_eq!(
        constraint(&open_x_conflict),
        Some("identity_lifecycle_selector_one_open_fact")
    );

    // Inactive Disable can assert X without a binding. Enable without Q births
    // one generation and consumes only that exact X.
    let never_principal = vec![91_u8; 32];
    let inactive_disable_operation = Uuid::new_v4();
    let inactive_disable_history = Uuid::new_v4();
    let inactive_disable_fingerprint = vec![92_u8; 32];
    let inactive_x = Uuid::new_v4();
    let mut inactive_disable = pool.begin().await.expect("begin inactive disable");
    insert_receipt(
        &mut *inactive_disable,
        community_id,
        inactive_disable_operation,
        &inactive_disable_fingerprint,
        4,
        1,
    )
    .await;
    insert_history(
        &mut inactive_disable,
        community_id,
        inactive_disable_history,
        4,
        1,
        None,
        None,
        None,
        None,
        None,
        None,
        inactive_disable_operation,
        &inactive_disable_fingerprint,
    )
    .await;
    insert_selector(
        &mut inactive_disable,
        community_id,
        inactive_x,
        2,
        93,
        1,
        Some(&never_principal),
        None,
        None,
        inactive_disable_history,
        inactive_disable_operation,
        &inactive_disable_fingerprint,
    )
    .await;
    (&mut *inactive_disable)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("inactive disable exact history");
    inactive_disable
        .commit()
        .await
        .expect("commit inactive disable");

    let blocked_birth_operation = Uuid::new_v4();
    let blocked_birth_history = Uuid::new_v4();
    let blocked_birth_fingerprint = vec![96_u8; 32];
    let mut blocked_birth = pool.begin().await.expect("begin blocked birth");
    insert_receipt(
        &mut *blocked_birth,
        community_id,
        blocked_birth_operation,
        &blocked_birth_fingerprint,
        1,
        1,
    )
    .await;
    let blocked_binding_id = Uuid::new_v4();
    let blocked_binding_version = insert_binding(
        &mut blocked_birth,
        community_id,
        blocked_binding_id,
        "blocked-by-open-x",
        &never_principal,
        &[96_u8; 32],
        blocked_birth_history,
        blocked_birth_operation,
        &blocked_birth_fingerprint,
    )
    .await;
    let blocked_binding = BindingFixture {
        id: blocked_binding_id,
        version: blocked_binding_version,
        principal: never_principal.clone(),
        key: vec![96_u8; 32],
        subject: "blocked-by-open-x".to_owned(),
    };
    insert_history(
        &mut blocked_birth,
        community_id,
        blocked_birth_history,
        1,
        1,
        None,
        None,
        None,
        None,
        None,
        Some(&blocked_binding),
        blocked_birth_operation,
        &blocked_birth_fingerprint,
    )
    .await;
    let blocked_birth_error = (&mut *blocked_birth)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("effective X rejects an unchecked binding birth");
    assert_eq!(
        constraint(&blocked_birth_error),
        Some("identity_bindings_birth_eligibility")
    );
    blocked_birth
        .rollback()
        .await
        .expect("rollback blocked birth");

    let enable_without_q_operation = Uuid::new_v4();
    let enable_without_q_history = Uuid::new_v4();
    let enable_without_q_fingerprint = vec![94_u8; 32];
    let mut enable_without_q = pool.begin().await.expect("begin Enable without Q");
    insert_receipt(
        &mut *enable_without_q,
        community_id,
        enable_without_q_operation,
        &enable_without_q_fingerprint,
        8,
        1,
    )
    .await;
    let never_binding_id = Uuid::new_v4();
    let never_binding_version = insert_binding(
        &mut enable_without_q,
        community_id,
        never_binding_id,
        "never-enrolled",
        &never_principal,
        &[95_u8; 32],
        enable_without_q_history,
        enable_without_q_operation,
        &enable_without_q_fingerprint,
    )
    .await;
    let never_binding = BindingFixture {
        id: never_binding_id,
        version: never_binding_version,
        principal: never_principal,
        key: vec![95_u8; 32],
        subject: "never-enrolled".to_owned(),
    };
    insert_history(
        &mut enable_without_q,
        community_id,
        enable_without_q_history,
        8,
        1,
        None,
        None,
        None,
        None,
        None,
        Some(&never_binding),
        enable_without_q_operation,
        &enable_without_q_fingerprint,
    )
    .await;
    insert_consumption(
        &mut enable_without_q,
        community_id,
        inactive_x,
        2,
        enable_without_q_history,
        enable_without_q_operation,
        &enable_without_q_fingerprint,
        &never_binding,
    )
    .await;
    (&mut *enable_without_q)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("Enable without Q exact composites");
    enable_without_q
        .commit()
        .await
        .expect("commit Enable without Q");

    // Deferred receipt/history cardinality distinguishes persisted outcomes.
    for outcome_code in [1_i16, 3_i16] {
        let operation_id = Uuid::new_v4();
        let fingerprint = vec![100_u8.wrapping_add(outcome_code as u8); 32];
        let mut missing_history = pool.begin().await.expect("begin missing history");
        insert_receipt(
            &mut *missing_history,
            community_id,
            operation_id,
            &fingerprint,
            3,
            outcome_code,
        )
        .await;
        insert_lifecycle_event(
            &mut missing_history,
            community_id,
            operation_id,
            &fingerprint,
            outcome_code,
            105_u8.wrapping_add(outcome_code as u8),
        )
        .await;
        let error = (&mut *missing_history)
            .execute("SET CONSTRAINTS ALL IMMEDIATE")
            .await
            .expect_err("Applied/NoOp lifecycle receipt requires one history row");
        assert_eq!(
            constraint(&error),
            Some("authorization_operation_receipt_history_cardinality")
        );
        missing_history
            .rollback()
            .await
            .expect("rollback missing history");
    }

    let denied_operation = Uuid::new_v4();
    let denied_fingerprint = vec![103_u8; 32];
    let mut denied = pool.begin().await.expect("begin authenticated denial");
    insert_receipt(
        &mut *denied,
        community_id,
        denied_operation,
        &denied_fingerprint,
        3,
        2,
    )
    .await;
    insert_lifecycle_event(
        &mut denied,
        community_id,
        denied_operation,
        &denied_fingerprint,
        2,
        109,
    )
    .await;
    (&mut *denied)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("denied lifecycle receipt has zero history");
    denied.commit().await.expect("commit authenticated denial");

    let noop_operation = Uuid::new_v4();
    let noop_history = Uuid::new_v4();
    let noop_fingerprint = vec![104_u8; 32];
    let mut noop = pool.begin().await.expect("begin no-op");
    insert_receipt(
        &mut *noop,
        community_id,
        noop_operation,
        &noop_fingerprint,
        3,
        3,
    )
    .await;
    insert_history(
        &mut noop,
        community_id,
        noop_history,
        3,
        3,
        None,
        None,
        None,
        None,
        None,
        None,
        noop_operation,
        &noop_fingerprint,
    )
    .await;
    (&mut *noop)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("no-op has exactly one immutable history row");
    noop.commit().await.expect("commit no-op");

    // A transition that mutates a binding but omits its exact history fails at
    // the deferred boundary and rolls every tentative row back.
    let rollback_binding = seed_binding(&pool, community_id, "rollback", 111, 121).await;
    let rollback_operation = Uuid::new_v4();
    let rollback_history = Uuid::new_v4();
    let rollback_fingerprint = vec![112_u8; 32];
    let mut rollback = pool.begin().await.expect("begin rollback proof");
    insert_receipt(
        &mut *rollback,
        community_id,
        rollback_operation,
        &rollback_fingerprint,
        3,
        1,
    )
    .await;
    insert_lifecycle_event(
        &mut rollback,
        community_id,
        rollback_operation,
        &rollback_fingerprint,
        1,
        113,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(rollback_history)
    .bind(community_id)
    .bind(rollback_binding.id)
    .execute(&mut *rollback)
    .await
    .expect("tentatively retire rollback binding");
    let rollback_error = (&mut *rollback)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("missing history fails closed");
    assert!(
        matches!(
            constraint(&rollback_error),
            Some(
                "authorization_operation_receipt_history_cardinality"
                    | "identity_bindings_exact_retirement_history_fk"
            )
        ),
        "unexpected deferred constraint: {rollback_error}"
    );
    rollback
        .rollback()
        .await
        .expect("rollback failed transition");
    let rollback_state: (i16, i64, Option<Uuid>) = sqlx::query_as(
        "SELECT binding_state, lifecycle_revision, retirement_history_id \
         FROM identity_bindings WHERE community_id=$1 AND binding_id=$2",
    )
    .bind(community_id)
    .bind(rollback_binding.id)
    .fetch_one(&pool)
    .await
    .expect("read binding after rollback");
    assert_eq!(rollback_state, (1, 1, None));
    let rolled_back_receipt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(rollback_operation)
    .fetch_one(&pool)
    .await
    .expect("count rolled-back receipt");
    assert_eq!(rolled_back_receipt, 0);

    let incomplete_operation = Uuid::new_v4();
    let incomplete_history = Uuid::new_v4();
    let incomplete_fingerprint = vec![113_u8; 32];
    let mut incomplete = pool.begin().await.expect("begin incomplete retire");
    insert_receipt(
        &mut *incomplete,
        community_id,
        incomplete_operation,
        &incomplete_fingerprint,
        3,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(incomplete_history)
    .bind(community_id)
    .bind(rollback_binding.id)
    .execute(&mut *incomplete)
    .await
    .expect("tentatively retire without companions");
    insert_history(
        &mut incomplete,
        community_id,
        incomplete_history,
        3,
        1,
        Some(&rollback_binding),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        incomplete_operation,
        &incomplete_fingerprint,
    )
    .await;
    let incomplete_error = (&mut *incomplete)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("Retire without exact P and Q companions fails closed");
    assert_eq!(
        constraint(&incomplete_error),
        Some("identity_lifecycle_transition_integrity")
    );
    incomplete
        .rollback()
        .await
        .expect("rollback incomplete retire");

    let active_old_operation = Uuid::new_v4();
    let active_old_history = Uuid::new_v4();
    let active_old_fingerprint = vec![114_u8; 32];
    let mut active_old = pool.begin().await.expect("begin active-old proof");
    insert_receipt(
        &mut *active_old,
        community_id,
        active_old_operation,
        &active_old_fingerprint,
        3,
        1,
    )
    .await;
    insert_history(
        &mut active_old,
        community_id,
        active_old_history,
        3,
        1,
        Some(&rollback_binding),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        active_old_operation,
        &active_old_fingerprint,
    )
    .await;
    insert_selector(
        &mut active_old,
        community_id,
        Uuid::new_v4(),
        1,
        115,
        1,
        Some(&rollback_binding.principal),
        Some(&rollback_binding.key),
        Some(&rollback_binding),
        active_old_history,
        active_old_operation,
        &active_old_fingerprint,
    )
    .await;
    insert_selector(
        &mut active_old,
        community_id,
        Uuid::new_v4(),
        4,
        116,
        1,
        Some(&rollback_binding.principal),
        Some(&rollback_binding.key),
        Some(&rollback_binding),
        active_old_history,
        active_old_operation,
        &active_old_fingerprint,
    )
    .await;
    let active_old_error = (&mut *active_old)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("history cannot claim an Active old binding was retired");
    assert_eq!(
        constraint(&active_old_error),
        Some("identity_lifecycle_transition_integrity")
    );
    active_old
        .rollback()
        .await
        .expect("rollback active-old proof");

    // AdmissionLoss is exact-target retirement only. Principal-wide
    // disablement is the canonical Disable transition, so an AdmissionLoss
    // transition may never assert X even when its required P and Q are exact.
    let admission_binding = seed_binding(&pool, community_id, "admission-loss", 117, 118).await;
    let admission_operation = Uuid::new_v4();
    let admission_history = Uuid::new_v4();
    let admission_fingerprint = vec![119_u8; 32];
    let mut admission_x = pool.begin().await.expect("begin AdmissionLoss X proof");
    insert_receipt(
        &mut *admission_x,
        community_id,
        admission_operation,
        &admission_fingerprint,
        9,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(admission_history)
    .bind(community_id)
    .bind(admission_binding.id)
    .execute(&mut *admission_x)
    .await
    .expect("tentatively retire exact AdmissionLoss target");
    insert_history(
        &mut admission_x,
        community_id,
        admission_history,
        9,
        1,
        Some(&admission_binding),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        admission_operation,
        &admission_fingerprint,
    )
    .await;
    for (kind, fingerprint_byte, key, binding) in [
        (
            1_i16,
            120_u8,
            Some(admission_binding.key.as_slice()),
            Some(&admission_binding),
        ),
        (
            4_i16,
            121_u8,
            Some(admission_binding.key.as_slice()),
            Some(&admission_binding),
        ),
        (2_i16, 122_u8, None, None),
    ] {
        insert_selector(
            &mut admission_x,
            community_id,
            Uuid::new_v4(),
            kind,
            fingerprint_byte,
            1,
            Some(&admission_binding.principal),
            key,
            binding,
            admission_history,
            admission_operation,
            &admission_fingerprint,
        )
        .await;
    }
    let admission_x_error = (&mut *admission_x)
        .execute("SET CONSTRAINTS identity_lifecycle_selector_history_semantics IMMEDIATE")
        .await
        .expect_err("AdmissionLoss must reject principal-wide X assertion");
    assert_eq!(
        constraint(&admission_x_error),
        Some("identity_lifecycle_selector_history_semantics")
    );
    admission_x
        .rollback()
        .await
        .expect("rollback AdmissionLoss X proof");

    let admission_exact_operation = Uuid::new_v4();
    let admission_exact_history = Uuid::new_v4();
    let admission_exact_fingerprint = vec![123_u8; 32];
    let mut admission_exact = pool.begin().await.expect("begin exact AdmissionLoss");
    insert_receipt(
        &mut *admission_exact,
        community_id,
        admission_exact_operation,
        &admission_exact_fingerprint,
        9,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(admission_exact_history)
    .bind(community_id)
    .bind(admission_binding.id)
    .execute(&mut *admission_exact)
    .await
    .expect("retire exact AdmissionLoss target");
    insert_history(
        &mut admission_exact,
        community_id,
        admission_exact_history,
        9,
        1,
        Some(&admission_binding),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        admission_exact_operation,
        &admission_exact_fingerprint,
    )
    .await;
    for (kind, fingerprint_byte) in [(1_i16, 124_u8), (4_i16, 125_u8)] {
        insert_selector(
            &mut admission_exact,
            community_id,
            Uuid::new_v4(),
            kind,
            fingerprint_byte,
            1,
            Some(&admission_binding.principal),
            Some(&admission_binding.key),
            Some(&admission_binding),
            admission_exact_history,
            admission_exact_operation,
            &admission_exact_fingerprint,
        )
        .await;
    }
    (&mut *admission_exact)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("AdmissionLoss commits with exact P and Q only");
    admission_exact
        .commit()
        .await
        .expect("commit exact AdmissionLoss");

    // The generation identifier and all birth bounds remain immutable.
    let version_rewrite = sqlx::query(
        "UPDATE identity_bindings SET binding_version=binding_version+1 \
         WHERE community_id=$1 AND binding_id=$2",
    )
    .bind(community_id)
    .bind(rotate_successor.id)
    .execute(&pool)
    .await
    .expect_err("binding_version is immutable per generation");
    assert_eq!(
        version_rewrite
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("428C9"),
        "PostgreSQL GENERATED ALWAYS rejects a binding-version rewrite before row triggers run"
    );
    let expiry_rewrite = sqlx::query(
        "UPDATE identity_bindings SET expires_at=transaction_timestamp()+INTERVAL '1 hour' \
         WHERE community_id=$1 AND binding_id=$2",
    )
    .bind(community_id)
    .bind(rotate_successor.id)
    .execute(&pool)
    .await
    .expect_err("binding expiry is an immutable birth bound");
    assert_eq!(
        constraint(&expiry_rewrite),
        Some("identity_bindings_immutable_generation")
    );

    // Competing recoveries serialize on the exact open Q. The winner commits
    // one receipt/history/successor/consumption; every tentative loser row is
    // rolled back after the selector primary key rejects the second consumer.
    let race_old = seed_binding(&pool, community_id, "race-old", 131, 141).await;
    let race_retire_operation = Uuid::new_v4();
    let race_retire_history = Uuid::new_v4();
    let race_retire_fingerprint = vec![132_u8; 32];
    let race_q = Uuid::new_v4();
    let mut race_retire = pool.begin().await.expect("begin race retire");
    insert_receipt(
        &mut *race_retire,
        community_id,
        race_retire_operation,
        &race_retire_fingerprint,
        3,
        1,
    )
    .await;
    sqlx::query(
        "UPDATE identity_bindings SET binding_state=2, lifecycle_revision=2, \
         retirement_history_id=$1 WHERE community_id=$2 AND binding_id=$3",
    )
    .bind(race_retire_history)
    .bind(community_id)
    .bind(race_old.id)
    .execute(&mut *race_retire)
    .await
    .expect("retire race predecessor");
    insert_history(
        &mut race_retire,
        community_id,
        race_retire_history,
        3,
        1,
        Some(&race_old),
        Some(1),
        Some(1),
        Some(2),
        Some(2),
        None,
        race_retire_operation,
        &race_retire_fingerprint,
    )
    .await;
    insert_selector(
        &mut race_retire,
        community_id,
        Uuid::new_v4(),
        1,
        133,
        1,
        Some(&race_old.principal),
        Some(&race_old.key),
        Some(&race_old),
        race_retire_history,
        race_retire_operation,
        &race_retire_fingerprint,
    )
    .await;
    insert_selector(
        &mut race_retire,
        community_id,
        race_q,
        4,
        134,
        1,
        Some(&race_old.principal),
        Some(&race_old.key),
        Some(&race_old),
        race_retire_history,
        race_retire_operation,
        &race_retire_fingerprint,
    )
    .await;
    (&mut *race_retire)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("race retirement exact companions");
    race_retire.commit().await.expect("commit race retirement");

    let winner_operation = Uuid::new_v4();
    let winner_history = Uuid::new_v4();
    let winner_fingerprint = vec![135_u8; 32];
    let winner_event_author_pubkey = Keys::generate().public_key();
    let winner_key = winner_event_author_pubkey.to_bytes().to_vec();
    let mut winner = pool.begin().await.expect("begin recovery winner");
    let winner_locks = IdentityLifecycleLockCoordinates::new(
        CommunityId::from_uuid(community_id),
        Some(race_principal(&race_old.principal)),
        None,
        Some(winner_event_author_pubkey),
    )
    .expect("valid recovery winner coordinates");
    lock_identity_lifecycle_transaction(&mut winner, &winner_locks)
        .await
        .expect("pre-lock recovery winner coordinates");
    insert_receipt(
        &mut *winner,
        community_id,
        winner_operation,
        &winner_fingerprint,
        7,
        1,
    )
    .await;
    let winner_id = Uuid::new_v4();
    let winner_version = insert_binding(
        &mut winner,
        community_id,
        winner_id,
        "race-winner",
        &race_old.principal,
        &winner_key,
        winner_history,
        winner_operation,
        &winner_fingerprint,
    )
    .await;
    let winner_binding = BindingFixture {
        id: winner_id,
        version: winner_version,
        principal: race_old.principal.clone(),
        key: winner_key,
        subject: "race-winner".to_owned(),
    };
    insert_history(
        &mut winner,
        community_id,
        winner_history,
        7,
        1,
        Some(&race_old),
        Some(2),
        Some(2),
        Some(2),
        Some(2),
        Some(&winner_binding),
        winner_operation,
        &winner_fingerprint,
    )
    .await;

    insert_consumption(
        &mut winner,
        community_id,
        race_q,
        4,
        winner_history,
        winner_operation,
        &winner_fingerprint,
        &winner_binding,
    )
    .await;

    let loser_operation = Uuid::new_v4();
    let loser_history = Uuid::new_v4();
    let loser_fingerprint = vec![136_u8; 32];
    let loser_id = Uuid::new_v4();
    let loser_event_author_pubkey = Keys::generate().public_key();
    let loser_key = loser_event_author_pubkey.to_bytes().to_vec();
    let loser_principal = race_old.principal.clone();
    let loser_old = race_old.clone();
    let loser_pool = pool.clone();
    let (loser_ready_tx, loser_ready_rx) = tokio::sync::oneshot::channel();
    let loser_task = tokio::spawn(async move {
        let mut loser = loser_pool.begin().await.expect("begin recovery loser");
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *loser)
            .await
            .expect("read recovery loser backend pid");
        loser_ready_tx
            .send(backend_pid)
            .expect("publish recovery loser backend pid");
        let loser_locks = IdentityLifecycleLockCoordinates::new(
            CommunityId::from_uuid(community_id),
            Some(race_principal(&loser_principal)),
            None,
            Some(loser_event_author_pubkey),
        )
        .expect("valid recovery loser coordinates");
        lock_identity_lifecycle_transaction(&mut loser, &loser_locks)
            .await
            .expect("resume recovery loser after winner commits");
        insert_receipt(
            &mut *loser,
            community_id,
            loser_operation,
            &loser_fingerprint,
            7,
            1,
        )
        .await;
        let loser_version = insert_binding(
            &mut loser,
            community_id,
            loser_id,
            "race-loser",
            &loser_principal,
            &loser_key,
            loser_history,
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        let loser_binding = BindingFixture {
            id: loser_id,
            version: loser_version,
            principal: loser_principal,
            key: loser_key,
            subject: "race-loser".to_owned(),
        };
        insert_history(
            &mut loser,
            community_id,
            loser_history,
            7,
            1,
            Some(&loser_old),
            Some(2),
            Some(2),
            Some(2),
            Some(2),
            Some(&loser_binding),
            loser_operation,
            &loser_fingerprint,
        )
        .await;
        let error = sqlx::query(
            "INSERT INTO identity_lifecycle_selector_consumptions \
             (community_id, selector_id, selector_kind, history_id, operation_id, \
              request_fingerprint, successor_binding_id, successor_binding_version) \
             VALUES ($1, $2, 4, $3, $4, $5, $6, $7)",
        )
        .bind(community_id)
        .bind(race_q)
        .bind(loser_history)
        .bind(loser_operation)
        .bind(&loser_fingerprint)
        .bind(loser_binding.id)
        .bind(loser_binding.version)
        .execute(&mut *loser)
        .await
        .expect_err("second Q consumer loses after serialized resume");
        let failed_constraint = constraint(&error).map(str::to_owned);
        loser
            .rollback()
            .await
            .expect("rollback losing serialized recovery");
        RaceLoser {
            operation_id: loser_operation,
            constraint: failed_constraint,
        }
    });
    let loser_backend_pid = tokio::time::timeout(Duration::from_secs(2), loser_ready_rx)
        .await
        .expect("recovery loser must start")
        .expect("recovery loser must publish backend pid");
    wait_for_advisory_lock(&pool, loser_backend_pid).await;
    (&mut *winner)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("winner recovery exact companions");
    winner.commit().await.expect("commit recovery winner");
    let loser = tokio::time::timeout(Duration::from_secs(5), loser_task)
        .await
        .expect("loser completes after winner commits")
        .expect("loser task joins");
    assert_eq!(
        loser.constraint.as_deref(),
        Some("identity_lifecycle_selector_consumptions_pkey")
    );
    let loser_residue: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2), \
           (SELECT count(*) FROM identity_lifecycle_history \
             WHERE community_id=$1 AND operation_id=$2), \
           (SELECT count(*) FROM identity_bindings \
             WHERE community_id=$1 AND binding_id=$3)",
    )
    .bind(community_id)
    .bind(loser_operation)
    .bind(loser_id)
    .fetch_one(&pool)
    .await
    .expect("read losing recovery residue");
    assert_eq!(loser_residue, (0, 0, 0));

    pool.close().await;
}
