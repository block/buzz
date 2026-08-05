//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in SQL files under `migrations/`. The
//! multi-tenant rewrite owns a clean consolidated `0001`; legacy single-tenant
//! cutover/backfill is a separate operator script, not startup migration state.

use sqlx::PgPool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[cfg(test)]
#[path = "migration_deterministic_tests.rs"]
mod deterministic_tests;

/// Run all pending Buzz database migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    reject_legacy_nip_rs_cardinality_ambiguity(pool).await?;
    MIGRATOR.run(pool).await?;
    // The replica-fence proof (see `replica_fence`) requires the commit-time
    // `created_at` floor trigger from migration 0021 — correctly shaped — on
    // the `events` parent and every partition. `CREATE TABLE .. PARTITION OF`
    // clones parent triggers, but a partition attached with `ATTACH
    // PARTITION` or created by an older code path would silently escape the
    // guard, so migration fails closed if any is missing. (The fence probe
    // re-runs this same check at startup on non-migrating relays.)
    crate::replica_fence::verify_floor_guard_catalog(pool).await?;
    Ok(())
}

/// Migration 0007 is checksum-frozen and predates exact NIP-RS tag-cardinality
/// enforcement. A populated database still on 0001-0006 must not let 0007
/// irreversibly purge duplicate-tag history. Fail before sqlx starts its
/// migration transaction so an operator can inspect and repair those rows.
async fn reject_legacy_nip_rs_cardinality_ambiguity(pool: &PgPool) -> Result<()> {
    let migrations_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
            .fetch_one(pool)
            .await?;
    if migrations_table.is_none() {
        return Ok(());
    }
    let applied: Option<i64> =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(pool)
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
    .fetch_one(pool)
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

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1
    type MigratedIdentityChainRow = (Vec<u8>, i64, Option<Vec<u8>>, String);

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

        assert_eq!(migrations.len(), 50);
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

        let desired_schema = include_str!("../../../schema/schema.sql");
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

        // Channel-id lookup index (0027): serves the tenant-independent
        // `channels` lookups that carry no community_id predicate, which no
        // community_id-leading index can satisfy. Covering + partial so the
        // planner can go index-only; asserted NOT UNIQUE because `id` alone is
        // not unique in this table (the same channel id may exist under more
        // than one community), so a unique index would encode a false
        // constraint and fail to build on such a database.
        assert_eq!(migrations[26].version, 27);
        let channel_id_index = migrations[26].sql.as_str();
        assert!(channel_id_index.contains("idx_channels_id_live"));
        assert!(channel_id_index.contains("INCLUDE (community_id)"));
        assert!(channel_id_index.contains("WHERE deleted_at IS NULL"));
        assert!(
            !channel_id_index.contains("CREATE UNIQUE INDEX"),
            "channels.id is not unique across communities — index must not be UNIQUE",
        );
        assert!(
            desired_schema.contains("idx_channels_id_live"),
            "desired-state schema must carry the channel-id lookup index",
        );

        // Relay-verified identity bindings are additive and community-scoped.
        assert_eq!(migrations[27].version, 28);
        assert!(migrations[27]
            .sql
            .as_str()
            .contains("CREATE TABLE identity_bindings"));
        assert!(migrations[27]
            .sql
            .as_str()
            .contains("idx_identity_bindings_active_principal"));
        assert_eq!(migrations[28].version, 29);
        assert!(migrations[28].sql.as_str().contains("revocation_scope"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("idx_identity_bindings_revoked_principal"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("CREATE TABLE identity_principals"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("INSERT INTO identity_principals"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("INSERT INTO identity_revoked_keys"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("rotation_completed_at"));
        assert!(migrations[28]
            .sql
            .as_str()
            .contains("CREATE TABLE identity_revoked_keys"));

        // Migration 0030 is a brownfield-safe additive projection over the
        // frozen 0028/0029 identity tables. It must not rewrite or discard
        // legacy authority.
        assert_eq!(migrations[29].version, 30);
        let projection = migrations[29].sql.as_str();
        for required in [
            "ADD COLUMN binding_id",
            "ADD COLUMN binding_version",
            "ADD COLUMN binding_state",
            "ADD COLUMN binding_provenance",
            "ADD COLUMN expires_at",
            "CREATE TABLE identity_enrollment_policies",
            "CREATE TRIGGER identity_enrollment_policy_lineage_guard",
            "CREATE TABLE identity_retired_pairs",
            "CREATE TABLE identity_pending_replacements",
            "CREATE TABLE identity_binding_history",
            "CREATE TABLE identity_lifecycle_operations",
            "CREATE TABLE identity_migration_denials",
        ] {
            assert!(
                projection.contains(required),
                "migration 0030 is missing {required}"
            );
        }

        assert_eq!(migrations[35].version, 36);
        let protected_domain_marker = migrations[35].sql.as_str();
        assert!(protected_domain_marker.contains("CREATE TABLE authorization_invalidation_domains"));

        assert_eq!(migrations[39].version, 40);
        let invalidation = migrations[39].sql.as_str();
        assert!(invalidation.contains("CREATE TABLE authorization_invalidation_receipts"));
        assert!(invalidation.contains("CREATE TABLE authorization_invalidation_floors"));

        assert_eq!(migrations[40].version, 41);
        let operation_receipts = migrations[40].sql.as_str();
        assert!(operation_receipts.contains("CREATE TABLE authorization_operation_receipts"));
        assert!(operation_receipts.contains("request_fingerprint"));
        assert!(operation_receipts.contains("result_payload"));
        assert!(operation_receipts.contains("authorization_operation_expiry_guard"));

        assert_eq!(migrations[41].version, 42);
        let authority_epochs = migrations[41].sql.as_str();
        assert!(authority_epochs.contains("CREATE TABLE authorization_authority_epochs"));
        assert!(authority_epochs.contains("CREATE TABLE client_status_revisions"));
        assert!(authority_epochs.contains("advance_authorization_authority_epoch"));
        assert!(authority_epochs.contains("IF TG_OP = 'DELETE'"));
        assert!(authority_epochs.contains("domain_id := OLD.community_id"));
        assert!(authority_epochs.contains("domain_id := NEW.community_id"));
        assert!(authority_epochs.contains("pg_trigger_depth() > 1"));
        assert!(authority_epochs.contains("ON DELETE CASCADE"));
        let function_position = authority_epochs
            .find("CREATE FUNCTION advance_authorization_authority_epoch")
            .expect("authority epoch function exists");
        for git_policy_trigger in [
            "git_policy_insert_authority_epoch",
            "git_policy_update_authority_epoch",
            "git_policy_delete_authority_epoch",
        ] {
            let trigger_position = authority_epochs
                .find(git_policy_trigger)
                .expect("git policy authority trigger exists");
            assert!(function_position < trigger_position);
        }
        for protected_table in [
            "identity_bindings",
            "identity_principals",
            "identity_revoked_keys",
            "identity_retired_pairs",
            "relay_members",
            "channel_members",
            "community_bans",
            "channels",
            "users",
            "authorization_invalidation_domains",
            "git_repo_publications",
            "media_publications",
            "protected_object_authority",
            "audio_session_admissions",
        ] {
            assert!(authority_epochs.contains(protected_table));
        }

        assert_eq!(migrations[42].version, 43);
        let client_status = migrations[42].sql.as_str();
        assert!(client_status.contains("ADD COLUMN supersedes_revision"));
        assert!(client_status.contains("client_status_revisions_withdrawal"));

        assert_eq!(migrations[43].version, 44);
        let projection_retirement = migrations[43].sql.as_str();
        assert!(projection_retirement.contains("identity_public_projection_heads"));
        assert!(projection_retirement.contains("identity_public_projection_retirements"));
        assert!(projection_retirement.contains("source_binding_version"));
        assert!(!projection_retirement.contains("issuer"));
        assert!(!projection_retirement.contains("subject TEXT"));
        assert!(!projection_retirement.contains("display_name"));

        assert_eq!(migrations[44].version, 45);
        let delegated_relationship = migrations[44].sql.as_str();
        assert!(delegated_relationship.contains("delegated_relationship"));
        assert!(delegated_relationship
            .contains("authorization_invalidation_floors_selector_kind_check"));

        assert_eq!(migrations[45].version, 46);
        let audit_outbox = migrations[45].sql.as_str();
        assert!(audit_outbox.contains("authorization_audit_outbox"));
        assert!(audit_outbox.contains("authorization_immutable_row_guard"));
        assert!(!audit_outbox.contains("JSONB"));

        assert_eq!(migrations[46].version, 47);
        let decisions = migrations[46].sql.as_str();
        assert!(decisions.contains("authorization_decision_events"));
        assert!(decisions.contains("authorization_decision_events_immutable"));

        assert_eq!(migrations[47].version, 48);
        let delivery = migrations[47].sql.as_str();
        assert!(delivery.contains("authorization_evidence_dead_letters"));
        assert!(delivery.contains("allow_reserve"));

        assert_eq!(migrations[48].version, 49);
        let operator = migrations[48].sql.as_str();
        assert!(operator.contains("authorization_operator_operation_receipts"));
        assert!(operator.contains("authorization_operator_authority_consumptions"));

        assert_eq!(migrations[49].version, 50);
        let previews = migrations[49].sql.as_str();
        assert!(previews.contains("authorization_lifecycle_previews"));
        assert!(previews.contains("authorization_decision_events"));
    }

    fn additive_identity_executable_sql(sql: &str) -> String {
        let mut output = String::with_capacity(sql.len());
        let mut chars = sql.chars().peekable();
        let mut in_line_comment = false;
        let mut in_string = false;
        while let Some(ch) = chars.next() {
            if in_line_comment {
                if ch == '\n' {
                    in_line_comment = false;
                    output.push(ch);
                }
                continue;
            }
            if !in_string && ch == '-' && chars.peek() == Some(&'-') {
                chars.next();
                in_line_comment = true;
                continue;
            }
            if ch == '\'' {
                if in_string && chars.peek() == Some(&'\'') {
                    chars.next();
                    continue;
                }
                in_string = !in_string;
                output.push(' ');
                continue;
            }
            output.push(if in_string { ' ' } else { ch });
        }
        output
    }

    #[test]
    fn migration_0030_is_strictly_additive() {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == 30)
            .expect("migration 0030");
        let executable = additive_identity_executable_sql(migration.sql.as_str());
        let normalized = normalize_sql(&executable);

        for forbidden in [" rename ", " drop ", " truncate ", " delete "] {
            assert!(
                !format!(" {normalized} ").contains(forbidden),
                "migration 0030 contains forbidden legacy mutation: {forbidden}"
            );
        }
        assert!(!normalized.contains("update identity_revoked_keys"));
        assert!(!normalized.contains("update identity_principals"));
        assert!(!normalized.contains("insert into identity_revoked_keys"));

        let allowed_binding_columns = [
            "binding_id",
            "binding_version",
            "binding_state",
            "binding_provenance",
            "replacement_binding_id",
            "creation_attribution_kind",
        ];
        for statement in split_sql_statements(&executable) {
            let statement = normalize_sql(&statement);
            if !statement.starts_with("update identity_bindings ") {
                continue;
            }
            let assignments = statement
                .split_once(" set ")
                .map(|(_, tail)| tail.split_once(" where ").map_or(tail, |(set, _)| set))
                .expect("identity binding metadata update has SET clause");
            for assignment in split_top_level_csv(assignments) {
                let column = assignment
                    .split_once('=')
                    .map(|(column, _)| column.trim())
                    .expect("metadata assignment");
                assert!(
                    allowed_binding_columns.contains(&column),
                    "migration 0030 rewrites legacy identity_bindings.{column}"
                );
            }
        }
    }

    #[test]
    fn migration_0030_enforces_authoritative_binding_state_and_attribution() {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == 30)
            .expect("migration 0030");
        // This gate intentionally checks persisted enum literals; unlike the
        // additive-mutation scanner above, it must retain SQL string contents.
        let normalized = normalize_sql(migration.sql.as_str());

        assert!(
            normalized.contains("binding_state = 'active' and revoked_at is null"),
            "every authoritative partial index and lookup contract must require active state"
        );
        assert!(
            normalized.contains("'archived'"),
            "migration 0030 must represent archived bindings explicitly"
        );
        assert!(
            normalized.contains("creation_attribution_kind"),
            "migration 0030 must distinguish verified creation attribution from legacy unknowns"
        );
        assert!(
            !normalized.contains("set binding_provenance = 'provisioned'"),
            "legacy db_binding rows must remain tofu, including imported successors"
        );

        let desired = normalize_sql(include_str!("../../../schema/schema.sql"));
        for required in [
            "binding_state = 'active' and revoked_at is null",
            "creation_attribution_kind",
            "archived_at",
            "archived_by",
            "archived_reason",
            "identity_enrollment_policies",
            "identity_enrollment_policy_lineage_guard",
            "expires_at",
        ] {
            assert!(
                desired.contains(required),
                "desired schema is missing identity-binding authority invariant: {required}"
            );
        }
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct LegacyIdentitySnapshot {
        bindings: Vec<String>,
        principals: Vec<String>,
        revoked_keys: Vec<String>,
        catalog: Vec<String>,
    }

    async fn legacy_identity_snapshot(pool: &PgPool) -> LegacyIdentitySnapshot {
        async fn json_rows(pool: &PgPool, query: &'static str) -> Vec<String> {
            let mut rows = sqlx::query_scalar::<_, serde_json::Value>(query)
                .fetch_all(pool)
                .await
                .expect("snapshot identity rows")
                .into_iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            rows.sort();
            rows
        }

        let bindings = json_rows(
            pool,
            "SELECT to_jsonb(binding) - ARRAY[\
                'binding_id', 'binding_version', 'binding_state',\
                'binding_provenance', 'replacement_binding_id', 'created_by',\
                'created_policy_version', 'expires_at', 'creation_attribution_kind',\
                'archived_at', 'archived_by', 'archived_reason']::text[] \
             FROM identity_bindings binding",
        )
        .await;
        let principals = json_rows(
            pool,
            "SELECT to_jsonb(principal) FROM identity_principals principal",
        )
        .await;
        let revoked_keys = json_rows(
            pool,
            "SELECT to_jsonb(revoked) FROM identity_revoked_keys revoked",
        )
        .await;
        let mut catalog = sqlx::query_scalar::<_, String>(
            r#"
            SELECT definition FROM (
                SELECT 'constraint:' || conrelid::regclass::text || ':' || conname || ':' || pg_get_constraintdef(oid) AS definition
                FROM pg_constraint
                WHERE conrelid IN ('identity_bindings'::regclass, 'identity_principals'::regclass, 'identity_revoked_keys'::regclass)
                UNION ALL
                SELECT 'index:' || tablename || ':' || indexname || ':' || indexdef
                FROM pg_indexes
                WHERE schemaname='public'
                  AND tablename IN ('identity_bindings', 'identity_principals', 'identity_revoked_keys')
            ) definitions
            ORDER BY definition
            "#,
        )
        .fetch_all(pool)
        .await
        .expect("snapshot identity catalog");
        catalog.sort();
        LegacyIdentitySnapshot {
            bindings,
            principals,
            revoked_keys,
            catalog,
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
        assert_eq!(applied_versions(&pool).await.last().copied(), Some(30));
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

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn identity_0030_additive_upgrade_preserves_legacy_state_and_handles_history() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(29, &pool)
            .await
            .expect("apply migrations through legacy identity lifecycle");

        let domain_a = uuid::Uuid::new_v4();
        let domain_b = uuid::Uuid::new_v4();
        for (domain, label) in [(domain_a, "a"), (domain_b, "b")] {
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(domain)
                .bind(format!("identity-0030-{label}-{}.example", domain.simple()))
                .execute(&pool)
                .await
                .expect("insert fixture community");
        }

        let active_key = vec![1_u8; 32];
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source, created_at, updated_at, last_seen_at) \
             VALUES ($1, ' Issuer ', ' Subject ', $2, 'jwt_npub', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z', TIMESTAMPTZ '2025-01-01 00:00:00Z', TIMESTAMPTZ '2025-01-01 00:00:00Z')",
        )
        .bind(domain_a)
        .bind(&active_key)
        .execute(&pool)
        .await
        .expect("insert literal active principal");
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source, created_at, updated_at, last_seen_at) \
             VALUES ($1, ' Issuer ', ' Subject ', $2, 'jwt_npub', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z', TIMESTAMPTZ '2025-01-01 00:00:00Z', TIMESTAMPTZ '2025-01-01 00:00:00Z')",
        )
        .bind(domain_b)
        .bind(&active_key)
        .execute(&pool)
        .await
        .expect("insert cross-domain control");

        let chain_keys = [vec![10_u8; 32], vec![11_u8; 32], vec![12_u8; 32]];
        for (index, key) in chain_keys.iter().enumerate() {
            if let Some(successor) = chain_keys.get(index + 1) {
                sqlx::query(
                    "INSERT INTO identity_bindings \
                     (community_id, issuer, uid, pubkey, source, created_at, updated_at, last_seen_at, \
                      revoked_at, revoked_reason, revocation_scope, rotation_completed_at, \
                      rotated_to_pubkey, rotation_reason) \
                     VALUES ($1, 'https://idp.example', 'chain', $2, 'db_binding', \
                             TIMESTAMPTZ '2025-02-01 00:00:00Z', TIMESTAMPTZ '2025-02-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-02-01 00:00:00Z', TIMESTAMPTZ '2025-02-01 00:00:00Z', \
                             'legacy rotation', 'rotation', TIMESTAMPTZ '2025-02-01 00:00:00Z', \
                             $3, 'legacy rotation')",
                )
                .bind(domain_a)
                .bind(key)
                .bind(successor)
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("insert retired chain node {index}: {error}"));
            } else {
                sqlx::query(
                    "INSERT INTO identity_bindings \
                     (community_id, issuer, uid, pubkey, source, created_at, updated_at, last_seen_at) \
                     VALUES ($1, 'https://idp.example', 'chain', $2, 'db_binding', \
                             TIMESTAMPTZ '2025-02-01 00:00:00Z', TIMESTAMPTZ '2025-02-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-02-01 00:00:00Z')",
                )
                .bind(domain_a)
                .bind(key)
                .execute(&pool)
                .await
                .unwrap_or_else(|error| panic!("insert active chain node {index}: {error}"));
            }
        }
        for old in chain_keys.iter().take(chain_keys.len() - 1) {
            sqlx::query(
                "INSERT INTO identity_revoked_keys (community_id, pubkey, revoked_at, reason) \
                 VALUES ($1, $2, TIMESTAMPTZ '2025-02-01 00:00:00Z', 'legacy rotation')",
            )
            .bind(domain_a)
            .bind(old)
            .execute(&pool)
            .await
            .expect("insert legacy rotation tombstone");
        }

        let revoked_key = vec![20_u8; 32];
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source, revoked_at, revoked_reason, revocation_scope) \
             VALUES ($1, 'https://idp.example', 'pending', $2, 'db_binding', NOW(), 'explicit revoke', 'key')",
        )
        .bind(domain_a)
        .bind(&revoked_key)
        .execute(&pool)
        .await
        .expect("insert pending revoked binding");
        sqlx::query(
            "INSERT INTO identity_revoked_keys (community_id, pubkey, revoked_at, reason) \
             VALUES ($1, $2, TIMESTAMPTZ '2025-03-01 00:00:00Z', 'explicit revoke')",
        )
        .bind(domain_a)
        .bind(&revoked_key)
        .execute(&pool)
        .await
        .expect("insert explicit key selector");

        let active_tombstoned_key = vec![21_u8; 32];
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source) \
             VALUES ($1, 'https://idp.example', 'active-tombstone-owner', $2, 'db_binding')",
        )
        .bind(domain_a)
        .bind(&active_tombstoned_key)
        .execute(&pool)
        .await
        .expect("insert active binding overlapping legacy key tombstone");
        sqlx::query(
            "INSERT INTO identity_revoked_keys (community_id, pubkey, revoked_at, reason) \
             VALUES ($1, $2, TIMESTAMPTZ '2025-03-02 00:00:00Z', 'legacy active overlap')",
        )
        .bind(domain_a)
        .bind(&active_tombstoned_key)
        .execute(&pool)
        .await
        .expect("insert authoritative tombstone overlapping active binding");

        sqlx::query(
            "INSERT INTO identity_principals \
             (community_id, issuer, uid, disabled_at, disabled_reason) \
             VALUES ($1, 'https://idp.example', 'never-enrolled', NOW(), 'disabled before enrollment')",
        )
        .bind(domain_a)
        .execute(&pool)
        .await
        .expect("insert disabled never-enrolled principal");

        let missing_old = vec![30_u8; 32];
        let missing_target = vec![31_u8; 32];
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source, revoked_at, revoked_reason, revocation_scope, \
              rotation_completed_at, rotated_to_pubkey, rotation_reason) \
             VALUES ($1, 'https://idp.example', 'ambiguous', $2, 'jwt_npub', NOW(), \
                     'missing successor', 'rotation', NOW(), $3, 'missing successor')",
        )
        .bind(domain_a)
        .bind(&missing_old)
        .bind(&missing_target)
        .execute(&pool)
        .await
        .expect("insert missing-lineage fixture");
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source) \
             VALUES ($1, 'https://idp.example', 'active-target-owner', $2, 'db_binding')",
        )
        .bind(domain_a)
        .bind(&missing_target)
        .execute(&pool)
        .await
        .expect("insert cross-principal active target fixture");
        sqlx::query(
            "INSERT INTO identity_revoked_keys (community_id, pubkey, reason) VALUES ($1, $2, 'ambiguous legacy selector')",
        )
        .bind(domain_a)
        .bind(&missing_old)
        .execute(&pool)
        .await
        .expect("insert ambiguous selector");

        let before = legacy_identity_snapshot(&pool).await;
        run_migrations(&pool)
            .await
            .expect("additive identity migration succeeds on populated history");
        let after = legacy_identity_snapshot(&pool).await;
        assert_eq!(after.bindings, before.bindings, "legacy bindings changed");
        assert_eq!(
            after.principals, before.principals,
            "legacy principals changed"
        );
        assert_eq!(after.revoked_keys, before.revoked_keys, "legacy Y changed");
        assert!(
            before
                .catalog
                .iter()
                .all(|definition| after.catalog.contains(definition)),
            "legacy identity constraints/indexes must remain unchanged"
        );

        let chain: Vec<MigratedIdentityChainRow> = sqlx::query_as(
            "SELECT binding.pubkey, binding.binding_version, replacement.pubkey, binding.binding_provenance \
             FROM identity_bindings binding \
             LEFT JOIN identity_bindings replacement \
               ON replacement.community_id=binding.community_id \
              AND replacement.binding_id=binding.replacement_binding_id \
             WHERE binding.community_id=$1 AND binding.issuer='https://idp.example' AND binding.uid='chain' \
             ORDER BY binding.pubkey",
        )
        .bind(domain_a)
        .fetch_all(&pool)
        .await
        .expect("read migrated chain");
        assert_eq!(chain.len(), 3);
        assert_eq!(
            chain[0],
            (
                chain_keys[0].clone(),
                1,
                Some(chain_keys[1].clone()),
                "tofu".to_owned()
            )
        );
        assert_eq!(
            chain[1],
            (
                chain_keys[1].clone(),
                1,
                Some(chain_keys[2].clone()),
                "tofu".to_owned()
            )
        );
        assert_eq!(
            chain[2],
            (chain_keys[2].clone(), 1, None, "tofu".to_owned())
        );

        let pending: (Vec<u8>, i64, i64) = sqlx::query_as(
            "SELECT retired_pubkey, retired_binding_version, selector_version \
             FROM identity_pending_replacements \
             WHERE community_id=$1 AND issuer='https://idp.example' AND subject='pending' AND cleared_at IS NULL",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("read pending selector");
        assert_eq!(pending, (revoked_key.clone(), 1, 1));

        let quarantined: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_migration_denials \
             WHERE community_id=$1 AND issuer='https://idp.example' AND subject='ambiguous')",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("read migration quarantine");
        assert!(quarantined);

        let target_quarantined: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_migration_denied_keys \
             WHERE community_id=$1 AND pubkey=$2)",
        )
        .bind(domain_a)
        .bind(&missing_target)
        .fetch_one(&pool)
        .await
        .expect("read migrated key quarantine");
        assert!(target_quarantined);

        assert!(
            crate::identity_binding::get_active_identity_binding_by_pubkey(
                &pool,
                buzz_core::CommunityId::from_uuid(domain_a),
                &missing_target,
            )
            .await
            .is_err(),
            "migrated domain-key quarantine must deny active authorization lookup"
        );

        let clean_replacement_key = vec![32_u8; 32];
        let clean_replacement =
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &clean_replacement_key,
                None,
                crate::identity_binding::BindingProvenance::Provisioned,
                "migration-test-policy",
            )
            .expect("construct clean rotation replacement");
        assert!(crate::identity_lifecycle::rotate_identity_binding(
            &pool,
            buzz_core::CommunityId::from_uuid(domain_a),
            crate::identity_lifecycle::LifecycleContext {
                operation_id: crate::identity_lifecycle::LifecycleOperationId::issue(),
                actor: &missing_target,
                reason: "migrated key quarantine must not be laundered",
            },
            crate::identity_lifecycle::IdentityPrincipal {
                issuer: "https://idp.example",
                subject: "active-target-owner",
            },
            &missing_target,
            clean_replacement,
        )
        .await
        .is_err());

        assert!(
            crate::identity_binding::get_active_identity_binding_by_pubkey(
                &pool,
                buzz_core::CommunityId::from_uuid(domain_a),
                &active_tombstoned_key,
            )
            .await
            .is_err(),
            "legacy key tombstone must deny migrated active authorization lookup"
        );
        let tombstone_rotation_key = vec![33_u8; 32];
        let tombstone_rotation =
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &tombstone_rotation_key,
                None,
                crate::identity_binding::BindingProvenance::Provisioned,
                "migration-test-policy",
            )
            .expect("construct tombstone rotation replacement");
        assert!(crate::identity_lifecycle::rotate_identity_binding(
            &pool,
            buzz_core::CommunityId::from_uuid(domain_a),
            crate::identity_lifecycle::LifecycleContext {
                operation_id: crate::identity_lifecycle::LifecycleOperationId::issue(),
                actor: &active_tombstoned_key,
                reason: "legacy key tombstone must not be laundered",
            },
            crate::identity_lifecycle::IdentityPrincipal {
                issuer: "https://idp.example",
                subject: "active-tombstone-owner",
            },
            &active_tombstoned_key,
            tombstone_rotation,
        )
        .await
        .is_err());

        let before_denials: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1)",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("snapshot migrated authority state");
        let domain_a_id = buzz_core::CommunityId::from_uuid(domain_a);
        for (subject, key) in [
            ("ambiguous", missing_old.as_slice()),
            ("other-principal", missing_target.as_slice()),
            ("other-revoked-principal", revoked_key.as_slice()),
        ] {
            let result = crate::identity_binding::resolve_identity_binding(
                &pool,
                &crate::identity_binding::ResolveBindingInput {
                    authorization_domain: domain_a_id,
                    issuer: "https://idp.example",
                    subject,
                    pubkey: key,
                    display_name: None,
                    enrollment_mode: crate::identity_binding::EnrollmentMode::AttestedKey,
                    key_attested: true,
                    policy_version: "migration-test-policy-v1",
                    evidence_valid_from: 0,
                    evidence_valid_until: i64::MAX as u64,
                },
            )
            .await
            .expect("resolve migrated denied identity");
            assert_eq!(
                result,
                crate::identity_binding::ResolveBindingResult::Denied(
                    crate::identity_binding::BindingDenial::Revoked
                )
            );
        }
        let denied_replacement =
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &missing_target,
                None,
                crate::identity_binding::BindingProvenance::Provisioned,
                "migration-test-policy",
            )
            .expect("construct denied migrated replacement");
        assert!(crate::identity_lifecycle::provision_identity_binding(
            &pool,
            domain_a_id,
            crate::identity_lifecycle::LifecycleContext {
                operation_id: crate::identity_lifecycle::LifecycleOperationId::issue(),
                actor: &missing_target,
                reason: "migrated ambiguity must block lifecycle",
            },
            crate::identity_lifecycle::IdentityPrincipal {
                issuer: "https://idp.example",
                subject: "lifecycle-other-principal",
            },
            crate::identity_binding::EnrollmentMode::Provisioned,
            denied_replacement,
        )
        .await
        .is_err());
        let after_denials: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1)",
        )
        .bind(domain_a)
        .fetch_one(&pool)
        .await
        .expect("verify migrated denials did not mutate authority");
        assert_eq!(after_denials, before_denials);

        let domain_b_result = crate::identity_binding::resolve_identity_binding(
            &pool,
            &crate::identity_binding::ResolveBindingInput {
                authorization_domain: buzz_core::CommunityId::from_uuid(domain_b),
                issuer: "https://idp.example",
                subject: "cross-domain-allowed",
                pubkey: &missing_target,
                display_name: None,
                enrollment_mode: crate::identity_binding::EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "migration-test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("resolve same key in independent domain");
        assert!(matches!(
            domain_b_result,
            crate::identity_binding::ResolveBindingResult::Enrolled(_)
        ));
    }

    const IDENTITY_HISTORY_LENGTHS: [usize; 6] = [0, 1, 2, 3, 8, 32];

    fn identity_history_keys(length: usize, namespace: u8) -> Vec<Vec<u8>> {
        (0..length)
            .map(|index| {
                let mut key = vec![0_u8; 32];
                key[0] = length as u8;
                key[1] = index as u8;
                key[30] = namespace;
                key[31] = 0xa5;
                key
            })
            .collect()
    }

    async fn insert_legacy_identity_history(
        pool: &PgPool,
        domain: uuid::Uuid,
        issuer: &str,
        subject: &str,
        keys: &[Vec<u8>],
    ) {
        // Deliberately reverse insertion order. Equal timestamps model the
        // transaction-stable NOW() values emitted by the legacy helper.
        for index in (0..keys.len()).rev() {
            if index + 1 == keys.len() {
                sqlx::query(
                    "INSERT INTO identity_bindings \
                     (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at) \
                     VALUES ($1,$2,$3,$4,'db_binding', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z')",
                )
                .bind(domain)
                .bind(issuer)
                .bind(subject)
                .bind(&keys[index])
                .execute(pool)
                .await
                .expect("insert terminal history node");
            } else {
                sqlx::query(
                    "INSERT INTO identity_bindings \
                     (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at, \
                      revoked_at,revoked_reason,revocation_scope,rotation_completed_at, \
                      rotated_to_pubkey,rotation_reason) \
                     VALUES ($1,$2,$3,$4,'db_binding', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z','legacy rotation','rotation', \
                             TIMESTAMPTZ '2025-04-01 00:00:00Z',$5,'legacy rotation')",
                )
                .bind(domain)
                .bind(issuer)
                .bind(subject)
                .bind(&keys[index])
                .bind(&keys[index + 1])
                .execute(pool)
                .await
                .expect("insert retired history node");
            }
        }
    }

    async fn assert_legacy_domain_sentinels(
        pool: &PgPool,
        domain: uuid::Uuid,
        keys: &[Vec<u8>],
        expected_facts: &[String],
        expected_authorization: &[bool],
        expected_audit: &[String],
        expected_history: &[String],
    ) {
        assert_eq!(
            super::deterministic_tests::legacy_identity_facts(pool, domain).await,
            expected_facts
        );
        let mut authorization = Vec::with_capacity(keys.len());
        for key in keys {
            authorization
                .push(super::deterministic_tests::raw_domain_authorized(pool, domain, key).await);
        }
        assert_eq!(authorization, expected_authorization);
        assert_eq!(
            super::deterministic_tests::domain_audit_snapshot(pool, domain).await,
            expected_audit
        );
        assert_eq!(
            super::deterministic_tests::domain_legacy_history_snapshot(pool, domain).await,
            expected_history
        );
    }

    async fn assert_imported_identity_history(
        pool: &PgPool,
        domain: uuid::Uuid,
        issuer: &str,
        subject: &str,
        keys: &[Vec<u8>],
    ) {
        type BindingRow = (uuid::Uuid, i64, Vec<u8>, String, String, Option<uuid::Uuid>);
        let rows: Vec<BindingRow> = sqlx::query_as(
            "SELECT binding_id,binding_version,pubkey,binding_state, \
                    binding_provenance,replacement_binding_id \
             FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3 \
             ORDER BY pubkey",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_all(pool)
        .await
        .expect("read imported history");
        assert_eq!(rows.len(), keys.len());
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.1, 1);
            assert_eq!(row.2, keys[index]);
            assert_eq!(
                row.3,
                if index + 1 == keys.len() {
                    "active"
                } else {
                    "rotated"
                }
            );
            assert_eq!(row.4, "tofu");
            assert_eq!(row.5, rows.get(index + 1).map(|successor| successor.0));
        }

        let edges: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
            "SELECT predecessor.binding_id,successor.binding_id \
             FROM identity_binding_lineage lineage \
             JOIN identity_bindings predecessor \
               ON predecessor.community_id=lineage.community_id \
              AND predecessor.binding_id=lineage.predecessor_binding_id \
             JOIN identity_bindings successor \
               ON successor.community_id=lineage.community_id \
              AND successor.binding_id=lineage.successor_binding_id \
             WHERE predecessor.community_id=$1 \
               AND predecessor.issuer=$2 AND predecessor.uid=$3 \
             ORDER BY predecessor.pubkey",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_all(pool)
        .await
        .expect("read imported lineage");
        let expected_edges = rows
            .windows(2)
            .map(|pair| (pair[0].0, pair[1].0))
            .collect::<Vec<_>>();
        assert_eq!(edges, expected_edges);

        let retired: Vec<(Vec<u8>, Option<uuid::Uuid>, Option<i64>)> = sqlx::query_as(
            "SELECT pubkey,retired_binding_id,retired_binding_version \
             FROM identity_retired_pairs \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 \
             ORDER BY pubkey",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_all(pool)
        .await
        .expect("read imported retired pairs");
        let expected_retired = rows
            .iter()
            .take(rows.len().saturating_sub(1))
            .map(|row| (row.2.clone(), Some(row.0), Some(row.1)))
            .collect::<Vec<_>>();
        assert_eq!(retired, expected_retired);

        type HistoryRow = (
            uuid::Uuid,
            i64,
            Vec<u8>,
            String,
            String,
            String,
            Option<uuid::Uuid>,
        );
        let history: Vec<HistoryRow> = sqlx::query_as(
            "SELECT binding_id,binding_version,pubkey,binding_state, \
                    binding_provenance,transition_kind,replacement_binding_id \
             FROM identity_binding_history \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 \
             ORDER BY pubkey",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_all(pool)
        .await
        .expect("read imported binding history");
        assert_eq!(history.len(), rows.len());
        for (binding, history_row) in rows.iter().zip(&history) {
            assert_eq!(history_row.0, binding.0);
            assert_eq!(history_row.1, binding.1);
            assert_eq!(history_row.2, binding.2);
            assert_eq!(history_row.3, binding.3);
            assert_eq!(history_row.4, binding.4);
            assert_eq!(history_row.5, "legacy_import");
            assert_eq!(history_row.6, binding.5);
        }

        let denied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM identity_migration_denials \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3)",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("read imported principal denial");
        assert!(!denied);
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_pending_replacements \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(domain)
        .bind(issuer)
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("read imported pending selectors");
        assert_eq!(pending, 0);
        for (index, key) in keys.iter().enumerate() {
            let denied_key: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM identity_migration_denied_keys \
                 WHERE community_id=$1 AND pubkey=$2)",
            )
            .bind(domain)
            .bind(key)
            .fetch_one(pool)
            .await
            .expect("read imported key denial");
            assert!(!denied_key);
            let active = crate::identity_binding::get_active_identity_binding_by_pubkey(
                pool,
                buzz_core::CommunityId::from_uuid(domain),
                key,
            )
            .await
            .expect("read imported authorization");
            if index + 1 == keys.len() {
                let active = active.expect("history head remains authoritative");
                assert_eq!(active.binding_id, rows[index].0);
                assert_eq!(active.binding_version, rows[index].1 as u64);
                assert_eq!(active.issuer, issuer);
                assert_eq!(active.uid, subject);
                assert_eq!(active.pubkey, *key);
                assert_eq!(
                    active.binding_state,
                    crate::identity_binding::BindingState::Active
                );
            } else {
                assert!(active.is_none(), "retired history key regained authority");
            }
        }
        if keys.is_empty() {
            let absent = crate::identity_binding::get_active_identity_binding_by_pubkey(
                pool,
                buzz_core::CommunityId::from_uuid(domain),
                &[0xe0_u8; 32],
            )
            .await
            .expect("read zero-history authorization sentinel");
            assert!(absent.is_none(), "zero history invented authority");
        }
    }

    async fn assert_temporal_inversion_quarantined(
        pool: &PgPool,
        domain: uuid::Uuid,
        older_target: &[u8],
    ) {
        let inversion_denied: (bool, bool) = sqlx::query_as(
            "SELECT \
               EXISTS(SELECT 1 FROM identity_migration_denials \
                      WHERE community_id=$1 AND issuer='https://idp.example' AND subject='temporal-inversion'), \
               EXISTS(SELECT 1 FROM identity_migration_denied_keys \
                      WHERE community_id=$1 AND pubkey=$2)",
        )
        .bind(domain)
        .bind(older_target)
        .fetch_one(pool)
        .await
        .expect("read temporal inversion quarantine");
        assert_eq!(inversion_denied, (true, true));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn identity_0030_imports_arbitrary_histories_and_quarantines_temporal_inversion() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(29, &pool)
            .await
            .expect("apply migrations through legacy identity lifecycle");
        let domain = uuid::Uuid::new_v4();
        let domain_b = uuid::Uuid::new_v4();
        for (community, suffix) in [(domain, "a"), (domain_b, "b")] {
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
                .bind(community)
                .bind(format!(
                    "identity-0030-history-{suffix}-{}.example",
                    community.simple()
                ))
                .execute(&pool)
                .await
                .expect("insert history community");
        }

        let mut seeded_lengths = BTreeSet::new();
        for length in IDENTITY_HISTORY_LENGTHS {
            let subject = format!("history-{length}");
            let keys = identity_history_keys(length, 0);
            insert_legacy_identity_history(&pool, domain, "https://idp.example", &subject, &keys)
                .await;
            assert!(seeded_lengths.insert(length));
        }
        assert_eq!(
            seeded_lengths,
            IDENTITY_HISTORY_LENGTHS.into_iter().collect()
        );

        let domain_b_keys = identity_history_keys(3, 0xb7);
        insert_legacy_identity_history(
            &pool,
            domain_b,
            "https://domain-b.example",
            "domain-b-history-sentinel",
            &domain_b_keys,
        )
        .await;
        sqlx::query(
            "INSERT INTO audit_log \
             (community_id,seq,hash,action,object_id,detail,created_at) \
             VALUES ($1,1,$2,'preexisting_domain_b_history', \
                     'domain-b-history-sentinel', \
                     '{\"sentinel\":\"before-domain-a-operation\"}'::jsonb, \
                     TIMESTAMPTZ '2025-04-01 00:00:00Z')",
        )
        .bind(domain_b)
        .bind(vec![0xb7_u8; 32])
        .execute(&pool)
        .await
        .expect("insert substantive domain-B audit sentinel");

        let domain_b_facts_pre =
            super::deterministic_tests::legacy_identity_facts(&pool, domain_b).await;
        let domain_b_audit_pre =
            super::deterministic_tests::domain_audit_snapshot(&pool, domain_b).await;
        let domain_b_history_pre =
            super::deterministic_tests::domain_legacy_history_snapshot(&pool, domain_b).await;
        let domain_b_authorization_pre = vec![false, false, true];
        assert_legacy_domain_sentinels(
            &pool,
            domain_b,
            &domain_b_keys,
            &domain_b_facts_pre,
            &domain_b_authorization_pre,
            &domain_b_audit_pre,
            &domain_b_history_pre,
        )
        .await;

        let older_target = vec![240_u8; 32];
        let newer_predecessor = vec![241_u8; 32];
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at) \
             VALUES ($1,'https://idp.example','temporal-inversion',$2,'db_binding', \
                     TIMESTAMPTZ '2024-01-01 00:00:00Z', \
                     TIMESTAMPTZ '2024-01-01 00:00:00Z', \
                     TIMESTAMPTZ '2024-01-01 00:00:00Z')",
        )
        .bind(domain)
        .bind(&older_target)
        .execute(&pool)
        .await
        .expect("insert older target");
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,issuer,uid,pubkey,source,created_at,updated_at,last_seen_at, \
              revoked_at,revoked_reason,revocation_scope,rotation_completed_at, \
              rotated_to_pubkey,rotation_reason) \
             VALUES ($1,'https://idp.example','temporal-inversion',$2,'db_binding', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z','invalid rotation','rotation', \
                     TIMESTAMPTZ '2025-01-01 00:00:00Z',$3,'invalid rotation')",
        )
        .bind(domain)
        .bind(&newer_predecessor)
        .bind(&older_target)
        .execute(&pool)
        .await
        .expect("insert temporally inverted predecessor");

        run_migrations(&pool)
            .await
            .expect("import arbitrary valid histories without aborting");
        // Prove domain B before the first domain-A read after the operation.
        assert_legacy_domain_sentinels(
            &pool,
            domain_b,
            &domain_b_keys,
            &domain_b_facts_pre,
            &domain_b_authorization_pre,
            &domain_b_audit_pre,
            &domain_b_history_pre,
        )
        .await;
        assert_imported_identity_history(
            &pool,
            domain_b,
            "https://domain-b.example",
            "domain-b-history-sentinel",
            &domain_b_keys,
        )
        .await;
        let domain_b_state_post =
            super::deterministic_tests::domain_identity_snapshot(&pool, domain_b).await;
        let domain_b_history_post =
            super::deterministic_tests::domain_binding_history_snapshot(&pool, domain_b).await;
        assert_eq!(domain_b_history_post.len(), domain_b_keys.len());

        let mut verified_lengths = BTreeSet::new();
        for length in IDENTITY_HISTORY_LENGTHS {
            let subject = format!("history-{length}");
            let keys = identity_history_keys(length, 0);
            assert_imported_identity_history(&pool, domain, "https://idp.example", &subject, &keys)
                .await;
            assert!(verified_lengths.insert(length));
        }
        assert_eq!(
            verified_lengths,
            IDENTITY_HISTORY_LENGTHS.into_iter().collect()
        );
        assert_temporal_inversion_quarantined(&pool, domain, &older_target).await;
        let domain_a_state_post =
            super::deterministic_tests::domain_identity_snapshot(&pool, domain).await;
        assert_legacy_domain_sentinels(
            &pool,
            domain_b,
            &domain_b_keys,
            &domain_b_facts_pre,
            &domain_b_authorization_pre,
            &domain_b_audit_pre,
            &domain_b_history_pre,
        )
        .await;
        assert_eq!(
            super::deterministic_tests::domain_identity_snapshot(&pool, domain_b).await,
            domain_b_state_post
        );
        assert_eq!(
            super::deterministic_tests::domain_binding_history_snapshot(&pool, domain_b).await,
            domain_b_history_post
        );

        pool.close().await;
        let pool = connect_test_pool().await;
        for length in IDENTITY_HISTORY_LENGTHS {
            let subject = format!("history-{length}");
            let keys = identity_history_keys(length, 0);
            assert_imported_identity_history(&pool, domain, "https://idp.example", &subject, &keys)
                .await;
        }
        assert_temporal_inversion_quarantined(&pool, domain, &older_target).await;
        assert_eq!(
            super::deterministic_tests::domain_identity_snapshot(&pool, domain).await,
            domain_a_state_post
        );
        assert_legacy_domain_sentinels(
            &pool,
            domain_b,
            &domain_b_keys,
            &domain_b_facts_pre,
            &domain_b_authorization_pre,
            &domain_b_audit_pre,
            &domain_b_history_pre,
        )
        .await;
        assert_eq!(
            super::deterministic_tests::domain_identity_snapshot(&pool, domain_b).await,
            domain_b_state_post
        );
        assert_eq!(
            super::deterministic_tests::domain_binding_history_snapshot(&pool, domain_b).await,
            domain_b_history_post
        );

        run_migrations(&pool)
            .await
            .expect("retry arbitrary-history migration idempotently");
        for length in IDENTITY_HISTORY_LENGTHS {
            let subject = format!("history-{length}");
            let keys = identity_history_keys(length, 0);
            assert_imported_identity_history(&pool, domain, "https://idp.example", &subject, &keys)
                .await;
        }
        assert_temporal_inversion_quarantined(&pool, domain, &older_target).await;
        assert_eq!(
            super::deterministic_tests::domain_identity_snapshot(&pool, domain).await,
            domain_a_state_post
        );
        assert_legacy_domain_sentinels(
            &pool,
            domain_b,
            &domain_b_keys,
            &domain_b_facts_pre,
            &domain_b_authorization_pre,
            &domain_b_audit_pre,
            &domain_b_history_pre,
        )
        .await;
        assert_eq!(
            super::deterministic_tests::domain_identity_snapshot(&pool, domain_b).await,
            domain_b_state_post
        );
        assert_eq!(
            super::deterministic_tests::domain_binding_history_snapshot(&pool, domain_b).await,
            domain_b_history_post
        );
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn identity_0030_failure_rolls_back_every_additive_projection() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        MIGRATOR
            .run_to(29, &pool)
            .await
            .expect("apply migrations through legacy identity lifecycle");
        let domain = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(domain)
            .bind(format!(
                "identity-0030-rollback-{}.example",
                domain.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert rollback community");
        sqlx::query(
            "INSERT INTO identity_bindings (community_id, issuer, uid, pubkey, source) \
             VALUES ($1,'https://idp.example','rollback',$2,'db_binding')",
        )
        .bind(domain)
        .bind(vec![99_u8; 32])
        .execute(&pool)
        .await
        .expect("insert rollback fixture");
        // Reserve the name of 0030's final index so the transaction fails only
        // after every preceding additive DDL and backfill statement has run.
        sqlx::query(
            "CREATE INDEX idx_identity_lifecycle_operations_key \
             ON identity_bindings (community_id, pubkey)",
        )
        .execute(&pool)
        .await
        .expect("create late migration conflict");

        assert!(MIGRATOR.run_to(30, &pool).await.is_err());
        let projected_tables: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT to_regclass('identity_retired_pairs')::text, \
                    to_regclass('identity_pending_replacements')::text, \
                    to_regclass('identity_binding_history')::text",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect rolled back tables");
        assert_eq!(projected_tables, (None, None, None));
        let projected_columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns \
             WHERE table_schema='public' AND table_name='identity_bindings' \
               AND column_name IN ('binding_id','binding_version','binding_state','binding_provenance')",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect rolled back columns");
        assert_eq!(projected_columns, 0);
        let latest: i64 =
            sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
                .fetch_one(&pool)
                .await
                .expect("read latest migration");
        assert_eq!(latest, 29);
        // sqlx's failed pool-backed migration attempt can return the session
        // while its session advisory lock is still held. Closing this
        // disposable pool releases that lock before the explicit retry.
        pool.close().await;
        let pool = connect_test_pool().await;
        sqlx::query("DROP INDEX idx_identity_lifecycle_operations_key")
            .execute(&pool)
            .await
            .expect("drop late migration conflict");
        run_migrations(&pool)
            .await
            .expect("retry additive projection after rollback");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn audio_visibility_0039_applies_from_empty_database_without_later_objects() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        MIGRATOR
            .run_to(39, &pool)
            .await
            .expect("apply migrations through audio visibility");

        assert_eq!(applied_versions(&pool).await.last().copied(), Some(39));
        let admission_columns: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns \
             WHERE table_schema='public' AND table_name='audio_session_admissions' \
               AND column_name IN ('claimant_id', 'attachment_generation', \
                                   'claim_expires_at', 'visibility_observed_at')",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect audio admission columns");
        assert_eq!(admission_columns, 4);

        let admission_constraints: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_constraint \
             WHERE conrelid='audio_session_admissions'::regclass \
               AND conname IN ('audio_session_admissions_state', \
                               'audio_session_admissions_claim', \
                               'audio_session_admissions_visibility')",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect audio admission constraints");
        assert_eq!(admission_constraints, 3);

        let later_authority_table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('authorization_authority_epochs')::text")
                .fetch_one(&pool)
                .await
                .expect("inspect later authority table");
        assert_eq!(later_authority_table, None);
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
            "identity_bindings",
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authority_triggers_preserve_off_and_deny_unwitnessed_protected_teardown() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        run_migrations(&pool).await.expect("apply all migrations");

        let community_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!(
                "authority-trigger-{}.example",
                community_id.simple()
            ))
            .execute(&pool)
            .await
            .expect("insert legacy community");
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')",
        )
        .bind(community_id)
        .bind("11".repeat(32))
        .execute(&pool)
        .await
        .expect("legacy membership remains writable");
        let legacy_domains: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community_id)
        .fetch_one(&pool)
        .await
        .expect("read legacy authorization rows");
        assert_eq!(legacy_domains, 0, "Off must not acquire protected state");

        sqlx::query("INSERT INTO authorization_invalidation_domains (community_id) VALUES ($1)")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("initialize protected domain");
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')",
        )
        .bind(community_id)
        .bind("22".repeat(32))
        .execute(&pool)
        .await
        .expect("protected membership mutation");
        let generation: i64 = sqlx::query_scalar(
            "SELECT generation FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community_id)
        .fetch_one(&pool)
        .await
        .expect("read protected generation");
        assert_eq!(generation, 1, "one mutation advances generation once");

        sqlx::query("DELETE FROM relay_members WHERE community_id=$1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete ordinary community-owned rows first");
        assert!(sqlx::query("DELETE FROM communities WHERE id=$1")
            .bind(community_id)
            .execute(&pool)
            .await
            .is_err());
        let retained: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_authority_epochs WHERE community_id=$1",
        )
        .bind(community_id)
        .fetch_one(&pool)
        .await
        .expect("read retained authority state");
        assert_eq!(retained, 1, "denied teardown retains the monotonic floor");

        assert!(sqlx::query(
            "DELETE FROM authorization_invalidation_domains WHERE community_id=$1"
        )
        .bind(community_id)
        .execute(&pool)
        .await
        .is_err());
        let marker: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community_id)
        .fetch_one(&pool)
        .await
        .expect("read retained activation marker");
        assert_eq!(marker, 1, "protected activation is a one-way cutover");
    }
}
