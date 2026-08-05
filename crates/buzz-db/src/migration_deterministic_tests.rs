use super::MIGRATOR;

use crate::identity_binding::{
    get_active_identity_binding_by_pubkey, resolve_identity_binding, BindingDenial,
    BindingProvenance, EnrollmentMode, ResolveBindingInput, ResolveBindingResult,
};
use crate::identity_lifecycle::{
    disable_identity_principal, enable_identity_principal, provision_identity_binding,
    recover_identity_binding, retire_identity_pair, revoke_identity_key, rotate_identity_binding,
    IdentityPrincipal, LifecycleContext, LifecycleOperationId, PendingLineage,
    VerifiedReplacementKey,
};
use buzz_core::CommunityId;
use sqlx::{Acquire, PgPool};
use std::collections::BTreeSet;
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| TEST_DB_URL.to_owned())
}

async fn connect_pool() -> PgPool {
    PgPool::connect(&database_url())
        .await
        .expect("connect deterministic migration DB")
}

async fn reset_empty_to_0029(pool: &PgPool, label: &str) -> (Uuid, Uuid) {
    sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(pool)
        .await
        .expect("drop public schema");
    sqlx::query("CREATE SCHEMA public")
        .execute(pool)
        .await
        .expect("create public schema");
    MIGRATOR
        .run_to(29, pool)
        .await
        .expect("migrate through 0029");
    let domain_a = Uuid::new_v4();
    let domain_b = Uuid::new_v4();
    for (domain, suffix) in [(domain_a, "a"), (domain_b, "b")] {
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain)
            .bind(format!("{label}-{suffix}-{}.example", domain.simple()))
            .execute(pool)
            .await
            .expect("insert deterministic migration domain");
    }
    sqlx::query(
        "INSERT INTO identity_bindings (community_id,issuer,uid,pubkey,source) \
         VALUES ($1,'https://domain-b.example','domain-b-sentinel',$2,'jwt_npub')",
    )
    .bind(domain_b)
    .bind(vec![72_u8; 32])
    .execute(pool)
    .await
    .expect("insert domain-B migration sentinel");
    (domain_a, domain_b)
}

async fn reset_to_0029(pool: &PgPool) -> (Uuid, Uuid) {
    let (domain_a, domain_b) = reset_empty_to_0029(pool, "migration-fault").await;
    sqlx::query(
        "INSERT INTO identity_bindings (community_id,issuer,uid,pubkey,source) \
         VALUES ($1,'https://idp.example','migration-fault-a-subject',$2,'db_binding')",
    )
    .bind(domain_a)
    .bind(vec![71_u8; 32])
    .execute(pool)
    .await
    .expect("insert migration fault binding");
    (domain_a, domain_b)
}

fn split_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut single_quote = false;
    let mut dollar_quote = false;
    let mut line_comment = false;
    while index < bytes.len() {
        if line_comment {
            if bytes[index] == b'\n' {
                line_comment = false;
            }
            index += 1;
            continue;
        }
        if !single_quote
            && !dollar_quote
            && index + 1 < bytes.len()
            && &bytes[index..index + 2] == b"--"
        {
            line_comment = true;
            index += 2;
            continue;
        }
        match bytes[index] {
            b'\'' if !dollar_quote => {
                if single_quote && index + 1 < bytes.len() && bytes[index + 1] == b'\'' {
                    index += 2;
                    continue;
                }
                single_quote = !single_quote;
                index += 1;
            }
            b'$' if !single_quote && index + 1 < bytes.len() && bytes[index + 1] == b'$' => {
                dollar_quote = !dollar_quote;
                index += 2;
            }
            b';' if !single_quote && !dollar_quote => {
                let statement = sql[start..index].trim();
                if !statement.is_empty() {
                    statements.push(statement.to_owned());
                }
                start = index + 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        statements.push(tail.to_owned());
    }
    statements
}

fn migration_0030() -> &'static sqlx::migrate::Migration {
    MIGRATOR
        .iter()
        .find(|migration| migration.version == 30)
        .expect("embedded migration 0030")
}

async fn legacy_snapshot(pool: &PgPool) -> Vec<String> {
    let tables = [
        "communities",
        "identity_bindings",
        "identity_principals",
        "identity_revoked_keys",
        "audit_log",
    ];
    let mut snapshot = Vec::new();
    for table in tables {
        let query = format!("SELECT to_jsonb(t)::text FROM {table} t ORDER BY 1");
        let rows = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(query))
            .fetch_all(pool)
            .await
            .expect("snapshot legacy rows");
        snapshot.extend(rows.into_iter().map(|row| format!("row:{table}:{row}")));
    }
    let catalog = sqlx::query_scalar::<_, String>(
        "SELECT value FROM (\
           SELECT 'column:'||table_name||':'||column_name||':'||data_type||':'||is_nullable||':'||COALESCE(column_default,'') AS value \
           FROM information_schema.columns WHERE table_schema='public' AND table_name LIKE 'identity_%' \
           UNION ALL \
           SELECT 'constraint:'||conrelid::regclass::text||':'||conname||':'||pg_get_constraintdef(oid) \
           FROM pg_constraint WHERE conrelid::regclass::text LIKE 'identity_%' \
           UNION ALL \
           SELECT 'index:'||tablename||':'||indexname||':'||indexdef \
           FROM pg_indexes WHERE schemaname='public' AND tablename LIKE 'identity_%'\
         ) catalog ORDER BY value",
    )
    .fetch_all(pool)
    .await
    .expect("snapshot legacy catalog");
    snapshot.extend(catalog);
    let versions = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("snapshot migration versions");
    snapshot.extend(
        versions
            .into_iter()
            .map(|version| format!("version:{version}")),
    );
    snapshot.sort();
    snapshot
}

async fn identity_catalog_contract(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM (\
           SELECT 'column:'||class.relname||':'||attribute.attname||':'||\
                  format_type(attribute.atttypid,attribute.atttypmod)||':'||\
                  attribute.attnotnull::text||':'||\
                  COALESCE(pg_get_expr(default_value.adbin,default_value.adrelid),'') AS value \
           FROM pg_attribute attribute \
           JOIN pg_class class ON class.oid=attribute.attrelid \
           JOIN pg_namespace namespace ON namespace.oid=class.relnamespace \
           LEFT JOIN pg_attrdef default_value \
             ON default_value.adrelid=attribute.attrelid \
            AND default_value.adnum=attribute.attnum \
           WHERE namespace.nspname='public' AND class.relname LIKE 'identity_%' \
             AND class.relkind='r' AND attribute.attnum>0 AND NOT attribute.attisdropped \
           UNION ALL \
           SELECT 'constraint:'||conrelid::regclass::text||':'||conname||':'||pg_get_constraintdef(oid) \
           FROM pg_constraint WHERE conrelid::regclass::text LIKE 'identity_%' \
           UNION ALL \
           SELECT 'index:'||tablename||':'||indexname||':'||indexdef \
           FROM pg_indexes WHERE schemaname='public' AND tablename LIKE 'identity_%'\
         ) catalog ORDER BY value",
    )
    .fetch_all(pool)
    .await
    .expect("snapshot normalized identity catalog")
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn populated_0030_upgrade_matches_desired_identity_catalog() {
    let pool = connect_pool().await;
    sqlx::raw_sql("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("reset for desired identity catalog");
    sqlx::raw_sql(include_str!("../../../schema/schema.sql"))
        .execute(&pool)
        .await
        .expect("apply desired schema");
    let desired = identity_catalog_contract(&pool).await;

    sqlx::raw_sql("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("reset for populated upgrade");
    MIGRATOR
        .run_to(29, &pool)
        .await
        .expect("apply migrations through 0029");
    let domain = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(domain)
        .bind(format!("catalog-{}.example", domain.simple()))
        .execute(&pool)
        .await
        .expect("insert populated-upgrade domain");
    insert_rotated_legacy_row(&pool, domain, "catalog-principal", &[0xC1; 32], &[0xC2; 32]).await;
    sqlx::query(
        "INSERT INTO identity_bindings (community_id,issuer,uid,pubkey,source) \
         VALUES ($1,'https://idp.example','catalog-principal',$2,'db_binding')",
    )
    .bind(domain)
    .bind([0xC2_u8; 32])
    .execute(&pool)
    .await
    .expect("insert populated-upgrade successor");
    super::run_migrations(&pool)
        .await
        .expect("apply populated 0030 upgrade");
    let upgraded = identity_catalog_contract(&pool).await;
    assert_eq!(upgraded, desired);
}

async fn full_identity_snapshot(pool: &PgPool) -> Vec<String> {
    let tables = [
        "identity_bindings",
        "identity_enrollment_policies",
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
    let mut snapshot = Vec::new();
    for table in tables {
        let query = format!("SELECT to_jsonb(t)::text FROM {table} t ORDER BY 1");
        let rows = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(query))
            .fetch_all(pool)
            .await
            .expect("snapshot migrated identity rows");
        snapshot.extend(rows.into_iter().map(|row| format!("{table}:{row}")));
    }
    let marker: Vec<String> =
        sqlx::query_scalar("SELECT to_jsonb(m)::text FROM _sqlx_migrations m WHERE version=30")
            .fetch_all(pool)
            .await
            .expect("snapshot 0030 marker");
    snapshot.extend(marker.into_iter().map(|row| format!("marker:{row}")));
    snapshot.sort();
    snapshot
}

pub(super) async fn raw_domain_authorized(pool: &PgPool, domain: Uuid, key: &[u8]) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM identity_bindings binding \
         WHERE community_id=$1 AND pubkey=$2 AND revoked_at IS NULL \
           AND COALESCE(to_jsonb(binding)->>'binding_state','active')='active')",
    )
    .bind(domain)
    .bind(key)
    .fetch_one(pool)
    .await
    .expect("read raw legacy authorization sentinel")
}

pub(super) async fn domain_audit_snapshot(pool: &PgPool, domain: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT to_jsonb(row_value)::text FROM audit_log row_value \
         WHERE community_id=$1 ORDER BY 1",
    )
    .bind(domain)
    .fetch_all(pool)
    .await
    .expect("read domain audit sentinel")
}

pub(super) async fn legacy_identity_facts(pool: &PgPool, domain: Uuid) -> Vec<String> {
    let mut facts = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT to_jsonb(binding)-ARRAY[\
            'binding_id','binding_version','binding_state','binding_provenance',\
            'replacement_binding_id','created_by','created_policy_version',\
            'expires_at','creation_attribution_kind','archived_at','archived_by','archived_reason']::text[] \
         FROM identity_bindings binding WHERE community_id=$1",
    )
    .bind(domain)
    .fetch_all(pool)
    .await
    .expect("read legacy binding facts")
    .into_iter()
    .map(|value| format!("binding:{value}"))
    .collect::<Vec<_>>();
    for (table, query) in [
        (
            "principal",
            "SELECT to_jsonb(row_value) FROM identity_principals row_value WHERE community_id=$1",
        ),
        (
            "revoked_key",
            "SELECT to_jsonb(row_value) FROM identity_revoked_keys row_value WHERE community_id=$1",
        ),
    ] {
        let rows = sqlx::query_scalar::<_, serde_json::Value>(query)
            .bind(domain)
            .fetch_all(pool)
            .await
            .expect("read legacy selector facts");
        facts.extend(rows.into_iter().map(|value| format!("{table}:{value}")));
    }
    facts.sort();
    facts
}

pub(super) async fn domain_legacy_history_snapshot(pool: &PgPool, domain: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT jsonb_build_object(\
            'issuer_hex',encode(convert_to(issuer,'UTF8'),'hex'),\
            'subject_hex',encode(convert_to(uid,'UTF8'),'hex'),\
            'pubkey_hex',encode(pubkey,'hex'),\
            'source_hex',encode(convert_to(source,'UTF8'),'hex'),\
            'revoked_at',revoked_at,\
            'revoked_reason_hex',CASE WHEN revoked_reason IS NULL THEN NULL ELSE encode(convert_to(revoked_reason,'UTF8'),'hex') END,\
            'revocation_scope_hex',CASE WHEN revocation_scope IS NULL THEN NULL ELSE encode(convert_to(revocation_scope,'UTF8'),'hex') END,\
            'rotation_completed_at',rotation_completed_at,\
            'rotated_to_pubkey_hex',CASE WHEN rotated_to_pubkey IS NULL THEN NULL ELSE encode(rotated_to_pubkey,'hex') END,\
            'rotation_reason_hex',CASE WHEN rotation_reason IS NULL THEN NULL ELSE encode(convert_to(rotation_reason,'UTF8'),'hex') END\
         )::text \
         FROM identity_bindings WHERE community_id=$1 ORDER BY 1",
    )
    .bind(domain)
    .fetch_all(pool)
    .await
    .expect("read normalized legacy history sentinel")
}

pub(super) async fn domain_binding_history_snapshot(pool: &PgPool, domain: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT to_jsonb(row_value)::text FROM identity_binding_history row_value \
         WHERE community_id=$1 ORDER BY 1",
    )
    .bind(domain)
    .fetch_all(pool)
    .await
    .expect("read domain binding history sentinel")
}

pub(super) async fn domain_identity_snapshot(pool: &PgPool, domain: Uuid) -> Vec<String> {
    let tables = [
        "identity_bindings",
        "identity_enrollment_policies",
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
    let mut snapshot = Vec::new();
    for table in tables {
        let query = format!(
            "SELECT to_jsonb(row_value)::text FROM {table} row_value WHERE community_id=$1 ORDER BY 1"
        );
        let rows = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(query))
            .bind(domain)
            .fetch_all(pool)
            .await
            .expect("read domain identity snapshot");
        snapshot.extend(rows.into_iter().map(|row| format!("{table}:{row}")));
    }
    snapshot
}

async fn assert_response_loss_legacy_sentinels(
    pool: &PgPool,
    domain: Uuid,
    expected_facts: &[String],
    expected_authorized: bool,
    expected_audit: &[String],
    expected_history: &[String],
) {
    assert_eq!(legacy_identity_facts(pool, domain).await, expected_facts);
    assert_eq!(
        raw_domain_authorized(pool, domain, &[72_u8; 32]).await,
        expected_authorized
    );
    assert_eq!(domain_audit_snapshot(pool, domain).await, expected_audit);
    assert_eq!(
        domain_legacy_history_snapshot(pool, domain).await,
        expected_history
    );
}

async fn response_loss_migrated_domain_sentinels(
    pool: &PgPool,
    domain: Uuid,
) -> (Vec<String>, Vec<String>) {
    let marker_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success")
            .fetch_one(pool)
            .await
            .expect("count successful response-loss migration markers");
    assert_eq!(marker_count, 1);

    type MigratedRow = (Uuid, i64, String, String, Vec<u8>, String, String);
    let binding: MigratedRow = sqlx::query_as(
        "SELECT binding_id,binding_version,issuer,uid,pubkey,binding_state,binding_provenance \
         FROM identity_bindings WHERE community_id=$1",
    )
    .bind(domain)
    .fetch_one(pool)
    .await
    .expect("read exact migrated domain-B binding");
    assert_eq!(binding.1, 1);
    assert_eq!(binding.2, "https://domain-b.example");
    assert_eq!(binding.3, "domain-b-sentinel");
    assert_eq!(binding.4, vec![72_u8; 32]);
    assert_eq!(binding.5, "active");
    assert_eq!(binding.6, "attested_key");

    type HistoryRow = (Uuid, i64, String, String, Vec<u8>, String, String, String);
    let history_rows: Vec<HistoryRow> = sqlx::query_as(
        "SELECT binding_id,binding_version,issuer,subject,pubkey,binding_state, \
                binding_provenance,transition_kind \
         FROM identity_binding_history WHERE community_id=$1 \
         ORDER BY binding_version,history_id",
    )
    .bind(domain)
    .fetch_all(pool)
    .await
    .expect("read exact migrated domain-B history");
    assert_eq!(history_rows.len(), 1);
    let history = &history_rows[0];
    assert_eq!(history.0, binding.0);
    assert_eq!(history.1, binding.1);
    assert_eq!(history.2, binding.2);
    assert_eq!(history.3, binding.3);
    assert_eq!(history.4, binding.4);
    assert_eq!(history.5, binding.5);
    assert_eq!(history.6, binding.6);
    assert_eq!(history.7, "legacy_import");

    let authorized =
        get_active_identity_binding_by_pubkey(pool, CommunityId::from_uuid(domain), &[72_u8; 32])
            .await
            .expect("read migrated response-loss authorization")
            .expect("domain-B active binding survives response-loss operation");
    assert_eq!(authorized.binding_id, binding.0);
    assert_eq!(authorized.binding_version, binding.1 as u64);
    assert_eq!(authorized.issuer, binding.2);
    assert_eq!(authorized.uid, binding.3);
    assert_eq!(authorized.pubkey, binding.4);

    (
        domain_identity_snapshot(pool, domain).await,
        domain_binding_history_snapshot(pool, domain).await,
    )
}

async fn insert_rotated_legacy_row(
    pool: &PgPool,
    domain: Uuid,
    subject: &str,
    key: &[u8],
    target: &[u8],
) {
    sqlx::query(
        "INSERT INTO identity_bindings \
         (community_id,issuer,uid,pubkey,source,revoked_at,revoked_reason,revocation_scope,\
          rotation_completed_at,rotated_to_pubkey,rotation_reason) \
         VALUES ($1,'https://idp.example',$2,$3,'db_binding',NOW(),'legacy rotation',\
                 'rotation',NOW(),$4,'legacy rotation')",
    )
    .bind(domain)
    .bind(subject)
    .bind(key)
    .bind(target)
    .execute(pool)
    .await
    .expect("insert rotated legacy row");
}

async fn insert_revoked_legacy_row(pool: &PgPool, domain: Uuid, subject: &str, key: &[u8]) {
    sqlx::query(
        "INSERT INTO identity_bindings \
         (community_id,issuer,uid,pubkey,source,revoked_at,revoked_reason,revocation_scope) \
         VALUES ($1,'https://idp.example',$2,$3,'db_binding',NOW(),'legacy revoke','key')",
    )
    .bind(domain)
    .bind(subject)
    .bind(key)
    .execute(pool)
    .await
    .expect("insert revoked legacy row");
}

fn lifecycle_context(id: u128, reason: &'static str) -> LifecycleContext<'static> {
    const ACTOR: [u8; 32] = [0xA2; 32];
    LifecycleContext {
        operation_id: LifecycleOperationId::from_uuid_for_test(Uuid::from_u128(id)),
        actor: &ACTOR,
        reason,
    }
}

fn replacement(
    key: &'static [u8; 32],
    provenance: BindingProvenance,
) -> VerifiedReplacementKey<'static> {
    VerifiedReplacementKey::after_verified_proof(
        key,
        None,
        provenance,
        "migration-denial-policy-v1",
    )
    .expect("construct migrated denial replacement")
}

const MASK_BINDING: u8 = 0b1_0000;
const MASK_REVOCATION: u8 = 0b0_1000;
const MASK_RETIRED_PAIR: u8 = 0b0_0100;
const MASK_DISABLED_IDENTITY: u8 = 0b0_0010;
const MASK_PENDING_LINEAGE: u8 = 0b0_0001;

const MATRIX_BINDING_KEY: [u8; 32] = [101; 32];
const MATRIX_REVOCATION_KEY: [u8; 32] = [102; 32];
const MATRIX_RETIRED_KEY: [u8; 32] = [103; 32];
const MATRIX_SUCCESSOR_KEY: [u8; 32] = [104; 32];
const MATRIX_PENDING_KEY: [u8; 32] = [105; 32];
const MATRIX_FRESH_KEY: [u8; 32] = [106; 32];

type LiteralBindingRow = (Vec<u8>, Vec<u8>, Vec<u8>, Uuid, i64, String, String);
type MatrixBindingRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, String, String, Option<Uuid>);
type MatrixHistoryRow = (Uuid, Vec<u8>, i64, String, String, Option<Uuid>);

async fn seed_legacy_presence_mask(pool: &PgPool, domain: Uuid, mask: u8) {
    if mask & MASK_BINDING != 0 {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at) \
             VALUES ($1,' Issuer://EXAMPLE/%2f/é ',' Subject/Case/%41/é ',$2,'jwt_npub', \
                     '2026-01-01T03:04:05Z','2026-01-01T03:04:05Z','2026-01-01T03:04:05Z')",
        )
        .bind(domain)
        .bind(MATRIX_BINDING_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix active binding");
    }
    if mask & MASK_REVOCATION != 0 {
        sqlx::query(
            "INSERT INTO identity_revoked_keys \
             (community_id,pubkey,revoked_at,reason) \
             VALUES ($1,$2,'2026-01-02T03:04:05Z','oracle-revoked')",
        )
        .bind(domain)
        .bind(MATRIX_REVOCATION_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix standalone revocation");
    }
    if mask & MASK_RETIRED_PAIR != 0 {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at, \
              revoked_at,revoked_reason,revocation_scope,rotation_completed_at, \
              rotated_to_pubkey,rotation_reason) \
             VALUES ($1,'Retired://Issuer/%2F/ß',' Retired Subject ',$2,'db_binding', \
                     '2026-01-03T01:04:05Z','2026-01-03T03:04:05Z','2026-01-03T03:04:05Z', \
                     '2026-01-03T03:04:05Z','oracle retired pair','rotation', \
                     '2026-01-03T03:04:05Z',$3,'oracle retired pair')",
        )
        .bind(domain)
        .bind(MATRIX_RETIRED_KEY.as_slice())
        .bind(MATRIX_SUCCESSOR_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix retired predecessor");
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at) \
             VALUES ($1,'Retired://Issuer/%2F/ß',' Retired Subject ',$2,'db_binding', \
                     '2026-01-03T04:04:05Z','2026-01-03T04:04:05Z','2026-01-03T04:04:05Z')",
        )
        .bind(domain)
        .bind(MATRIX_SUCCESSOR_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix retired successor");
        sqlx::query(
            "INSERT INTO identity_revoked_keys \
             (community_id,pubkey,revoked_at,reason) \
             VALUES ($1,$2,'2026-01-03T03:04:05Z','oracle retired support')",
        )
        .bind(domain)
        .bind(MATRIX_RETIRED_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix retired tombstone support");
    }
    if mask & MASK_DISABLED_IDENTITY != 0 {
        sqlx::query(
            "INSERT INTO identity_principals \
             (community_id,issuer,uid,disabled_at,disabled_reason) \
             VALUES ($1,'Disabled://Issuer/%2f/é',' Disabled Subject ', \
                     '2026-01-04T03:04:05Z','oracle-disabled')",
        )
        .bind(domain)
        .execute(pool)
        .await
        .expect("seed matrix disabled identity");
    }
    if mask & MASK_PENDING_LINEAGE != 0 {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at, \
              revoked_at,revoked_reason,revocation_scope) \
             VALUES ($1,'Pending://Issuer/%2F/é',' Pending Subject ',$2,'db_binding', \
                     '2026-01-05T01:04:05Z','2026-01-05T03:04:05Z','2026-01-05T03:04:05Z', \
                     '2026-01-05T03:04:05Z','oracle pending lineage','key')",
        )
        .bind(domain)
        .bind(MATRIX_PENDING_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix terminal revoked binding");
        sqlx::query(
            "INSERT INTO identity_revoked_keys \
             (community_id,pubkey,revoked_at,reason) \
             VALUES ($1,$2,'2026-01-05T03:04:05Z','oracle pending support')",
        )
        .bind(domain)
        .bind(MATRIX_PENDING_KEY.as_slice())
        .execute(pool)
        .await
        .expect("seed matrix pending tombstone support");
    }
}

async fn domain_table_count(pool: &PgPool, table: &str, domain: Uuid) -> i64 {
    let query = format!("SELECT COUNT(*) FROM {table} WHERE community_id=$1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .bind(domain)
        .fetch_one(pool)
        .await
        .expect("count domain table rows")
}

async fn resolve_result(
    pool: &PgPool,
    domain: Uuid,
    issuer: &str,
    subject: &str,
    key: &[u8],
) -> ResolveBindingResult {
    resolve_identity_binding(
        pool,
        &ResolveBindingInput {
            authorization_domain: CommunityId::from_uuid(domain),
            issuer,
            subject,
            pubkey: key,
            display_name: None,
            enrollment_mode: EnrollmentMode::AttestedKey,
            key_attested: true,
            policy_version: "migration-test-policy-v1",
            evidence_valid_from: 0,
            evidence_valid_until: i64::MAX as u64,
        },
    )
    .await
    .expect("resolve matrix identity coordinate")
}

async fn legacy_authorization_projection(pool: &PgPool, domain: Uuid) -> Vec<(&'static str, bool)> {
    let mut projection = Vec::new();
    for (name, key) in [
        ("binding", &MATRIX_BINDING_KEY[..]),
        ("revocation", &MATRIX_REVOCATION_KEY[..]),
        ("retired", &MATRIX_RETIRED_KEY[..]),
        ("retired_successor", &MATRIX_SUCCESSOR_KEY[..]),
        ("disabled_reenroll", &MATRIX_FRESH_KEY[..]),
        ("pending_retired", &MATRIX_PENDING_KEY[..]),
        ("pending_reenroll", &MATRIX_FRESH_KEY[..]),
    ] {
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
               SELECT 1 FROM identity_bindings binding \
               WHERE binding.community_id=$1 AND binding.pubkey=$2 \
                 AND COALESCE(to_jsonb(binding)->>'binding_state','active')='active' \
                 AND binding.revoked_at IS NULL \
                 AND NOT EXISTS(\
                   SELECT 1 FROM identity_principals principal \
                   WHERE principal.community_id=binding.community_id \
                     AND principal.issuer=binding.issuer AND principal.uid=binding.uid \
                     AND principal.disabled_at IS NOT NULL\
                 ) \
                 AND NOT EXISTS(\
                   SELECT 1 FROM identity_revoked_keys revoked \
                   WHERE revoked.community_id=binding.community_id \
                     AND revoked.pubkey=binding.pubkey\
                 )\
             )",
        )
        .bind(domain)
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("read legacy authorization projection");
        projection.push((name, authorized));
    }
    projection
}

async fn migrated_authorization_projection(
    pool: &PgPool,
    domain: Uuid,
) -> Vec<(&'static str, bool)> {
    let mut projection = Vec::new();
    for (name, key) in [
        ("binding", &MATRIX_BINDING_KEY[..]),
        ("revocation", &MATRIX_REVOCATION_KEY[..]),
        ("retired", &MATRIX_RETIRED_KEY[..]),
        ("retired_successor", &MATRIX_SUCCESSOR_KEY[..]),
        ("disabled_reenroll", &MATRIX_FRESH_KEY[..]),
        ("pending_retired", &MATRIX_PENDING_KEY[..]),
        ("pending_reenroll", &MATRIX_FRESH_KEY[..]),
    ] {
        let authorized = matches!(
            get_active_identity_binding_by_pubkey(pool, CommunityId::from_uuid(domain), key,).await,
            Ok(Some(_))
        );
        projection.push((name, authorized));
    }
    projection
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_all_32_legacy_presence_masks_are_lossless_and_fail_closed() {
    let mut executed = BTreeSet::new();
    for mask in 0_u8..32 {
        let case_id = format!("MIG-CART-{mask:05b}");
        assert!(executed.insert(case_id.clone()), "duplicate {case_id}");

        let pool = connect_pool().await;
        let (domain_a, domain_b) = reset_empty_to_0029(&pool, "migration-mask").await;
        seed_legacy_presence_mask(&pool, domain_a, mask).await;
        let legacy_a = legacy_identity_facts(&pool, domain_a).await;
        let legacy_b = legacy_identity_facts(&pool, domain_b).await;
        let audit_b = domain_audit_snapshot(&pool, domain_b).await;
        let authorization_a_before = legacy_authorization_projection(&pool, domain_a).await;
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);

        MIGRATOR
            .run_to(30, &pool)
            .await
            .unwrap_or_else(|error| panic!("{case_id} migration failed: {error}"));
        assert_eq!(
            legacy_identity_facts(&pool, domain_a).await,
            legacy_a,
            "{case_id}"
        );
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            legacy_b,
            "{case_id}"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            audit_b,
            "{case_id}"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success",
            )
            .fetch_one(&pool)
            .await
            .expect("count first matrix migration marker"),
            1,
            "{case_id}"
        );
        let domain_b_post = domain_identity_snapshot(&pool, domain_b).await;
        assert_eq!(
            migrated_authorization_projection(&pool, domain_a).await,
            authorization_a_before,
            "{case_id} pre/post authorization decisions"
        );

        let binding_rows = i64::from(mask & MASK_BINDING != 0)
            + 2 * i64::from(mask & MASK_RETIRED_PAIR != 0)
            + i64::from(mask & MASK_PENDING_LINEAGE != 0);
        let tombstone_rows = i64::from(mask & MASK_REVOCATION != 0)
            + i64::from(mask & MASK_RETIRED_PAIR != 0)
            + i64::from(mask & MASK_PENDING_LINEAGE != 0);
        assert_eq!(
            domain_table_count(&pool, "identity_bindings", domain_a).await,
            binding_rows,
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_principals", domain_a).await,
            i64::from(mask & MASK_DISABLED_IDENTITY != 0),
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_revoked_keys", domain_a).await,
            tombstone_rows,
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_binding_lineage", domain_a).await,
            i64::from(mask & MASK_RETIRED_PAIR != 0),
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_retired_pairs", domain_a).await,
            i64::from(mask & MASK_RETIRED_PAIR != 0) + i64::from(mask & MASK_PENDING_LINEAGE != 0),
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_pending_replacements", domain_a).await,
            i64::from(mask & MASK_PENDING_LINEAGE != 0),
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_binding_history", domain_a).await,
            binding_rows,
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_migration_denials", domain_a).await,
            0,
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_migration_denied_keys", domain_a).await,
            0,
            "{case_id}"
        );
        assert_eq!(
            domain_table_count(&pool, "identity_lifecycle_operations", domain_a).await,
            0,
            "{case_id}"
        );

        let binding_coordinate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_bindings \
             WHERE community_id=$1 AND issuer=' Issuer://EXAMPLE/%2f/é ' \
               AND uid=' Subject/Case/%41/é ' AND pubkey=$2",
        )
        .bind(domain_a)
        .bind(MATRIX_BINDING_KEY.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count exact matrix binding coordinate");
        assert_eq!(
            binding_coordinate_count,
            i64::from(mask & MASK_BINDING != 0),
            "{case_id}"
        );
        let revocation_coordinate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(domain_a)
        .bind(MATRIX_REVOCATION_KEY.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count exact matrix revocation coordinate");
        assert_eq!(
            revocation_coordinate_count,
            i64::from(mask & MASK_REVOCATION != 0),
            "{case_id}"
        );
        let retired_coordinate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_bindings \
             WHERE community_id=$1 AND issuer='Retired://Issuer/%2F/ß' \
               AND uid=' Retired Subject '",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("count exact matrix retired coordinate");
        assert_eq!(
            retired_coordinate_count,
            2 * i64::from(mask & MASK_RETIRED_PAIR != 0),
            "{case_id}"
        );
        let disabled_coordinate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_principals \
             WHERE community_id=$1 AND issuer='Disabled://Issuer/%2f/é' \
               AND uid=' Disabled Subject '",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("count exact matrix disabled coordinate");
        assert_eq!(
            disabled_coordinate_count,
            i64::from(mask & MASK_DISABLED_IDENTITY != 0),
            "{case_id}"
        );
        let pending_coordinate_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_bindings \
             WHERE community_id=$1 AND issuer='Pending://Issuer/%2F/é' \
               AND uid=' Pending Subject ' AND pubkey=$2",
        )
        .bind(domain_a)
        .bind(MATRIX_PENDING_KEY.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count exact matrix pending coordinate");
        assert_eq!(
            pending_coordinate_count,
            i64::from(mask & MASK_PENDING_LINEAGE != 0),
            "{case_id}"
        );

        let ids_are_valid: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM identity_bindings \
             WHERE community_id=$1 AND (binding_id='00000000-0000-0000-0000-000000000000' OR binding_version < 1))",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("validate matrix binding coordinates");
        assert!(ids_are_valid, "{case_id}");
        let exact_history_mirrors: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_binding_history history \
             JOIN identity_bindings binding \
               ON binding.community_id=history.community_id \
              AND binding.binding_id=history.binding_id \
              AND binding.binding_version=history.binding_version \
              AND binding.issuer=history.issuer AND binding.uid=history.subject \
              AND binding.pubkey=history.pubkey \
              AND binding.binding_state=history.binding_state \
              AND binding.binding_provenance=history.binding_provenance \
             WHERE binding.community_id=$1 AND history.transition_kind='legacy_import'",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("count exact matrix history mirrors");
        assert_eq!(exact_history_mirrors, binding_rows, "{case_id}");

        if mask & MASK_BINDING != 0 {
            let binding: MatrixBindingRow = sqlx::query_as(
                    "SELECT convert_to(issuer,'UTF8'),convert_to(uid,'UTF8'),pubkey, \
                            binding_version,binding_state,binding_provenance,replacement_binding_id \
                     FROM identity_bindings WHERE community_id=$1 AND pubkey=$2",
                )
                .bind(domain_a)
                .bind(MATRIX_BINDING_KEY.as_slice())
                .fetch_one(&pool)
                .await
                .expect("read exact matrix active representation");
            assert_eq!(binding.0, " Issuer://EXAMPLE/%2f/é ".as_bytes());
            assert_eq!(binding.1, " Subject/Case/%41/é ".as_bytes());
            assert_eq!(binding.2, MATRIX_BINDING_KEY);
            assert_eq!(
                (binding.3, binding.4.as_str(), binding.5.as_str()),
                (1, "active", "attested_key")
            );
            assert!(binding.6.is_none());
            assert!(matches!(
                resolve_result(
                    &pool,
                    domain_a,
                    " Issuer://EXAMPLE/%2f/é ",
                    " Subject/Case/%41/é ",
                    &MATRIX_BINDING_KEY,
                )
                .await,
                ResolveBindingResult::Existing(_)
            ));
        } else {
            assert!(!raw_domain_authorized(&pool, domain_a, &MATRIX_BINDING_KEY).await);
        }
        if mask & MASK_REVOCATION != 0 {
            let revocation_exact: bool = sqlx::query_scalar(
                "SELECT reason='oracle-revoked' AND revoked_at='2026-01-02T03:04:05Z'::TIMESTAMPTZ \
                 FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(domain_a)
            .bind(MATRIX_REVOCATION_KEY.as_slice())
            .fetch_one(&pool)
            .await
            .expect("read exact matrix revocation representation");
            assert!(revocation_exact, "{case_id}");
            assert_eq!(
                resolve_result(
                    &pool,
                    domain_a,
                    "Revoked://Issuer",
                    "Revoked Subject",
                    &MATRIX_REVOCATION_KEY,
                )
                .await,
                ResolveBindingResult::Denied(BindingDenial::Revoked)
            );
        }
        if mask & MASK_RETIRED_PAIR != 0 {
            let retired_rows: Vec<MatrixHistoryRow> = sqlx::query_as(
                "SELECT binding_id,pubkey,binding_version,binding_state,binding_provenance, \
                            replacement_binding_id \
                     FROM identity_bindings WHERE community_id=$1 \
                       AND issuer='Retired://Issuer/%2F/ß' AND uid=' Retired Subject ' \
                     ORDER BY pubkey",
            )
            .bind(domain_a)
            .fetch_all(&pool)
            .await
            .expect("read exact retired history representation");
            assert_eq!(retired_rows.len(), 2, "{case_id}");
            assert_eq!(retired_rows[0].1, MATRIX_RETIRED_KEY);
            assert_eq!(
                (
                    retired_rows[0].2,
                    retired_rows[0].3.as_str(),
                    retired_rows[0].4.as_str()
                ),
                (1, "rotated", "tofu")
            );
            assert_eq!(retired_rows[0].5, Some(retired_rows[1].0));
            assert_eq!(retired_rows[1].1, MATRIX_SUCCESSOR_KEY);
            assert_eq!(
                (
                    retired_rows[1].2,
                    retired_rows[1].3.as_str(),
                    retired_rows[1].4.as_str()
                ),
                (1, "active", "tofu")
            );
            assert!(retired_rows[1].5.is_none());
            let lineage: (Uuid, Uuid) = sqlx::query_as(
                "SELECT predecessor_binding_id,successor_binding_id \
                 FROM identity_binding_lineage WHERE community_id=$1 \
                   AND predecessor_binding_id=$2",
            )
            .bind(domain_a)
            .bind(retired_rows[0].0)
            .fetch_one(&pool)
            .await
            .expect("read exact matrix lineage edge");
            assert_eq!(lineage, (retired_rows[0].0, retired_rows[1].0));
            let retired_pair: (Vec<u8>, Option<Uuid>, Option<i64>, String, bool) = sqlx::query_as(
                "SELECT pubkey,retired_binding_id,retired_binding_version,reason, \
                            retired_at='2026-01-03T03:04:05Z'::TIMESTAMPTZ \
                     FROM identity_retired_pairs WHERE community_id=$1 \
                       AND issuer='Retired://Issuer/%2F/ß' AND subject=' Retired Subject '",
            )
            .bind(domain_a)
            .fetch_one(&pool)
            .await
            .expect("read exact retired-pair representation");
            assert_eq!(retired_pair.0, MATRIX_RETIRED_KEY);
            assert_eq!(retired_pair.1, Some(retired_rows[0].0));
            assert_eq!(retired_pair.2, Some(1));
            assert_eq!(retired_pair.3, "oracle retired pair");
            assert!(retired_pair.4);
            assert_eq!(
                resolve_result(
                    &pool,
                    domain_a,
                    "Retired://Issuer/%2F/ß",
                    " Retired Subject ",
                    &MATRIX_RETIRED_KEY,
                )
                .await,
                ResolveBindingResult::Denied(BindingDenial::Revoked)
            );
            assert!(get_active_identity_binding_by_pubkey(
                &pool,
                CommunityId::from_uuid(domain_a),
                &MATRIX_SUCCESSOR_KEY,
            )
            .await
            .expect("read matrix successor")
            .is_some());
        }
        if mask & MASK_DISABLED_IDENTITY != 0 {
            let disabled_exact: bool = sqlx::query_scalar(
                "SELECT disabled_at='2026-01-04T03:04:05Z'::TIMESTAMPTZ \
                        AND disabled_reason='oracle-disabled' \
                 FROM identity_principals WHERE community_id=$1 \
                   AND issuer='Disabled://Issuer/%2f/é' AND uid=' Disabled Subject '",
            )
            .bind(domain_a)
            .fetch_one(&pool)
            .await
            .expect("read exact disabled identity representation");
            assert!(disabled_exact, "{case_id}");
            assert_eq!(
                resolve_result(
                    &pool,
                    domain_a,
                    "Disabled://Issuer/%2f/é",
                    " Disabled Subject ",
                    &MATRIX_FRESH_KEY,
                )
                .await,
                ResolveBindingResult::Denied(BindingDenial::Revoked)
            );
        }
        if mask & MASK_PENDING_LINEAGE != 0 {
            let pending_binding: (Uuid, i64, String, String) = sqlx::query_as(
                "SELECT binding_id,binding_version,binding_state,binding_provenance \
                 FROM identity_bindings WHERE community_id=$1 \
                   AND issuer='Pending://Issuer/%2F/é' AND uid=' Pending Subject ' \
                   AND pubkey=$2",
            )
            .bind(domain_a)
            .bind(MATRIX_PENDING_KEY.as_slice())
            .fetch_one(&pool)
            .await
            .expect("read exact pending source representation");
            assert_eq!(
                (
                    pending_binding.1,
                    pending_binding.2.as_str(),
                    pending_binding.3.as_str()
                ),
                (1, "revoked", "tofu")
            );
            let pending: (i64, Vec<u8>, Uuid, i64, bool) = sqlx::query_as(
                "SELECT selector_version,retired_pubkey,retired_binding_id, \
                        retired_binding_version,cleared_at IS NULL \
                 FROM identity_pending_replacements WHERE community_id=$1 \
                   AND issuer='Pending://Issuer/%2F/é' AND subject=' Pending Subject '",
            )
            .bind(domain_a)
            .fetch_one(&pool)
            .await
            .expect("read exact pending selector representation");
            assert_eq!(pending.0, 1);
            assert_eq!(pending.1, MATRIX_PENDING_KEY);
            assert_eq!(pending.2, pending_binding.0);
            assert_eq!(pending.3, 1);
            assert!(pending.4);
            let pending_retired: (Option<Uuid>, Option<i64>, String, bool) = sqlx::query_as(
                "SELECT retired_binding_id,retired_binding_version,reason, \
                        retired_at='2026-01-05T03:04:05Z'::TIMESTAMPTZ \
                 FROM identity_retired_pairs WHERE community_id=$1 \
                   AND issuer='Pending://Issuer/%2F/é' AND subject=' Pending Subject ' \
                   AND pubkey=$2",
            )
            .bind(domain_a)
            .bind(MATRIX_PENDING_KEY.as_slice())
            .fetch_one(&pool)
            .await
            .expect("read exact pending retired-pair support");
            assert_eq!(pending_retired.0, Some(pending_binding.0));
            assert_eq!(pending_retired.1, Some(1));
            assert_eq!(pending_retired.2, "oracle pending lineage");
            assert!(pending_retired.3);
            for key in [&MATRIX_PENDING_KEY[..], &MATRIX_FRESH_KEY[..]] {
                assert_eq!(
                    resolve_result(
                        &pool,
                        domain_a,
                        "Pending://Issuer/%2F/é",
                        " Pending Subject ",
                        key,
                    )
                    .await,
                    ResolveBindingResult::Denied(BindingDenial::Revoked)
                );
            }
        }

        assert_eq!(
            domain_identity_snapshot(&pool, domain_b).await,
            domain_b_post,
            "{case_id} domain-A calls changed domain B"
        );
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            legacy_b,
            "{case_id} domain-A calls changed domain-B legacy bytes"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            audit_b,
            "{case_id} domain-A calls changed domain-B audit state"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        let complete_post = full_identity_snapshot(&pool).await;
        pool.close().await;
        let pool = connect_pool().await;
        assert_eq!(
            domain_identity_snapshot(&pool, domain_b).await,
            domain_b_post,
            "{case_id} restart domain B"
        );
        assert_eq!(legacy_identity_facts(&pool, domain_b).await, legacy_b);
        assert_eq!(domain_audit_snapshot(&pool, domain_b).await, audit_b);
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "{case_id} restart"
        );
        MIGRATOR
            .run_to(30, &pool)
            .await
            .unwrap_or_else(|error| panic!("{case_id} retry failed: {error}"));
        assert_eq!(
            domain_identity_snapshot(&pool, domain_b).await,
            domain_b_post,
            "{case_id} retry domain B"
        );
        assert_eq!(legacy_identity_facts(&pool, domain_b).await, legacy_b);
        assert_eq!(domain_audit_snapshot(&pool, domain_b).await, audit_b);
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "{case_id} retry"
        );
        pool.close().await;
    }
    assert_eq!(executed.len(), 32);
    assert!(executed.contains("MIG-CART-00000"));
    assert!(executed.contains("MIG-CART-11111"));
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_frozen_20_case_literal_selector_corpus_preserves_exact_bytes() {
    const LITERALS: [(&str, &str); 10] = [
        ("Issuer", "Subject"),
        ("issuer", "subject"),
        (" Issuer", "Subject "),
        ("Issuer://EXAMPLE/a", "Issuer://example/a"),
        ("https://example.test/%2F", "https://example.test//"),
        ("é", "é"),
        ("A%41", "AA"),
        ("subject+one", "subject one"),
        ("urn:example:01", "urn:example:1"),
        ("/a/../b", "/b"),
    ];
    let mut executed = BTreeSet::new();
    for (index, pair) in LITERALS.iter().enumerate() {
        for varied_field in ["ISSUER", "SUBJECT"] {
            let case_id = format!("LITERAL-{:02}-{varied_field}", index + 1);
            assert!(executed.insert(case_id.clone()), "duplicate {case_id}");
            assert_ne!(pair.0.as_bytes(), pair.1.as_bytes(), "{case_id}");

            let pool = connect_pool().await;
            let (domain_a, domain_b) = reset_empty_to_0029(&pool, "literal-corpus").await;
            let fixed_issuer = "literal://fixed/issuer";
            let fixed_subject = " literal fixed subject ";
            let keys = [vec![111_u8; 32], vec![112_u8; 32]];
            for (literal_index, literal) in [pair.0, pair.1].into_iter().enumerate() {
                let (issuer, subject) = if varied_field == "ISSUER" {
                    (literal, fixed_subject)
                } else {
                    (fixed_issuer, literal)
                };
                sqlx::query(
                    "INSERT INTO identity_bindings \
                     (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at) \
                     VALUES ($1,$2,$3,$4,'jwt_npub','2026-02-01T00:00:00Z', \
                             '2026-02-01T00:00:00Z','2026-02-01T00:00:00Z')",
                )
                .bind(domain_a)
                .bind(issuer)
                .bind(subject)
                .bind(&keys[literal_index])
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("seed {case_id}: {error}"));
            }
            let domain_b_legacy_before = legacy_identity_facts(&pool, domain_b).await;
            let domain_b_audit_before = domain_audit_snapshot(&pool, domain_b).await;
            assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);

            MIGRATOR
                .run_to(30, &pool)
                .await
                .unwrap_or_else(|error| panic!("migrate {case_id}: {error}"));
            assert_eq!(
                legacy_identity_facts(&pool, domain_b).await,
                domain_b_legacy_before,
                "{case_id} domain-B legacy bytes after migration"
            );
            assert_eq!(
                domain_audit_snapshot(&pool, domain_b).await,
                domain_b_audit_before,
                "{case_id} domain-B audit after migration"
            );
            assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
            let domain_b_post = domain_identity_snapshot(&pool, domain_b).await;
            let rows: Vec<LiteralBindingRow> = sqlx::query_as(
                "SELECT convert_to(issuer,'UTF8'),convert_to(uid,'UTF8'),pubkey, \
                            binding_id,binding_version,binding_state,binding_provenance \
                     FROM identity_bindings WHERE community_id=$1 ORDER BY pubkey",
            )
            .bind(domain_a)
            .fetch_all(&pool)
            .await
            .expect("read literal selector rows");
            assert_eq!(rows.len(), 2, "{case_id}");
            assert_ne!(rows[0].3, rows[1].3, "{case_id}");
            for (literal_index, row) in rows.iter().enumerate() {
                let literal = if literal_index == 0 { pair.0 } else { pair.1 };
                let expected_issuer = if varied_field == "ISSUER" {
                    literal
                } else {
                    fixed_issuer
                };
                let expected_subject = if varied_field == "SUBJECT" {
                    literal
                } else {
                    fixed_subject
                };
                assert_eq!(row.0, expected_issuer.as_bytes(), "{case_id} issuer bytes");
                assert_eq!(
                    row.1,
                    expected_subject.as_bytes(),
                    "{case_id} subject bytes"
                );
                assert_eq!(row.2, keys[literal_index], "{case_id} key");
                assert_eq!(row.4, 1, "{case_id} version");
                assert_eq!(row.5, "active", "{case_id} state");
                assert_eq!(row.6, "attested_key", "{case_id} provenance");
                assert!(matches!(
                    resolve_result(
                        &pool,
                        domain_a,
                        expected_issuer,
                        expected_subject,
                        &keys[literal_index]
                    )
                    .await,
                    ResolveBindingResult::Existing(_)
                ));
                let binding = get_active_identity_binding_by_pubkey(
                    &pool,
                    CommunityId::from_uuid(domain_a),
                    &keys[literal_index],
                )
                .await
                .expect("read literal binding")
                .expect("literal binding remains active");
                assert_eq!(
                    binding.issuer.as_bytes(),
                    expected_issuer.as_bytes(),
                    "{case_id}"
                );
                assert_eq!(
                    binding.uid.as_bytes(),
                    expected_subject.as_bytes(),
                    "{case_id}"
                );
            }
            let (issuer_a, subject_a) = if varied_field == "ISSUER" {
                (pair.0, fixed_subject)
            } else {
                (fixed_issuer, pair.0)
            };
            let (issuer_b, subject_b) = if varied_field == "ISSUER" {
                (pair.1, fixed_subject)
            } else {
                (fixed_issuer, pair.1)
            };
            assert_eq!(
                resolve_result(&pool, domain_a, issuer_a, subject_a, &keys[1]).await,
                ResolveBindingResult::Denied(BindingDenial::Conflict),
                "{case_id} cross near-miss A"
            );
            assert_eq!(
                resolve_result(&pool, domain_a, issuer_b, subject_b, &keys[0]).await,
                ResolveBindingResult::Denied(BindingDenial::Conflict),
                "{case_id} cross near-miss B"
            );

            assert_eq!(
                domain_identity_snapshot(&pool, domain_b).await,
                domain_b_post,
                "{case_id} domain-B state after domain-A selections"
            );
            assert_eq!(
                domain_audit_snapshot(&pool, domain_b).await,
                domain_b_audit_before,
                "{case_id} domain-B audit after domain-A selections"
            );
            assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
            let post = full_identity_snapshot(&pool).await;
            pool.close().await;
            let pool = connect_pool().await;
            assert_eq!(
                domain_identity_snapshot(&pool, domain_b).await,
                domain_b_post,
                "{case_id} domain-B restart"
            );
            assert_eq!(
                full_identity_snapshot(&pool).await,
                post,
                "{case_id} restart"
            );
            MIGRATOR
                .run_to(30, &pool)
                .await
                .unwrap_or_else(|error| panic!("retry {case_id}: {error}"));
            assert_eq!(full_identity_snapshot(&pool).await, post, "{case_id} retry");
            assert_eq!(
                domain_identity_snapshot(&pool, domain_b).await,
                domain_b_post,
                "{case_id} domain-B retry"
            );
            pool.close().await;
        }
    }
    assert_eq!(executed.len(), 20);
    assert!(executed.contains("LITERAL-01-ISSUER"));
    assert!(executed.contains("LITERAL-10-SUBJECT"));
}

#[derive(Debug, Clone, Copy)]
enum UnreadableLegacyVariant {
    Binding,
    Revocation,
    RetiredPair,
    DisabledIdentity,
    PendingLineage,
}

impl UnreadableLegacyVariant {
    const ALL: [Self; 5] = [
        Self::Binding,
        Self::Revocation,
        Self::RetiredPair,
        Self::DisabledIdentity,
        Self::PendingLineage,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Binding => "binding",
            Self::Revocation => "revocation",
            Self::RetiredPair => "retired_pair",
            Self::DisabledIdentity => "disabled_identity",
            Self::PendingLineage => "pending_lineage",
        }
    }

    fn table_and_policy_expression(self) -> (&'static str, String, String) {
        match self {
            Self::Binding => (
                "identity_bindings",
                "migration_test_legacy_row_readable('binding',community_id,encode(pubkey,'hex'))"
                    .to_owned(),
                hex::encode(MATRIX_BINDING_KEY),
            ),
            Self::Revocation => (
                "identity_revoked_keys",
                "migration_test_legacy_row_readable('revocation',community_id,encode(pubkey,'hex'))"
                    .to_owned(),
                hex::encode(MATRIX_REVOCATION_KEY),
            ),
            Self::RetiredPair => (
                "identity_bindings",
                "migration_test_legacy_row_readable('retired_pair',community_id,encode(pubkey,'hex'))"
                    .to_owned(),
                hex::encode(MATRIX_RETIRED_KEY),
            ),
            Self::DisabledIdentity => (
                "identity_principals",
                "migration_test_legacy_row_readable('disabled_identity',community_id,uid)"
                    .to_owned(),
                " Disabled Subject ".to_owned(),
            ),
            Self::PendingLineage => (
                "identity_bindings",
                "migration_test_legacy_row_readable('pending_lineage',community_id,encode(pubkey,'hex'))"
                    .to_owned(),
                hex::encode(MATRIX_PENDING_KEY),
            ),
        }
    }
}

async fn arm_unreadable_legacy_row(
    pool: &PgPool,
    role: &str,
    domain: Uuid,
    variant: UnreadableLegacyVariant,
) -> &'static str {
    let (table, policy_expression, coordinate) = variant.table_and_policy_expression();
    sqlx::raw_sql(
        r#"
        CREATE TABLE migration_test_unreadable_control (
            kind TEXT NOT NULL,
            community_id UUID NOT NULL,
            coordinate TEXT NOT NULL
        );
        CREATE FUNCTION migration_test_legacy_row_readable(
            row_kind TEXT,
            row_domain UUID,
            row_coordinate TEXT
        ) RETURNS BOOLEAN
        LANGUAGE plpgsql VOLATILE SECURITY DEFINER
        SET search_path=pg_catalog,public
        AS $$
        BEGIN
            IF EXISTS (
                SELECT 1 FROM public.migration_test_unreadable_control fault
                WHERE fault.kind=row_kind
                  AND fault.community_id=row_domain
                  AND fault.coordinate=row_coordinate
            ) THEN
                RAISE EXCEPTION USING
                    ERRCODE='P0001',
                    MESSAGE='legacy identity state is unreadable';
            END IF;
            RETURN TRUE;
        END
        $$;
        REVOKE ALL ON FUNCTION migration_test_legacy_row_readable(TEXT,UUID,TEXT) FROM PUBLIC;
        "#,
    )
    .execute(pool)
    .await
    .expect("create unreadable legacy guard");
    sqlx::query(
        "INSERT INTO migration_test_unreadable_control (kind,community_id,coordinate) VALUES ($1,$2,$3)",
    )
    .bind(variant.name())
    .bind(domain)
    .bind(coordinate)
    .execute(pool)
    .await
    .expect("arm unreadable legacy guard");
    let policy_sql = format!(
        "ALTER TABLE {table} ENABLE ROW LEVEL SECURITY; \
         ALTER TABLE {table} FORCE ROW LEVEL SECURITY; \
         CREATE POLICY migration_test_unreadable_policy ON {table} USING ({policy_expression}); \
         GRANT EXECUTE ON FUNCTION migration_test_legacy_row_readable(TEXT,UUID,TEXT) TO {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(policy_sql))
        .execute(pool)
        .await
        .expect("install unreadable row policy");
    table
}

async fn disarm_unreadable_legacy_row(pool: &PgPool, table: &str) {
    let sql = format!(
        "DROP POLICY migration_test_unreadable_policy ON {table}; \
         ALTER TABLE {table} NO FORCE ROW LEVEL SECURITY; \
         ALTER TABLE {table} DISABLE ROW LEVEL SECURITY; \
         DROP FUNCTION migration_test_legacy_row_readable(TEXT,UUID,TEXT); \
         DROP TABLE migration_test_unreadable_control;"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .expect("remove unreadable row policy without changing legacy rows");
}

async fn create_restricted_migration_role(pool: &PgPool, role: &str) -> String {
    let controller: String = sqlx::query_scalar("SELECT quote_ident(current_user)")
        .fetch_one(pool)
        .await
        .expect("read controller role");
    let sql = format!(
        "CREATE ROLE {role} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
             NOINHERIT NOREPLICATION NOBYPASSRLS; \
         GRANT USAGE,CREATE ON SCHEMA public TO {role}; \
         GRANT ALL PRIVILEGES ON TABLE identity_bindings,_sqlx_migrations TO {role}; \
         GRANT SELECT ON TABLE identity_principals,identity_revoked_keys TO {role}; \
         GRANT SELECT,REFERENCES ON TABLE communities TO {role}; \
         ALTER TABLE identity_bindings OWNER TO {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .expect("create restricted migration owner");
    controller
}

async fn run_migration_as_role(
    pool: &PgPool,
    role: &str,
) -> Result<(), sqlx::migrate::MigrateError> {
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire restricted migration backend");
    let set_role = format!("SET ROLE {role}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(set_role))
        .execute(&mut *connection)
        .await
        .expect("enter restricted migration role");
    let result = MIGRATOR.run_to(30, &mut *connection).await;
    sqlx::query("RESET ROLE")
        .execute(&mut *connection)
        .await
        .expect("leave restricted migration role");
    result
}

async fn drop_restricted_migration_role(pool: &PgPool, role: &str, controller: &str) {
    let sql = format!(
        "REASSIGN OWNED BY {role} TO {controller}; \
         DROP OWNED BY {role}; \
         DROP ROLE {role};"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .expect("drop restricted migration owner");
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_all_five_unreadable_legacy_variants_rollback_then_retry() {
    let mut executed = BTreeSet::new();
    for variant in UnreadableLegacyVariant::ALL {
        let case_id = format!("MIG-AMB-005-{}", variant.name());
        assert!(executed.insert(case_id.clone()), "duplicate {case_id}");
        let pool = connect_pool().await;
        let (domain_a, domain_b) = reset_empty_to_0029(&pool, "unreadable-legacy").await;
        seed_legacy_presence_mask(&pool, domain_a, 0b1_1111).await;
        let before = legacy_snapshot(&pool).await;
        let domain_b_facts = legacy_identity_facts(&pool, domain_b).await;
        let domain_b_audit = domain_audit_snapshot(&pool, domain_b).await;
        let role = format!("identity_unreadable_{}", Uuid::new_v4().simple());
        let controller = create_restricted_migration_role(&pool, &role).await;
        let table = arm_unreadable_legacy_row(&pool, &role, domain_a, variant).await;

        let error = run_migration_as_role(&pool, &role)
            .await
            .expect_err("unreadable retained state must abort 0030");
        let error_text = error.to_string();
        assert!(
            error_text.contains("legacy identity state is unreadable"),
            "{case_id}: {error_text}"
        );
        for forbidden in [
            " Issuer://EXAMPLE/%2f/é ",
            " Subject/Case/%41/é ",
            " Retired Subject ",
            " Disabled Subject ",
            " Pending Subject ",
            &hex::encode(MATRIX_BINDING_KEY),
        ] {
            assert!(
                !error_text.contains(forbidden),
                "{case_id} disclosed a legacy coordinate"
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success",
            )
            .fetch_one(&pool)
            .await
            .expect("count failed unreadable marker"),
            0,
            "{case_id}"
        );
        let projected_table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('identity_retired_pairs')::TEXT")
                .fetch_one(&pool)
                .await
                .expect("read failed projection table");
        assert!(projected_table.is_none(), "{case_id}");
        let projected_column: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns \
             WHERE table_schema='public' AND table_name='identity_bindings' \
               AND column_name='binding_id'",
        )
        .fetch_one(&pool)
        .await
        .expect("read failed projection column");
        assert_eq!(projected_column, 0, "{case_id}");
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            domain_b_facts,
            "{case_id}"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            domain_b_audit,
            "{case_id}"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        pool.close().await;

        let pool = connect_pool().await;
        let fault_still_armed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM migration_test_unreadable_control WHERE kind=$1 AND community_id=$2)",
        )
        .bind(variant.name())
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("read durable unreadable fault after restart");
        assert!(fault_still_armed, "{case_id}");
        assert_eq!(
            legacy_snapshot(&pool).await,
            before,
            "{case_id} retained legacy bytes while the read fault remained armed"
        );
        assert!(
            resolve_identity_binding(
                &pool,
                &ResolveBindingInput {
                    authorization_domain: CommunityId::from_uuid(domain_a),
                    issuer: " Issuer://EXAMPLE/%2f/é ",
                    subject: " Subject/Case/%41/é ",
                    pubkey: &MATRIX_BINDING_KEY,
                    display_name: None,
                    enrollment_mode: EnrollmentMode::AttestedKey,
                    key_attested: true,
                    policy_version: "migration-test-policy-v1",
                    evidence_valid_from: 0,
                    evidence_valid_until: i64::MAX as u64,
                },
            )
            .await
            .is_err(),
            "{case_id} incomplete migration must fail closed in the application path"
        );
        disarm_unreadable_legacy_row(&pool, table).await;
        assert_eq!(
            legacy_snapshot(&pool).await,
            before,
            "{case_id} exact rollback"
        );

        run_migration_as_role(&pool, &role)
            .await
            .unwrap_or_else(|error| panic!("{case_id} retry failed: {error}"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success",
            )
            .fetch_one(&pool)
            .await
            .expect("count retried unreadable marker"),
            1,
            "{case_id}"
        );
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            domain_b_facts,
            "{case_id}"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            domain_b_audit,
            "{case_id}"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        assert_eq!(
            resolve_result(
                &pool,
                domain_a,
                "Pending://Issuer/%2F/é",
                " Pending Subject ",
                &MATRIX_FRESH_KEY,
            )
            .await,
            ResolveBindingResult::Denied(BindingDenial::Revoked),
            "{case_id}"
        );
        let complete_post = full_identity_snapshot(&pool).await;
        pool.close().await;

        let pool = connect_pool().await;
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "{case_id} restart"
        );
        MIGRATOR
            .run_to(30, &pool)
            .await
            .unwrap_or_else(|error| panic!("{case_id} no-op retry failed: {error}"));
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "{case_id} no-op retry"
        );
        drop_restricted_migration_role(&pool, &role, &controller).await;
        pool.close().await;
    }
    assert_eq!(executed.len(), 5);
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_fault_at_every_statement_boundary_restarts_and_retries() {
    let statements = split_statements(migration_0030().sql.as_ref());
    assert_eq!(
        statements.len(),
        34,
        "0030 boundary count is part of the oracle adapter"
    );

    for boundary in 0..=statements.len() {
        let pool = connect_pool().await;
        let (_domain_a, domain_b) = reset_to_0029(&pool).await;
        let before = legacy_snapshot(&pool).await;
        let domain_b_facts = legacy_identity_facts(&pool, domain_b).await;
        let domain_b_audit = domain_audit_snapshot(&pool, domain_b).await;
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);

        let mut connection = pool.acquire().await.expect("acquire crashable backend");
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .expect("read crashable backend pid");
        let mut tx = connection
            .begin()
            .await
            .expect("begin crashable boundary migration");
        for statement in &statements[..boundary] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement.as_str()))
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|error| {
                    panic!("0030 statement before boundary {boundary} failed: {error}")
                });
        }
        let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
            .bind(backend_pid)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| {
                panic!("terminate boundary {boundary} migration backend: {error}")
            });
        assert!(terminated, "boundary {boundary} backend must terminate");
        assert!(
            sqlx::query("SELECT 1").execute(&mut *tx).await.is_err(),
            "boundary {boundary} transaction must observe backend loss"
        );
        drop(tx);
        drop(connection);
        pool.close().await;

        let pool = connect_pool().await;
        assert_eq!(legacy_snapshot(&pool).await, before, "boundary {boundary}");
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            domain_b_facts,
            "boundary {boundary} domain-B facts before retry"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            domain_b_audit,
            "boundary {boundary} domain-B audit before retry"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);

        MIGRATOR.run_to(30, &pool).await.unwrap_or_else(|error| {
            panic!("boundary {boundary} retry after restart failed: {error}")
        });
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success"
            )
            .fetch_one(&pool)
            .await
            .expect("count boundary retry marker"),
            1,
            "boundary {boundary} must produce one durable migration marker"
        );
        assert_eq!(
            legacy_identity_facts(&pool, domain_b).await,
            domain_b_facts,
            "boundary {boundary} domain-B facts after retry"
        );
        assert_eq!(
            domain_audit_snapshot(&pool, domain_b).await,
            domain_b_audit,
            "boundary {boundary} domain-B audit after retry"
        );
        assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
        let complete_post = full_identity_snapshot(&pool).await;
        pool.close().await;

        let pool = connect_pool().await;
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "boundary {boundary} complete post-state must survive restart"
        );
        MIGRATOR.run_to(30, &pool).await.unwrap_or_else(|error| {
            panic!("boundary {boundary} idempotent post-restart retry failed: {error}")
        });
        assert_eq!(
            full_identity_snapshot(&pool).await,
            complete_post,
            "boundary {boundary} retry must remain an exact no-op"
        );
        pool.close().await;
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_commit_failure_rolls_back_projection_and_success_marker() {
    let pool = connect_pool().await;
    let (_domain_a, domain_b) = reset_to_0029(&pool).await;
    let before = legacy_snapshot(&pool).await;
    let migration = migration_0030();
    let statements = split_statements(migration.sql.as_ref());
    let mut tx = pool.begin().await.expect("begin commit-failure migration");
    for statement in statements {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
            .execute(&mut *tx)
            .await
            .expect("execute 0030 before commit failure");
    }
    sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version,description,installed_on,success,checksum,execution_time) \
         VALUES ($1,$2,NOW(),TRUE,$3,0)",
    )
    .bind(migration.version)
    .bind(migration.description.as_ref())
    .bind(migration.checksum.as_ref())
    .execute(&mut *tx)
    .await
    .expect("insert transactional 0030 marker");
    sqlx::query(
        "UPDATE identity_bindings SET replacement_binding_id=$1 \
         WHERE community_id=$2",
    )
    .bind(Uuid::from_u128(u128::MAX))
    .bind(domain_b)
    .execute(&mut *tx)
    .await
    .expect("deferred FK accepts invalid replacement before commit");
    assert!(
        tx.commit().await.is_err(),
        "deferred FK must fail at commit"
    );
    assert_eq!(legacy_snapshot(&pool).await, before);
    assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);

    MIGRATOR
        .run_to(30, &pool)
        .await
        .expect("retry after commit failure");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version=30 AND success"
        )
        .fetch_one(&pool)
        .await
        .expect("count successful 0030 marker"),
        1
    );
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_backend_loss_restarts_cleanly_and_retry_converges() {
    let pool = connect_pool().await;
    let (_domain_a, domain_b) = reset_to_0029(&pool).await;
    let before = legacy_snapshot(&pool).await;
    let statements = split_statements(migration_0030().sql.as_ref());
    let mut connection = pool.acquire().await.expect("acquire migration backend");
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await
        .expect("read migration backend pid");
    let mut tx = connection.begin().await.expect("begin crashable migration");
    for statement in &statements[..statements.len() / 2] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement.as_str()))
            .execute(&mut *tx)
            .await
            .expect("execute pre-crash migration prefix");
    }
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(backend_pid)
        .fetch_one(&pool)
        .await
        .expect("terminate migration backend");
    assert!(terminated);
    assert!(sqlx::query("SELECT 1").execute(&mut *tx).await.is_err());
    drop(tx);
    drop(connection);
    pool.close().await;

    let pool = connect_pool().await;
    assert_eq!(legacy_snapshot(&pool).await, before);
    assert!(raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await);
    MIGRATOR
        .run_to(30, &pool)
        .await
        .expect("retry after backend restart");
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_real_post_commit_response_loss_is_idempotent_after_restart() {
    const GATE_CLASS: i32 = 2_147_400_030;
    const GATE_OBJECT: i32 = 30;
    let pool = connect_pool().await;
    let (_domain_a, domain_b) = reset_to_0029(&pool).await;
    sqlx::query(
        "INSERT INTO audit_log \
         (community_id,seq,hash,action,object_id,detail,created_at) \
         VALUES ($1,1,$2,'preexisting_response_loss_sentinel','domain-b-sentinel', \
                 '{\"sentinel\":\"before-response-loss-operation\"}'::jsonb, \
                 TIMESTAMPTZ '2025-04-01 00:00:00Z')",
    )
    .bind(domain_b)
    .bind(vec![72_u8; 32])
    .execute(&pool)
    .await
    .expect("insert substantive response-loss audit sentinel");
    let domain_b_facts = legacy_identity_facts(&pool, domain_b).await;
    let domain_b_audit = domain_audit_snapshot(&pool, domain_b).await;
    let domain_b_history_pre = domain_legacy_history_snapshot(&pool, domain_b).await;
    let domain_b_authorized = raw_domain_authorized(&pool, domain_b, &[72_u8; 32]).await;
    assert!(domain_b_authorized);
    assert_response_loss_legacy_sentinels(
        &pool,
        domain_b,
        &domain_b_facts,
        domain_b_authorized,
        &domain_b_audit,
        &domain_b_history_pre,
    )
    .await;
    let migration = migration_0030();
    let statements = split_statements(migration.sql.as_ref());

    let mut gate = pool
        .acquire()
        .await
        .expect("acquire response-loss gate backend");
    sqlx::query("SELECT pg_advisory_lock($1,$2)")
        .bind(GATE_CLASS)
        .bind(GATE_OBJECT)
        .execute(&mut *gate)
        .await
        .expect("hold post-commit response gate");

    let migration_pool = pool.clone();
    let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel();
    let migration_task = tokio::spawn(async move {
        let mut connection = migration_pool
            .acquire()
            .await
            .expect("acquire response-loss migration backend");
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .expect("read response-loss backend pid");
        pid_sender
            .send(backend_pid)
            .expect("send response-loss backend pid");
        let mut tx = connection
            .begin()
            .await
            .expect("begin response-loss migration");
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .expect("require durable response-loss commit");
        for statement in statements {
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
                .execute(&mut *tx)
                .await
                .expect("execute response-loss migration statement");
        }
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version,description,installed_on,success,checksum,execution_time) \
             VALUES ($1,$2,NOW(),TRUE,$3,0)",
        )
        .bind(migration.version)
        .bind(migration.description.as_ref())
        .bind(migration.checksum.as_ref())
        .execute(&mut *tx)
        .await
        .expect("insert response-loss migration marker");
        let commit_then_block =
            format!("COMMIT; SELECT pg_advisory_lock({GATE_CLASS},{GATE_OBJECT})");
        let response = sqlx::raw_sql(sqlx::AssertSqlSafe(commit_then_block))
            .execute(&mut *tx)
            .await;
        assert!(
            response.is_err(),
            "terminated post-commit backend must lose the real response stream"
        );
    });
    let migration_pid = pid_receiver.await.expect("receive migration backend pid");

    let mut observed_durable_commit_and_waiter = false;
    for _ in 0..20_000 {
        let observed: bool = sqlx::query_scalar(
            "SELECT \
               EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=30 AND success) \
               AND EXISTS(\
                 SELECT 1 FROM pg_locks lock_row \
                 WHERE lock_row.locktype='advisory' \
                   AND lock_row.pid=$1 AND NOT lock_row.granted \
                   AND lock_row.classid::BIGINT=$2 AND lock_row.objid::BIGINT=$3\
               )",
        )
        .bind(migration_pid)
        .bind(i64::from(GATE_CLASS))
        .bind(i64::from(GATE_OBJECT))
        .fetch_one(&pool)
        .await
        .expect("observe committed migration blocked before success response");
        if observed {
            observed_durable_commit_and_waiter = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        observed_durable_commit_and_waiter,
        "migration never reached the proven post-commit/pre-response boundary"
    );
    assert_response_loss_legacy_sentinels(
        &pool,
        domain_b,
        &domain_b_facts,
        domain_b_authorized,
        &domain_b_audit,
        &domain_b_history_pre,
    )
    .await;
    let (domain_b_state_post, domain_b_history_post) =
        response_loss_migrated_domain_sentinels(&pool, domain_b).await;
    let committed = full_identity_snapshot(&pool).await;
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(migration_pid)
        .fetch_one(&pool)
        .await
        .expect("terminate backend after durable commit before response");
    assert!(terminated);
    migration_task
        .await
        .expect("join real response-loss migration task");
    sqlx::query("SELECT pg_advisory_unlock($1,$2)")
        .bind(GATE_CLASS)
        .bind(GATE_OBJECT)
        .execute(&mut *gate)
        .await
        .expect("release response-loss gate");
    drop(gate);
    pool.close().await;

    let pool = connect_pool().await;
    assert_response_loss_legacy_sentinels(
        &pool,
        domain_b,
        &domain_b_facts,
        domain_b_authorized,
        &domain_b_audit,
        &domain_b_history_pre,
    )
    .await;
    let (reconnected_domain_b_state, reconnected_domain_b_history) =
        response_loss_migrated_domain_sentinels(&pool, domain_b).await;
    assert_eq!(reconnected_domain_b_state, domain_b_state_post);
    assert_eq!(reconnected_domain_b_history, domain_b_history_post);
    assert_eq!(full_identity_snapshot(&pool).await, committed);
    MIGRATOR
        .run_to(30, &pool)
        .await
        .expect("idempotent retry after response loss");
    assert_response_loss_legacy_sentinels(
        &pool,
        domain_b,
        &domain_b_facts,
        domain_b_authorized,
        &domain_b_audit,
        &domain_b_history_pre,
    )
    .await;
    let (retried_domain_b_state, retried_domain_b_history) =
        response_loss_migrated_domain_sentinels(&pool, domain_b).await;
    assert_eq!(retried_domain_b_state, domain_b_state_post);
    assert_eq!(retried_domain_b_history, domain_b_history_post);
    assert_eq!(full_identity_snapshot(&pool).await, committed);
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn identity_0030_readable_ambiguities_preserve_facts_and_never_create_authority() {
    static PROVISION_KEY: [u8; 32] = [91; 32];
    static ROTATE_KEY: [u8; 32] = [92; 32];
    static RECOVER_KEY: [u8; 32] = [93; 32];
    static ENABLE_KEY: [u8; 32] = [94; 32];

    let pool = connect_pool().await;
    let (domain_a, domain_b) = reset_to_0029(&pool).await;
    let active_key = vec![71_u8; 32];
    let missing_predecessor = vec![73_u8; 32];
    let missing_target = vec![74_u8; 32];
    insert_rotated_legacy_row(
        &pool,
        domain_a,
        "migration-fault-a-subject",
        &missing_predecessor,
        &missing_target,
    )
    .await;

    let duplicate_key = vec![80_u8; 32];
    insert_revoked_legacy_row(&pool, domain_a, "duplicate", &duplicate_key).await;
    insert_revoked_legacy_row(&pool, domain_a, "duplicate", &duplicate_key).await;

    let cycle_a = vec![81_u8; 32];
    let cycle_b = vec![82_u8; 32];
    insert_rotated_legacy_row(&pool, domain_a, "cycle", &cycle_a, &cycle_b).await;
    insert_rotated_legacy_row(&pool, domain_a, "cycle", &cycle_b, &cycle_a).await;

    let fork_source = vec![83_u8; 32];
    let fork_target = vec![84_u8; 32];
    insert_rotated_legacy_row(&pool, domain_a, "fork", &fork_source, &fork_target).await;
    insert_revoked_legacy_row(&pool, domain_a, "fork", &fork_target).await;
    insert_revoked_legacy_row(&pool, domain_a, "fork", &fork_target).await;

    let conflicting_key = vec![85_u8; 32];
    sqlx::query(
        "INSERT INTO identity_bindings (community_id,issuer,uid,pubkey,source) \
         VALUES ($1,'https://idp.example','active-with-tombstone',$2,'db_binding')",
    )
    .bind(domain_a)
    .bind(&conflicting_key)
    .execute(&pool)
    .await
    .expect("insert active binding selected by legacy tombstone");
    sqlx::query(
        "INSERT INTO identity_revoked_keys (community_id,pubkey,reason) \
         VALUES ($1,$2,'legacy key tombstone')",
    )
    .bind(domain_a)
    .bind(&conflicting_key)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy tombstone");

    let legacy_a = legacy_identity_facts(&pool, domain_a).await;
    let legacy_b = legacy_identity_facts(&pool, domain_b).await;
    MIGRATOR
        .run_to(30, &pool)
        .await
        .expect("readable ambiguity migrates into fail-closed quarantine");
    assert_eq!(legacy_identity_facts(&pool, domain_a).await, legacy_a);
    assert_eq!(legacy_identity_facts(&pool, domain_b).await, legacy_b);

    let quarantined: Vec<String> = sqlx::query_scalar(
        "SELECT subject FROM identity_migration_denials WHERE community_id=$1 ORDER BY subject",
    )
    .bind(domain_a)
    .fetch_all(&pool)
    .await
    .expect("read principal quarantines");
    assert_eq!(
        quarantined,
        vec![
            "cycle".to_owned(),
            "duplicate".to_owned(),
            "fork".to_owned(),
            "migration-fault-a-subject".to_owned(),
        ]
    );
    for key in [
        active_key.as_slice(),
        missing_predecessor.as_slice(),
        missing_target.as_slice(),
        duplicate_key.as_slice(),
        cycle_a.as_slice(),
        cycle_b.as_slice(),
        fork_source.as_slice(),
        fork_target.as_slice(),
    ] {
        let denied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM identity_migration_denied_keys \
             WHERE community_id=$1 AND pubkey=$2)",
        )
        .bind(domain_a)
        .bind(key)
        .fetch_one(&pool)
        .await
        .expect("read implicated key quarantine");
        assert!(
            denied,
            "every stored or referenced implicated key is denied"
        );
    }
    let imported_ambiguous_edges: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_binding_lineage lineage \
         JOIN identity_bindings binding \
           ON binding.community_id=lineage.community_id \
          AND binding.binding_id=lineage.predecessor_binding_id \
         WHERE binding.community_id=$1 AND binding.uid IN \
               ('cycle','duplicate','fork','migration-fault-a-subject')",
    )
    .bind(domain_a)
    .fetch_one(&pool)
    .await
    .expect("count ambiguous imported lineage");
    assert_eq!(imported_ambiguous_edges, 0);

    let domain_a_id = CommunityId::from_uuid(domain_a);
    let domain_b_id = CommunityId::from_uuid(domain_b);
    let main_principal = IdentityPrincipal {
        issuer: "https://idp.example",
        subject: "migration-fault-a-subject",
    };
    assert!(
        get_active_identity_binding_by_pubkey(&pool, domain_a_id, &active_key)
            .await
            .is_err()
    );
    assert!(
        get_active_identity_binding_by_pubkey(&pool, domain_a_id, &conflicting_key)
            .await
            .is_err()
    );
    for (subject, key) in [
        (main_principal.subject, active_key.as_slice()),
        ("different-principal", missing_target.as_slice()),
        ("active-with-tombstone", conflicting_key.as_slice()),
    ] {
        let result = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: domain_a_id,
                issuer: "https://idp.example",
                subject,
                pubkey: key,
                display_name: None,
                enrollment_mode: EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "migration-test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("migrated denial resolves without authority");
        assert_eq!(result, ResolveBindingResult::Denied(BindingDenial::Revoked));
    }

    let domain_b_cross = resolve_identity_binding(
        &pool,
        &ResolveBindingInput {
            authorization_domain: domain_b_id,
            issuer: "https://idp.example",
            subject: "independent-domain-subject",
            pubkey: &missing_target,
            display_name: None,
            enrollment_mode: EnrollmentMode::AttestedKey,
            key_attested: true,
            policy_version: "migration-test-policy-v1",
            evidence_valid_from: 0,
            evidence_valid_until: i64::MAX as u64,
        },
    )
    .await
    .expect("same bytes remain independent in domain B");
    assert!(matches!(domain_b_cross, ResolveBindingResult::Enrolled(_)));
    let domain_b_sentinel = domain_identity_snapshot(&pool, domain_b).await;

    let (retired_binding_id, retired_binding_version): (Uuid, i64) = sqlx::query_as(
        "SELECT binding_id,binding_version FROM identity_bindings \
         WHERE community_id=$1 AND issuer='https://idp.example' \
           AND uid='migration-fault-a-subject' AND pubkey=$2",
    )
    .bind(domain_a)
    .bind(&missing_predecessor)
    .fetch_one(&pool)
    .await
    .expect("read quarantined retired coordinate");
    let fabricated_pending = PendingLineage {
        retired_pubkey: missing_predecessor.clone(),
        retired_binding_id,
        retired_binding_version: u64::try_from(retired_binding_version).expect("positive version"),
        selector_version: 1,
    };
    let before_denials = domain_identity_snapshot(&pool, domain_a).await;
    assert!(provision_identity_binding(
        &pool,
        domain_a_id,
        lifecycle_context(301, "quarantine provision denial"),
        main_principal,
        EnrollmentMode::Provisioned,
        replacement(&PROVISION_KEY, BindingProvenance::Provisioned),
    )
    .await
    .is_err());
    assert!(retire_identity_pair(
        &pool,
        domain_a_id,
        lifecycle_context(302, "quarantine retire denial"),
        main_principal,
        &active_key,
    )
    .await
    .is_err());
    assert!(disable_identity_principal(
        &pool,
        domain_a_id,
        lifecycle_context(303, "quarantine disable denial"),
        main_principal,
    )
    .await
    .is_err());
    assert!(revoke_identity_key(
        &pool,
        domain_a_id,
        lifecycle_context(304, "quarantine revoke denial"),
        &active_key,
    )
    .await
    .is_err());
    assert!(rotate_identity_binding(
        &pool,
        domain_a_id,
        lifecycle_context(305, "quarantine rotate denial"),
        main_principal,
        &active_key,
        replacement(&ROTATE_KEY, BindingProvenance::AttestedKey),
    )
    .await
    .is_err());
    assert!(recover_identity_binding(
        &pool,
        domain_a_id,
        lifecycle_context(306, "quarantine recover denial"),
        main_principal,
        &fabricated_pending,
        replacement(&RECOVER_KEY, BindingProvenance::AttestedKey),
    )
    .await
    .is_err());
    assert!(enable_identity_principal(
        &pool,
        domain_a_id,
        lifecycle_context(307, "quarantine enable denial"),
        main_principal,
        Some(&fabricated_pending),
        replacement(&ENABLE_KEY, BindingProvenance::AttestedKey),
    )
    .await
    .is_err());
    assert_eq!(
        domain_identity_snapshot(&pool, domain_a).await,
        before_denials
    );
    assert_eq!(
        domain_identity_snapshot(&pool, domain_b).await,
        domain_b_sentinel
    );
    assert!(
        get_active_identity_binding_by_pubkey(&pool, domain_b_id, &missing_target)
            .await
            .expect("domain-B authorization sentinel")
            .is_some()
    );
}
