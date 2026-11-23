//! Postgres-backed store: migrations, runtime-role grants, readiness, and
//! retention for the gateway's own tables.
use chrono::{DateTime, Duration, Utc};
use sqlx::{AssertSqlSafe, PgPool};
use thiserror::Error;

static GATEWAY_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const GATEWAY_TABLES: [&str; 4] = [
    "ingress_gateway_integrations",
    "ingress_gateway_integration_secrets",
    "ingress_gateway_applied_operations",
    "ingress_gateway_deliveries",
];

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store unavailable")]
    Unavailable(#[from] sqlx::Error),
    #[error("migration failed")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0}")]
    Invalid(&'static str),
}

pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Run the bootstrap guard and all pending migrations, then grant the
    /// runtime role data access only. The runtime role never owns DDL.
    pub async fn apply_migrations_and_grants(
        pool: &PgPool,
        runtime_role: &str,
    ) -> Result<(), StoreError> {
        sqlx::raw_sql(include_str!("postgres/bootstrap_guard.sql"))
            .execute(pool)
            .await?;
        GATEWAY_MIGRATOR.run(pool).await?;
        if !is_postgres_identifier(runtime_role) {
            return Err(StoreError::Invalid(
                "runtime database role must be a PostgreSQL identifier",
            ));
        }
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(pool)
            .await?;
        let quote_ident = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
        let role = quote_ident(runtime_role);
        let database = quote_ident(&database);
        let grants = format!(
            "REVOKE CREATE ON DATABASE {database} FROM {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE CREATE ON SCHEMA public FROM {role};
             GRANT CONNECT ON DATABASE {database} TO {role};
             GRANT USAGE ON SCHEMA public TO {role};
             GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE {tables} TO {role};",
            tables = GATEWAY_TABLES.join(", "),
        );
        sqlx::raw_sql(AssertSqlSafe(grants)).execute(pool).await?;
        Ok(())
    }

    /// Readiness: every gateway table is reachable with the runtime role.
    pub async fn ready(&self) -> Result<(), StoreError> {
        for table in GATEWAY_TABLES {
            sqlx::raw_sql(AssertSqlSafe(format!("SELECT 1 FROM {table} LIMIT 0")))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Delete terminal deliveries and applied operations older than the
    /// retention window. Queued deliveries are never reaped here.
    pub async fn reap_expired(
        &self,
        now: DateTime<Utc>,
        retention_days: i64,
    ) -> Result<u64, StoreError> {
        let cutoff = now - Duration::days(retention_days);
        let deliveries = sqlx::query(
            "DELETE FROM ingress_gateway_deliveries
             WHERE state <> 'queued' AND completed_at < $1",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?
        .rows_affected();
        let operations =
            sqlx::query("DELETE FROM ingress_gateway_applied_operations WHERE applied_at < $1")
                .bind(cutoff)
                .execute(&self.pool)
                .await?
                .rows_affected();
        Ok(deliveries + operations)
    }
}

fn is_postgres_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn crate_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn strip_comments(text: &str) -> String {
        text.lines()
            .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn created_tables(sql: &str) -> BTreeSet<String> {
        strip_comments(sql)
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(3)
            .filter(|w| w[0] == "CREATE" && w[1] == "TABLE")
            .map(|w| w[2].trim_end_matches('(').to_owned())
            .collect()
    }

    #[test]
    fn runtime_role_identifier_rules() {
        assert!(is_postgres_identifier("ingress_runtime"));
        assert!(is_postgres_identifier("_r1"));
        assert!(!is_postgres_identifier(""));
        assert!(!is_postgres_identifier("1abc"));
        assert!(!is_postgres_identifier("role;DROP"));
        assert!(!is_postgres_identifier(&"a".repeat(64)));
    }

    #[test]
    fn migrations_declare_exactly_the_granted_tables() {
        let mut from_migrations = BTreeSet::new();
        for entry in std::fs::read_dir(crate_dir().join("migrations")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "sql") {
                from_migrations.extend(created_tables(&std::fs::read_to_string(path).unwrap()));
            }
        }
        let granted: BTreeSet<String> = GATEWAY_TABLES.iter().map(|t| (*t).to_owned()).collect();
        assert_eq!(from_migrations, granted);
    }

    #[test]
    fn desired_state_schema_matches_migrations() {
        let schema_dir = crate_dir().join("schema");
        let manifest = std::fs::read_to_string(schema_dir.join("schema.sql")).unwrap();
        let mut included = BTreeSet::new();
        for line in strip_comments(&manifest).lines().map(str::trim) {
            if line.is_empty() {
                continue;
            }
            let include = line
                .strip_prefix("\\i ")
                .unwrap_or_else(|| panic!("schema.sql may only hold \\i includes: {line}"));
            assert!(
                included.insert(include.trim().to_owned()),
                "duplicate include {include}"
            );
        }
        let mut on_disk = BTreeSet::new();
        for entry in std::fs::read_dir(schema_dir.join("tables/public")).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(&schema_dir).unwrap();
            on_disk.insert(relative.to_str().unwrap().to_owned());
            let table = Path::new(&path)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let sql = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                created_tables(&sql),
                BTreeSet::from([table.clone()]),
                "{table}.sql must declare exactly that table"
            );
            assert!(
                !sql.contains("IF NOT EXISTS") && !sql.contains("CONCURRENTLY"),
                "{table}.sql uses syntax declarative schema tools refuse"
            );
            assert!(
                GATEWAY_TABLES.contains(&table.as_str()),
                "{table} is not a gateway table"
            );
        }
        assert_eq!(
            included, on_disk,
            "schema.sql must include every table file once"
        );
        assert_eq!(on_disk.len(), GATEWAY_TABLES.len());
    }
}
