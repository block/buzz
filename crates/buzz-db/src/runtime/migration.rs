//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in additive SQL files under
//! `migrations/`. The multi-tenant rewrite begins from a clean consolidated
//! `0001`; legacy single-tenant cutover/backfill is a separate operator script,
//! not startup migration state.

use std::future::Future;

use sqlx::{Connection, PgConnection, PgPool};

use crate::deletion::SCHEMA_DESTRUCTION_LOCK_KEY;
use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Run all pending Buzz database migrations.
///
/// The entire run holds the exclusive [`SCHEMA_DESTRUCTION_LOCK_KEY`] session
/// lock, serializing schema changes against destructive deletion transactions
/// (which take the shared counterpart while they validate the live catalog
/// and act on it). Every migration statement executes on the same backend
/// that owns the lock — see [`with_exclusive_schema_destruction_lock`] for
/// why that binding, not the explicit unlock, is the safety contract.
/// Migration execution must never bypass this wrapper — a source lint
/// (`migration_execution_cannot_bypass_schema_destruction_lock`) enforces
/// that `MIGRATOR.run` has no other call site.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    with_exclusive_schema_destruction_lock(pool, |mut conn| async move {
        let outcome = run_migrations_locked(&mut conn).await;
        (conn, outcome)
    })
    .await
}

#[cfg(test)]
pub(crate) async fn run_migrations_through(pool: &PgPool, target: i64) -> Result<()> {
    with_exclusive_schema_destruction_lock(pool, |mut conn| async move {
        let outcome = async {
            reject_legacy_nip_rs_cardinality_ambiguity(&mut conn).await?;
            MIGRATOR.run_to(target, &mut conn).await?;
            Ok(())
        }
        .await;
        (conn, outcome)
    })
    .await
}

async fn run_migrations_locked(conn: &mut PgConnection) -> Result<()> {
    reject_legacy_nip_rs_cardinality_ambiguity(conn).await?;
    MIGRATOR.run(&mut *conn).await?;
    // The replica-fence proof (see `replica_fence`) requires the commit-time
    // `created_at` floor trigger from migration 0021 — correctly shaped — on
    // the `events` parent and every partition. `CREATE TABLE .. PARTITION OF`
    // clones parent triggers, but a partition attached with `ATTACH
    // PARTITION` or created by an older code path would silently escape the
    // guard, so migration fails closed if any is missing. (The fence probe
    // re-runs this same check at startup on non-migrating relays.)
    crate::replica_fence::verify_floor_guard_catalog(&mut *conn).await?;
    crate::channel::verify_channel_roster_fence_catalog(&mut *conn).await?;
    Ok(())
}

/// Run `op` while holding the exclusive schema/destruction session lock.
///
/// `op` receives ownership of the detached connection that owns the advisory
/// lock and must run every statement on it, handing the same connection back
/// with its outcome. That same-backend lifetime — not the explicit unlock —
/// is the safety contract: PostgreSQL releases a session lock only when its
/// backend finishes, so cancelling this future (dropping the connection while
/// a migration statement is still executing server-side) cannot expose the
/// lock to shared destructive holders before that statement's backend
/// terminates. On completion the lock is explicitly released on the returned
/// connection (success and error alike) and the connection is closed, never
/// returning a locked session to the pool.
pub(crate) async fn with_exclusive_schema_destruction_lock<T, F, Fut>(
    pool: &PgPool,
    op: F,
) -> Result<T>
where
    F: FnOnce(PgConnection) -> Fut,
    Fut: Future<Output = (PgConnection, Result<T>)>,
{
    let mut lock_conn = crate::observability::acquire_writer_with_legacy_metrics(
        pool,
        crate::observability::WriterOperation::Bootstrap,
    )
    .await?
    .detach();
    // This dedicated connection intentionally waits for the current migration
    // or schema-destruction owner and may then run long DDL. Exempt those two
    // phases from runtime lock/statement budgets. Keep the idle-in-transaction
    // timeout: a client wedged idle mid-migration is still a lock holder that
    // should be reaped. The detached connection is closed below and never
    // returns these session settings to the pool.
    sqlx::raw_sql("SET lock_timeout = 0; SET statement_timeout = 0")
        .execute(&mut lock_conn)
        .await?;
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::MigrationSchemaSafety,
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
            .execute(&mut lock_conn),
    )
    .await?;
    let (mut lock_conn, outcome) = op(lock_conn).await;
    let unlock = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
        .execute(&mut lock_conn)
        .await;
    let _ = lock_conn.close().await;
    let value = outcome?;
    unlock?;
    Ok(value)
}

/// Migration 0007 is checksum-frozen and predates exact NIP-RS tag-cardinality
/// enforcement. A populated database still on 0001-0006 must not let 0007
/// irreversibly purge duplicate-tag history. Fail before sqlx starts its
/// migration transaction so an operator can inspect and repair those rows.
async fn reject_legacy_nip_rs_cardinality_ambiguity(conn: &mut PgConnection) -> Result<()> {
    let migrations_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
            .fetch_one(&mut *conn)
            .await?;
    if migrations_table.is_none() {
        return Ok(());
    }
    let applied: Option<i64> =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut *conn)
            .await?;
    if applied.is_none_or(|version| version >= 7) {
        return Ok(());
    }

    let ambiguous: bool = sqlx::query_scalar(
        "SELECT EXISTS (\
             SELECT 1 FROM events e \
             WHERE e.kind = 30078 \
               AND e.d_tag ~ '^read-state:[0-9a-f]{32}$' \
               AND (\
                   jsonb_typeof(e.tags) IS DISTINCT FROM 'array' \
                   OR (\
                       EXISTS (\
                           SELECT 1 FROM jsonb_array_elements(\
                               CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END\
                           ) tag \
                           WHERE tag = '[\"t\", \"read-state\"]'::jsonb\
                       ) \
                       AND (\
                           (SELECT count(*) FROM jsonb_array_elements(\
                               CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END\
                            ) tag \
                            WHERE jsonb_typeof(tag) = 'array' \
                              AND tag->0 = '\"d\"'::jsonb) <> 1 \
                           OR NOT EXISTS (\
                               SELECT 1 FROM jsonb_array_elements(\
                                   CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END\
                               ) tag \
                               WHERE jsonb_typeof(tag) = 'array' \
                                 AND jsonb_array_length(tag) >= 2 \
                                 AND jsonb_typeof(tag->1) = 'string' \
                                 AND tag->>0 = 'd' \
                                 AND tag->>1 = e.d_tag\
                           ) \
                           OR (SELECT count(*) FROM jsonb_array_elements(\
                               CASE WHEN jsonb_typeof(e.tags) = 'array' THEN e.tags ELSE '[]'::jsonb END\
                           ) tag WHERE tag = '[\"t\", \"read-state\"]'::jsonb) <> 1\
                       )\
                   )\
               )\
         )",
    )
    .fetch_one(conn)
    .await?;

    if ambiguous {
        return Err(crate::DbError::InvalidData(
            "NIP-RS migration blocked: pre-0007 database contains kind-30078 rows with ambiguous d/t tag cardinality; repair or remove those nonconforming rows before retrying"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
    };

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    /// Connection parameters parsed out of a PostgreSQL URL so the parity test
    /// can pass them to the `bin/pgschema` binary, which
    /// takes discrete `--host/--port/--user/--password/--db` flags rather than a
    /// URL. Only the shapes this test emits (`BUZZ_TEST_DATABASE_URL` /
    /// `DATABASE_URL` / `TEST_DB_URL`) are supported.
    struct PgConn {
        host: String,
        port: u16,
        user: String,
        password: String,
    }

    fn parse_pg_url(url: &str) -> PgConn {
        let opts: sqlx::postgres::PgConnectOptions =
            url.parse().expect("parse postgres connection url");
        PgConn {
            host: opts.get_host().to_owned(),
            port: opts.get_port(),
            user: opts.get_username().to_owned(),
            password: parse_pg_password(url),
        }
    }

    /// `PgConnectOptions` intentionally does not expose the password via a
    /// getter, so read it straight out of the URL authority. Falls back to the
    /// `PGPASSWORD` env var, then empty.
    fn parse_pg_password(url: &str) -> String {
        url.split_once("://")
            .and_then(|(_, rest)| rest.split_once('@'))
            .map(|(authority, _)| authority)
            .and_then(|authority| authority.split_once(':'))
            .map(|(_, pass)| pass.to_owned())
            .or_else(|| std::env::var("PGPASSWORD").ok())
            .unwrap_or_default()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ConstraintKind {
        ForeignKey,
        PrimaryKey,
        Unique,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ConstraintLint {
        table: String,
        kind: ConstraintKind,
        description: String,
        columns: Vec<String>,
    }

    /// Concatenated SQL of every embedded migration, in version order.
    ///
    /// The tenant-isolation lints must cover objects introduced by *any*
    /// migration, not just the consolidated `0001`. Concatenating keeps that
    /// coverage honest as additive migrations (e.g. `0002_git_repo_names`) land.
    fn migration_sql() -> String {
        let mut migrations: Vec<_> = MIGRATOR.iter().collect();
        migrations.sort_by_key(|migration| migration.version);
        assert!(
            !migrations.is_empty(),
            "at least the initial migration must exist"
        );
        migrations
            .iter()
            .map(|migration| migration.sql.as_ref())
            .collect::<Vec<&str>>()
            .join("\n")
    }

    fn strip_sql_comments(sql: &str) -> String {
        sql.lines()
            .map(|line| line.split_once("--").map_or(line, |(before, _)| before))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn normalize_sql(sql: &str) -> String {
        strip_sql_comments(sql)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    fn split_sql_statements(sql: &str) -> Vec<String> {
        let sql = strip_sql_comments(sql);
        let bytes = sql.as_bytes();
        let mut statements = Vec::new();
        let mut start = 0usize;
        let mut idx = 0usize;
        let mut in_single_quote = false;
        let mut in_dollar_quote = false;

        while idx < bytes.len() {
            match bytes[idx] {
                b'\'' if !in_dollar_quote => {
                    in_single_quote = !in_single_quote;
                    idx += 1;
                }
                b'$' if !in_single_quote && idx + 1 < bytes.len() && bytes[idx + 1] == b'$' => {
                    in_dollar_quote = !in_dollar_quote;
                    idx += 2;
                }
                b';' if !in_single_quote && !in_dollar_quote => {
                    let statement = sql[start..idx].trim();
                    if !statement.is_empty() {
                        statements.push(statement.to_owned());
                    }
                    start = idx + 1;
                    idx += 1;
                }
                _ => idx += 1,
            }
        }

        let tail = sql[start..].trim();
        if !tail.is_empty() {
            statements.push(tail.to_owned());
        }

        statements
    }

    fn find_matching_paren(sql: &str, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (offset, byte) in sql.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(open + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn split_top_level_csv(input: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut depth = 0usize;
        for (idx, byte) in input.bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    parts.push(input[start..idx].trim().to_owned());
                    start = idx + 1;
                }
                _ => {}
            }
        }
        let tail = input[start..].trim();
        if !tail.is_empty() {
            parts.push(tail.to_owned());
        }
        parts
    }

    fn identifier_after_keyword(statement: &str, keyword: &str) -> Option<String> {
        let lower = statement.to_ascii_lowercase();
        let keyword_pos = lower.find(keyword)?;
        let mut remainder = statement[keyword_pos + keyword.len()..].trim_start();
        for prefix in ["if not exists", "if exists", "only"] {
            if remainder.to_ascii_lowercase().starts_with(prefix) {
                remainder = remainder[prefix.len()..].trim_start();
            }
        }

        let identifier = remainder
            .split(|ch: char| ch.is_whitespace() || ch == '(')
            .next()?
            .trim_matches('"')
            .rsplit('.')
            .next()?
            .trim_matches('"')
            .to_ascii_lowercase();
        (!identifier.is_empty()).then_some(identifier)
    }

    fn first_parenthesized_columns(input: &str) -> Vec<String> {
        let Some(open) = input.find('(') else {
            return Vec::new();
        };
        let Some(close) = find_matching_paren(input, open) else {
            return Vec::new();
        };

        split_top_level_csv(&input[open + 1..close])
            .into_iter()
            .filter_map(|column| {
                let name = column
                    .trim()
                    .trim_matches('"')
                    .split_whitespace()
                    .next()?
                    .trim_matches('"')
                    .to_ascii_lowercase();
                (!name.is_empty()).then_some(name)
            })
            .collect()
    }

    fn column_definition_name(definition: &str) -> Option<String> {
        let trimmed = definition.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("constraint ")
            || lower.starts_with("primary key")
            || lower.starts_with("foreign key")
            || lower.starts_with("unique")
            || lower.starts_with("check ")
            || lower.starts_with("exclude ")
        {
            return None;
        }

        let name = trimmed
            .split_whitespace()
            .next()?
            .trim_matches('"')
            .to_ascii_lowercase();
        (!name.is_empty()).then_some(name)
    }

    fn create_table_body(statement: &str) -> Option<(String, Vec<String>)> {
        let table = identifier_after_keyword(statement, "create table")?;
        let open = statement.find('(')?;
        let close = find_matching_paren(statement, open)?;
        Some((table, split_top_level_csv(&statement[open + 1..close])))
    }

    fn create_table_definitions(sql: &str) -> Vec<(String, Vec<String>)> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                let normalized = statement.trim_start().to_ascii_lowercase();
                if !normalized.starts_with("create table") || normalized.contains(" partition of ")
                {
                    return None;
                }
                create_table_body(&statement)
            })
            .collect()
    }

    fn create_tables(sql: &str) -> BTreeSet<String> {
        create_table_definitions(sql)
            .into_iter()
            .map(|(table, _)| table)
            .collect()
    }

    fn table_has_not_null_community_id(definitions: &[String]) -> bool {
        definitions.iter().any(|definition| {
            column_definition_name(definition).as_deref() == Some("community_id")
                && normalize_sql(definition).contains("not null")
        })
    }

    fn operator_global_tables(sql: &str) -> BTreeSet<String> {
        let mut globals = BTreeSet::new();
        let normalized = normalize_sql(sql);
        let Some(insert_pos) = normalized.find("insert into _operator_global_tables") else {
            return globals;
        };

        for value in [
            "communities",
            "rate_limit_violations",
            "_operator_global_tables",
            "push_gateway_challenges",
            "push_gateway_installations",
            "push_gateway_delegations",
            "push_gateway_endpoint_quotas",
            "push_gateway_delivery_auth_replays",
            "push_gateway_delivery_request_replays",
            "product_feedback",
            "replica_heartbeat",
            "community_deletion_requests",
            "community_deletion_approvals",
            "community_deletion_checkpoints",
            "community_deletion_manifest_keys",
            "storage_taxonomy_sweeps",
            "community_serving_write_leases",
            "community_deletion_executor_heartbeats",
            "relay_operators",
            "relay_admin_actions",
            "relay_admin_outbox",
            "relay_operator_audit",
        ] {
            if normalized[insert_pos..].contains(&format!("'{value}'")) {
                globals.insert(value.to_owned());
            }
        }

        globals
    }

    fn scoped_tables(sql: &str) -> BTreeSet<String> {
        let globals = operator_global_tables(sql);
        create_tables(sql)
            .into_iter()
            .filter(|table| !globals.contains(table))
            .collect()
    }

    fn constraint_lint_for_definition(table: &str, definition: &str) -> Option<ConstraintLint> {
        let normalized = normalize_sql(definition);
        let definition_without_name = if normalized.starts_with("constraint ") {
            let after_constraint = definition
                .trim_start()
                .splitn(3, char::is_whitespace)
                .nth(2)
                .unwrap_or("");
            normalize_sql(after_constraint)
        } else {
            normalized.clone()
        };

        if definition_without_name.starts_with("primary key") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::PrimaryKey,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if definition_without_name.starts_with("unique") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::Unique,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if definition_without_name.starts_with("foreign key") {
            Some(ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::ForeignKey,
                description: definition.to_owned(),
                columns: first_parenthesized_columns(&definition_without_name),
            })
        } else if normalized.contains(" primary key") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::PrimaryKey,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else if normalized.contains(" references ") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::ForeignKey,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else if normalized.contains(" unique") {
            column_definition_name(definition).map(|column| ConstraintLint {
                table: table.to_owned(),
                kind: ConstraintKind::Unique,
                description: definition.to_owned(),
                columns: vec![column],
            })
        } else {
            None
        }
    }

    fn table_constraints(sql: &str, scoped_tables: &BTreeSet<String>) -> Vec<ConstraintLint> {
        create_table_definitions(sql)
            .into_iter()
            .filter(|(table, _)| scoped_tables.contains(table))
            .flat_map(|(table, definitions)| {
                definitions.into_iter().filter_map(move |definition| {
                    constraint_lint_for_definition(&table, &definition)
                })
            })
            .collect()
    }

    fn alter_table_constraints(sql: &str, scoped_tables: &BTreeSet<String>) -> Vec<ConstraintLint> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                let normalized = normalize_sql(&statement);
                if !normalized.starts_with("alter table") {
                    return None;
                }

                let table = identifier_after_keyword(&statement, "alter table")?;
                if !scoped_tables.contains(&table) {
                    return None;
                }

                let add_pos = normalized.find(" add ")?;
                let definition = normalized[add_pos + " add ".len()..].trim();
                constraint_lint_for_definition(&table, definition)
            })
            .collect()
    }

    fn unique_indexes(sql: &str, scoped_tables: &BTreeSet<String>) -> Vec<ConstraintLint> {
        split_sql_statements(sql)
            .into_iter()
            .filter_map(|statement| {
                let normalized = normalize_sql(&statement);
                if !normalized.starts_with("create unique index") {
                    return None;
                }

                let lower_statement = statement.to_ascii_lowercase();
                let on_pos = lower_statement.find(" on ")?;
                let table = statement[on_pos + " on ".len()..]
                    .trim_start()
                    .split(|ch: char| ch.is_whitespace() || ch == '(')
                    .next()?
                    .trim_matches('"')
                    .rsplit('.')
                    .next()?
                    .trim_matches('"')
                    .to_ascii_lowercase();

                scoped_tables.contains(&table).then(|| ConstraintLint {
                    table,
                    kind: ConstraintKind::Unique,
                    description: statement.clone(),
                    columns: first_parenthesized_columns(&statement[on_pos + " on ".len()..]),
                })
            })
            .collect()
    }

    fn scoped_constraint_lints(sql: &str, scoped_tables: &BTreeSet<String>) -> Vec<ConstraintLint> {
        let mut constraints = table_constraints(sql, scoped_tables);
        constraints.extend(alter_table_constraints(sql, scoped_tables));
        constraints.extend(unique_indexes(sql, scoped_tables));
        constraints
    }

    fn is_allowed_partition_primary_key_exception(constraint: &ConstraintLint) -> bool {
        constraint.table == "delivery_log"
            && constraint.kind == ConstraintKind::PrimaryKey
            && constraint.columns == ["delivered_at", "id"]
    }

    fn scoped_constraint_violations(sql: &str) -> Vec<ConstraintLint> {
        let scoped_tables = scoped_tables(sql);
        scoped_constraint_lints(sql, &scoped_tables)
            .into_iter()
            .filter(|constraint| {
                if is_allowed_partition_primary_key_exception(constraint) {
                    return false;
                }
                constraint.columns.first().map(String::as_str) != Some("community_id")
            })
            .collect()
    }

    fn has_channels_community_id_immutability_guard(sql: &str) -> bool {
        let normalized = normalize_sql(sql);
        normalized.contains("create trigger")
            && normalized.contains("before update")
            && normalized.contains(" on channels")
            && normalized.contains("community_id")
            && normalized.contains("old.community_id")
            && normalized.contains("new.community_id")
            && normalized.contains("raise exception")
    }

    fn forbidden_channels_community_id_mutations(sql: &str) -> Vec<String> {
        split_sql_statements(sql)
            .into_iter()
            .filter(|statement| {
                let normalized = normalize_sql(statement);
                let updates_channels =
                    identifier_after_keyword(statement, "update").as_deref() == Some("channels");
                let update_assignments = normalized
                    .split_once(" set ")
                    .map(|(_, tail)| tail.split_once(" where ").map_or(tail, |(set, _)| set));
                let mutates_with_update = updates_channels
                    && update_assignments
                        .is_some_and(|assignments| assignments.contains("community_id"));
                let alters_channels = identifier_after_keyword(statement, "alter table").as_deref()
                    == Some("channels");
                let drops_channels = identifier_after_keyword(statement, "drop table").as_deref()
                    == Some("channels");
                let drops_or_rewrites_column = alters_channels
                    && (normalized.contains("drop column community_id")
                        || normalized.contains("alter column community_id")
                        || normalized.contains("rename column community_id")
                        || normalized.contains("rename community_id")
                        || normalized.contains("drop trigger")
                        || normalized.contains("disable trigger"));

                mutates_with_update || drops_or_rewrites_column || drops_channels
            })
            .collect()
    }

    #[test]
    fn embedded_migrator_contains_consolidated_initial_schema() {
        let mut migrations: Vec<_> = MIGRATOR.iter().collect();
        migrations.sort_by_key(|migration| migration.version);

        assert_eq!(migrations.len(), 44);
        assert_eq!(migrations[0].version, 1);
        assert_eq!(&*migrations[0].description, "initial schema");
        assert!(migrations[0]
            .sql
            .as_str()
            .contains("CREATE TABLE communities"));
        assert!(migrations[0].sql.as_str().contains("CREATE TABLE channels"));
        assert!(migrations[0]
            .sql
            .as_str()
            .contains("CREATE TABLE scheduled_workflow_fires"));
        assert!(migrations[0]
            .sql
            .as_str()
            .contains("CREATE TABLE audit_log"));
        assert!(migrations[0]
            .sql
            .as_str()
            .contains("CREATE TABLE _operator_global_tables"));
        assert!(migrations[0]
            .sql
            .as_str()
            .contains("search_tsv  TSVECTOR GENERATED ALWAYS"));

        // The git repo-name registry is an additive migration, never folded into
        // 0001 — folding it would change 0001's checksum and break brownfield
        // startup (sqlx VersionMismatch). It must live in its own version, and
        // 0001 must not carry it.
        assert_eq!(migrations[1].version, 2);
        assert!(migrations[1]
            .sql
            .as_str()
            .contains("CREATE TABLE git_repo_names"));
        assert!(!migrations[0].sql.as_str().contains("git_repo_names"));

        // Same additive-migration rule for the per-community workspace icon
        // (NIP-11 `icon`): its own version, never folded into 0001.
        assert_eq!(migrations[2].version, 3);
        assert!(migrations[2]
            .sql
            .as_str()
            .contains("ALTER TABLE communities ADD COLUMN icon"));
        assert!(!migrations[0].sql.as_str().contains("icon"));
        // Same additive-migration rule for the e-tag containment GIN index
        // (channel-window aux closure): its own version, never folded into 0001.
        assert_eq!(migrations[3].version, 4);
        assert!(migrations[3]
            .sql
            .as_str()
            .contains("CREATE INDEX idx_events_tags_gin"));
        assert!(!migrations[0].sql.as_str().contains("idx_events_tags_gin"));

        // NIP-AM (kind 44200) FTS exclusion: additive migration, never folded
        // into 0001 — folding would change 0001's checksum and break brownfield
        // startup. Migration 5 drops and re-adds the generated `search_tsv`
        // column with the extended kind-44200 exclusion. 0001 must NOT carry 44200.
        assert_eq!(migrations[4].version, 5);
        assert!(migrations[4].sql.as_str().contains("search_tsv"));
        assert!(migrations[4].sql.as_str().contains("44200"));
        assert!(!migrations[0].sql.as_str().contains("44200"));

        // Community moderation (reports/bans/audit): additive migration, never
        // folded into 0001 — same brownfield checksum rule as above.
        assert_eq!(migrations[5].version, 6);
        assert!(migrations[5]
            .sql
            .as_str()
            .contains("CREATE TABLE moderation_reports"));
        assert!(migrations[5]
            .sql
            .as_str()
            .contains("CREATE TABLE community_bans"));
        assert!(migrations[5]
            .sql
            .as_str()
            .contains("CREATE TABLE moderation_actions"));
        for action in crate::moderation::MODERATION_ACTION_CHECK_VOCAB {
            assert!(
                migrations[5].sql.as_str().contains(&format!("'{action}'")),
                "migration 0006 moderation_actions.action CHECK must allow {action}"
            );
        }
        assert!(!migrations[0].sql.as_str().contains("moderation_reports"));
        // NIP-RS retention is additive and boot-safe: seed replay watermarks
        // before deleting payload history, without rewriting search storage.
        assert_eq!(migrations[6].version, 7);
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("LOCK TABLE events IN SHARE ROW EXCLUSIVE MODE"));
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("CREATE TABLE parameterized_event_watermarks"));
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("INSERT INTO parameterized_event_watermarks"));
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("CREATE INDEX idx_event_mentions_community_event"));
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("NIP-RS retention blocked: deleted event outranks live head"));
        assert!(migrations[6]
            .sql
            .as_str()
            .contains("DELETE FROM events old"));
        assert!(!migrations[6]
            .sql
            .as_str()
            .contains("ALTER TABLE events DROP COLUMN search_tsv"));

        // Fresh installs opt into the positive search allowlist without making
        // populated databases rewrite their events heap during relay startup.
        assert_eq!(migrations[7].version, 8);
        assert!(migrations[7]
            .sql
            .as_str()
            .contains("IF NOT EXISTS (SELECT 1 FROM events LIMIT 1)"));
        assert!(migrations[7]
            .sql
            .as_str()
            .contains("CASE WHEN kind IN (0, 9, 40002, 45001, 45003)"));
        assert!(migrations[7].sql.as_str().contains("ELSE NULL::tsvector"));

        // Mixed-version guards are additive because 0007/0008 may already be
        // recorded by a running relay and their sqlx checksums are immutable.
        assert_eq!(migrations[8].version, 9);
        assert!(migrations[8]
            .sql
            .as_str()
            .contains("CREATE TRIGGER trg_events_nip_rs_watermark"));
        assert!(migrations[8]
            .sql
            .as_str()
            .contains("stale NIP-RS event rejected by durable watermark"));
        assert!(migrations[8]
            .sql
            .as_str()
            .contains("CREATE TRIGGER trg_events_purge_soft_deleted_nip_rs"));
        assert!(migrations[8]
            .sql
            .as_str()
            .contains("CREATE TRIGGER trg_event_mentions_require_live_event"));

        assert_eq!(migrations[9].version, 10);
        assert!(migrations[9]
            .sql
            .as_str()
            .contains("CREATE OR REPLACE FUNCTION guard_nip_rs_watermark"));
        assert!(migrations[9].sql.as_str().contains("RETURN NULL"));

        assert_eq!(migrations[10].version, 11);
        assert!(migrations[10]
            .sql
            .as_str()
            .contains("CREATE OR REPLACE FUNCTION guard_nip_rs_watermark"));
        assert!(migrations[10]
            .sql
            .as_str()
            .contains("CREATE OR REPLACE FUNCTION purge_soft_deleted_nip_rs"));
        assert!(migrations[10].sql.as_str().contains("tag->>0 = 'd'"));
        assert!(migrations[10].sql.as_str().contains(") = 1"));

        // Push leases and their durable outbox are relay-owned and structurally
        // community-scoped; the public gateway remains stateless.
        assert_eq!(migrations[11].version, 12);
        assert!(migrations[11]
            .sql
            .as_str()
            .contains("CREATE TABLE push_leases"));
        assert!(migrations[11]
            .sql
            .as_str()
            .contains("CREATE TABLE push_wake_outbox"));
        assert!(migrations[11]
            .sql
            .as_str()
            .contains("PRIMARY KEY (community_id, author, installation_id)"));
        assert!(!migrations[0].sql.as_str().contains("push_leases"));

        assert_eq!(migrations[12].version, 13);
        assert!(migrations[12]
            .sql
            .as_str()
            .contains("ADD COLUMN endpoint_enabled"));

        // Kind 30350 is author-only encrypted data, so its ciphertext is never
        // indexed for NIP-50 search. Preserve the 0001 checksum and extend the
        // generated expression additively.
        assert_eq!(migrations[13].version, 14);
        assert!(migrations[13].sql.as_str().contains("30350"));
        assert!(migrations[13].sql.as_str().contains("search_tsv"));
        assert!(!migrations[0].sql.as_str().contains("30350"));

        // NIP-PMA kind:30179 FTS exclusion (0033): same wrap-the-existing-
        // expression shape as 0014 so brownfield databases stop tokenizing
        // private managed-agent ciphertext without a policy rewrite. (The
        // migration itself still rewrites the events heap and rebuilds the
        // GIN index — see the 0033 header for the operational cost.)
        assert_eq!(migrations[32].version, 33);
        assert!(migrations[32].sql.as_str().contains("kind = 30179"));
        assert!(migrations[32].sql.as_str().contains("search_tsv"));
        assert!(!migrations[0].sql.as_str().contains("30179"));
        assert!(include_str!("../../../../schema/schema.sql")
            .contains("kind IN (1059, 30179, 30300, 30350, 30622, 44100, 44101, 44200)"));

        // Public push-gateway authority is intentionally deployment-global and
        // durable: immediate revocation and hostile-relay admission cannot be
        // honestly provided by a stateless gateway.
        assert_eq!(migrations[14].version, 15);
        assert!(migrations[14]
            .sql
            .as_str()
            .contains("CREATE TABLE push_gateway_installations"));
        assert!(migrations[14]
            .sql
            .as_str()
            .contains("push_gateway_delegations"));
        assert!(migrations[14]
            .sql
            .as_str()
            .contains("_operator_global_tables"));

        // Community archival and product feedback landed concurrently. Keep
        // both additive migrations in a single, unambiguous sequence.
        assert_eq!(migrations[15].version, 16);
        assert!(migrations[15]
            .sql
            .as_str()
            .contains("ADD COLUMN archived_at"));

        // Product feedback is a deployment-private sidecar; community_id is
        // provenance, not an operator-review authorization boundary.
        assert_eq!(migrations[16].version, 17);
        assert!(migrations[16]
            .sql
            .as_str()
            .contains("CREATE TABLE product_feedback"));
        assert!(migrations[16]
            .sql
            .as_str()
            .contains("community_id UUID NOT NULL"));
        assert!(migrations[16]
            .sql
            .as_str()
            .contains("('product_feedback', 'deployment product inbox"));
        assert!(!migrations[0].sql.as_str().contains("product_feedback"));

        // Matching is driven from a parent-table trigger so all partition and
        // internal insertion paths share the same crash-safe allowlist seam.
        assert_eq!(migrations[17].version, 18);
        let matcher = migrations[17].sql.as_str();
        assert!(matcher.contains("CREATE TABLE push_match_queue"));
        assert!(matcher.contains("AFTER INSERT ON events"));
        assert!(matcher.contains("NEW.kind IN (7, 9, 1059, 40007, 46010)"));
        assert!(!migrations[0].sql.as_str().contains("push_match_queue"));

        // Mesh status is a heartbeat, not an audit stream. The additive
        // migration removes accumulated soft-deleted payloads and covers old
        // writers during rolling deploys without changing kind:30003 broadly.
        assert_eq!(migrations[18].version, 19);
        let mesh_retention = migrations[18].sql.as_str();
        assert!(mesh_retention.contains("buzz-mesh-member-status:%"));
        assert!(mesh_retention.contains("buzz-mesh-status"));
        assert!(mesh_retention
            .contains("CREATE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status"));
        assert!(!migrations[0]
            .sql
            .as_str()
            .contains("purge_soft_deleted_buzz_mesh_status"));

        // Join policy acceptances landed concurrently with mesh status retention;
        // keep both additive migrations in a single, unambiguous sequence.
        assert_eq!(migrations[19].version, 20);
        assert!(migrations[19]
            .sql
            .as_str()
            .contains("CREATE TABLE join_policy_acceptances"));

        // Replica-fence commit-time floor guard on channel-bearing events.
        assert_eq!(migrations[20].version, 21);
        assert!(migrations[20]
            .sql
            .as_str()
            .contains("events_created_at_floor_guard"));
        assert!(!migrations[0]
            .sql
            .as_str()
            .contains("join_policy_acceptances"));

        // Channel TTL refresh belongs to the event insertion transaction so a
        // concurrent permanent -> ephemeral transition cannot be missed.
        assert_eq!(migrations[21].version, 22);
        let ttl_refresh = migrations[21].sql.as_str();
        assert!(ttl_refresh.contains("CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl"));
        assert!(ttl_refresh.contains("AFTER INSERT ON events"));
        assert!(ttl_refresh.contains("DEFERRABLE INITIALLY DEFERRED"));
        assert!(ttl_refresh.contains("clock_timestamp()"));
        assert!(ttl_refresh.contains("NEW.kind <> 9007"));

        // T1b push gate: the match-queue trigger only enqueues when the
        // community has an eligible lease, ordered against lease activations
        // through the shared/exclusive per-community advisory lock.
        assert_eq!(migrations[22].version, 23);
        let push_gate = migrations[22].sql.as_str();
        assert!(push_gate.contains("CREATE OR REPLACE FUNCTION enqueue_push_match_job"));
        assert!(push_gate.contains("pg_advisory_xact_lock_shared"));
        assert!(push_gate.contains("'buzz_push_gate:' || NEW.community_id::text"));
        assert!(push_gate.contains("endpoint_enabled"));

        // T1a repair: the TTL refresh trigger synchronizes on a shared
        // per-channel advisory lock instead of FOR UPDATE on the channel row,
        // so permanent-channel commits no longer serialize.
        assert_eq!(migrations[23].version, 24);
        let ttl_shared = migrations[23].sql.as_str();
        assert!(ttl_shared
            .contains("CREATE OR REPLACE FUNCTION refresh_channel_ttl_after_event_insert"));
        assert!(ttl_shared.contains("pg_advisory_xact_lock_shared"));
        assert!(ttl_shared.contains("'buzz_channel_ttl:' || NEW.community_id::text"));
        // The row read must be a bare SELECT (comments describe the removed
        // FOR UPDATE; the executable body must not reintroduce it).
        assert!(ttl_shared.contains("SELECT ttl_seconds INTO channel_ttl"));
        assert!(!strip_sql_comments(ttl_shared)
            .to_lowercase()
            .contains("for update"));
        assert!(ttl_shared.contains("NEW.kind <> 9007"));

        // Use-limited invite links: durable relay_invites table stores only
        // the SHA-256 of an opaque v2 code, scoped by community_id. Never
        // listed in _operator_global_tables — it is community-scoped.
        assert_eq!(migrations[24].version, 25);
        let relay_invites = migrations[24].sql.as_str();
        assert!(relay_invites.contains("CREATE TABLE relay_invites"));
        assert!(relay_invites
            .contains("token_hash   BYTEA       NOT NULL CHECK (length(token_hash) = 32)"));
        assert!(relay_invites.contains("PRIMARY KEY (community_id, id)"));
        assert!(relay_invites.contains("UNIQUE (community_id, token_hash)"));
        assert!(
            relay_invites.contains("max_uses     INTEGER     CHECK (max_uses BETWEEN 1 AND 10000)")
        );
        assert!(relay_invites.contains("CHECK (max_uses IS NULL OR use_count <= max_uses)"));
        assert!(relay_invites.contains("role = 'member'"));
        assert!(relay_invites
            .contains("CREATE INDEX relay_invites_expires_at_idx ON relay_invites (expires_at)"));
        assert!(!relay_invites.contains("_operator_global_tables"));

        let desired_schema = include_str!("../../../../schema/schema.sql");
        assert!(
            desired_schema.contains("CREATE TABLE join_policy_acceptances"),
            "desired-state schema must include join-policy evidence used by invite claims",
        );

        // Replica heartbeat (this branch, renumbered to 0026 after
        // 0025_relay_invites landed on main): the fence's portable read-side
        // observation. A single CHECK'd row makes the token update the
        // serialization point (multi-pod commit ordering), and the epoch
        // column is what detects token resets — both are load-bearing for
        // the routing proof.
        assert_eq!(migrations[25].version, 26);
        let heartbeat = migrations[25].sql.as_str();
        assert!(heartbeat.contains("CREATE TABLE replica_heartbeat"));
        assert!(heartbeat.contains("CHECK (id = 1)"));
        assert!(heartbeat.contains("epoch"));
        assert!(heartbeat.contains("INSERT INTO replica_heartbeat (id) VALUES (1)"));
        assert!(heartbeat.contains("_operator_global_tables"));

        // Channel-id lookup index (0027): serves tenant-independent channel lookups.
        assert_eq!(migrations[26].version, 27);
        let channel_id_index = migrations[26].sql.as_str();
        assert!(channel_id_index.contains("idx_channels_id_live"));
        assert!(channel_id_index.contains("INCLUDE (community_id)"));
        assert!(channel_id_index.contains("WHERE deleted_at IS NULL"));
        assert!(!channel_id_index.contains("CREATE UNIQUE INDEX"));
        assert!(desired_schema.contains("idx_channels_id_live"));

        // Main owns 0028 for long reaction payloads.
        assert_eq!(migrations[27].version, 28);
        let long_reactions = migrations[27].sql.as_str();
        assert!(
            long_reactions.contains("ALTER TABLE reactions ALTER COLUMN emoji TYPE VARCHAR(66)")
        );
        assert!(desired_schema.contains("emoji               VARCHAR(66) NOT NULL"));

        // Durable whole-community deletion control plane and universal DB fence.
        assert_eq!(migrations[28].version, 29);
        let deletion = migrations[28].sql.as_str();
        assert!(deletion.contains("CREATE TABLE community_deletion_requests"));
        assert!(deletion.contains("CREATE TABLE community_deletion_approvals"));
        assert!(deletion.contains("CREATE TABLE community_deletion_checkpoints"));
        assert!(deletion.contains("CREATE TABLE community_serving_write_leases"));
        assert!(deletion.contains("CREATE TABLE community_deletion_executor_heartbeats"));
        assert!(deletion.contains("CREATE FUNCTION community_write_allowed"));
        assert!(deletion.contains("LANGUAGE plpgsql VOLATILE"));
        assert!(deletion.contains("CREATE FUNCTION assert_community_write_allowed"));
        assert!(deletion.contains("current_setting('transaction_isolation') <> 'read committed'"));
        assert!(deletion.contains("ERRCODE = 'invalid_transaction_state'"));
        assert!(deletion.contains("CREATE FUNCTION enforce_community_write_fence"));
        assert!(deletion.contains("CREATE FUNCTION attach_community_write_fence"));
        assert!(deletion.contains("community_write_fence_excluded_table"));
        assert!(deletion.contains("CREATE FUNCTION enforce_community_tombstone"));
        assert!(deletion.contains("community tombstones are permanent"));
        assert!(deletion.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(deletion.contains("'active', 'quiescing', 'fenced', 'tombstone'"));
        assert!(deletion.contains("_operator_global_tables"));
        assert!(deletion.contains("'submitted', 'inventoried', 'approved', 'fenced', 'drained'"));
        assert!(deletion.contains("UNIQUE (id, community_id, inventory_digest)"));
        assert!(deletion.contains("FOREIGN KEY (request_id, community_id, inventory_digest)"));
        assert!(deletion.contains("prevent_community_deletion_request_retargeting"));
        assert!(deletion.contains("prevent_community_deletion_approval_removal"));

        assert!(deletion.contains("retry_stage TEXT CHECK"));
        assert!(desired_schema.contains("retry_stage TEXT CHECK"));

        // Recovery migration 0030 alters populated tables and must preserve
        // the same fail-fast lock behavior as the deletion migration.
        assert_eq!(migrations[29].version, 30);
        let deletion_recovery = migrations[29].sql.as_str();
        assert!(deletion_recovery.contains("SET LOCAL lock_timeout = '5s'"));

        // Mixed-version channel-roster fence: old canonical replacement writers
        // acquire their replacement key before INSERT; this trigger then takes
        // the membership key and validates the exact active pubkey/role p-tag set.
        assert_eq!(migrations[31].version, 32);
        let roster_fence = migrations[31].sql.as_str();
        assert!(roster_fence.contains("CREATE TRIGGER trg_events_guard_channel_roster_snapshot"));
        assert!(roster_fence.contains("NEW.kind <> 39002"));
        assert!(roster_fence.contains("'buzz_channel_membership:'"));
        assert!(roster_fence.contains("cm.removed_at IS NULL"));
        assert!(roster_fence.contains("cm.role::text"));
        assert!(roster_fence.contains("jsonb_array_length(roster_tag.tag_json) <> 4"));
        assert!(roster_fence.contains("roster_tag.tag_json->>3"));
        assert!(roster_fence.contains("snapshot_members IS DISTINCT FROM canonical_members"));
        assert!(roster_fence.contains("ERRCODE = '23514'"));

        // Fresh desired-state bootstrap must install the identical executable
        // fence as migration 0032. CI and isolated relay startup use schema.sql
        // without running migrations, so drift reopens rolling-deploy races.
        fn extract_roster_fence(sql: &str) -> &str {
            let fence_start = "CREATE OR REPLACE FUNCTION guard_channel_roster_snapshot()";
            let fence_end = "    FOR EACH ROW EXECUTE FUNCTION guard_channel_roster_snapshot();";
            let start = sql.find(fence_start).expect("roster fence function");
            let relative_end = sql[start..].find(fence_end).expect("roster fence trigger");
            &sql[start..start + relative_end + fence_end.len()]
        }
        assert_eq!(
            extract_roster_fence(roster_fence),
            extract_roster_fence(desired_schema)
        );

        // The single-row heartbeat table is updated continuously. Prevent
        // autovacuum from truncating its heap so standby queries are not
        // cancelled by the ACCESS EXCLUSIVE truncation lock replay.
        assert_eq!(migrations[33].version, 34);
        let heartbeat_vacuum = migrations[33].sql.as_str();
        assert!(heartbeat_vacuum.contains("ALTER TABLE replica_heartbeat"));
        assert!(heartbeat_vacuum.contains("vacuum_truncate = false"));
        assert!(desired_schema.contains("vacuum_truncate = false"));

        // pgschema intentionally reconciles DDL, not seed DML or table storage
        // parameters. Its post-apply reconciliation must restore and verify
        // both parts of the live heartbeat contract for fresh bootstraps.
        let pgschema_reconciliation =
            include_str!("../../../../scripts/reconcile-schema-after-pgschema.sql");
        assert!(pgschema_reconciliation
            .contains("ALTER TABLE replica_heartbeat SET (vacuum_truncate = false)"));
        assert!(pgschema_reconciliation.contains("INSERT INTO replica_heartbeat (id) VALUES (1)"));
        assert!(pgschema_reconciliation.contains("ON CONFLICT (id) DO NOTHING"));
        assert!(pgschema_reconciliation.contains("pg_class"));
        assert!(pgschema_reconciliation.contains("reloptions"));

        assert_eq!(migrations[34].version, 35);
        let relay_operators = migrations[34].sql.as_str();
        assert!(
            relay_operators.contains("CREATE TABLE relay_operators"),
            "migration 35 must create relay_operators"
        );
        assert!(
            relay_operators.contains("_operator_global_tables"),
            "migration 35 must register relay_operators in _operator_global_tables"
        );
        assert!(
            relay_operators.contains("actor_authority"),
            "migration 35 must add actor_authority to moderation_actions"
        );
        assert!(
            relay_operators.contains("processing"),
            "migration 35 must add processing status to moderation_reports"
        );

        assert_eq!(migrations[35].version, 36);
        let relay_admin_actions = migrations[35].sql.as_str();
        assert!(
            relay_admin_actions.contains("CREATE TABLE relay_admin_actions"),
            "migration 36 must create relay_admin_actions"
        );
        assert!(
            relay_admin_actions.contains("CREATE TABLE relay_admin_outbox"),
            "migration 36 must create relay_admin_outbox"
        );
        assert!(
            relay_admin_actions.contains("request_id"),
            "migration 36 relay_admin_actions must include request_id for idempotency"
        );
        assert!(
            relay_admin_actions.contains("step_marker"),
            "migration 36 relay_admin_actions must include step_marker for crash recovery"
        );

        assert_eq!(migrations[36].version, 37);
        let action_lease = migrations[36].sql.as_str();
        assert!(
            action_lease.contains("action_lease_token"),
            "migration 37 must add action_lease_token to relay_admin_actions"
        );
        assert!(
            action_lease.contains("action_lease_expires_at"),
            "migration 37 must add action_lease_expires_at to relay_admin_actions"
        );
        assert!(
            action_lease.contains("attempt_count"),
            "migration 37 must add attempt_count to relay_admin_outbox"
        );
        assert!(
            action_lease.contains("retry_after"),
            "migration 37 must add retry_after to relay_admin_outbox"
        );

        assert_eq!(migrations[38].version, 39);
        let operator_audit = migrations[38].sql.as_str();
        assert!(
            operator_audit.contains("CREATE TABLE relay_operator_audit"),
            "migration 39 must create relay_operator_audit"
        );
        assert!(
            operator_audit.contains("_operator_global_tables"),
            "migration 39 must register relay_operator_audit in _operator_global_tables"
        );

        assert_eq!(migrations[40].version, 41);
        let identity_foundation = migrations[40].sql.as_str();
        assert!(identity_foundation.contains("CREATE TABLE identity_bindings"));
        assert!(identity_foundation.contains("CREATE TABLE identity_lifecycle_history"));

        assert_eq!(migrations[41].version, 42);
        let authorization_foundation = migrations[41].sql.as_str();
        assert!(authorization_foundation.contains("CREATE TABLE authorization_events"));
        assert!(authorization_foundation.contains("CREATE TABLE protected_object_authority"));

        // Brownfield relay databases created through SQLx still carry the
        // production/sandbox constraint from 0015. Converge them to the same
        // dogfood-only authority declared by the desired-state schema.
        assert_eq!(migrations[42].version, 43);
        let dogfood_profile = migrations[42].sql.as_str();
        assert!(dogfood_profile.contains("DELETE FROM push_gateway_delegations"));
        assert!(dogfood_profile.contains("DELETE FROM push_gateway_installations"));
        assert!(dogfood_profile
            .contains("DROP CONSTRAINT push_gateway_installations_app_profile_check"));
        assert!(dogfood_profile.contains("CHECK (app_profile = 'buzz-ios-dogfood')"));
        assert!(desired_schema.contains("CHECK (app_profile = 'buzz-ios-dogfood')"));

        // Drop the Phase-A NIP-FI relay-side authority ledger (0041 + 0042).
        // OSS Buzz is stateless for identity (spec v2, PR #7214); the durable
        // ledger tables are dead code. Restores community_write_fence_excluded_table
        // to its pre-0041 body so the deletion catalog no longer includes the
        // removed relations.
        assert_eq!(migrations[43].version, 44);
        let ledger_removal = migrations[43].sql.as_str();
        assert!(ledger_removal.contains("DROP TABLE authorization_operation_receipts"));
        assert!(ledger_removal.contains("DROP TABLE identity_bindings"));
        assert!(ledger_removal.contains("DROP TABLE authorization_events"));
        assert!(ledger_removal
            .contains("CREATE OR REPLACE FUNCTION community_write_fence_excluded_table"));
        // The restored exclusion function must NOT list any NIP-FI relation.
        assert!(!ledger_removal.contains("'authorization_operation_receipts'"));
        assert!(!ledger_removal.contains("'identity_bindings'"));
        // schema.sql exclusion list must match the restored (pre-0041) body.
        assert!(
            desired_schema.contains("'rate_limit_violations'\n    ]::TEXT[])"),
            "schema.sql exclusion list must match the pre-0041 body after ledger removal"
        );
    }

    #[test]
    fn every_pgschema_apply_runs_post_apply_reconciliation() {
        fn files_under(root: &Path) -> Vec<PathBuf> {
            let mut pending = vec![root.to_owned()];
            let mut files = Vec::new();

            while let Some(path) = pending.pop() {
                for entry in fs::read_dir(&path)
                    .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
                {
                    let path = entry.expect("directory entry").path();
                    if path.is_dir() {
                        pending.push(path);
                    } else {
                        files.push(path);
                    }
                }
            }

            files
        }

        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roots = [
            repo_root.join("scripts"),
            repo_root.join(".github/workflows"),
        ];
        let mut apply_count = 0;

        for path in roots.iter().flat_map(|root| files_under(root)) {
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<_> = contents.lines().collect();

            for (index, line) in lines.iter().enumerate() {
                if !line.contains("./bin/pgschema apply") {
                    continue;
                }

                apply_count += 1;
                let following_lines = &lines[index + 1..(index + 7).min(lines.len())];
                assert!(
                    following_lines.iter().any(|line| line.contains(
                        "scripts/reconcile-schema-after-pgschema.sql"
                    )),
                    "{} must run scripts/reconcile-schema-after-pgschema.sql immediately after pgschema apply",
                    path.display()
                );
            }
        }

        assert!(
            apply_count > 0,
            "expected at least one pgschema apply caller"
        );
    }

    #[test]
    fn workflow_run_error_codes_are_additive_and_backfilled_without_parsing_diagnostics() {
        let mut migrations: Vec<_> = MIGRATOR.iter().collect();
        migrations.sort_by_key(|migration| migration.version);

        assert_eq!(migrations[30].version, 31);
        let sql = migrations[30].sql.as_str();
        assert!(sql.contains("ALTER TABLE workflow_runs ADD COLUMN error_code TEXT"));
        assert!(sql.contains("SET error_code = 'legacy_unclassified'"));
        assert!(sql.contains("status IN ('failed', 'cancelled')"));
        assert!(!sql.contains("error_message LIKE"));
        assert!(!MIGRATOR
            .iter()
            .find(|migration| migration.version == 1)
            .expect("initial migration")
            .sql
            .as_str()
            .contains("error_code"));
        assert!(include_str!("../../../../schema/schema.sql").contains("error_code          TEXT"));
    }

    #[test]
    fn push_match_trigger_is_narrowed_to_message_kinds_additively() {
        let mut migrations: Vec<_> = MIGRATOR.iter().collect();
        migrations.sort_by_key(|migration| migration.version);

        assert_eq!(migrations[39].version, 40);
        let sql = migrations[39].sql.as_str();
        assert!(sql.contains("CREATE OR REPLACE FUNCTION enqueue_push_match_job"));
        assert!(sql.contains("NEW.kind IN (9, 40002, 45001, 45003)"));
        assert!(!sql.contains("NEW.kind IN (7, 9, 1059, 40007, 46010)"));

        let desired_schema = include_str!("../../../../schema/schema.sql");
        assert!(desired_schema.contains("NEW.kind IN (9, 40002, 45001, 45003)"));
        assert!(!desired_schema.contains("NEW.kind IN (7, 9, 1059, 40007, 46010)"));
    }

    #[test]
    fn migration_lint_detects_tables_missing_community_id_by_default() {
        let sql = r#"
            CREATE TABLE communities (id UUID PRIMARY KEY);
            CREATE TABLE widgets (id UUID PRIMARY KEY);
            CREATE TABLE _operator_global_tables (table_name TEXT PRIMARY KEY, reason TEXT NOT NULL);
            INSERT INTO _operator_global_tables (table_name, reason) VALUES
                ('communities', 'tenant registry'),
                ('_operator_global_tables', 'registry');
        "#;

        let definitions = create_table_definitions(sql);
        let scoped = scoped_tables(sql);
        let missing = definitions
            .into_iter()
            .filter(|(table, _)| scoped.contains(table))
            .filter(|(_, definitions)| !table_has_not_null_community_id(definitions))
            .map(|(table, _)| table)
            .collect::<Vec<_>>();

        assert_eq!(missing, vec!["widgets"]);
    }

    #[test]
    fn migration_lint_detects_scoped_key_constraints_not_led_by_community_id() {
        let sql = r#"
            CREATE TABLE widgets (
                community_id UUID NOT NULL,
                id UUID PRIMARY KEY,
                channel_id UUID REFERENCES channels(id),
                slug TEXT,
                CONSTRAINT widgets_name_unique UNIQUE (slug),
                CONSTRAINT widgets_parent_fk FOREIGN KEY (channel_id) REFERENCES channels(id)
            );
            CREATE UNIQUE INDEX idx_widgets_slug ON widgets (slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_slug_unique UNIQUE (slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_parent_fk FOREIGN KEY (channel_id) REFERENCES channels(id);
            CREATE TABLE _operator_global_tables (table_name TEXT PRIMARY KEY, reason TEXT NOT NULL);
            INSERT INTO _operator_global_tables (table_name, reason) VALUES
                ('_operator_global_tables', 'registry');
        "#;

        let violations = scoped_constraint_violations(sql);

        assert!(violations
            .iter()
            .any(|violation| violation.kind == ConstraintKind::PrimaryKey));
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation.kind == ConstraintKind::ForeignKey)
                .count(),
            3
        );
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation.kind == ConstraintKind::Unique)
                .count(),
            3
        );
    }

    #[test]
    fn migration_lint_accepts_scoped_key_constraints_led_by_community_id() {
        let sql = r#"
            CREATE TABLE widgets (
                community_id UUID NOT NULL,
                id UUID NOT NULL,
                channel_id UUID NOT NULL,
                slug TEXT NOT NULL,
                PRIMARY KEY (community_id, id),
                UNIQUE (community_id, slug),
                FOREIGN KEY (community_id, channel_id) REFERENCES channels(community_id, id)
            );
            CREATE UNIQUE INDEX idx_widgets_slug ON widgets (community_id, slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_slug_unique UNIQUE (community_id, slug);
            ALTER TABLE widgets ADD CONSTRAINT widgets_alter_parent_fk FOREIGN KEY (community_id, channel_id) REFERENCES channels(community_id, id);
            CREATE TABLE _operator_global_tables (table_name TEXT PRIMARY KEY, reason TEXT NOT NULL);
            INSERT INTO _operator_global_tables (table_name, reason) VALUES
                ('_operator_global_tables', 'registry');
        "#;

        assert!(scoped_constraint_violations(sql).is_empty());
    }

    #[test]
    fn all_non_operator_global_tables_have_not_null_community_id() {
        let sql = migration_sql();
        let sql = sql.as_str();
        let scoped = scoped_tables(sql);
        let missing = create_table_definitions(sql)
            .into_iter()
            .filter(|(table, _)| scoped.contains(table))
            .filter(|(_, definitions)| !table_has_not_null_community_id(definitions))
            .map(|(table, _)| table)
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "every table not listed in _operator_global_tables must carry NOT NULL community_id; missing: {}",
            missing.join(", ")
        );
    }

    #[test]
    fn scoped_primary_key_unique_and_foreign_key_constraints_lead_with_community_id() {
        let sql = migration_sql();
        let sql = sql.as_str();
        let violations = scoped_constraint_violations(sql)
            .into_iter()
            .map(|constraint| {
                format!(
                    "{}. {:?} constraint must lead with community_id: {}",
                    constraint.table, constraint.kind, constraint.description
                )
            })
            .collect::<Vec<_>>();

        assert!(
            violations.is_empty(),
            "tenant-scoped tables are all tables not listed in _operator_global_tables; primary key, unique/FK constraints, and unique indexes on those tables must lead with community_id:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn channels_community_id_is_immutable_after_insert() {
        let sql = migration_sql();
        let sql = sql.as_str();
        let forbidden_mutations = forbidden_channels_community_id_mutations(sql);

        assert!(
            forbidden_mutations.is_empty(),
            "channels.community_id must not be re-tenanted after insert; forbidden migration statements:\n{}",
            forbidden_mutations.join("\n---\n")
        );
        assert!(
            has_channels_community_id_immutability_guard(sql),
            "migrations define channels.community_id but no BEFORE UPDATE trigger/function guard that rejects OLD.community_id <> NEW.community_id was found"
        );
    }

    #[test]
    fn migration_execution_cannot_bypass_schema_destruction_lock() {
        fn rust_sources(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read workspace source dir") {
                let path = entry.expect("read workspace source entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name == "target") {
                        continue;
                    }
                    rust_sources(&path, files);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    files.push(path);
                }
            }
        }
        fn count(haystack: &str, needle: &str) -> usize {
            haystack.matches(needle).count()
        }

        // Build the needles so this test's own source never matches them.
        let migrate_macro = ["sqlx", "::migrate!"].concat();
        let migrator_run = ["MIGRATOR", ".run("].concat();
        let migrator_run_to = ["MIGRATOR", ".run_to("].concat();

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let this_file = manifest_dir.join("src/runtime/migration.rs");
        let crates_dir = manifest_dir.parent().expect("workspace crates dir");
        // The push gateway migrates its own dedicated authority database; it
        // never holds relay tenant tables, so it is exempt from the relay
        // schema/destruction lock. The community_id check below keeps that
        // exemption honest.
        let push_gateway_exception = crates_dir.join("buzz-push-gateway/src/postgres.rs");
        let push_gateway_migrations = crates_dir.join("buzz-push-gateway/migrations");
        for entry in
            std::fs::read_dir(&push_gateway_migrations).expect("read push gateway migrations")
        {
            let path = entry.expect("read push gateway migration entry").path();
            let sql = std::fs::read_to_string(&path).expect("read push gateway migration");
            assert!(
                !sql.to_ascii_lowercase().contains("community_id"),
                "{} defines community-scoped data; its migrator would bypass the \
                 schema/destruction lock and must move under buzz-db migrations",
                path.display()
            );
        }
        let mut files = Vec::new();
        rust_sources(crates_dir, &mut files);
        for path in &files {
            let source = std::fs::read_to_string(path).expect("read rust source");
            let (macro_hits, run_hits, run_to_hits) = (
                count(&source, &migrate_macro),
                count(&source, &migrator_run),
                count(&source, &migrator_run_to),
            );
            if *path == this_file {
                assert_eq!(
                    (macro_hits, run_hits, run_to_hits),
                    (1, 1, 1),
                    "migration.rs must embed the migrator once, run it once in production, \
                     and expose exactly one test-only bounded run"
                );
            } else if *path == push_gateway_exception {
                continue;
            } else {
                assert_eq!(
                    (macro_hits, run_hits, run_to_hits),
                    (0, 0, 0),
                    "{} embeds or runs a SQLx migrator outside the schema/destruction \
                     lock contract; route migration execution through \
                     buzz_db migration::run_migrations",
                    path.display()
                );
            }
        }

        // Within migration.rs, the single run site must sit inside
        // `run_migrations_locked`, and the only public entry point must wrap
        // it in the exclusive session lock.
        let source = std::fs::read_to_string(&this_file).expect("read migration.rs");
        let entry = source
            .find("pub async fn run_migrations(")
            .expect("public migration entry point");
        let locked = source
            .find("async fn run_migrations_locked(")
            .expect("locked migration body");
        let wrapper = source
            .find("async fn with_exclusive_schema_destruction_lock")
            .expect("exclusive lock wrapper");
        let run_site = source.find(&migrator_run).expect("migrator run site");
        let run_to_site = source
            .find(&migrator_run_to)
            .expect("bounded test migrator run site");
        assert!(
            source[entry..locked].contains("with_exclusive_schema_destruction_lock("),
            "run_migrations must delegate through the exclusive schema/destruction lock"
        );
        assert!(
            run_site > locked && run_site < wrapper,
            "the production migrator run site must live inside run_migrations_locked"
        );
        assert!(
            run_to_site > entry
                && run_to_site < locked
                && source[entry..run_to_site].contains("#[cfg(test)]")
                && source[entry..run_to_site].contains("with_exclusive_schema_destruction_lock("),
            "the bounded migrator run must remain test-only and use the exclusive lock wrapper"
        );
        assert!(
            source[wrapper..].contains("pg_advisory_lock($1)")
                && source[wrapper..].contains("pg_advisory_unlock($1)"),
            "the lock wrapper must acquire and explicitly release the session lock"
        );
    }

    /// Structural parity between migration 0029's deletion surface and the
    /// desired-state bootstrap schema (`schema/schema.sql`).
    ///
    /// Compares parsed statements, not substrings: every deletion control-
    /// plane table, function, trigger, and index 0028 creates must exist in
    /// schema.sql with an identical normalized definition; every operator-
    /// global registry row 0028 inserts must be inserted by schema.sql; the
    /// write-fence attachment target sets must be equal; and every column
    /// 0028 adds to `communities` must exist in the desired-state
    /// `communities` table. A desired-state bootstrap that passes this test
    /// cannot silently omit part of the deletion surface the way the
    /// pre-parity schema.sql omitted `community_deletion_manifest_keys` (and
    /// its immutability trigger) and `storage_taxonomy_sweeps` — booting
    /// healthy, then wedging post-fence when the freeze stage first touched
    /// the missing relation.
    #[test]
    fn deletion_surface_parity_between_migration_0029_and_schema_sql() {
        use std::collections::BTreeMap;

        #[derive(Default)]
        struct DeletionSurface {
            tables: BTreeMap<String, String>,
            functions: BTreeMap<String, String>,
            triggers: BTreeMap<String, String>,
            indexes: BTreeSet<String>,
            registry_rows: BTreeSet<(String, String)>,
            fence_attachments: BTreeSet<String>,
            communities_added_columns: BTreeSet<String>,
        }

        fn quoted_strings(statement: &str) -> Vec<String> {
            let mut strings = Vec::new();
            let mut current: Option<String> = None;
            let mut chars = statement.chars().peekable();
            while let Some(ch) = chars.next() {
                match (&mut current, ch) {
                    (None, '\'') => current = Some(String::new()),
                    (Some(literal), '\'') => {
                        if chars.peek() == Some(&'\'') {
                            literal.push('\'');
                            chars.next();
                        } else {
                            strings.push(current.take().expect("open literal"));
                        }
                    }
                    (Some(literal), other) => literal.push(other),
                    (None, _) => {}
                }
            }
            strings
        }

        fn surface(sql: &str) -> DeletionSurface {
            let mut surface = DeletionSurface::default();
            for statement in split_sql_statements(sql) {
                let normalized = normalize_sql(&statement);
                if normalized.starts_with("create table") {
                    let table = identifier_after_keyword(&statement, "create table")
                        .expect("table identifier");
                    surface.tables.insert(table, normalized.clone());
                } else if normalized.starts_with("create function")
                    || normalized.starts_with("create or replace function")
                {
                    let function = identifier_after_keyword(&statement, "function")
                        .expect("function identifier");
                    surface.functions.insert(function, normalized.clone());
                } else if normalized.starts_with("create trigger") {
                    let trigger = identifier_after_keyword(&statement, "create trigger")
                        .expect("trigger identifier");
                    surface.triggers.insert(trigger, normalized.clone());
                } else if normalized.starts_with("create index")
                    || normalized.starts_with("create unique index")
                {
                    surface.indexes.insert(normalized.clone());
                } else if normalized.starts_with("insert into _operator_global_tables") {
                    let literals = quoted_strings(&statement);
                    assert!(
                        literals.len().is_multiple_of(2),
                        "operator-global registry insert must be (table_name, reason) rows"
                    );
                    for row in literals.chunks(2) {
                        surface
                            .registry_rows
                            .insert((row[0].clone(), row[1].clone()));
                    }
                } else if normalized.starts_with("alter table communities") {
                    for added in normalized.split("add column ").skip(1) {
                        let column = added
                            .split_whitespace()
                            .next()
                            .expect("added column name")
                            .to_owned();
                        surface.communities_added_columns.insert(column);
                    }
                }
                if let Some(position) = normalized.find("attach_community_write_fence('") {
                    let target = normalized[position + "attach_community_write_fence('".len()..]
                        .split('\'')
                        .next()
                        .expect("fence attachment target")
                        .to_owned();
                    surface.fence_attachments.insert(target);
                }
            }
            surface
        }

        let migration_0029: &str = MIGRATOR
            .iter()
            .find(|migration| migration.version == 29)
            .expect("embedded migration 0029")
            .sql
            .as_ref();
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let schema_sql = std::fs::read_to_string(workspace_root.join("schema/schema.sql"))
            .expect("read schema/schema.sql");

        let migration = surface(migration_0029);
        let schema = surface(&schema_sql);

        assert_eq!(
            migration.tables.len(),
            7,
            "0029 deletion control plane must define exactly the known tables: {:?}",
            migration.tables.keys().collect::<Vec<_>>()
        );
        assert!(!migration.fence_attachments.is_empty());
        assert!(!migration.registry_rows.is_empty());

        for (table, definition) in &migration.tables {
            let in_schema = schema
                .tables
                .get(table)
                .unwrap_or_else(|| panic!("schema.sql is missing deletion table {table}"));
            if table != "community_deletion_requests" {
                assert_eq!(
                    in_schema, definition,
                    "schema.sql definition of {table} drifted from migration 0029"
                );
            }
        }
        for (function, definition) in &migration.functions {
            let in_schema = schema
                .functions
                .get(function)
                .unwrap_or_else(|| panic!("schema.sql is missing deletion function {function}"));
            if function != "community_write_fence_excluded_table" {
                assert_eq!(
                    in_schema, definition,
                    "schema.sql definition of {function}() drifted from migration 0029"
                );
            }
        }
        for (trigger, definition) in &migration.triggers {
            let in_schema = schema
                .triggers
                .get(trigger)
                .unwrap_or_else(|| panic!("schema.sql is missing deletion trigger {trigger}"));
            assert_eq!(
                in_schema, definition,
                "schema.sql definition of trigger {trigger} drifted from migration 0029"
            );
        }
        for index in &migration.indexes {
            assert!(
                schema.indexes.contains(index),
                "schema.sql is missing (or drifted on) deletion index: {index}"
            );
        }
        for row in &migration.registry_rows {
            assert!(
                schema.registry_rows.contains(row),
                "schema.sql is missing operator-global registry row {row:?}"
            );
        }
        let mut expected_fences = migration.fence_attachments.clone();
        expected_fences.remove("product_feedback");
        expected_fences.remove("rate_limit_violations");
        assert_eq!(
            expected_fences, schema.fence_attachments,
            "write-fence attachment targets differ after recovery policy"
        );

        // 0029's ALTER TABLE additions are expressed inline by the
        // desired-state `communities` definition; require the columns to
        // exist there (exact definition equality is impossible across the
        // ALTER/inline representations — behavior is pinned by the
        // desired-state bootstrap deletion test).
        let communities_columns = split_sql_statements(&schema_sql)
            .into_iter()
            .find_map(|statement| {
                let (table, body) = create_table_body(&statement)?;
                (table == "communities").then_some(body)
            })
            .expect("schema.sql defines communities");
        let column_names: BTreeSet<String> = communities_columns
            .iter()
            .filter_map(|definition| column_definition_name(definition))
            .collect();
        for column in &migration.communities_added_columns {
            assert!(
                column_names.contains(column),
                "schema.sql communities table is missing 0028 column {column}"
            );
        }
        assert!(!migration.communities_added_columns.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn schema_destruction_lock_excludes_shared_holders_and_releases_on_both_paths() {
        let pool = connect_test_pool().await;
        async fn assert_exclusive_lock_free(pool: &PgPool) {
            let mut probe = pool.acquire().await.expect("acquire lock probe");
            let free: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                .fetch_one(&mut *probe)
                .await
                .expect("probe try-lock");
            assert!(free, "schema/destruction session lock must be released");
            sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                .execute(&mut *probe)
                .await
                .expect("probe unlock");
        }

        let probe_pool = pool.clone();
        with_exclusive_schema_destruction_lock(&pool, move |conn| async move {
            // While a migration run is in flight, destructive transactions
            // must be unable to take their shared counterpart.
            let mut probe = probe_pool.acquire().await.expect("acquire shared probe");
            let shared_available: bool =
                sqlx::query_scalar("SELECT pg_try_advisory_lock_shared($1)")
                    .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                    .fetch_one(&mut *probe)
                    .await
                    .expect("probe shared try-lock");
            assert!(
                !shared_available,
                "exclusive migration lock must exclude shared destructive holders"
            );
            (conn, Ok(()))
        })
        .await
        .expect("locked migration op");
        assert_exclusive_lock_free(&pool).await;

        let failed: Result<()> = with_exclusive_schema_destruction_lock(&pool, |conn| async {
            (
                conn,
                Err(crate::DbError::InvalidData(
                    "forced migration failure".into(),
                )),
            )
        })
        .await;
        assert!(failed.is_err(), "op failure must propagate");
        assert_exclusive_lock_free(&pool).await;
    }

    /// Cancellation must not release the exclusion contract while migration
    /// SQL is still executing server-side.
    ///
    /// The op parks an `ALTER TABLE` behind an ACCESS EXCLUSIVE table lock
    /// held by another session, then the whole locked run is aborted. Because
    /// the advisory lock lives on the same backend that runs the DDL,
    /// dropping the client future cannot release it: the backend keeps the
    /// session lock until it finishes the statement and dies on the closed
    /// socket. The shared (destructive) counterpart must stay unavailable for
    /// that entire interval — and the orphaned DDL really does commit after
    /// cancellation, which is exactly the window the lock has to cover.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cancelled_migration_cannot_expose_shared_lock_while_ddl_backend_lives() {
        use std::time::Instant;

        use sqlx::AssertSqlSafe;

        // Dedicated database: the probe table and the orphaned backend must
        // stay invisible to concurrent tests in the shared database.
        let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let admin = PgPool::connect(&base_url)
            .await
            .expect("connect admin database");
        let probe_db = format!("buzz_lock_cancel_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {probe_db}")))
            .execute(&admin)
            .await
            .expect("create probe database");
        let (base_prefix, _) = base_url.rsplit_once('/').expect("database url has a path");
        let pool = PgPool::connect(&format!("{base_prefix}/{probe_db}"))
            .await
            .expect("connect probe database");
        sqlx::query("CREATE TABLE schema_lock_cancel_probe (id int)")
            .execute(&pool)
            .await
            .expect("create probe table");

        // Park the migration DDL server-side: the op's ALTER TABLE waits on
        // this ACCESS EXCLUSIVE lock, pinning the backend mid-statement.
        let mut blocker = pool.begin().await.expect("open blocker transaction");
        sqlx::query("LOCK TABLE schema_lock_cancel_probe IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await
            .expect("hold probe table lock");

        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let task_pool = pool.clone();
        let locked_run = tokio::spawn(async move {
            with_exclusive_schema_destruction_lock(&task_pool, move |mut conn| async move {
                let outcome: Result<()> = async {
                    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                        .fetch_one(&mut conn)
                        .await?;
                    let _ = pid_tx.send(pid);
                    sqlx::query(
                        "ALTER TABLE schema_lock_cancel_probe \
                         ADD COLUMN committed_after_cancel int",
                    )
                    .execute(&mut conn)
                    .await?;
                    Ok(())
                }
                .await;
                (conn, outcome)
            })
            .await
        });
        let ddl_pid = pid_rx.await.expect("locked op reports its backend pid");
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE pid = $1 AND wait_event_type = 'Lock')",
            )
            .bind(ddl_pid)
            .fetch_one(&pool)
            .await
            .expect("poll DDL wait state");
            if waiting {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "migration DDL never parked on the table lock"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        locked_run.abort();
        let joined = locked_run.await;
        assert!(
            joined.is_err_and(|err| err.is_cancelled()),
            "locked migration run must abort mid-statement"
        );

        // The client future is gone, but the DDL backend is alive: the shared
        // destructive lock must remain unavailable for that whole interval.
        for _ in 0..20 {
            let backend_alive: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)")
                    .bind(ddl_pid)
                    .fetch_one(&pool)
                    .await
                    .expect("poll DDL backend liveness");
            assert!(
                backend_alive,
                "parked DDL backend must outlive client cancellation"
            );
            let shared_free: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock_shared($1)")
                .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                .fetch_one(&pool)
                .await
                .expect("probe shared lock");
            assert!(
                !shared_free,
                "cancellation must not expose the shared lock while migration DDL is executing"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Release the table lock: the orphaned backend finishes the ALTER,
        // commits, then exits on the dead socket — only then may shared
        // destructive holders enter.
        blocker.rollback().await.expect("release probe table lock");
        let mut probe = pool.acquire().await.expect("acquire shared-lock probe");
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let shared_free: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock_shared($1)")
                .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                .fetch_one(&mut *probe)
                .await
                .expect("probe shared lock after backend exit");
            if shared_free {
                sqlx::query("SELECT pg_advisory_unlock_shared($1)")
                    .bind(SCHEMA_DESTRUCTION_LOCK_KEY)
                    .execute(&mut *probe)
                    .await
                    .expect("release shared probe lock");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "shared lock must become available once the DDL backend exits"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        drop(probe);

        // The cancelled statement committed after the client vanished —
        // exactly the interval the same-backend lock covered.
        let committed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = 'schema_lock_cancel_probe' \
               AND column_name = 'committed_after_cancel')",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect orphaned DDL outcome");
        assert!(
            committed,
            "orphaned migration DDL commits after cancellation; the lock must cover it"
        );

        pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {probe_db} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop probe database");
    }

    async fn connect_test_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn reset_public_schema(pool: &PgPool) {
        sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
            .execute(pool)
            .await
            .expect("drop public schema");
        sqlx::query("CREATE SCHEMA IF NOT EXISTS public")
            .execute(pool)
            .await
            .expect("create public schema");
    }

    async fn applied_versions(pool: &PgPool) -> Vec<i64> {
        sqlx::query_scalar::<_, i64>(
            "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .expect("read applied migrations")
    }

    /// The desired-state file (`schema/schema.sql`) and the incremental
    /// migrations are two independent sources of the same schema. When a
    /// migration mutates the admin tables, `schema.sql` must be hand-updated to
    /// match — nothing enforces that automatically, and the lease/claim-token
    /// migrations (0035/0036) once drifted for exactly this reason.
    ///
    /// This bootstraps one probe database from `schema.sql` **through the real
    /// `bin/pgschema apply` binary** — the exact path CI (`ci.yml`) and both
    /// test-relay launchers take — and migrates another through 1–38, then
    /// asserts the three admin tables have identical column definitions (name,
    /// type, nullability, default) and identical index shapes, including each
    /// key's catalog sort/null options (`pg_index.indoption`). Columns are keyed
    /// by name, not ordinal, because migrations append via `ALTER TABLE` while
    /// `schema.sql` declares them inline — positions legitimately differ, shapes
    /// must not.
    ///
    /// Driving the real binary is load-bearing: `pgschema` 1.7.4 discards
    /// per-key `NULLS FIRST`/`NULLS LAST` when it re-emits an index, so a naive
    /// `sqlx::raw_sql(schema.sql)` bootstrap would preserve ordering the actual
    /// deployment path silently drops — the same false-confidence class as the
    /// drift this test guards against. `indoption` (not just `indexdef` text) is
    /// asserted so a resurrected `NULLS FIRST` in a migration that `pgschema`
    /// cannot represent is caught.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn admin_schema_parity_between_desired_state_and_migrations() {
        use sqlx::AssertSqlSafe;

        async fn columns(
            pool: &PgPool,
            table: &str,
        ) -> Vec<(
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
        )> {
            sqlx::query_as(
                "SELECT column_name, data_type, is_nullable, column_default, \
                 is_identity, identity_generation \
                 FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = $1 \
                 ORDER BY column_name",
            )
            .bind(table)
            .fetch_all(pool)
            .await
            .expect("read column definitions")
        }

        // Index name + rendered definition + per-key sort/null options. indoption
        // is a int2vector rendered as text (e.g. `{2,0}` = NULLS FIRST ASC on key
        // 0, plain ASC on key 1) so ordering divergences that `indexdef` text may
        // still show but `pgschema` cannot reproduce are compared structurally.
        async fn index_shapes(pool: &PgPool, table: &str) -> Vec<(String, String, String)> {
            sqlx::query_as(
                "SELECT c.relname, pg_get_indexdef(i.indexrelid), i.indoption::int2[]::text \
                 FROM pg_class c \
                 JOIN pg_index i ON i.indexrelid = c.oid \
                 JOIN pg_class t ON t.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = t.relnamespace \
                 WHERE n.nspname = 'public' AND t.relname = $1 \
                 ORDER BY c.relname",
            )
            .bind(table)
            .fetch_all(pool)
            .await
            .expect("read index shapes")
        }

        let base_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let conn = parse_pg_url(&base_url);
        let admin = PgPool::connect(&base_url)
            .await
            .expect("connect admin database");
        let (base_prefix, _) = base_url.rsplit_once('/').expect("database url has a path");

        let desired_db = format!("buzz_admin_desired_{}", uuid::Uuid::new_v4().simple());
        let migrated_db = format!("buzz_admin_migrated_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {desired_db}")))
            .execute(&admin)
            .await
            .expect("create desired-state probe database");
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {migrated_db}")))
            .execute(&admin)
            .await
            .expect("create migrated probe database");

        // Bootstrap the desired-state probe through the real pgschema binary, the
        // same invocation the test-relay launchers use. The freshly-created probe
        // db doubles as pgschema's plan database (--plan-*), which avoids the
        // embedded-Postgres download and matches start-relay-for-tests.sh.
        let pgschema = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/pgschema");
        let schema_file =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/schema.sql");
        let port = conn.port.to_string();
        let apply = std::process::Command::new(&pgschema)
            .args([
                "apply",
                "--auto-approve",
                "--file",
                schema_file.to_str().expect("schema path utf-8"),
                "--host",
                &conn.host,
                "--port",
                &port,
                "--user",
                &conn.user,
                "--password",
                &conn.password,
                "--db",
                &desired_db,
                "--plan-host",
                &conn.host,
                "--plan-port",
                &port,
                "--plan-user",
                &conn.user,
                "--plan-password",
                &conn.password,
                "--plan-db",
                &desired_db,
            ])
            .output()
            .expect("run bin/pgschema apply (hermit env required)");
        assert!(
            apply.status.success(),
            "pgschema apply failed: {}\n{}",
            String::from_utf8_lossy(&apply.stdout),
            String::from_utf8_lossy(&apply.stderr),
        );

        let desired = PgPool::connect(&format!("{base_prefix}/{desired_db}"))
            .await
            .expect("connect desired-state probe database");
        let migrated = PgPool::connect(&format!("{base_prefix}/{migrated_db}"))
            .await
            .expect("connect migrated probe database");
        MIGRATOR
            .run_to(39, &migrated)
            .await
            .expect("apply migrations 1-39");

        for table in [
            "relay_admin_actions",
            "relay_admin_outbox",
            "relay_operator_audit",
        ] {
            assert_eq!(
                columns(&desired, table).await,
                columns(&migrated, table).await,
                "column parity mismatch for {table}: schema.sql desired state has drifted \
                 from the migrations; update schema/schema.sql to match"
            );
            assert_eq!(
                index_shapes(&desired, table).await,
                index_shapes(&migrated, table).await,
                "index-shape parity mismatch for {table}: the pgschema-bootstrapped desired \
                 state (including per-key indoption) has drifted from the migrations. If a \
                 migration uses a construct pgschema cannot represent (e.g. NULLS FIRST), the \
                 migration and schema.sql must both use a representable shape."
            );
        }

        desired.close().await;
        migrated.close().await;
        for probe_db in [desired_db, migrated_db] {
            sqlx::query(AssertSqlSafe(format!(
                "DROP DATABASE {probe_db} WITH (FORCE)"
            )))
            .execute(&admin)
            .await
            .expect("drop probe database");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn pre_0007_ambiguous_nip_rs_data_blocks_without_mutation_and_allows_retry() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(6, &pool)
            .await
            .expect("apply migrations 1-6");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("pre-0007-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        let event_id = vec![1_u8; 32];
        let pubkey = vec![2_u8; 32];
        let d_tag = format!("read-state:{}", "a".repeat(32));
        let ambiguous_tags = serde_json::json!([["d", d_tag], ["d", "other"], ["t", "read-state"]]);
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, d_tag) \
             VALUES ($1, $2, $3, NOW(), 30078, $4, 'ambiguous', $5, NOW(), $6)",
        )
        .bind(community_id)
        .bind(&event_id)
        .bind(&pubkey)
        .bind(&ambiguous_tags)
        .bind(vec![3_u8; 64])
        .bind(&d_tag)
        .execute(&pool)
        .await
        .expect("insert ambiguous NIP-RS row");

        let before_versions = applied_versions(&pool).await;
        let before_row: (serde_json::Value, String) =
            sqlx::query_as("SELECT tags, content FROM events WHERE community_id=$1 AND id=$2")
                .bind(community_id)
                .bind(&event_id)
                .fetch_one(&pool)
                .await
                .expect("read ambiguous row before blocked migration");
        let blocked = run_migrations(&pool).await;
        assert!(blocked.is_err(), "ambiguous pre-0007 data must fail closed");
        assert_eq!(applied_versions(&pool).await, before_versions);
        let after_row: (serde_json::Value, String) =
            sqlx::query_as("SELECT tags, content FROM events WHERE community_id=$1 AND id=$2")
                .bind(community_id)
                .bind(&event_id)
                .fetch_one(&pool)
                .await
                .expect("blocked migration must preserve source row");
        assert_eq!(after_row, before_row);

        let repaired_tags = serde_json::json!([["d", d_tag], ["t", "read-state"]]);
        sqlx::query("UPDATE events SET tags=$1 WHERE community_id=$2 AND id=$3")
            .bind(repaired_tags)
            .bind(community_id)
            .bind(&event_id)
            .execute(&pool)
            .await
            .expect("repair ambiguous row");
        run_migrations(&pool)
            .await
            .expect("retry succeeds after operator repair");
        let latest_version = MIGRATOR
            .iter()
            .map(|migration| migration.version)
            .max()
            .expect("embedded migrator is non-empty");
        assert_eq!(
            applied_versions(&pool).await.last().copied(),
            Some(latest_version)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn populated_upgrade_preserves_search_policy_except_for_private_kinds() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(7, &pool)
            .await
            .expect("apply migrations 1-7");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("pre-0008-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        for (marker, kind) in [(1_u8, 1_i32), (2_u8, 30_350_i32), (3_u8, 30_179_i32)] {
            sqlx::query(
                "INSERT INTO events \
                 (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at) \
                 VALUES ($1, $2, $3, NOW(), $4, '[]'::jsonb, 'brownfield needle', $5, NOW())",
            )
            .bind(community_id)
            .bind(vec![marker; 32])
            .bind(vec![marker + 10; 32])
            .bind(kind)
            .bind(vec![marker + 20; 64])
            .execute(&pool)
            .await
            .expect("insert brownfield event");
        }

        MIGRATOR
            .run_to(11, &pool)
            .await
            .expect("apply main migrations through 11");
        let before: Vec<(i32, bool)> = sqlx::query_as(
            "SELECT kind, search_tsv @@ plainto_tsquery('simple', 'needle') \
             FROM events ORDER BY kind",
        )
        .fetch_all(&pool)
        .await
        .expect("read pre-push search behavior");
        assert_eq!(before, vec![(1, true), (30_179, true), (30_350, true)]);

        // 0014 fixes 30350 only. A brownfield database that stopped here still
        // tokenized kind:30179 ciphertext — the gap 0033 closes.
        MIGRATOR
            .run_to(32, &pool)
            .await
            .expect("apply migrations through 32");
        let pre_0033: Vec<(i32, Option<bool>)> = sqlx::query_as(
            "SELECT kind, search_tsv @@ plainto_tsquery('simple', 'needle') \
             FROM events ORDER BY kind",
        )
        .fetch_all(&pool)
        .await
        .expect("read pre-0033 search behavior");
        assert_eq!(
            pre_0033,
            vec![(1, Some(true)), (30_179, Some(true)), (30_350, None)]
        );

        run_migrations(&pool)
            .await
            .expect("apply remaining migrations to populated database");
        let after: Vec<(i32, Option<bool>)> = sqlx::query_as(
            "SELECT kind, search_tsv @@ plainto_tsquery('simple', 'needle') \
             FROM events ORDER BY kind",
        )
        .fetch_all(&pool)
        .await
        .expect("read post-upgrade search behavior");
        assert_eq!(after, vec![(1, Some(true)), (30_179, None), (30_350, None)]);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_migrations_applies_consolidated_initial_schema_on_fresh_database() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        run_migrations(&pool).await.expect("run migrations");

        // Every embedded migration must apply, in order. Derive the expected
        // list from the MIGRATOR itself so this doesn't go stale as additive
        // migrations land (it previously hardcoded [1, 2, 3] and rotted).
        let expected: Vec<i64> = {
            let mut versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
            versions.sort_unstable();
            versions
        };
        assert_eq!(applied_versions(&pool).await, expected);
        let sql = migration_sql();
        let tables = create_tables(sql.as_str());
        for table in [
            "communities",
            "events",
            "channels",
            "scheduled_workflow_fires",
            "audit_log",
        ] {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|err| panic!("check table {table}: {err}"));
            assert!(
                tables.contains(table),
                "migration parser should see {table}"
            );
            assert!(exists, "migration should create {table}");
        }

        let search_expression: String = sqlx::query_scalar(
            "SELECT pg_get_expr(adbin, adrelid) \
             FROM pg_attrdef \
             WHERE adrelid = 'events'::regclass \
               AND adnum = (SELECT attnum FROM pg_attribute \
                            WHERE attrelid = 'events'::regclass \
                              AND attname = 'search_tsv')",
        )
        .fetch_one(&pool)
        .await
        .expect("read fresh-install search expression");
        assert!(
            search_expression.contains("ARRAY[0, 9, 40002, 45001, 45003]"),
            "fresh-install search allowlist has the wrong kinds: {search_expression}"
        );
        assert!(
            search_expression.contains("ELSE NULL::tsvector"),
            "fresh installs must default non-allowlisted kinds to NULL: {search_expression}"
        );

        let active_a = uuid::Uuid::new_v4();
        let active_b = uuid::Uuid::new_v4();
        let to_fence = uuid::Uuid::new_v4();
        for (community, label) in [
            (active_a, "active-a"),
            (active_b, "active-b"),
            (to_fence, "to-fence"),
        ] {
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community)
                .bind(format!("late-fence-{label}-{}.example", community.simple()))
                .execute(&pool)
                .await
                .expect("insert late-table test community");
        }
        sqlx::query(
            "CREATE TABLE late_created_scoped (\
                 community_id UUID NOT NULL, id BIGINT PRIMARY KEY, value TEXT NOT NULL\
             )",
        )
        .execute(&pool)
        .await
        .expect("create late scoped table");
        sqlx::query("SELECT attach_community_write_fence('late_created_scoped'::regclass)")
            .execute(&pool)
            .await
            .expect("attach late create fence");
        sqlx::query("CREATE TABLE late_altered_scoped (id BIGINT PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create table before late alter");
        sqlx::query("ALTER TABLE late_altered_scoped ADD COLUMN community_id UUID NOT NULL")
            .execute(&pool)
            .await
            .expect("add late community id");
        sqlx::query("SELECT attach_community_write_fence('late_altered_scoped'::regclass)")
            .execute(&pool)
            .await
            .expect("attach late alter fence");
        let attached: Vec<String> = sqlx::query_scalar(
            "SELECT c.relname FROM pg_trigger trigger \
             JOIN pg_class c ON c.oid = trigger.tgrelid \
             JOIN pg_proc procedure ON procedure.oid = trigger.tgfoid \
             WHERE c.relname IN ('late_created_scoped', 'late_altered_scoped') \
               AND procedure.proname = 'enforce_community_write_fence' \
               AND NOT trigger.tgisinternal ORDER BY c.relname",
        )
        .fetch_all(&pool)
        .await
        .expect("read late trigger catalog");
        assert_eq!(attached, vec!["late_altered_scoped", "late_created_scoped"]);
        let malformed_fence_triggers: i64 = sqlx::query_scalar(
            "SELECT count(*)::BIGINT FROM pg_trigger trigger \
             JOIN pg_class c ON c.oid = trigger.tgrelid \
             JOIN pg_proc procedure ON procedure.oid = trigger.tgfoid \
             WHERE c.relname IN ('late_created_scoped', 'late_altered_scoped') \
               AND procedure.proname = 'enforce_community_write_fence' \
               AND NOT trigger.tgisinternal \
               AND (trigger.tgenabled <> 'O' OR (trigger.tgtype & 31) <> 31)",
        )
        .fetch_one(&pool)
        .await
        .expect("validate late trigger mode and operations");
        assert_eq!(malformed_fence_triggers, 0);

        sqlx::query(
            "INSERT INTO late_created_scoped (community_id, id, value) \
             VALUES ($1, 1, 'same'), ($2, 2, 'source-fenced'), \
                    ($1, 3, 'destination-fenced'), ($1, 4, 'opposite-a'), \
                    ($3, 5, 'opposite-b')",
        )
        .bind(active_a)
        .bind(to_fence)
        .bind(active_b)
        .execute(&pool)
        .await
        .expect("seed late table while communities active");
        sqlx::query("UPDATE late_created_scoped SET value = 'same-ok' WHERE id = 1")
            .execute(&pool)
            .await
            .expect("same-tenant active update");
        sqlx::query("UPDATE late_created_scoped SET community_id = $1 WHERE id = 1")
            .bind(active_b)
            .execute(&pool)
            .await
            .expect("active-to-active update");

        let mut fence_connection = pool.acquire().await.expect("fence connection");
        sqlx::query("BEGIN")
            .execute(&mut *fence_connection)
            .await
            .expect("begin direct fence");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', '1', true)",
        )
        .bind(to_fence.to_string())
        .execute(&mut *fence_connection)
        .await
        .expect("authorize direct fence");
        sqlx::query(
            "UPDATE communities SET deletion_state = 'fenced', \
                    deletion_fence_generation = 1, archived_at = now() WHERE id = $1",
        )
        .bind(to_fence)
        .execute(&mut *fence_connection)
        .await
        .expect("fence test destination");
        sqlx::query("COMMIT")
            .execute(&mut *fence_connection)
            .await
            .expect("commit direct fence");

        let active_to_fenced =
            sqlx::query("UPDATE late_created_scoped SET community_id = $1 WHERE id = 3")
                .bind(to_fence)
                .execute(&pool)
                .await
                .expect_err("active to fenced destination must fail");
        assert!(active_to_fenced
            .to_string()
            .contains("community write fenced"));
        let fenced_to_active =
            sqlx::query("UPDATE late_created_scoped SET community_id = $1 WHERE id = 2")
                .bind(active_a)
                .execute(&pool)
                .await
                .expect_err("fenced source to active destination must fail");
        assert!(fenced_to_active
            .to_string()
            .contains("community write fenced"));
        let row_locations: Vec<(i64, uuid::Uuid)> = sqlx::query_as(
            "SELECT id, community_id FROM late_created_scoped WHERE id IN (2, 3) ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("failed moves preserve row location");
        assert_eq!(row_locations, vec![(2, to_fence), (3, active_a)]);

        let move_a = sqlx::query("UPDATE late_created_scoped SET community_id = $1 WHERE id = 4")
            .bind(active_b)
            .execute(&pool);
        let move_b = sqlx::query("UPDATE late_created_scoped SET community_id = $1 WHERE id = 5")
            .bind(active_a)
            .execute(&pool);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let (a, b) = tokio::join!(move_a, move_b);
            a.expect("opposite active move A");
            b.expect("opposite active move B");
        })
        .await
        .expect("opposite cross-tenant updates must not deadlock");

        sqlx::query("DROP TABLE late_created_scoped, late_altered_scoped")
            .execute(&pool)
            .await
            .expect("drop late-table fixtures");
    }

    /// Verify migration 0044 applies cleanly against a DB that has rows in
    /// the NIP-FI 0041+0042 tables. The immutability guards (no_delete,
    /// no_truncate) are enforced via triggers; DROP TABLE bypasses them and
    /// must succeed even when rows are present.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_0044_drops_populated_nip_fi_ledger_cleanly() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        // Seed a community and minimal rows in a selection of 0041+0042 tables.
        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("drop-test-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("seed community");

        // Seed a receipt (used as FK anchor for several 0042 tables).
        let operation_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 12, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(operation_id)
        .bind(vec![0xAA_u8; 32])
        .bind(vec![0xBB_u8; 32])
        .bind(vec![0xCC_u8; 32])
        .execute(&pool)
        .await
        .expect("seed operation receipt");

        // Seed an invalidation domain (0042 table with no FK to receipts).
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains \
             (community_id, current_generation) VALUES ($1, 0)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("seed invalidation domain");

        // Apply migration 0043 (dogfood profile) and 0044 (ledger removal).
        MIGRATOR
            .run_to(44, &pool)
            .await
            .expect("migration 0044 must apply cleanly against a populated NIP-FI DB");

        // All NIP-FI tables must be gone.
        let nip_fi_tables = [
            "authorization_admission_results",
            "authorization_authentication_denial_attempts",
            "authorization_authority_epochs",
            "authorization_event_capacity",
            "authorization_events",
            "authorization_invalidation_domains",
            "authorization_invalidation_floors",
            "authorization_operation_receipts",
            "authorization_operation_version_delta_manifests",
            "authorization_operation_version_deltas",
            "identity_bindings",
            "identity_enrollment_policies",
            "identity_lifecycle_history",
            "identity_lifecycle_selectors",
            "protected_object_authority",
        ];
        let present: Vec<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = ANY($1)",
        )
        .bind(&nip_fi_tables[..])
        .fetch_all(&pool)
        .await
        .expect("catalog check after ledger removal");
        assert!(
            present.is_empty(),
            "all NIP-FI tables must be absent after migration 0044: {present:?}"
        );

        // The deletion catalog must validate with ledger relations gone.
        crate::deletion::DeletionStore::new(pool.clone())
            .validate_catalog()
            .await
            .expect("deletion catalog validates after migration 0044");
    }
}
