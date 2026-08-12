use super::*;

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

const INVITATION_OBJECT_MIGRATION: &str =
    include_str!("../../../../../migrations/0036_nip_fi_invitation_object_kind.sql");
const ACTIVE_OBJECT_KINDS_MIGRATION: &str =
    include_str!("../../../../../migrations/0040_nip_fi_event_status_object_kinds.sql");
const DESIRED_SCHEMA: &str = include_str!("../../../../../schema/schema.sql");
const INVITATION_OBJECT_KIND_EXPRESSION: &str = "object_kind IN (1, 2, 3, 4, 5, 6, 9)";
const FINAL_OBJECT_KIND_EXPRESSION: &str = "object_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9)";
const OBJECT_KIND_CHECKS: [(&str, &str); 3] = [
    (
        "authorization_admission_results",
        "authorization_admission_results_object_kind_check",
    ),
    (
        "authorization_authority_epochs",
        "authorization_authority_epochs_object_kind_check",
    ),
    (
        "protected_object_authority",
        "protected_object_authority_object_kind_check",
    ),
];

#[derive(Clone, Copy)]
enum ProbeTable {
    AdmissionResults,
    AuthorityEpochs,
    ProtectedAuthority,
}

#[test]
fn invitation_object_migration_and_desired_schema_are_exact() {
    let executable = INVITATION_OBJECT_MIGRATION
        .lines()
        .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(executable.matches("DROP CONSTRAINT").count(), 3);
    assert_eq!(executable.matches("ADD CONSTRAINT").count(), 3);
    assert_eq!(executable.matches("NOT VALID").count(), 3);
    assert_eq!(executable.matches("VALIDATE CONSTRAINT").count(), 3);
    assert_eq!(
        executable
            .matches(INVITATION_OBJECT_KIND_EXPRESSION)
            .count(),
        3
    );
    for forbidden in [
        "CREATE TABLE",
        "DROP TABLE",
        "ADD COLUMN",
        "DROP COLUMN",
        "ALTER COLUMN",
        "INSERT INTO",
        "UPDATE ",
        "DELETE FROM",
        "TRUNCATE",
    ] {
        assert!(
            !executable.contains(forbidden),
            "migration 0036 must change only the three object-kind checks: {forbidden}",
        );
    }

    for (table, constraint) in OBJECT_KIND_CHECKS {
        let drop = format!("ALTER TABLE {table}\n    DROP CONSTRAINT {constraint};");
        let add = format!(
            "ALTER TABLE {table}\n    ADD CONSTRAINT {constraint}\n    CHECK ({INVITATION_OBJECT_KIND_EXPRESSION}) NOT VALID;"
        );
        let validate = format!("ALTER TABLE {table}\n    VALIDATE CONSTRAINT {constraint};");
        assert_eq!(executable.matches(&drop).count(), 1, "missing {drop}");
        assert_eq!(executable.matches(&add).count(), 1, "missing {add}");
        assert_eq!(
            executable.matches(&validate).count(),
            1,
            "missing {validate}"
        );

        let final_migration = format!(
            "ADD CONSTRAINT {constraint}\n    CHECK ({FINAL_OBJECT_KIND_EXPRESSION}) NOT VALID;"
        );
        assert_eq!(
            ACTIVE_OBJECT_KINDS_MIGRATION
                .matches(&final_migration)
                .count(),
            1,
            "migration 0040 must allocate the final active kinds for {constraint}",
        );

        let desired = format!("CONSTRAINT {constraint} CHECK ({FINAL_OBJECT_KIND_EXPRESSION})");
        assert_eq!(
            DESIRED_SCHEMA.matches(&desired).count(),
            1,
            "desired schema must carry exactly one {constraint}",
        );
    }

    for rejected in [7_i16, 8, 10] {
        assert!(
            !INVITATION_OBJECT_KIND_EXPRESSION
                .split(|character: char| !character.is_ascii_digit())
                .filter_map(|value| value.parse::<i16>().ok())
                .any(|value| value == rejected),
            "unallocated object kind {rejected} must remain rejected",
        );
    }
}

async fn assert_exact_catalog(pool: &PgPool, expected_codes: &[i16]) {
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT c.conrelid::regclass::text,pg_get_constraintdef(c.oid,true),c.convalidated \
         FROM pg_constraint c \
         WHERE c.connamespace='public'::regnamespace AND c.conname=ANY($1) \
         ORDER BY c.conrelid::regclass::text",
    )
    .bind(
        OBJECT_KIND_CHECKS
            .iter()
            .map(|(_, constraint)| *constraint)
            .collect::<Vec<_>>(),
    )
    .fetch_all(pool)
    .await
    .expect("read Invitation object-kind constraint catalog");

    assert_eq!(rows.len(), OBJECT_KIND_CHECKS.len());
    assert!(rows.iter().all(|(_, _, validated)| *validated));
    assert!(rows.windows(2).all(|pair| pair[0].1 == pair[1].1));
    let codes = rows[0]
        .1
        .split(|character: char| !character.is_ascii_digit())
        .filter_map(|value| value.parse::<i16>().ok())
        .collect::<Vec<_>>();
    assert_eq!(codes, expected_codes);
}

async fn create_probe_tables(connection: &mut PgConnection) {
    sqlx::raw_sql(
        "CREATE TEMP TABLE invitation_admission_probe \
             (LIKE authorization_admission_results INCLUDING DEFAULTS INCLUDING CONSTRAINTS); \
         CREATE TEMP TABLE invitation_epoch_probe \
             (LIKE authorization_authority_epochs INCLUDING DEFAULTS INCLUDING CONSTRAINTS); \
         CREATE TEMP TABLE invitation_authority_probe \
             (LIKE protected_object_authority INCLUDING DEFAULTS INCLUDING CONSTRAINTS);",
    )
    .execute(&mut *connection)
    .await
    .expect("clone object-kind checks into isolated probe tables");
}

async fn drop_probe_tables(connection: &mut PgConnection) {
    sqlx::raw_sql(
        "DROP TABLE invitation_admission_probe; \
         DROP TABLE invitation_epoch_probe; \
         DROP TABLE invitation_authority_probe;",
    )
    .execute(&mut *connection)
    .await
    .expect("drop isolated object-kind probe tables");
}

async fn probe_kind(
    connection: &mut PgConnection,
    table: ProbeTable,
    kind: i16,
) -> std::result::Result<(), sqlx::Error> {
    let marker = u8::try_from(kind).unwrap_or_default().saturating_add(1);
    match table {
        ProbeTable::AdmissionResults => {
            sqlx::query(
                "INSERT INTO invitation_admission_probe \
                 (community_id,operation_id,request_fingerprint,semantic_fingerprint, \
                  object_kind,object_key) VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(vec![marker; 32])
            .bind(vec![marker.saturating_add(1); 32])
            .bind(kind)
            .bind(vec![marker.saturating_add(2); 32])
            .execute(&mut *connection)
            .await?;
        }
        ProbeTable::AuthorityEpochs => {
            sqlx::query(
                "INSERT INTO invitation_epoch_probe \
                 (community_id,object_kind,object_key,authority_epoch,fence,operation_id, \
                  request_fingerprint) VALUES ($1,$2,$3,1,$4,$5,$6)",
            )
            .bind(Uuid::new_v4())
            .bind(kind)
            .bind(vec![marker; 32])
            .bind(vec![marker.saturating_add(1); 32])
            .bind(Uuid::new_v4())
            .bind(vec![marker.saturating_add(2); 32])
            .execute(&mut *connection)
            .await?;
        }
        ProbeTable::ProtectedAuthority => {
            sqlx::query(
                "INSERT INTO invitation_authority_probe \
                 (community_id,object_kind,object_key,capability,actor_pubkey,binding_id, \
                  binding_version,policy_revision,invalidation_generation,authority_epoch, \
                  fence,issued_at,expires_at,operation_id,request_fingerprint) \
                 VALUES ($1,$2,$3,1,$4,$5,1,1,0,1,$6,transaction_timestamp(), \
                         transaction_timestamp()+INTERVAL '5 minutes',$7,$8)",
            )
            .bind(Uuid::new_v4())
            .bind(kind)
            .bind(vec![marker; 32])
            .bind(vec![marker.saturating_add(1); 32])
            .bind(Uuid::new_v4())
            .bind(vec![marker.saturating_add(2); 32])
            .bind(Uuid::new_v4())
            .bind(vec![marker.saturating_add(3); 32])
            .execute(&mut *connection)
            .await?;
        }
    }
    Ok(())
}

async fn assert_kind_behavior(
    connection: &mut PgConnection,
    allowed_kinds: &[i16],
    rejected_kinds: &[i16],
) {
    for table in [
        ProbeTable::AdmissionResults,
        ProbeTable::AuthorityEpochs,
        ProbeTable::ProtectedAuthority,
    ] {
        for kind in allowed_kinds {
            probe_kind(connection, table, *kind)
                .await
                .unwrap_or_else(|error| {
                    panic!("active object kind {kind} was rejected: {error:?}")
                });
        }
        for kind in rejected_kinds {
            let error = probe_kind(connection, table, *kind)
                .await
                .expect_err("unallocated object kind must remain rejected");
            assert_eq!(
                error
                    .as_database_error()
                    .and_then(|database_error| database_error.code().map(|code| code.into_owned()))
                    .as_deref(),
                Some("23514"),
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn invitation_object_kind_is_exact_on_brownfield_and_fresh_catalogs() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    MIGRATOR
        .run_to(34, &pool)
        .await
        .expect("apply brownfield migrations through 0034");
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire brownfield probe session");
    create_probe_tables(&mut connection).await;
    assert_kind_behavior(&mut connection, &[], &[7, 8, 9, 10]).await;
    drop_probe_tables(&mut connection).await;
    drop(connection);

    MIGRATOR
        .run_to(36, &pool)
        .await
        .expect("upgrade brownfield catalog through 0036");
    assert_eq!(applied_versions(&pool).await.last().copied(), Some(36));
    assert_exact_catalog(&pool, &[1, 2, 3, 4, 5, 6, 9]).await;
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire upgraded probe session");
    create_probe_tables(&mut connection).await;
    assert_kind_behavior(&mut connection, &[9], &[7, 8, 10]).await;
    drop_probe_tables(&mut connection).await;
    drop(connection);

    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply Invitation object kind on a fresh database");
    assert_exact_catalog(&pool, &[1, 2, 3, 4, 5, 6, 7, 8, 9]).await;
    let mut connection = pool.acquire().await.expect("acquire final probe session");
    create_probe_tables(&mut connection).await;
    assert_kind_behavior(&mut connection, &[7, 8, 9], &[10]).await;
    drop_probe_tables(&mut connection).await;
}
