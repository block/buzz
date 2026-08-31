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
    let mut lock_conn = crate::observability::acquire(pool, crate::observability::PoolRole::Writer)
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
mod tests {
    use super::*;
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
    };

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    /// Connection parameters parsed out of a `postgres://user:pass@host:port/db`
    /// URL so the parity test can pass them to the `bin/pgschema` binary, which
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

        assert_eq!(migrations.len(), 42);
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

        // NIP-FI core identity + base-lifecycle foundation (migration 0041) and
        // final-admission foundation (0042). Both widen the single SQL source of
        // truth `community_write_fence_excluded_table` so their durable,
        // immutable ledger relations are never fence-attached, purged, or
        // counted as tenant-scoped drift. schema.sql keeps one consolidated
        // definition of that function whose body must match 0042's exactly.
        assert_eq!(migrations[40].version, 41);
        let identity_foundation = migrations[40].sql.as_str();
        assert!(identity_foundation.contains("CREATE TABLE identity_bindings"));
        assert!(identity_foundation.contains("CREATE TABLE identity_lifecycle_history"));
        assert!(identity_foundation
            .contains("CREATE OR REPLACE FUNCTION community_write_fence_excluded_table"));
        assert!(identity_foundation.contains("'identity_bindings'"));

        assert_eq!(migrations[41].version, 42);
        let authorization_foundation = migrations[41].sql.as_str();
        assert!(authorization_foundation.contains("CREATE TABLE authorization_events"));
        assert!(authorization_foundation.contains("CREATE TABLE protected_object_authority"));
        assert!(authorization_foundation.contains("CREATE TABLE authorization_admission_results"));

        // The consolidated desired-state exclusion function must byte-match
        // migration 0042's CREATE OR REPLACE body, or a future schema
        // consolidation would silently drop NIP-FI relations from the ledger.
        fn extract_excluded_table_array(sql: &str) -> &str {
            let anchor = "community_write_fence_excluded_table(target NAME) RETURNS BOOLEAN";
            let start = sql.find(anchor).expect("exclusion function definition");
            let array_start = sql[start..].find("ARRAY[").expect("exclusion array") + start;
            let array_end = sql[array_start..]
                .find("]::TEXT[]")
                .expect("exclusion array end")
                + array_start;
            &sql[array_start..array_end]
        }
        assert_eq!(
            extract_excluded_table_array(authorization_foundation),
            extract_excluded_table_array(desired_schema),
            "schema.sql exclusion list drifted from migration 0042"
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

    /// NIP-FI intermediate state: migration 0041 (identity + base lifecycle)
    /// alone must present a coherent catalog. Its five community-scoped ledger
    /// relations are immutable and durable, so they are registered in the
    /// write-fence exclusion — never counted as tenant-scoped drift, never
    /// fence-attached — and the exact deletion catalog must still validate.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_0041_identity_foundation_is_durable_ledger_after_migration_a() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(41, &pool)
            .await
            .expect("apply migrations 1-41");

        // The five identity relations exist.
        let identity_tables = [
            "authorization_operation_receipts",
            "identity_enrollment_policies",
            "identity_bindings",
            "identity_lifecycle_history",
            "identity_lifecycle_selectors",
        ];
        for table in identity_tables {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|err| panic!("check table {table}: {err}"));
            assert!(exists, "migration 0041 must create {table}");
        }

        // Migration B's relations must NOT exist yet.
        for table in ["authorization_events", "protected_object_authority"] {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_name = $1)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|err| panic!("check table {table}: {err}"));
            assert!(!exists, "{table} belongs to migration 0042, not 0041");
        }

        // Every identity relation is excluded from the write fence: none may
        // appear as tenant-scoped drift or carry the fence trigger.
        let scoped_or_fenced: Vec<String> = sqlx::query_scalar(
            "WITH scoped AS ( \
                 SELECT c.relname FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 JOIN pg_attribute a ON a.attrelid = c.oid \
                 WHERE n.nspname = 'public' AND c.relkind IN ('r','p') \
                   AND NOT c.relispartition AND a.attname = 'community_id' \
                   AND NOT a.attisdropped \
                   AND NOT community_write_fence_excluded_table(c.relname) \
             ) \
             SELECT relname FROM scoped \
             WHERE relname = ANY($1) ORDER BY relname",
        )
        .bind(&identity_tables[..])
        .fetch_all(&pool)
        .await
        .expect("read scoped identity relations");
        assert!(
            scoped_or_fenced.is_empty(),
            "identity ledger relations must be write-fence excluded, not scoped: {scoped_or_fenced:?}"
        );

        // The exact deletion catalog validates: the excluded ledger relations
        // do not perturb the scoped-table/fence equality check.
        crate::deletion::DeletionStore::new(pool.clone())
            .validate_catalog()
            .await
            .expect("deletion catalog validates after migration 0041");

        // The immutability contract is enforced, not merely declared. TRUNCATE
        // fires the statement-level guard unconditionally, so this proves the
        // rejection without constructing a fully valid ledger row.
        let rejected = sqlx::query("TRUNCATE identity_lifecycle_selectors")
            .execute(&pool)
            .await
            .expect_err("identity_lifecycle_selectors truncation must be rejected");
        assert!(
            rejected.to_string().contains("cannot be truncated"),
            "expected immutability rejection, got: {rejected}"
        );
    }

    /// NIP-FI full state: migrations 0041 + 0042 together must present a
    /// coherent 15-relation catalog with zero dangling foreign keys, all
    /// relations write-fence excluded, and an intact exact deletion catalog.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn nip_fi_foundation_is_a_closed_durable_ledger_after_migrations_a_and_b() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

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

        // All fifteen relations exist.
        let present: Vec<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = ANY($1) ORDER BY table_name",
        )
        .bind(&nip_fi_tables[..])
        .fetch_all(&pool)
        .await
        .expect("read NIP-FI table catalog");
        let mut expected: Vec<String> = nip_fi_tables.iter().map(|t| t.to_string()).collect();
        expected.sort();
        assert_eq!(
            present, expected,
            "all NIP-FI relations must exist after 0042"
        );

        // Zero dangling foreign keys: every FK target is a live relation.
        let invalid_fks: i64 = sqlx::query_scalar(
            "SELECT count(*)::BIGINT FROM pg_constraint \
             WHERE contype = 'f' AND NOT convalidated",
        )
        .fetch_one(&pool)
        .await
        .expect("read FK validity");
        assert_eq!(
            invalid_fks, 0,
            "no NIP-FI foreign key may be left unvalidated"
        );

        // None of the fifteen appear as tenant-scoped drift; all are excluded.
        let scoped: Vec<String> = sqlx::query_scalar(
            "SELECT c.relname FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_attribute a ON a.attrelid = c.oid \
             WHERE n.nspname = 'public' AND c.relkind IN ('r','p') \
               AND NOT c.relispartition AND a.attname = 'community_id' \
               AND NOT a.attisdropped \
               AND NOT community_write_fence_excluded_table(c.relname) \
               AND c.relname = ANY($1) ORDER BY c.relname",
        )
        .bind(&nip_fi_tables[..])
        .fetch_all(&pool)
        .await
        .expect("read scoped NIP-FI relations");
        assert!(
            scoped.is_empty(),
            "all NIP-FI ledger relations must be write-fence excluded: {scoped:?}"
        );

        // The exact deletion catalog validates with the full ledger present.
        crate::deletion::DeletionStore::new(pool.clone())
            .validate_catalog()
            .await
            .expect("deletion catalog validates after migrations 0041 + 0042");

        // A migration-B relation is immutable too. TRUNCATE fires the
        // statement-level guard unconditionally.
        let rejected = sqlx::query("TRUNCATE authorization_admission_results")
            .execute(&pool)
            .await
            .expect_err("authorization_admission_results truncation must be rejected");
        assert!(
            rejected.to_string().contains("cannot be truncated"),
            "expected immutability rejection, got: {rejected}"
        );
    }

    /// NIP-FI monotonic invalidation-floor advancement must actually run
    /// through the `BEFORE UPDATE` guard. PL/pgSQL defers record-field
    /// resolution to execution, so a guard that references a column absent from
    /// its Phase-A table passes every catalog/parity test yet aborts the first
    /// real advancement. This test exercises live UPDATEs: legitimate forward
    /// moves on `floor_generation` and `binding_version_floor` must commit, and
    /// equal/regressive moves must be rejected.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authorization_invalidation_floor_advances_through_guard() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("floor-guard-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // Each floor state points at an operation receipt via
        // (community_id, operation_id, request_fingerprint). Seed one receipt
        // per operation the test advances through.
        let operations: [(uuid::Uuid, u8); 4] = [
            (uuid::Uuid::new_v4(), 0x11),
            (uuid::Uuid::new_v4(), 0x22),
            (uuid::Uuid::new_v4(), 0x33),
            (uuid::Uuid::new_v4(), 0x44),
        ];
        for (operation_id, fp_byte) in operations {
            sqlx::query(
                "INSERT INTO authorization_operation_receipts \
                 (community_id, operation_id, request_fingerprint, operation_kind, \
                  actor_fingerprint, outcome_code, result_digest) \
                 VALUES ($1, $2, $3, 12, $4, 1, $5)",
            )
            .bind(community_id)
            .bind(operation_id)
            .bind(vec![fp_byte; 32])
            .bind(vec![0xAA_u8; 32])
            .bind(vec![0xBB_u8; 32])
            .execute(&pool)
            .await
            .expect("seed operation receipt");
        }

        // selector_kind 3 requires binding_version_floor, so this row exercises
        // both monotonic dimensions the guard still governs.
        let selector_fingerprint = vec![0xCC_u8; 32];
        sqlx::query(
            "INSERT INTO authorization_invalidation_floors \
             (community_id, selector_kind, selector_fingerprint, floor_generation, \
              binding_version_floor, operation_id, request_fingerprint, updated_at) \
             VALUES ($1, 3, $2, 1, 1, $3, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(&selector_fingerprint)
        .bind(operations[0].0)
        .bind(vec![operations[0].1; 32])
        .execute(&pool)
        .await
        .expect("insert initial invalidation floor");

        let advance =
            |generation: i64, binding_floor: i64, op_index: usize, updated_at: &'static str| {
                sqlx::query(
                    "UPDATE authorization_invalidation_floors \
                 SET floor_generation = $1, binding_version_floor = $2, \
                     operation_id = $3, request_fingerprint = $4, updated_at = $5::timestamptz \
                 WHERE community_id = $6 AND selector_kind = 3 AND selector_fingerprint = $7",
                )
                .bind(generation)
                .bind(binding_floor)
                .bind(operations[op_index].0)
                .bind(vec![operations[op_index].1; 32])
                .bind(updated_at)
                .bind(community_id)
                .bind(selector_fingerprint.clone())
                .execute(&pool)
            };

        // Forward generation advance commits.
        advance(2, 1, 1, "2026-01-01T00:01:00Z")
            .await
            .expect("forward floor_generation advance must pass the guard");

        // Forward binding_version_floor advance commits (generation unchanged).
        advance(2, 2, 2, "2026-01-01T00:02:00Z")
            .await
            .expect("forward binding_version_floor advance must pass the guard");

        // Regressive generation is rejected.
        let regressive = advance(1, 2, 3, "2026-01-01T00:03:00Z")
            .await
            .expect_err("regressive floor_generation must be rejected");
        assert!(
            regressive.to_string().contains("cannot move backward"),
            "expected monotonic rejection, got: {regressive}"
        );

        // Equal floors with only a new operation is a rejected no-op advance.
        let no_op = advance(2, 2, 3, "2026-01-01T00:03:00Z")
            .await
            .expect_err("equal-floor no-op advance must be rejected");
        assert!(
            no_op.to_string().contains("cannot move backward"),
            "expected no-op rejection, got: {no_op}"
        );

        // The committed state reflects only the two accepted advances.
        let (generation, binding_floor): (i64, i64) = sqlx::query_as(
            "SELECT floor_generation, binding_version_floor \
             FROM authorization_invalidation_floors \
             WHERE community_id = $1 AND selector_kind = 3 AND selector_fingerprint = $2",
        )
        .bind(community_id)
        .bind(&selector_fingerprint)
        .fetch_one(&pool)
        .await
        .expect("read final floor state");
        assert_eq!(
            (generation, binding_floor),
            (2, 2),
            "only the accepted forward advances may persist"
        );
    }

    /// NIP-FI identity FK contract: a binding's provenance is determined from
    /// operation evidence and is independent of the enrollment policy's mode.
    /// The corrected FK references only `(community_id, policy_revision)`;
    /// the original composite FK `(community_id, policy_revision,
    /// binding_provenance) → (community_id, policy_revision, enrollment_mode)`
    /// would have rejected valid admissions such as TOFU policy +
    /// attested-key provenance (NIP-FI.md §352, §424).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_binding_provenance_is_independent_of_enrollment_mode() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(41, &pool)
            .await
            .expect("apply migrations 1-41");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("provenance-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // Enrollment policy: mode 3 (TOFU).
        let policy_revision: i64 = 1;
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, $2, 3, $3, '2026-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(policy_revision)
        .bind(vec![0xA0_u8; 32]) // policy_digest
        .execute(&pool)
        .await
        .expect("insert TOFU enrollment policy");

        // Insert a binding with provenance 1 (attested-key) under the TOFU
        // policy.  The circular deferred FK between identity_bindings and
        // identity_lifecycle_history requires both to be committed in one
        // transaction; all cross-table FKs in this pair are DEFERRABLE
        // INITIALLY DEFERRED.  A pinned connection is required so that BEGIN
        // and each subsequent statement share the same session/transaction.
        let binding_id = uuid::Uuid::new_v4();
        let history_id = uuid::Uuid::new_v4();
        let operation_id = uuid::Uuid::new_v4();
        let request_fingerprint = vec![0xAB_u8; 32];

        let mut conn = pool.acquire().await.expect("acquire connection");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");

        // Enrollment history must be inserted BEFORE the operation receipt:
        // authorization_operation_receipt_history_guard_v1 fires AFTER INSERT
        // on authorization_operation_receipts and checks that lifecycle receipts
        // already have exactly one history row.  The history → receipt FK is
        // DEFERRABLE INITIALLY DEFERRED, so this order is safe.
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id, history_id, transition_kind, outcome_code, \
              successor_binding_id, successor_binding_version, \
              successor_lifecycle_revision, successor_state, \
              operation_id, request_fingerprint, transition_digest) \
             VALUES ($1, $2, 1, 1, $3, 1, 1, 1, $4, $5, $6)",
        )
        .bind(community_id)
        .bind(history_id)
        .bind(binding_id)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(vec![0xAE_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert lifecycle history");

        // Operation receipt: kind 1 (enroll), outcome 1 (applied).
        // The receipt_history_cardinality trigger fires here and validates the
        // history row inserted above.
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(vec![0xAC_u8; 32])
        .bind(vec![0xAD_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert operation receipt");

        // Binding: provenance 1 (attested-key) under TOFU-mode policy.
        // Before the FK fix this INSERT would fail at commit with a FK
        // violation because 1 (attested-key) ≠ 3 (TOFU mode).
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, binding_id, issuer, subject, \
              principal_fingerprint, event_author_pubkey, \
              binding_state, lifecycle_revision, binding_provenance, \
              policy_revision, enrollment_evidence_digest, \
              birth_history_id, creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1, $2, 'https://issuer.example', 'sub-01', \
                     $3, $4, 1, 1, 1, $5, $6, $7, $8, $9)",
        )
        .bind(community_id)
        .bind(binding_id)
        .bind(vec![0xAF_u8; 32]) // principal_fingerprint
        .bind(vec![0xB0_u8; 32]) // event_author_pubkey
        .bind(policy_revision)
        .bind(vec![0xB1_u8; 32]) // enrollment_evidence_digest
        .bind(history_id)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .execute(&mut *conn)
        .await
        .expect("insert binding");

        sqlx::query("COMMIT")
            .execute(&mut *conn)
            .await
            .expect("attested-key binding under TOFU policy must commit — FK is on (community_id, policy_revision) only");

        // Confirm the binding persisted with provenance 1, policy mode 3.
        let (stored_provenance, stored_mode): (i16, i16) = sqlx::query_as(
            "SELECT b.binding_provenance, p.enrollment_mode \
             FROM identity_bindings b \
             JOIN identity_enrollment_policies p \
               ON p.community_id = b.community_id AND p.policy_revision = b.policy_revision \
             WHERE b.community_id = $1 AND b.binding_id = $2",
        )
        .bind(community_id)
        .bind(binding_id)
        .fetch_one(&pool)
        .await
        .expect("read persisted binding");
        assert_eq!(stored_provenance, 1, "provenance must be attested-key (1)");
        assert_eq!(stored_mode, 3, "enrollment mode must be TOFU (3)");
        assert_ne!(
            stored_provenance, stored_mode,
            "provenance and mode are independent: they must differ here"
        );

        // --- Negative half: absent policy revision ---
        //
        // Two-sided mutation sensitivity requires that a FK dropped or neutered
        // entirely is also detected.  A second otherwise-valid deferred
        // transaction uses a nonexistent policy_revision (999) and must fail
        // with SQLSTATE 23503 — the narrowed FK
        // identity_bindings(community_id, policy_revision)
        //   → identity_enrollment_policies(community_id, policy_revision)
        // rejects the row.  This FK is not deferred, so it fires at INSERT
        // time; a `COMMIT` is unnecessary and not reached.  If the FK were
        // absent the INSERT would succeed and this assertion would catch the
        // regression.
        let absent_binding_id = uuid::Uuid::new_v4();
        let absent_history_id = uuid::Uuid::new_v4();
        let absent_operation_id = uuid::Uuid::new_v4();
        let absent_fp = vec![0xC0_u8; 32];
        let nonexistent_policy_revision: i64 = 999;

        let mut conn2 = pool.acquire().await.expect("acquire second connection");

        sqlx::query("BEGIN")
            .execute(&mut *conn2)
            .await
            .expect("begin absent-policy transaction");

        // History first (receipt_history_cardinality guard fires on receipt
        // insert and requires the history row to already exist).
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id, history_id, transition_kind, outcome_code, \
              successor_binding_id, successor_binding_version, \
              successor_lifecycle_revision, successor_state, \
              operation_id, request_fingerprint, transition_digest) \
             VALUES ($1, $2, 1, 1, $3, 2, 1, 1, $4, $5, $6)",
        )
        .bind(community_id)
        .bind(absent_history_id)
        .bind(absent_binding_id)
        .bind(absent_operation_id)
        .bind(&absent_fp)
        .bind(vec![0xC1_u8; 32])
        .execute(&mut *conn2)
        .await
        .expect("insert absent-policy lifecycle history");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(absent_operation_id)
        .bind(&absent_fp)
        .bind(vec![0xC2_u8; 32])
        .bind(vec![0xC3_u8; 32])
        .execute(&mut *conn2)
        .await
        .expect("insert absent-policy operation receipt");

        // The policy FK is not deferred; it fires at INSERT, not COMMIT.
        let absent_policy_err = sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, binding_id, issuer, subject, \
              principal_fingerprint, event_author_pubkey, \
              binding_state, lifecycle_revision, binding_provenance, \
              policy_revision, enrollment_evidence_digest, \
              birth_history_id, creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1, $2, 'https://issuer.example', 'sub-02', \
                     $3, $4, 1, 1, 1, $5, $6, $7, $8, $9)",
        )
        .bind(community_id)
        .bind(absent_binding_id)
        .bind(vec![0xC4_u8; 32]) // principal_fingerprint (unique, different from first binding)
        .bind(vec![0xC5_u8; 32]) // event_author_pubkey (unique, different from first binding)
        .bind(nonexistent_policy_revision)
        .bind(vec![0xC6_u8; 32]) // enrollment_evidence_digest
        .bind(absent_history_id)
        .bind(absent_operation_id)
        .bind(&absent_fp)
        .execute(&mut *conn2)
        .await
        .expect_err("binding with nonexistent policy_revision must be rejected by the FK");
        assert!(
            absent_policy_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23503"))
                .unwrap_or(false),
            "expected FK violation (23503) for absent policy_revision, got: {absent_policy_err}"
        );

        sqlx::query("ROLLBACK")
            .execute(&mut *conn2)
            .await
            .expect("rollback absent-policy transaction");
    }

    /// NIP-FI policy-revision monotonicity: each new policy revision for a
    /// community must strictly exceed the current maximum revision
    /// (FI-INV-06 — stable assertion policy). `effective_at` ordering is
    /// deliberately not enforced — the downstream constructor stamps every
    /// immediately-effective revision with Unix epoch.
    ///
    /// Mutation sensitivity is two-sided:
    /// - neutering the guard lets a replayed or backfilled revision through
    ///   (the positive half detects insertion into a guarded table);
    /// - leaving the guard intact rejects equal/regressive inserts (negative
    ///   halves detect that each rejection fires).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_enrollment_policy_revision_is_monotonic() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(41, &pool)
            .await
            .expect("apply migrations 1-41");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("policy-mono-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // First insertion: no prior rows — should always succeed.
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 1, 1, $2, '2026-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA1_u8; 32])
        .execute(&pool)
        .await
        .expect("first policy insertion (revision 1) must succeed");

        // Forward advance: revision 2.
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 2, 1, $2, '2026-06-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA2_u8; 32])
        .execute(&pool)
        .await
        .expect("forward advance to revision 2 must succeed");

        // Seed a gap: skip from 2 to 100, then advance to 101.
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 100, 1, $2, '2027-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA3_u8; 32])
        .execute(&pool)
        .await
        .expect("jump to revision 100 must succeed");

        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 101, 1, $2, '2027-06-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA4_u8; 32])
        .execute(&pool)
        .await
        .expect("advance to revision 101 must succeed");

        // Negative: unused lower revision 99 — not a PK duplicate (never inserted),
        // but the guard must reject it because 99 < MAX(100, 101). This is the
        // case a plain PK constraint cannot catch; the named guard must fire.
        let backfill_err = sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 99, 1, $2, '2028-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA5_u8; 32])
        .execute(&pool)
        .await
        .expect_err("unused lower revision 99 must be rejected by the guard");
        assert!(
            backfill_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) from identity_enrollment_policy_revision_monotonic \
             guard for backfilled revision 99, got: {backfill_err}"
        );

        // Negative: equal revision (101 <= 101). The PK is (community_id, policy_revision)
        // so this is a PK duplicate regardless of policy_digest; either 23505 from the PK
        // or 23514 from the guard fires first. This case is secondary — the load-bearing
        // proof is the unused-99 case above, which is not a PK duplicate.
        let replay_err = sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 101, 2, $2, '2028-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xA6_u8; 32])
        .execute(&pool)
        .await
        .expect_err("equal revision must be rejected");
        // PK (23505) or guard (23514) — either proves the insert cannot commit.
        assert!(
            replay_err
                .as_database_error()
                .map(|e| {
                    let code = e.code();
                    let c = code.as_deref().unwrap_or("");
                    c == "23514" || c == "23505"
                })
                .unwrap_or(false),
            "expected check_violation (23514) or unique_violation (23505) for replayed revision, \
             got: {replay_err}"
        );

        // Concurrency regression: prove the advisory lock is load-bearing. The
        // test uses a controlled two-connection schedule:
        //
        //   1. tx1 opens a transaction and inserts revision 102. The BEFORE INSERT
        //      trigger acquires `pg_advisory_xact_lock(lock_key)` and completes the
        //      INSERT — tx1 now holds the advisory lock until it commits.
        //   2. tx2 opens a transaction on a second backend and issues INSERT for
        //      revision 103. The trigger fires and blocks inside
        //      `pg_advisory_xact_lock(lock_key)` waiting for tx1 to release.
        //   3. We observe tx2's backend entering a Lock-wait state via
        //      pg_stat_activity (wait_event_type='Lock', wait_event='advisory'),
        //      with a bounded timeout — not a sleep. If the advisory-lock call is
        //      removed from the guard, the trigger returns immediately; tx2 never
        //      enters the advisory wait, and the poll times out, failing the test.
        //      This is the mutation-sensitivity guarantee.
        //   4. tx1 commits, releasing the advisory lock. tx2 unblocks, its trigger
        //      reads the fresh MAX=102, and the INSERT succeeds (103 > 102).
        //   5. tx2 commits. Both revisions 102 and 103 are present.
        use std::time::Instant;

        // tx1: open a transaction and insert revision 102. The INSERT returns after
        // the trigger acquires the lock and succeeds; the advisory lock stays held
        // until the transaction commits.
        let mut conn1 = pool.acquire().await.expect("acquire conn1");
        sqlx::query("BEGIN")
            .execute(&mut *conn1)
            .await
            .expect("begin tx1");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, 102, 1, $2, '2029-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(vec![0xB1_u8; 32])
        .execute(&mut *conn1)
        .await
        .expect("tx1 INSERT revision 102 must succeed");
        // tx1 holds the advisory lock. Do NOT commit yet.

        // tx2: acquire a separate backend, record its PID, then issue the INSERT.
        // The trigger will block on the advisory lock held by tx1.
        let pool2 = pool.clone();
        let pool3 = pool.clone();
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let tx2_task = tokio::spawn(async move {
            let mut conn2 = pool2.acquire().await.expect("acquire conn2");
            // Report this backend's PID so the observer can poll pg_stat_activity.
            let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *conn2)
                .await
                .expect("get conn2 backend pid");
            let _ = pid_tx.send(backend_pid);
            sqlx::query("BEGIN")
                .execute(&mut *conn2)
                .await
                .expect("begin tx2");
            // This INSERT will block inside the trigger waiting for tx1's advisory lock.
            let insert_r = sqlx::query(
                "INSERT INTO identity_enrollment_policies \
                 (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
                 VALUES ($1, 103, 1, $2, '2029-06-01T00:00:00Z')",
            )
            .bind(community_id)
            .bind(vec![0xB2_u8; 32])
            .execute(&mut *conn2)
            .await;
            let commit_r = sqlx::query("COMMIT").execute(&mut *conn2).await;
            (insert_r, commit_r)
        });

        // Receive tx2's backend PID and wait until it enters an advisory-lock wait.
        // Mutation proof: without pg_advisory_xact_lock in the guard, the trigger
        // returns immediately; tx2 never parks on an advisory lock; the poll below
        // times out and panics, making this test deterministically red.
        let tx2_pid = pid_rx.await.expect("tx2 reports its backend pid");
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (\
                     SELECT 1 FROM pg_stat_activity \
                     WHERE pid = $1 \
                       AND wait_event_type = 'Lock' \
                       AND wait_event = 'advisory'\
                 )",
            )
            .bind(tx2_pid)
            .fetch_one(&pool3)
            .await
            .expect("poll tx2 advisory-lock wait");
            if waiting {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "tx2 never entered advisory-lock wait — pg_advisory_xact_lock \
                 must be present in the guard for the lock to serialize writers"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        // tx2 is observably blocked. Commit tx1, releasing the advisory lock.
        sqlx::query("COMMIT")
            .execute(&mut *conn1)
            .await
            .expect("tx1 COMMIT must succeed");

        // tx2 unblocks: the trigger re-runs its SELECT MAX, sees committed 102,
        // and INSERT 103 succeeds. Both the INSERT and COMMIT must complete.
        let (insert2, commit2) = tx2_task.await.expect("tx2 task completed");
        insert2.expect("tx2 INSERT revision 103 must succeed after tx1 commits");
        commit2.expect("tx2 COMMIT must succeed");

        // Both revisions 102 and 103 must be present (total: 1, 2, 100, 101, 102, 103).
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM identity_enrollment_policies WHERE community_id = $1",
        )
        .bind(community_id)
        .fetch_one(&pool)
        .await
        .expect("count persisted policy revisions");
        assert_eq!(
            count, 6,
            "exactly six revisions must persist after the controlled concurrency sequence"
        );
    }

    /// NIP-FI admission-result ↔ kind-11 receipt cardinality: a kind-11
    /// (protected-mutation) receipt must commit with exactly one admission
    /// result; an admission result must commit against a kind-11 receipt.
    ///
    /// Mutation sensitivity is two-sided:
    /// - the guard is load-bearing when a kind-11 receipt has no result row
    ///   (negative A) — without the guard this commits silently;
    /// - the guard is load-bearing when a result attaches to a non-kind-11
    ///   receipt (negative B) — without the guard this commits silently.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authorization_admission_result_requires_kind_11_receipt_bidirectional() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("adm-result-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // Capacity must exist for authorization_events inserts; admission-result
        // tests exercise only authorization_operation_receipts and
        // authorization_admission_results — no authorization_events rows are
        // needed here, but insert capacity anyway to satisfy any trigger
        // that reads the policy row defensively.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        let mut conn = pool.acquire().await.expect("acquire connection");

        // --- Positive: kind-11 receipt + admission result in one transaction ---
        let op1 = uuid::Uuid::new_v4();
        let fp1 = vec![0xB1_u8; 32];

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op1)
        .bind(&fp1)
        .bind(vec![0xB2_u8; 32])
        .bind(vec![0xB3_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert kind-11 receipt");

        sqlx::query(
            "INSERT INTO authorization_admission_results \
             (community_id, operation_id, request_fingerprint, semantic_fingerprint, \
              object_kind, object_key) \
             VALUES ($1, $2, $3, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op1)
        .bind(&fp1)
        .bind(vec![0xB4_u8; 32]) // semantic_fingerprint
        .bind(vec![0xB5_u8; 32]) // object_key
        .execute(&mut *conn)
        .await
        .expect("insert admission result");

        sqlx::query("COMMIT")
            .execute(&mut *conn)
            .await
            .expect("kind-11 receipt + result must commit");
        drop(conn);

        // --- Negative A: kind-11 receipt without result must be rejected ---
        let op2 = uuid::Uuid::new_v4();
        let fp2 = vec![0xC1_u8; 32];

        let mut conn_a = pool.acquire().await.expect("acquire connection A");
        sqlx::query("BEGIN")
            .execute(&mut *conn_a)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op2)
        .bind(&fp2)
        .bind(vec![0xC2_u8; 32])
        .bind(vec![0xC3_u8; 32])
        .execute(&mut *conn_a)
        .await
        .expect("insert kind-11 receipt for negative A");

        let no_result_err = sqlx::query("COMMIT")
            .execute(&mut *conn_a)
            .await
            .expect_err("kind-11 receipt without result must be rejected at commit");
        assert!(
            no_result_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) for kind-11 without result, got: {no_result_err}"
        );
        drop(conn_a);

        // --- Negative B: admission result against non-kind-11 receipt ---
        // Use operation_kind 12 (invalidation) — no admission result should
        // ever attach to it. The guard fires at COMMIT (deferred trigger).
        let op3 = uuid::Uuid::new_v4();
        let fp3 = vec![0xD1_u8; 32];

        let mut conn_b = pool.acquire().await.expect("acquire connection B");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 12, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op3)
        .bind(&fp3)
        .bind(vec![0xD2_u8; 32])
        .bind(vec![0xD3_u8; 32])
        .execute(&mut *conn_b)
        .await
        .expect("insert kind-12 receipt");

        // The guard is deferred: the INSERT succeeds; the violation surfaces
        // at COMMIT when the guard checks that the receipt is kind-11.
        sqlx::query(
            "INSERT INTO authorization_admission_results \
             (community_id, operation_id, request_fingerprint, semantic_fingerprint, \
              object_kind, object_key) \
             VALUES ($1, $2, $3, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op3)
        .bind(&fp3)
        .bind(vec![0xD4_u8; 32])
        .bind(vec![0xD5_u8; 32])
        .execute(&mut *conn_b)
        .await
        .expect("result insert must pass — deferred guard fires at commit, not here");

        let wrong_kind_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b)
            .await
            .expect_err("result against non-kind-11 receipt must be rejected at commit");
        assert!(
            wrong_kind_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) for result against non-kind-11 receipt, got: {wrong_kind_err}"
        );
        drop(conn_b);

        // --- Negative C: mismatched request_fingerprint rejected by composite FK ---
        // The admission result table has an immediate composite FK
        //   (community_id, operation_id, request_fingerprint)
        //   REFERENCES authorization_operation_receipts(...)
        // A result referencing a receipt that exists but with a different
        // request_fingerprint must be rejected. This exercises the semantic half
        // of Carl finding 2 — cardinality is handled by the deferred trigger;
        // coordinate binding is handled by the structural FK.
        let op4 = uuid::Uuid::new_v4();
        let fp4_receipt = vec![0xE1_u8; 32]; // fingerprint stored in the receipt
        let fp4_wrong = vec![0xE2_u8; 32]; // wrong fingerprint used in the result

        let mut conn_c = pool.acquire().await.expect("acquire connection C");
        sqlx::query("BEGIN")
            .execute(&mut *conn_c)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 11, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op4)
        .bind(&fp4_receipt)
        .bind(vec![0xE3_u8; 32])
        .bind(vec![0xE4_u8; 32])
        .execute(&mut *conn_c)
        .await
        .expect("insert kind-11 receipt for negative C");

        // The admission result FK is immediate (not deferred), so the INSERT
        // itself rejects a fingerprint with no matching receipt row.
        let wrong_fp_err = sqlx::query(
            "INSERT INTO authorization_admission_results \
             (community_id, operation_id, request_fingerprint, semantic_fingerprint, \
              object_kind, object_key) \
             VALUES ($1, $2, $3, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op4)
        .bind(&fp4_wrong) // wrong fingerprint — no matching receipt row
        .bind(vec![0xE5_u8; 32])
        .bind(vec![0xE6_u8; 32])
        .execute(&mut *conn_c)
        .await
        .expect_err("result with mismatched request_fingerprint must be rejected at INSERT");
        // Immediate composite FK fires as foreign_key_violation (23503).
        assert!(
            wrong_fp_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23503"))
                .unwrap_or(false),
            "expected foreign_key_violation (23503) for mismatched request_fingerprint, got: {wrong_fp_err}"
        );
        sqlx::query("ROLLBACK").execute(&mut *conn_c).await.ok();
    }

    /// NIP-FI denial-attempt ↔ kind-9 event cardinality: a kind-9
    /// (pre-authentication denial) audit event must commit with exactly one
    /// denial attempt; a denial attempt must commit with a matching kind-9
    /// audit event.
    ///
    /// Mutation sensitivity is two-sided:
    /// - the event-side guard is load-bearing when a kind-9 event has no
    ///   attempt row (negative A) — without it this commits silently, making
    ///   replay reconstruction impossible;
    /// - the attempt-side guard is load-bearing for semantic mismatches (negatives
    ///   B1–B3) — the old deferred FK only checks event existence/kind and would
    ///   not catch a correlation, reason_code, or attempt_id mismatch.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authorization_denial_attempt_requires_kind_9_event_bidirectional() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("denial-attempt-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // Capacity is required by the authorization_events BEFORE INSERT trigger.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        let mut conn = pool.acquire().await.expect("acquire connection");

        // --- Positive: kind-9 event + denial attempt in one transaction ---
        let op1 = uuid::Uuid::new_v4();
        let event1 = uuid::Uuid::new_v4();
        let corr1 = uuid::Uuid::new_v4();
        let attempt1_id = uuid::Uuid::new_v4();

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");

        // Insert denial attempt first (FKs are deferred).
        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
        )
        .bind(community_id)
        .bind(op1)
        .bind(corr1)
        .bind(vec![0xE1_u8; 32]) // semantic_fingerprint
        .bind(attempt1_id)
        .bind(event1)
        .execute(&mut *conn)
        .await
        .expect("insert denial attempt before event");

        // Insert the kind-9 event (actor_kind 4, no request_fingerprint).
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 2, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event1)
        .bind(op1)
        .bind(corr1)
        .bind(attempt1_id)
        .bind(vec![0xE1_u8; 32]) // semantic_fingerprint matches denial attempt
        .bind(vec![0xE2_u8; 64]) // canonical_envelope (≤16384 bytes)
        .bind(vec![0xE3_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("insert kind-9 event");

        sqlx::query("COMMIT")
            .execute(&mut *conn)
            .await
            .expect("kind-9 event + denial attempt must commit");
        drop(conn);

        // --- Negative A: kind-9 event alone must be rejected at commit ---
        let op2 = uuid::Uuid::new_v4();
        let event2 = uuid::Uuid::new_v4();
        let corr2 = uuid::Uuid::new_v4();
        let attempt2_id = uuid::Uuid::new_v4();

        let mut conn_a = pool.acquire().await.expect("acquire connection A");
        sqlx::query("BEGIN")
            .execute(&mut *conn_a)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 1, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event2)
        .bind(op2)
        .bind(corr2)
        .bind(attempt2_id)
        .bind(vec![0xF0_u8; 32]) // semantic_fingerprint (non-zero)
        .bind(vec![0xF1_u8; 64])
        .bind(vec![0xF2_u8; 32])
        .execute(&mut *conn_a)
        .await
        .expect("insert kind-9 event without attempt");

        let no_attempt_err = sqlx::query("COMMIT")
            .execute(&mut *conn_a)
            .await
            .expect_err("kind-9 event without denial attempt must be rejected at commit");
        assert!(
            no_attempt_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) for kind-9 event without attempt, got: {no_attempt_err}"
        );
        drop(conn_a);

        // --- Negatives B1-B3: semantic coordinate mismatches, each attributed to
        // the named guard (23514), not the old deferred FK (23503). Each case
        // inserts a valid event then a denial attempt that matches everywhere
        // except one coordinate; the guard must fire for that mismatch.

        // B1: correlation_id mismatch — attempt carries a different correlation
        // than the event it references.
        let op_b1 = uuid::Uuid::new_v4();
        let event_b1 = uuid::Uuid::new_v4();
        let corr_b1_event = uuid::Uuid::new_v4();
        let corr_b1_wrong = uuid::Uuid::new_v4(); // different from corr_b1_event
        let attempt_b1 = uuid::Uuid::new_v4();

        let mut conn_b1 = pool.acquire().await.expect("acquire connection B1");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b1)
            .await
            .expect("begin B1");

        // Insert the event first (deferred FK allows this ordering).
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 2, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event_b1)
        .bind(op_b1)
        .bind(corr_b1_event)
        .bind(attempt_b1)
        .bind(vec![0xB3_u8; 32]) // semantic_fingerprint matches denial attempt
        .bind(vec![0xB1_u8; 64])
        .bind(vec![0xB2_u8; 32])
        .execute(&mut *conn_b1)
        .await
        .expect("insert kind-9 event for B1");

        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
        )
        .bind(community_id)
        .bind(op_b1)
        .bind(corr_b1_wrong) // wrong correlation_id
        .bind(vec![0xB3_u8; 32])
        .bind(attempt_b1)
        .bind(event_b1)
        .execute(&mut *conn_b1)
        .await
        .expect("insert denial attempt with wrong correlation_id (guard deferred)");

        let corr_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b1)
            .await
            .expect_err("mismatched correlation_id must be rejected at commit");
        assert!(
            corr_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) from authorization_denial_attempt_semantic_binding \
             for correlation_id mismatch, got: {corr_err}"
        );
        drop(conn_b1);

        // B2: reason_code mismatch — attempt carries denial_reason=1 (MissingCredential,
        // requires reason_code=2 per canonical mapping), but event carries reason_code=1.
        // The attempt INSERT passes (denial_reason=1↔reason_code=2 is a valid mapping pair),
        // then the deferred guard fires at commit because event reason_code=1 ≠ attempt
        // reason_code=2.
        let op_b2 = uuid::Uuid::new_v4();
        let event_b2 = uuid::Uuid::new_v4();
        let corr_b2 = uuid::Uuid::new_v4();
        let attempt_b2 = uuid::Uuid::new_v4();

        let mut conn_b2 = pool.acquire().await.expect("acquire connection B2");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b2)
            .await
            .expect("begin B2");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 1, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event_b2)
        .bind(op_b2)
        .bind(corr_b2)
        .bind(attempt_b2)
        .bind(vec![0xC3_u8; 32]) // semantic_fingerprint matches denial attempt
        .bind(vec![0xC1_u8; 64])
        .bind(vec![0xC2_u8; 32])
        .execute(&mut *conn_b2)
        .await
        .expect("insert kind-9 event for B2 (reason_code=1)");

        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
            // reason_code = 2 but event has reason_code = 1
        )
        .bind(community_id)
        .bind(op_b2)
        .bind(corr_b2)
        .bind(vec![0xC3_u8; 32])
        .bind(attempt_b2)
        .bind(event_b2)
        .execute(&mut *conn_b2)
        .await
        .expect(
            "insert denial attempt with wrong reason_code (deferred guard will fire at commit)",
        );

        let reason_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b2)
            .await
            .expect_err("mismatched reason_code must be rejected at commit");
        assert!(
            reason_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) from authorization_denial_attempt_semantic_binding \
             for reason_code mismatch, got: {reason_err}"
        );
        drop(conn_b2);

        // B3: attempt_id mismatch — the denial attempt's attempt_id FK references
        // a different event (attempt_b3_wrong) than the one being paired (event_b3).
        // The attempt_id FK on the denial attempt table binds
        //   (community_id, operation_id, audit_event_kind, attempt_id)
        //   -> authorization_events(community_id, operation_id, event_kind, attempt_id)
        // so using a different attempt_id that doesn't exist for this operation
        // will be caught as a FK violation (23503) at commit.
        let op_b3 = uuid::Uuid::new_v4();
        let event_b3 = uuid::Uuid::new_v4();
        let corr_b3 = uuid::Uuid::new_v4();
        let attempt_b3_correct = uuid::Uuid::new_v4();
        let attempt_b3_wrong = uuid::Uuid::new_v4(); // not registered for this operation

        let mut conn_b3 = pool.acquire().await.expect("acquire connection B3");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b3)
            .await
            .expect("begin B3");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 2, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event_b3)
        .bind(op_b3)
        .bind(corr_b3)
        .bind(attempt_b3_correct)
        .bind(vec![0xD3_u8; 32]) // semantic_fingerprint (matches denial attempt)
        .bind(vec![0xD1_u8; 64])
        .bind(vec![0xD2_u8; 32])
        .execute(&mut *conn_b3)
        .await
        .expect("insert kind-9 event for B3");

        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
        )
        .bind(community_id)
        .bind(op_b3)
        .bind(corr_b3)
        .bind(vec![0xD3_u8; 32])
        .bind(attempt_b3_wrong) // wrong attempt_id — no matching UNIQUE row on events
        .bind(event_b3)
        .execute(&mut *conn_b3)
        .await
        .expect("insert denial attempt with wrong attempt_id (FK is deferred)");

        let attempt_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b3)
            .await
            .expect_err("mismatched attempt_id must be rejected at commit");
        // The attempt_id FK is deferred and fires as foreign_key_violation (23503).
        assert!(
            attempt_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23503"))
                .unwrap_or(false),
            "expected foreign_key_violation (23503) for attempt_id mismatch \
             (deferred FK on denial attempt), got: {attempt_err}"
        );

        // B4: denial_reason ↔ reason_code mapping violation — the denial attempt
        // row carries denial_reason=2 (InvalidCredential) but reason_code=2
        // (Missing). The canonical mapping requires InvalidCredential(2)↔Invalid(3);
        // reason_code=2 is only valid for MissingCredential(denial_reason=1).
        // The immediate CHECK constraint authorization_denial_reason_reason_code_binding
        // fires at INSERT, not commit. Mutation-sensitive: removing the CHECK lets
        // this INSERT succeed (the guard does not compare denial_reason; only the
        // paired event's reason_code is checked at commit).
        let op_b4 = uuid::Uuid::new_v4();
        let event_b4 = uuid::Uuid::new_v4();
        let corr_b4 = uuid::Uuid::new_v4();
        let attempt_b4 = uuid::Uuid::new_v4();

        let mut conn_b4 = pool.acquire().await.expect("acquire connection B4");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b4)
            .await
            .expect("begin B4");

        // Insert the matching kind-9 event first.
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 2, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event_b4)
        .bind(op_b4)
        .bind(corr_b4)
        .bind(attempt_b4)
        .bind(vec![0xE4_u8; 32]) // semantic_fingerprint
        .bind(vec![0xE5_u8; 64])
        .bind(vec![0xE6_u8; 32])
        .execute(&mut *conn_b4)
        .await
        .expect("insert kind-9 event for B4");

        // Insert denial attempt with denial_reason=2 (InvalidCredential) but
        // reason_code=2 (Missing) — violates the canonical mapping (requires reason_code=3).
        let denial_reason_err = sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 2, 1, 1, 2, $5, $6, 9)",
            // denial_reason=2 (InvalidCredential) requires reason_code=3; reason_code=2 is wrong
        )
        .bind(community_id)
        .bind(op_b4)
        .bind(corr_b4)
        .bind(vec![0xE4_u8; 32])
        .bind(attempt_b4)
        .bind(event_b4)
        .execute(&mut *conn_b4)
        .await
        .expect_err("denial_reason/reason_code mapping violation must be rejected at INSERT");

        assert!(
            denial_reason_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) from \
             authorization_denial_reason_reason_code_binding for denial_reason mismatch, \
             got: {denial_reason_err}"
        );

        // B5: semantic_fingerprint mismatch — the event carries semantic_fingerprint
        // 0xB5…B5 while the denial attempt carries 0xB6…B6. correlation_id, reason_code,
        // and attempt_id all match; only the fingerprint differs. The deferred guard
        // authorization_denial_attempt_guard_v1 fires at COMMIT on the denial-attempt
        // side, compares found_semantic_fingerprint (from the event) with
        // NEW.semantic_fingerprint (from the attempt), and raises 23514 with named
        // constraint authorization_denial_attempt_semantic_binding.
        // Mutation-sensitive: removing the semantic_fingerprint comparison block from
        // the guard function lets this transaction commit.
        let op_b5 = uuid::Uuid::new_v4();
        let event_b5 = uuid::Uuid::new_v4();
        let corr_b5 = uuid::Uuid::new_v4();
        let attempt_b5 = uuid::Uuid::new_v4();
        let fp_event_b5 = vec![0xB5_u8; 32]; // event semantic_fingerprint
        let fp_attempt_b5 = vec![0xB6_u8; 32]; // mismatched attempt semantic_fingerprint

        let mut conn_b5 = pool.acquire().await.expect("acquire connection B5");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b5)
            .await
            .expect("begin B5");

        // Insert the kind-9 event with fingerprint 0xB5…B5.
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, operation_id, correlation_id, attempt_id, \
              semantic_fingerprint, occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 2, 4, $3, $4, $5, \
                     $6, '2026-01-01T00:00:00Z', $7, $8)",
        )
        .bind(community_id)
        .bind(event_b5)
        .bind(op_b5)
        .bind(corr_b5)
        .bind(attempt_b5)
        .bind(fp_event_b5)
        .bind(vec![0xB7_u8; 64]) // canonical_envelope
        .bind(vec![0xB8_u8; 32]) // envelope_digest
        .execute(&mut *conn_b5)
        .await
        .expect("insert kind-9 event for B5");

        // Insert denial attempt with the WRONG semantic_fingerprint (0xB6…B6).
        // correlation_id, reason_code=2, denial_reason=1 (MissingCredential↔Missing),
        // and attempt_id all match the event — only semantic_fingerprint differs.
        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
        )
        .bind(community_id)
        .bind(op_b5)
        .bind(corr_b5)
        .bind(fp_attempt_b5) // 0xB6…B6 ≠ event's 0xB5…B5
        .bind(attempt_b5)
        .bind(event_b5)
        .execute(&mut *conn_b5)
        .await
        .expect(
            "insert denial attempt with mismatched fingerprint (deferred guard fires at commit)",
        );

        let fp_mismatch_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b5)
            .await
            .expect_err("commit with mismatched semantic_fingerprint must be rejected");

        assert!(
            fp_mismatch_err
                .as_database_error()
                .map(|e| e.code().as_deref() == Some("23514"))
                .unwrap_or(false),
            "expected check_violation (23514) from \
             authorization_denial_attempt_semantic_binding for semantic_fingerprint mismatch, \
             got: {fp_mismatch_err}"
        );
    }

    /// NIP-FI authenticated kind-9 OperatorDenied denial: an authenticated
    /// kind-9 event (actor_kind 1–3, non-null request_fingerprint) must commit
    /// without an authorization_authentication_denial_attempts row and must
    /// reject any attempt to attach one.
    ///
    /// Mutation sensitivity:
    /// - Removing the `actor_kind <> 4` guard from the event-side trigger makes
    ///   positive A red: the COMMIT fails because the guard now requires a
    ///   denial-attempt row for the authenticated event and none is present.
    /// - Removing the `actor_kind <> 4` shape guard from the attempt-side
    ///   trigger makes negative B red: the COMMIT is rejected by the pre-existing
    ///   `authorization_denial_attempt_semantic_binding` guard instead (non-null
    ///   attempt `semantic_fingerprint` vs. null on the authenticated event), so
    ///   `assert_eq!` on the constraint name fails. The exact constraint name
    ///   assertion is therefore the load-bearing proof that the new shape guard —
    ///   not the pre-existing semantic-binding check — is what fires.
    ///
    /// The unresolved pre-auth positive path (actor_kind 4) is exercised in
    /// `authorization_denial_attempt_requires_kind_9_event_bidirectional` and
    /// is unchanged by this fix.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authenticated_kind_9_denial_commits_without_denial_attempt() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("auth-denial-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        // Seed an operation receipt for the authenticated denial. Use
        // operation_kind = 12 (invalidation) with outcome_code = 2 (denied):
        // this satisfies the receipt CHECK constraints without triggering the
        // lifecycle history guard (expected_count = 0 for non-lifecycle kinds)
        // and without requiring a lifecycle event (expected_event_kind = NULL).
        // The authorization_events FK on (community_id, operation_id,
        // request_fingerprint) requires a receipt row.
        let op_auth = uuid::Uuid::new_v4();
        let fp_auth = vec![0xA1_u8; 32];

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 12, $4, 2, $5)",
            // operation_kind 12 (invalidation), outcome_code 2 (denied)
        )
        .bind(community_id)
        .bind(op_auth)
        .bind(&fp_auth)
        .bind(vec![0xA2_u8; 32]) // actor_fingerprint
        .bind(vec![0xA3_u8; 32]) // result_digest
        .execute(&pool)
        .await
        .expect("seed authenticated denial receipt");

        let event_auth = uuid::Uuid::new_v4();
        let corr_auth = uuid::Uuid::new_v4();
        let attempt_auth = uuid::Uuid::new_v4();

        // --- Positive A: authenticated kind-9 denial (actor_kind = 1) commits
        // without any denial-attempt row. The semantic_fingerprint must be NULL
        // per the corrected shape CHECK. The deferred cardinality guard must
        // skip this event because actor_kind ≠ 4.
        let mut conn = pool.acquire().await.expect("acquire connection");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 9, 2, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
        )
        .bind(community_id)
        .bind(event_auth)
        .bind(vec![0xA4_u8; 32]) // actor_fingerprint (required for actor_kind 1)
        .bind(op_auth)
        .bind(&fp_auth) // non-null request_fingerprint (authenticated shape)
        .bind(corr_auth)
        .bind(attempt_auth)
        // semantic_fingerprint = NULL: authenticated kind-9 must not carry one
        .bind(vec![0xA5_u8; 64]) // canonical_envelope
        .bind(vec![0xA6_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("insert authenticated kind-9 event");

        sqlx::query("COMMIT").execute(&mut *conn).await.expect(
            "authenticated kind-9 denial must commit without a denial-attempt row \
                 — the event-side cardinality guard must skip actor_kind 1",
        );
        drop(conn);

        // Confirm no denial attempt was needed: the table must have zero rows
        // for this event.
        let attempt_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_authentication_denial_attempts \
             WHERE community_id = $1 AND audit_event_id = $2",
        )
        .bind(community_id)
        .bind(event_auth)
        .fetch_one(&pool)
        .await
        .expect("count denial attempts for authenticated event");
        assert_eq!(
            attempt_count, 0,
            "no denial-attempt row should exist for an authenticated kind-9 event"
        );

        // --- Negative B: a denial attempt cannot bind to the authenticated kind-9
        // event. The attempt-side shape guard must reject this at commit because
        // the referenced event has actor_kind = 1 (not 4). The rejection must
        // name the exact shape constraint (authorization_denial_attempt_event_kind)
        // rather than merely returning 23514, proving the new actor/request-fingerprint
        // guard fires — not the pre-existing semantic_fingerprint equality check
        // (which would fire as authorization_denial_attempt_semantic_binding if
        // the shape guard were absent, because the attempt carries a non-null
        // semantic_fingerprint while the authenticated event has null).
        //
        // Reuse attempt_auth from the committed event so the deferred attempt_id
        // FK resolves (wrong attempt_id would activate that FK first and make the
        // negative non-isolated to the new guard).
        let mut conn_b = pool.acquire().await.expect("acquire connection B");
        sqlx::query("BEGIN")
            .execute(&mut *conn_b)
            .await
            .expect("begin B");

        // Insert the denial attempt referencing the authenticated event.
        // The attempt table FKs are deferred, so this INSERT succeeds;
        // the shape guard fires at COMMIT.
        sqlx::query(
            "INSERT INTO authorization_authentication_denial_attempts \
             (community_id, operation_id, correlation_id, semantic_fingerprint, \
              denial_reason, expected_revision, action, reason_code, \
              attempt_id, audit_event_id, audit_event_kind) \
             VALUES ($1, $2, $3, $4, 1, 1, 1, 2, $5, $6, 9)",
        )
        .bind(community_id)
        .bind(op_auth)
        .bind(corr_auth)
        .bind(vec![0xA7_u8; 32]) // semantic_fingerprint on the attempt (non-null)
        .bind(attempt_auth) // reuse the event's attempt_id — FK isolation
        .bind(event_auth) // references the authenticated event (actor_kind = 1)
        .execute(&mut *conn_b)
        .await
        .expect("attempt INSERT must pass — shape guard is deferred");

        let cross_shape_err = sqlx::query("COMMIT")
            .execute(&mut *conn_b)
            .await
            .expect_err(
                "denial attempt binding to authenticated kind-9 event must be rejected at commit",
            );
        // The exact constraint name must be authorization_denial_attempt_event_kind —
        // the new actor/request_fingerprint shape guard. If the shape guard were
        // removed, the pre-existing semantic_fingerprint equality check would fire
        // instead, named authorization_denial_attempt_semantic_binding. Requiring
        // the exact name makes the mutation reliably red.
        assert_eq!(
            cross_shape_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_denial_attempt_event_kind"),
            "rejection must be attributed to authorization_denial_attempt_event_kind \
             shape guard (not an incidental FK or semantic-binding check), \
             got: {cross_shape_err}"
        );
    }

    /// NIP-FI denied lifecycle receipt: a denied core lifecycle receipt
    /// (outcome_code = 2) must commit without a paired audit event. Requiring
    /// one would falsely record that the lifecycle transition occurred.
    ///
    /// The denied branch forbids any event from the complete core
    /// success-transition class (kinds 1, 2, 3, 6). This test uses the mapped
    /// kind (kind 1 for enroll). Cross-kind rejection — a wrong success-transition
    /// kind on a denied receipt — is exercised by
    /// `denied_lifecycle_receipt_wrong_kind_receipt_side` (receipt-side trigger)
    /// and `denied_lifecycle_receipt_wrong_kind_event_side` (event-side trigger).
    ///
    /// Mutation sensitivity:
    /// - Removing the `outcome_code IN (1, 3)` branch entirely (or replacing it with a
    ///   blanket early-return) makes the positive case red — the denied enroll receipt
    ///   cannot commit alone because the guard then demands a paired enroll audit event
    ///   (expected_event_kind = 1) that is absent.
    /// - Removing the `ELSIF outcome_code = 2` zero-event branch makes the
    ///   receipt-then-event negative below green (COMMIT succeeds when it must not),
    ///   failing `expect_err`. The event-side isolation in
    ///   `denied_lifecycle_receipt_event_side_trigger_isolated` independently confirms
    ///   the same branch using only the `authorization_event_receipt_cardinality`
    ///   trigger direction.
    /// Applied/no-op lifecycle cardinality is exercised by
    /// `applied_lifecycle_receipt_requires_exactly_one_event`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn denied_lifecycle_receipt_commits_without_audit_event() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "denied-lifecycle-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert community");

        // Denied enroll receipt (operation_kind = 1, outcome_code = 2) must commit
        // without any paired authorization_events row. The guard must skip it
        // because outcome_code = 2 is not in (1, 3).
        //
        // The receipt history guard (migration 0041) uses `outcome_code IN (1, 3)`
        // for lifecycle receipts, so a denied enroll receipt (outcome_code = 2)
        // expects zero lifecycle history rows — no history setup is needed.
        let op_denied = uuid::Uuid::new_v4();
        let fp_denied = vec![0xB1_u8; 32];

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 2, $5)",
            // operation_kind 1 (enroll), outcome_code 2 (denied)
        )
        .bind(community_id)
        .bind(op_denied)
        .bind(&fp_denied)
        .bind(vec![0xB2_u8; 32])
        .bind(vec![0xB3_u8; 32])
        .execute(&pool)
        .await
        .expect("denied enroll receipt must commit without a paired audit event");

        // No audit event for this operation; confirm the table is empty for it.
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_events \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(community_id)
        .bind(op_denied)
        .fetch_one(&pool)
        .await
        .expect("count events for denied receipt");
        assert_eq!(
            event_count, 0,
            "no audit event should be required or present for a denied lifecycle receipt"
        );

        // --- Negative: denied enroll receipt paired with its mapped success-
        // transition event (event_kind = 1, enrolled) must be rejected at COMMIT.
        // The receipt-side deferred trigger fires here (receipt was inserted in
        // this same transaction). The event-side trigger direction is isolated in
        // `denied_lifecycle_receipt_event_side_trigger_isolated`.
        //
        // Seed event capacity; the authorization_events BEFORE INSERT trigger
        // requires a capacity row. No lifecycle history is needed: denied receipts
        // (outcome_code = 2) expect zero history rows per the history guard.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        let op_neg = uuid::Uuid::new_v4();
        let fp_neg = vec![0xD1_u8; 32];
        let event_neg = uuid::Uuid::new_v4();
        let corr_neg = uuid::Uuid::new_v4();
        let attempt_neg = uuid::Uuid::new_v4();

        let mut conn_neg = pool.acquire().await.expect("acquire connection neg");
        sqlx::query("BEGIN")
            .execute(&mut *conn_neg)
            .await
            .expect("begin neg");

        // Denied enroll receipt — no history row needed (outcome_code = 2).
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 2, $5)",
        )
        .bind(community_id)
        .bind(op_neg)
        .bind(&fp_neg)
        .bind(vec![0xD2_u8; 32])
        .bind(vec![0xD3_u8; 32])
        .execute(&mut *conn_neg)
        .await
        .expect("insert denied receipt — event guard is deferred");

        // Insert the mapped success-transition event (event_kind = 1, enrolled).
        // actor_kind = 1 requires a non-null actor_fingerprint and a matching
        // receipt FK (satisfied by the denied receipt above, which shares the
        // same (community_id, operation_id, request_fingerprint)).
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 1, 1, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
            // event_kind 1 (enrolled) — the mapped success transition for enroll
        )
        .bind(community_id)
        .bind(event_neg)
        .bind(vec![0xD4_u8; 32]) // actor_fingerprint
        .bind(op_neg)
        .bind(&fp_neg)
        .bind(corr_neg)
        .bind(attempt_neg)
        .bind(vec![0xD5_u8; 64]) // canonical_envelope
        .bind(vec![0xD6_u8; 32]) // envelope_digest
        .execute(&mut *conn_neg)
        .await
        .expect("event INSERT must pass — deferred guard fires at COMMIT");

        let contradiction_err = sqlx::query("COMMIT")
            .execute(&mut *conn_neg)
            .await
            .expect_err(
                "denied receipt + mapped success event must be rejected at COMMIT \
                 — contradictory durable facts must not be permitted",
            );
        assert_eq!(
            contradiction_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_denied_lifecycle_receipt_no_success_event"),
            "expected authorization_denied_lifecycle_receipt_no_success_event constraint \
             rejection for denied receipt + success event, got: {contradiction_err}"
        );
    }

    /// NIP-FI event-side trigger isolation: when a denied enroll receipt is already
    /// committed (auto-commit via pool), a new independent transaction that inserts
    /// only the mapped success-transition event must be rejected at COMMIT by
    /// `authorization_event_receipt_cardinality` (the event-side deferred trigger).
    ///
    /// This isolates the `authorization_event_receipt_cardinality` trigger path.
    /// In `denied_lifecycle_receipt_commits_without_audit_event`'s receipt-then-event
    /// negative, the receipt-side trigger (`authorization_operation_receipt_event_cardinality`)
    /// also fires. Here the committed receipt produces no deferred trigger, so rejection
    /// can only come from the event-side trigger. Uses the mapped kind (kind 1 for
    /// enroll). Wrong-kind event-side isolation is in
    /// `denied_lifecycle_receipt_wrong_kind_event_side`.
    ///
    /// Mutation sensitivity: disabling the
    /// `authorization_event_receipt_cardinality` trigger (DROP or ALTER TABLE
    /// DISABLE TRIGGER) makes this negative green — the COMMIT succeeds when it
    /// must not, so `expect_err` panics.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn denied_lifecycle_receipt_event_side_trigger_isolated() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("evt-side-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        // Seed event capacity before any event insert.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        // Commit a denied enroll receipt in auto-commit mode (no explicit BEGIN).
        // This receipt produces no deferred trigger — the receipt-side deferred
        // trigger only fires within the transaction that inserts the receipt row.
        let op_id = uuid::Uuid::new_v4();
        let fp = vec![0xE1_u8; 32];
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 2, $5)",
        )
        .bind(community_id)
        .bind(op_id)
        .bind(&fp)
        .bind(vec![0xE2_u8; 32])
        .bind(vec![0xE3_u8; 32])
        .execute(&pool)
        .await
        .expect("denied receipt must commit alone in auto-commit mode");

        // Now open a NEW transaction and insert only the mapped success-transition
        // event (event_kind = 1, enrolled). The receipt is already committed and
        // its deferred trigger is no longer active. Rejection at COMMIT must come
        // from authorization_event_receipt_cardinality (the event-side trigger).
        let event_id = uuid::Uuid::new_v4();
        let corr_id = uuid::Uuid::new_v4();
        let attempt_id = uuid::Uuid::new_v4();

        let mut conn = pool
            .acquire()
            .await
            .expect("acquire connection for event-side test");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin event-side transaction");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 1, 1, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
        )
        .bind(community_id)
        .bind(event_id)
        .bind(vec![0xE4_u8; 32]) // actor_fingerprint
        .bind(op_id)
        .bind(&fp)
        .bind(corr_id)
        .bind(attempt_id)
        .bind(vec![0xE5_u8; 64]) // canonical_envelope
        .bind(vec![0xE6_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("event INSERT must pass — event-side deferred guard fires at COMMIT");

        let event_side_err = sqlx::query("COMMIT").execute(&mut *conn).await.expect_err(
            "mapping a success-transition event to a committed denied receipt \
                 must be rejected at COMMIT by the event-side trigger",
        );
        assert_eq!(
            event_side_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_denied_lifecycle_receipt_no_success_event"),
            "event-side trigger must reject with authorization_denied_lifecycle_receipt_no_success_event, \
             got: {event_side_err}"
        );
    }

    /// NIP-FI applied lifecycle receipt: an applied core lifecycle enroll receipt
    /// (outcome_code = 1) requires exactly one mapped success-transition event
    /// (event_kind = 1, enrolled). This exercises the `outcome_code IN (1, 3)`
    /// branch of `authorization_operation_receipt_event_guard_v1` at migration 42.
    ///
    /// Mutation sensitivity:
    /// - Removing/bypassing the applied/no-op branch (replacing it with a blanket
    ///   RETURN NULL) makes the positive transaction commit without an event, leaving
    ///   the contract silently unenforced. The negative below requires the cardinality
    ///   constraint to fire when the event is absent.
    /// - Removing the negative assertion: the absent-event case would commit when it
    ///   must not.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn applied_lifecycle_receipt_requires_exactly_one_event() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "applied-lifecycle-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert community");

        // Enrollment policy (TOFU, mode 3).
        let policy_revision: i64 = 1;
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_revision, enrollment_mode, policy_digest, effective_at) \
             VALUES ($1, $2, 3, $3, '2026-01-01T00:00:00Z')",
        )
        .bind(community_id)
        .bind(policy_revision)
        .bind(vec![0xF0_u8; 32])
        .execute(&pool)
        .await
        .expect("insert enrollment policy");

        // Event capacity — required by authorization_event_capacity_before_insert_v1.
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        // --- Positive: applied enroll commits with exactly one mapped event ---
        //
        // All cross-table FKs between identity_lifecycle_history, identity_bindings,
        // authorization_operation_receipts, and authorization_events are
        // DEFERRABLE INITIALLY DEFERRED — insert order within the transaction is
        // flexible, but a pinned connection is required for BEGIN/COMMIT to share
        // the same session. The receipt_history_cardinality trigger (migration 0041)
        // fires at COMMIT and requires exactly one history row for applied enroll.
        let op_id = uuid::Uuid::new_v4();
        let binding_id = uuid::Uuid::new_v4();
        let history_id = uuid::Uuid::new_v4();
        let fp = vec![0xF1_u8; 32];
        let event_id = uuid::Uuid::new_v4();
        let corr_id = uuid::Uuid::new_v4();
        let attempt_id = uuid::Uuid::new_v4();

        let mut conn = pool
            .acquire()
            .await
            .expect("acquire connection for positive case");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin positive transaction");

        // History first: the receipt_history_cardinality AFTER INSERT trigger
        // on authorization_operation_receipts is DEFERRED and checks at COMMIT
        // time, but inserting history before receipt is idiomatic.
        // successor_binding_version = 1 because binding_version is an identity
        // sequence starting at 1 per community; this is the first binding.
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id, history_id, transition_kind, outcome_code, \
              successor_binding_id, successor_binding_version, \
              successor_lifecycle_revision, successor_state, \
              operation_id, request_fingerprint, transition_digest) \
             VALUES ($1, $2, 1, 1, $3, 1, 1, 1, $4, $5, $6)",
        )
        .bind(community_id)
        .bind(history_id)
        .bind(binding_id)
        .bind(op_id)
        .bind(&fp)
        .bind(vec![0xF2_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert lifecycle history");

        // Applied enroll receipt (operation_kind = 1, outcome_code = 1).
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op_id)
        .bind(&fp)
        .bind(vec![0xF3_u8; 32])
        .bind(vec![0xF4_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert applied enroll receipt");

        // Binding — birth_history_id FK is deferred; binding_version is generated.
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, binding_id, issuer, subject, \
              principal_fingerprint, event_author_pubkey, \
              binding_state, lifecycle_revision, binding_provenance, \
              policy_revision, enrollment_evidence_digest, \
              birth_history_id, creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1, $2, 'https://issuer.example', 'sub-applied', \
                     $3, $4, 1, 1, 1, $5, $6, $7, $8, $9)",
        )
        .bind(community_id)
        .bind(binding_id)
        .bind(vec![0xF5_u8; 32]) // principal_fingerprint
        .bind(vec![0xF6_u8; 32]) // event_author_pubkey
        .bind(policy_revision)
        .bind(vec![0xF7_u8; 32]) // enrollment_evidence_digest
        .bind(history_id)
        .bind(op_id)
        .bind(&fp)
        .execute(&mut *conn)
        .await
        .expect("insert identity binding");

        // Mapped success-transition event (event_kind = 1, enrolled).
        // actor_kind = 1 requires non-null actor_fingerprint and matching receipt FK.
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 1, 1, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
        )
        .bind(community_id)
        .bind(event_id)
        .bind(vec![0xF8_u8; 32]) // actor_fingerprint
        .bind(op_id)
        .bind(&fp)
        .bind(corr_id)
        .bind(attempt_id)
        .bind(vec![0xF9_u8; 64]) // canonical_envelope
        .bind(vec![0xFA_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("insert mapped success-transition event");

        sqlx::query("COMMIT").execute(&mut *conn).await.expect(
            "applied enroll receipt + exactly one mapped event must commit — \
                 authorization_operation_receipt_event_guard_v1 applied/no-op branch",
        );

        // Confirm exactly one event committed for this operation.
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_events \
             WHERE community_id = $1 AND operation_id = $2",
        )
        .bind(community_id)
        .bind(op_id)
        .fetch_one(&pool)
        .await
        .expect("count events for applied receipt");
        assert_eq!(
            event_count, 1,
            "exactly one audit event must be present for an applied enroll receipt"
        );

        // --- Negative: applied enroll receipt without a mapped event must reject ---
        //
        // A second applied enroll transaction that commits receipt + history + binding
        // but no event must be rejected with authorization_operation_receipt_event_cardinality.
        let op_neg = uuid::Uuid::new_v4();
        let binding_neg = uuid::Uuid::new_v4();
        let history_neg = uuid::Uuid::new_v4();
        let fp_neg = vec![0xFB_u8; 32];

        let mut conn_neg = pool
            .acquire()
            .await
            .expect("acquire connection for negative case");
        sqlx::query("BEGIN")
            .execute(&mut *conn_neg)
            .await
            .expect("begin negative transaction");

        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id, history_id, transition_kind, outcome_code, \
              successor_binding_id, successor_binding_version, \
              successor_lifecycle_revision, successor_state, \
              operation_id, request_fingerprint, transition_digest) \
             VALUES ($1, $2, 1, 1, $3, 2, 1, 1, $4, $5, $6)",
            // successor_binding_version = 2: second binding in this community
        )
        .bind(community_id)
        .bind(history_neg)
        .bind(binding_neg)
        .bind(op_neg)
        .bind(&fp_neg)
        .bind(vec![0xFC_u8; 32])
        .execute(&mut *conn_neg)
        .await
        .expect("insert negative lifecycle history");

        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 1, $5)",
        )
        .bind(community_id)
        .bind(op_neg)
        .bind(&fp_neg)
        .bind(vec![0xFD_u8; 32])
        .bind(vec![0xFE_u8; 32])
        .execute(&mut *conn_neg)
        .await
        .expect("insert negative applied receipt");

        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, binding_id, issuer, subject, \
              principal_fingerprint, event_author_pubkey, \
              binding_state, lifecycle_revision, binding_provenance, \
              policy_revision, enrollment_evidence_digest, \
              birth_history_id, creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1, $2, 'https://issuer.example', 'sub-applied-neg', \
                     $3, $4, 1, 1, 1, $5, $6, $7, $8, $9)",
        )
        .bind(community_id)
        .bind(binding_neg)
        .bind(vec![0xE7_u8; 32]) // principal_fingerprint (distinct from positive)
        .bind(vec![0xE8_u8; 32]) // event_author_pubkey (distinct from positive)
        .bind(policy_revision)
        .bind(vec![0xE9_u8; 32])
        .bind(history_neg)
        .bind(op_neg)
        .bind(&fp_neg)
        .execute(&mut *conn_neg)
        .await
        .expect("insert negative binding — no event inserted");

        // Commit without the mapped event — guard must reject.
        let absent_event_err = sqlx::query("COMMIT")
            .execute(&mut *conn_neg)
            .await
            .expect_err(
                "applied enroll receipt without a mapped success-transition event \
                 must be rejected at COMMIT",
            );
        assert_eq!(
            absent_event_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_operation_receipt_event_cardinality"),
            "expected authorization_operation_receipt_event_cardinality rejection \
             for applied receipt without event, got: {absent_event_err}"
        );
    }

    /// NIP-FI cross-kind denied lifecycle: a wrong success-transition kind paired
    /// with a denied lifecycle receipt must be rejected through the receipt-side
    /// deferred trigger. Uses a denied enroll receipt (operation_kind = 1, mapped
    /// kind = 1) with a kind-6 (retired) event — a different success-transition
    /// kind that is equally forbidden by the class-based guard (kinds 1, 2, 3, 6).
    ///
    /// Both the receipt and the wrong-kind event are inserted in the same
    /// transaction, so the receipt-side deferred trigger
    /// (`authorization_operation_receipt_event_cardinality`) fires at COMMIT.
    ///
    /// Mutation sensitivity: narrowing the denied filter back to
    /// `event_kind = expected_event_kind` (the mapped kind, 1) removes kind 6
    /// from the forbidden set, causing this negative to turn green — COMMIT
    /// succeeds when it must not, failing `expect_err`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn denied_lifecycle_receipt_wrong_kind_receipt_side() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("wrong-kind-rcpt-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        let op_id = uuid::Uuid::new_v4();
        let fp = vec![0xA0_u8; 32];
        let event_id = uuid::Uuid::new_v4();
        let corr_id = uuid::Uuid::new_v4();
        let attempt_id = uuid::Uuid::new_v4();

        let mut conn = pool
            .acquire()
            .await
            .expect("acquire connection for wrong-kind receipt-side test");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");

        // Denied enroll receipt (operation_kind = 1, outcome_code = 2).
        // No history row needed: outcome_code = 2 expects zero history rows.
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 2, $5)",
        )
        .bind(community_id)
        .bind(op_id)
        .bind(&fp)
        .bind(vec![0xA1_u8; 32])
        .bind(vec![0xA2_u8; 32])
        .execute(&mut *conn)
        .await
        .expect("insert denied enroll receipt — deferred guard");

        // Wrong success-transition kind: event_kind = 6 (retired), not the mapped
        // kind 1 (enrolled). Both are in the forbidden class (1, 2, 3, 6).
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 6, 1, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
            // event_kind 6 (retired) — wrong kind for a denied enroll receipt
        )
        .bind(community_id)
        .bind(event_id)
        .bind(vec![0xA3_u8; 32]) // actor_fingerprint
        .bind(op_id)
        .bind(&fp)
        .bind(corr_id)
        .bind(attempt_id)
        .bind(vec![0xA4_u8; 64]) // canonical_envelope
        .bind(vec![0xA5_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("event INSERT must pass — deferred guard fires at COMMIT");

        let wrong_kind_err = sqlx::query("COMMIT").execute(&mut *conn).await.expect_err(
            "denied receipt + wrong success-transition kind (6) must be rejected at COMMIT \
                 — class-based guard forbids all of kinds 1, 2, 3, 6",
        );
        assert_eq!(
            wrong_kind_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_denied_lifecycle_receipt_no_success_event"),
            "expected authorization_denied_lifecycle_receipt_no_success_event for \
             denied receipt + wrong kind (6), got: {wrong_kind_err}"
        );
    }

    /// NIP-FI cross-kind denied lifecycle event-side: after a denied enroll
    /// receipt is committed alone (auto-commit), a new transaction that inserts
    /// only a wrong success-transition kind (kind 6, retired) must be rejected at
    /// COMMIT by `authorization_event_receipt_cardinality` (event-side trigger).
    ///
    /// This isolates the event-side trigger path for the cross-kind case.
    /// The committed receipt produces no active deferred trigger, so rejection
    /// can only come from the event-side trigger.
    ///
    /// Mutation sensitivity: narrowing the denied filter to
    /// `event_kind = expected_event_kind` (kind 1) removes kind 6 from the
    /// forbidden set, making this negative green — COMMIT succeeds when it must
    /// not, failing `expect_err`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn denied_lifecycle_receipt_wrong_kind_event_side() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(42, &pool)
            .await
            .expect("apply migrations 1-42");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("wrong-kind-evt-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");

        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id, max_events_per_domain, max_bytes_per_domain, max_envelope_bytes) \
             VALUES ($1, 1000, 16777216, 16384)",
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .expect("insert event capacity");

        // Commit a denied enroll receipt in auto-commit mode. No deferred trigger
        // is active after this commit; the receipt-side trigger fires only within
        // the transaction that inserts the receipt.
        let op_id = uuid::Uuid::new_v4();
        let fp = vec![0xB0_u8; 32];
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, request_fingerprint, operation_kind, \
              actor_fingerprint, outcome_code, result_digest) \
             VALUES ($1, $2, $3, 1, $4, 2, $5)",
        )
        .bind(community_id)
        .bind(op_id)
        .bind(&fp)
        .bind(vec![0xB1_u8; 32])
        .bind(vec![0xB2_u8; 32])
        .execute(&pool)
        .await
        .expect("denied receipt must commit alone in auto-commit mode");

        // New transaction: insert only a kind-6 (retired) event for the same
        // operation. The event-side trigger is the only active deferred trigger.
        let event_id = uuid::Uuid::new_v4();
        let corr_id = uuid::Uuid::new_v4();
        let attempt_id = uuid::Uuid::new_v4();

        let mut conn = pool
            .acquire()
            .await
            .expect("acquire connection for wrong-kind event-side test");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin event-side wrong-kind transaction");

        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id, event_id, event_kind, outcome_code, reason_code, \
              actor_kind, actor_fingerprint, operation_id, request_fingerprint, \
              correlation_id, attempt_id, semantic_fingerprint, \
              occurred_at, canonical_envelope, envelope_digest) \
             VALUES ($1, $2, 6, 1, 4, 1, $3, $4, $5, $6, $7, NULL, \
                     '2026-01-01T00:00:00Z', $8, $9)",
            // event_kind 6 (retired) — wrong kind for the denied enroll receipt
        )
        .bind(community_id)
        .bind(event_id)
        .bind(vec![0xB3_u8; 32]) // actor_fingerprint
        .bind(op_id)
        .bind(&fp)
        .bind(corr_id)
        .bind(attempt_id)
        .bind(vec![0xB4_u8; 64]) // canonical_envelope
        .bind(vec![0xB5_u8; 32]) // envelope_digest
        .execute(&mut *conn)
        .await
        .expect("event INSERT must pass — event-side deferred guard fires at COMMIT");

        let wrong_kind_evt_err = sqlx::query("COMMIT").execute(&mut *conn).await.expect_err(
            "kind-6 event paired with a committed denied enroll receipt must be \
                 rejected at COMMIT by the event-side trigger",
        );
        assert_eq!(
            wrong_kind_evt_err
                .as_database_error()
                .and_then(|e| e.constraint()),
            Some("authorization_denied_lifecycle_receipt_no_success_event"),
            "event-side trigger must reject with authorization_denied_lifecycle_receipt_no_success_event \
             for wrong kind (6) against denied receipt, got: {wrong_kind_evt_err}"
        );
    }
}
