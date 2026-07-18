//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in SQL files under `migrations/`. The
//! multi-tenant rewrite owns a clean consolidated `0001`; legacy single-tenant
//! cutover/backfill is a separate operator script, not startup migration state.

use sqlx::PgPool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Run all pending Buzz database migrations.
///
/// Wraps the sqlx migrator in a session-scoped advisory lock so that
/// concurrent replicas (rolling deploy) serialize startup instead of
/// racing each other's concurrent index builds or repair guards.
///
/// All operations (lock, pre-migration guards, migrator, unlock) run on
/// a single acquired connection — `pg_advisory_lock` is session-scoped,
/// so the lock-holding session must be the one that runs the critical
/// section and releases it. The connection is marked `close_on_drop`,
/// so if unlock fails or the future is cancelled, dropping the
/// connection closes the PG session (rather than returning a locked
/// session to the pool) and releases the lock automatically.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    const MIGRATION_ADVISORY_LOCK_KEY: i64 = 0x42757a7a4d696772; // "BuzzMigr"

    let mut conn = pool.acquire().await?;
    conn.close_on_drop();

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_ADVISORY_LOCK_KEY)
        .execute(&mut *conn)
        .await?;

    let result = async {
        reject_legacy_nip_rs_cardinality_ambiguity(&mut conn).await?;
        repair_invalid_media_index(&mut conn).await?;
        MIGRATOR.run(&mut *conn).await?;
        Ok(())
    }
    .await;

    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_ADVISORY_LOCK_KEY)
        .execute(&mut *conn)
        .await;

    result
}

/// Drop `idx_audit_log_media_uploads` if it exists but is invalid
/// (`indisvalid = false`), so migration 21's `CREATE INDEX CONCURRENTLY
/// IF NOT EXISTS` builds a fresh valid index instead of skipping over
/// the broken leftover.
///
/// This handles the case where a prior concurrent build failed (e.g. OOM,
/// process kill, unique violation on a UNIQUE variant) and left an invalid
/// index behind. Without this guard, `IF NOT EXISTS` would skip creation,
/// SQLx would record version 21, and the relay would run permanently
/// without the required access path.
///
/// The probe joins `pg_index.indrelid = to_regclass('audit_log')` so it
/// finds the invalid index attached to whichever `audit_log` the
/// session's `search_path` resolves, regardless of schema. The DROP
/// statement is built server-side via `quote_ident(schema.index)` so it
/// targets the correct namespace without hardcoding `public`.
///
/// Multi-replica safety: the caller holds a session-scoped advisory lock
/// on the same connection, so a second replica starting simultaneously
/// cannot see replica A's in-progress concurrent build as invalid and
/// drop it mid-build.
async fn repair_invalid_media_index(conn: &mut sqlx::PgConnection) -> Result<()> {
    let drop_stmt: Option<String> = sqlx::query_scalar(
        "SELECT 'DROP INDEX IF EXISTS ' || quote_ident(n.nspname) || '.' || quote_ident(c.relname) \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indexrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relname = 'idx_audit_log_media_uploads' \
           AND i.indrelid = to_regclass('audit_log') \
           AND NOT i.indisvalid",
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(stmt) = drop_stmt {
        sqlx::query(sqlx::AssertSqlSafe(stmt))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Migration 0007 is checksum-frozen and predates exact NIP-RS tag-cardinality
/// enforcement. A populated database still on 0001-0006 must not let 0007
/// irreversibly purge duplicate-tag history. Fail before sqlx starts its
/// migration transaction so an operator can inspect and repair those rows.
async fn reject_legacy_nip_rs_cardinality_ambiguity(conn: &mut sqlx::PgConnection) -> Result<()> {
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
    .fetch_one(&mut *conn)
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
    use std::collections::BTreeSet;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

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
                let mutates_with_update = updates_channels
                    && normalized.contains(" set ")
                    && normalized.contains("community_id");
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

        assert_eq!(migrations.len(), 21);
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
        assert!(!migrations[0]
            .sql
            .as_str()
            .contains("join_policy_acceptances"));

        // Per-user storage attribution needs a non-transactional CONCURRENTLY
        // index build on audit_log; it must not lock out concurrent writers
        // during migration on a populated relay.
        assert_eq!(migrations[20].version, 21);
        assert!(
            migrations[20].no_tx,
            "migration 21 builds an index CONCURRENTLY and must run outside a transaction"
        );
        assert!(migrations[20]
            .sql
            .as_str()
            .contains("CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_audit_log_media_uploads"));
        assert!(!migrations[0]
            .sql
            .as_str()
            .contains("idx_audit_log_media_uploads"));
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
        assert_eq!(applied_versions(&pool).await.last().copied(), Some(21));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn populated_upgrade_preserves_search_policy_except_for_push_leases() {
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

        for (marker, kind) in [(1_u8, 1_i32), (2_u8, 30_350_i32)] {
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
        assert_eq!(before, vec![(1, true), (30_350, true)]);

        run_migrations(&pool)
            .await
            .expect("apply push migrations to populated database");
        let after: Vec<(i32, Option<bool>)> = sqlx::query_as(
            "SELECT kind, search_tsv @@ plainto_tsquery('simple', 'needle') \
             FROM events ORDER BY kind",
        )
        .fetch_all(&pool)
        .await
        .expect("read post-push search behavior");
        assert_eq!(after, vec![(1, Some(true)), (30_350, None)]);
    }

    /// Bookkeeping-race recovery: a process dies after the DDL succeeds but
    /// before SQLx records version 21 → a valid same-named index is left
    /// unrecorded. `IF NOT EXISTS` makes the retry idempotent: PG skips
    /// creation and SQLx records version 21 — the relay starts normally.
    /// The repair guard must NOT drop a valid index in this scenario.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_21_retries_after_leftover_unrecorded_index() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        MIGRATOR
            .run_to(20, &pool)
            .await
            .expect("apply migrations 1-20");

        sqlx::query(
            "CREATE INDEX idx_audit_log_media_uploads \
             ON audit_log (community_id, actor_pubkey, object_id) \
             WHERE action = 'media_uploaded'",
        )
        .execute(&pool)
        .await
        .expect("plant a same-named valid index (simulates successful DDL before bookkeeping)");

        let planted: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_class WHERE relname = 'idx_audit_log_media_uploads' \
             )",
        )
        .fetch_one(&pool)
        .await
        .expect("check planted index");
        assert!(planted, "planted index must exist before retry");

        run_migrations(&pool)
            .await
            .expect("migration 21 must not wedge when a same-named index already exists");

        assert_eq!(applied_versions(&pool).await.last().copied(), Some(21));

        let index_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_indexes \
                 WHERE indexname = 'idx_audit_log_media_uploads' \
             )",
        )
        .fetch_one(&pool)
        .await
        .expect("check index still present");
        assert!(
            index_exists,
            "index must still exist after idempotent retry"
        );
    }

    /// Failed-build recovery: a prior `CREATE INDEX CONCURRENTLY` failed
    /// (e.g. OOM, unique violation, process kill) and left an `indisvalid=false`
    /// index behind. Without the startup repair guard, `IF NOT EXISTS` would
    /// skip creation, SQLx would record version 21, and the relay would run
    /// permanently without the required access path. The repair guard drops
    /// the invalid index before the migrator runs, so a fresh valid build
    /// proceeds.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_21_recovers_after_failed_concurrent_build() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        MIGRATOR
            .run_to(20, &pool)
            .await
            .expect("apply migrations 1-20");

        // Plant duplicate data so a UNIQUE concurrent build fails, leaving
        // an indisvalid=false index behind.
        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("invalid-idx-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community for duplicate data");
        let hash_a = vec![0xAA_u8; 32];
        let hash_b = vec![0xBB_u8; 32];
        let actor = vec![0xCC_u8; 32];
        for (seq, hash) in [(1_i64, &hash_a), (2_i64, &hash_b)] {
            sqlx::query(
                "INSERT INTO audit_log (community_id, seq, hash, action, actor_pubkey, object_id, detail) \
                 VALUES ($1, $2, $3, 'media_uploaded', $4, 'same-obj', '{}'::jsonb)",
            )
            .bind(community_id)
            .bind(seq)
            .bind(hash)
            .bind(&actor)
            .execute(&pool)
            .await
            .expect("insert duplicate audit row");
        }

        // Attempt a UNIQUE concurrent build — it will fail on the duplicates,
        // leaving an indisvalid=false index.
        let unique_result = sqlx::query(
            "CREATE UNIQUE INDEX CONCURRENTLY idx_audit_log_media_uploads \
             ON audit_log (community_id, actor_pubkey, object_id) \
             WHERE action = 'media_uploaded'",
        )
        .execute(&pool)
        .await;
        assert!(
            unique_result.is_err(),
            "UNIQUE build must fail on duplicate data"
        );

        // Verify the invalid index exists.
        let invalid: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indexrelid \
                 WHERE c.relname = 'idx_audit_log_media_uploads' \
                   AND NOT i.indisvalid \
             )",
        )
        .fetch_one(&pool)
        .await
        .expect("check invalid index");
        assert!(
            invalid,
            "failed concurrent build must leave indisvalid=false index"
        );

        // Clean up the duplicate data so the real (non-unique) index build
        // in migration 21 succeeds.
        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 2")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("remove duplicate so real migration succeeds");

        // Run migrations — repair guard should drop the invalid index,
        // then migration 21 creates a fresh valid one.
        run_migrations(&pool)
            .await
            .expect("repair guard + migration 21 must recover from invalid index");

        assert_eq!(applied_versions(&pool).await.last().copied(), Some(21));

        // The index must exist AND be valid.
        let (exists, valid): (bool, bool) = sqlx::query_as(
            "SELECT \
                 EXISTS (SELECT 1 FROM pg_class WHERE relname = 'idx_audit_log_media_uploads'), \
                 COALESCE(( \
                     SELECT i.indisvalid FROM pg_index i \
                     JOIN pg_class c ON c.oid = i.indexrelid \
                     WHERE c.relname = 'idx_audit_log_media_uploads' \
                 ), false)",
        )
        .fetch_one(&pool)
        .await
        .expect("check repaired index");
        assert!(exists, "index must exist after repair");
        assert!(valid, "index must be valid (indisvalid=true) after repair");

        // Postcondition: advisory lock must be released after run_migrations
        // returns. The pool may return the same physical session, so this is
        // a global pg_locks absence check — valid because pg_locks is
        // cluster-wide. Scoped to current database to avoid false failures
        // from an unrelated database using the same key.
        let lock_key: i64 = 0x42757a7a4d696772;
        let lock_held: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_locks \
                 WHERE locktype = 'advisory' \
                   AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                   AND classid = ($1 >> 32)::oid \
                   AND objid = ($1 & x'FFFFFFFF'::bigint)::oid \
             )",
        )
        .bind(lock_key)
        .fetch_one(&pool)
        .await
        .expect("check pg_locks for advisory lock");
        assert!(
            !lock_held,
            "advisory lock must not be held after run_migrations returns"
        );
    }

    /// Same as `migration_21_recovers_after_failed_concurrent_build` but
    /// under a non-public `search_path`. This exercises the probe/drop path
    /// where the invalid index lives in a schema other than `public` — the
    /// repair guard must resolve the actual schema from the table OID
    /// (`to_regclass('audit_log')`) rather than hardcoding `public`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_21_recovers_after_failed_concurrent_build_nonpublic_schema() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());

        let schema_name = format!(
            "test_schema_{}",
            uuid::Uuid::new_v4().simple().to_string().get(..8).unwrap()
        );

        // Bootstrap pool (public schema) to create the custom schema.
        let bootstrap_pool = PgPool::connect(&database_url)
            .await
            .expect("connect bootstrap pool");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            &schema_name
        )))
        .execute(&bootstrap_pool)
        .await
        .expect("drop old test schema");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE SCHEMA {}",
            &schema_name
        )))
        .execute(&bootstrap_pool)
        .await
        .expect("create test schema");
        bootstrap_pool.close().await;

        // Build a pool whose connections default to the custom schema.
        let opts: sqlx::postgres::PgConnectOptions =
            database_url.parse().expect("parse database URL");
        let opts = opts.options([("search_path", schema_name.as_str())]);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .expect("connect custom-schema pool");

        // Verify search_path is set correctly.
        let current: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&pool)
            .await
            .expect("check current_schema");
        assert_eq!(
            current, schema_name,
            "pool connections must default to the custom schema"
        );

        MIGRATOR
            .run_to(20, &pool)
            .await
            .expect("apply migrations 1-20 in custom schema");

        // Plant duplicate data so a UNIQUE concurrent build fails.
        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("nonpub-idx-{}.example", community_id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        let hash_a = vec![0xAA_u8; 32];
        let hash_b = vec![0xBB_u8; 32];
        let actor = vec![0xCC_u8; 32];
        for (seq, hash) in [(1_i64, &hash_a), (2_i64, &hash_b)] {
            sqlx::query(
                "INSERT INTO audit_log (community_id, seq, hash, action, actor_pubkey, object_id, detail) \
                 VALUES ($1, $2, $3, 'media_uploaded', $4, 'same-obj', '{}'::jsonb)",
            )
            .bind(community_id)
            .bind(seq)
            .bind(hash)
            .bind(&actor)
            .execute(&pool)
            .await
            .expect("insert duplicate audit row");
        }

        let unique_result = sqlx::query(
            "CREATE UNIQUE INDEX CONCURRENTLY idx_audit_log_media_uploads \
             ON audit_log (community_id, actor_pubkey, object_id) \
             WHERE action = 'media_uploaded'",
        )
        .execute(&pool)
        .await;
        assert!(
            unique_result.is_err(),
            "UNIQUE build must fail on duplicate data"
        );

        // Verify the invalid index exists in the custom schema.
        let invalid: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indexrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relname = 'idx_audit_log_media_uploads' \
                   AND n.nspname = $1 \
                   AND NOT i.indisvalid \
             )",
        )
        .bind(&schema_name)
        .fetch_one(&pool)
        .await
        .expect("check invalid index in custom schema");
        assert!(
            invalid,
            "failed concurrent build must leave indisvalid=false in custom schema"
        );

        // Remove duplicate so the real migration build succeeds.
        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 2")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("remove duplicate");

        run_migrations(&pool)
            .await
            .expect("repair guard must handle non-public schema");

        assert_eq!(applied_versions(&pool).await.last().copied(), Some(21));

        // The index must exist AND be valid in the custom schema.
        let (exists, valid): (bool, bool) = sqlx::query_as(
            "SELECT \
                 EXISTS ( \
                     SELECT 1 FROM pg_class c \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE c.relname = 'idx_audit_log_media_uploads' AND n.nspname = $1 \
                 ), \
                 COALESCE(( \
                     SELECT i.indisvalid FROM pg_index i \
                     JOIN pg_class c ON c.oid = i.indexrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE c.relname = 'idx_audit_log_media_uploads' AND n.nspname = $1 \
                 ), false)",
        )
        .bind(&schema_name)
        .fetch_one(&pool)
        .await
        .expect("check repaired index in custom schema");
        assert!(exists, "index must exist in custom schema after repair");
        assert!(
            valid,
            "index must be valid (indisvalid=true) in custom schema after repair"
        );

        // Cleanup: close the pool and drop the custom schema.
        pool.close().await;
        let cleanup_pool = PgPool::connect(&database_url)
            .await
            .expect("connect cleanup pool");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            &schema_name
        )))
        .execute(&cleanup_pool)
        .await
        .expect("cleanup test schema");
        cleanup_pool.close().await;
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
    }
}
