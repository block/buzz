//! Postgres-backed store: migrations, runtime-role grants, readiness, and
//! retention for the gateway's own tables.
mod subscriptions;

use crate::store::{
    transition, AppliedOperation, ApplyOutcome, ClaimedDelivery, DeliveryPage, DeliveryState,
    DeliverySummary, InsertOutcome, Integration, IntegrationSecret, IntegrationView, MessageCounts,
    Mutation, NewDelivery, Operation, Provider, Store, StoreError,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{postgres::PgRow, AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

static GATEWAY_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const GATEWAY_TABLES: [&str; 6] = [
    "ingress_gateway_integrations",
    "ingress_gateway_integration_secrets",
    "ingress_gateway_applied_operations",
    "ingress_gateway_deliveries",
    "ingress_gateway_subscriptions",
    "ingress_gateway_messages",
];

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

    /// Apply pending migrations without the bootstrap guard or grants, for
    /// tests against a scratch database. Kept here so the gateway runs its
    /// migrator from one file only.
    #[cfg(test)]
    pub(crate) async fn migrate_scratch(pool: &PgPool) -> Result<(), StoreError> {
        GATEWAY_MIGRATOR.run(pool).await?;
        Ok(())
    }

    /// Delete terminal deliveries and messages, applied operations, retired
    /// secrets, and subscription tombstones older than the retention window.
    /// Queued deliveries and messages are never reaped here.
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
        let messages = sqlx::query(
            "DELETE FROM ingress_gateway_messages
             WHERE state <> 'queued' AND completed_at < $1",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?
        .rows_affected();
        let tombstones = sqlx::query(
            "DELETE FROM ingress_gateway_subscriptions
             WHERE deleted_at IS NOT NULL AND deleted_at < $1",
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
        let secrets = sqlx::query(
            "DELETE FROM ingress_gateway_integration_secrets
             WHERE retired_at IS NOT NULL AND (grace_until IS NULL OR grace_until < $1)",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(deliveries + messages + tombstones + operations + secrets)
    }
}

fn integration_from_row(row: PgRow) -> Result<Integration, StoreError> {
    integration_from_row_ref(&row)
}

fn integration_from_row_ref(row: &PgRow) -> Result<Integration, StoreError> {
    let provider: String = row.try_get("provider")?;
    Ok(Integration {
        id: row.try_get("id")?,
        community_host: row.try_get("community_host")?,
        provider: Provider::parse(&provider).ok_or(StoreError::Invalid(
            "unknown provider stored for integration",
        ))?,
        revision: row.try_get("revision")?,
        enabled: row.try_get("enabled")?,
        disabled_at: row.try_get("disabled_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn secret_from_row(row: PgRow) -> Result<IntegrationSecret, StoreError> {
    Ok(IntegrationSecret {
        id: row.try_get("id")?,
        integration_id: row.try_get("integration_id")?,
        key_id: row.try_get("key_id")?,
        ciphertext: row.try_get("secret_ciphertext")?,
        created_at: row.try_get("created_at")?,
        retired_at: row.try_get("retired_at")?,
        grace_until: row.try_get("grace_until")?,
    })
}

fn operation_from_row(row: PgRow) -> Result<AppliedOperation, StoreError> {
    Ok(AppliedOperation {
        operation_id: row.try_get("operation_id")?,
        integration_id: row.try_get("integration_id")?,
        revision: row.try_get("revision")?,
        fingerprint: row.try_get("request_fingerprint")?,
        response: row.try_get("response")?,
        applied_at: row.try_get("applied_at")?,
    })
}

const VERIFYING_SECRETS_SQL: &str =
    "SELECT id, integration_id, key_id, secret_ciphertext, created_at, retired_at, grace_until
     FROM ingress_gateway_integration_secrets
     WHERE integration_id = $1
       AND (retired_at IS NULL OR grace_until > $2)
     ORDER BY created_at DESC, id DESC";

async fn load_verifying_secrets(
    tx: &mut Transaction<'_, Postgres>,
    integration_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Vec<IntegrationSecret>, StoreError> {
    sqlx::query(VERIFYING_SECRETS_SQL)
        .bind(integration_id)
        .bind(now)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(secret_from_row)
        .collect()
}

#[async_trait]
impl Store for PostgresStore {
    /// Readiness: every gateway table is reachable with the runtime role.
    async fn ready(&self) -> Result<(), StoreError> {
        for table in GATEWAY_TABLES {
            sqlx::raw_sql(AssertSqlSafe(format!("SELECT 1 FROM {table} LIMIT 0")))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    async fn integration(&self, id: Uuid) -> Result<Option<Integration>, StoreError> {
        sqlx::query(
            "SELECT id, community_host, provider, revision, enabled, disabled_at,
                    created_at, updated_at FROM ingress_gateway_integrations WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(integration_from_row)
        .transpose()
    }

    async fn integration_with_secrets(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<(Integration, Vec<IntegrationSecret>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT i.id, i.community_host, i.provider, i.revision, i.enabled, i.disabled_at,
                    i.created_at, i.updated_at,
                    s.id AS secret_id, s.key_id, s.secret_ciphertext,
                    s.created_at AS secret_created_at, s.retired_at, s.grace_until
             FROM ingress_gateway_integrations i
             LEFT JOIN ingress_gateway_integration_secrets s
               ON s.integration_id = i.id
              AND (s.retired_at IS NULL OR s.grace_until > $2)
             WHERE i.id = $1
             ORDER BY s.created_at DESC, s.id DESC",
        )
        .bind(id)
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let Some(first) = rows.first() else {
            return Ok(None);
        };
        let integration = integration_from_row_ref(first)?;
        let mut secrets = Vec::with_capacity(rows.len());
        for row in &rows {
            let secret_id: Option<Uuid> = row.try_get("secret_id")?;
            if let Some(secret_id) = secret_id {
                secrets.push(IntegrationSecret {
                    id: secret_id,
                    integration_id: id,
                    key_id: row.try_get("key_id")?,
                    ciphertext: row.try_get("secret_ciphertext")?,
                    created_at: row.try_get("secret_created_at")?,
                    retired_at: row.try_get("retired_at")?,
                    grace_until: row.try_get("grace_until")?,
                });
            }
        }
        Ok(Some((integration, secrets)))
    }

    async fn verifying_secrets(
        &self,
        integration_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<IntegrationSecret>, StoreError> {
        sqlx::query(VERIFYING_SECRETS_SQL)
            .bind(integration_id)
            .bind(now)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(secret_from_row)
            .collect()
    }

    async fn applied_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<AppliedOperation>, StoreError> {
        sqlx::query("SELECT operation_id, integration_id, revision, request_fingerprint, response, applied_at FROM ingress_gateway_applied_operations
             WHERE operation_id = $1")
        .bind(operation_id)
        .fetch_optional(&self.pool)
        .await?
        .map(operation_from_row)
        .transpose()
    }

    async fn apply(
        &self,
        operation: Operation,
        now: DateTime<Utc>,
    ) -> Result<ApplyOutcome, StoreError> {
        let mut tx = self.pool.begin().await?;
        // Serialize concurrent operations on one integration: the row lock (or
        // the advisory lock when the row does not exist yet) makes the
        // idempotency check and the revision check a single linearizable step.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text))")
            .bind(operation.integration_id)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT operation_id, integration_id, revision, request_fingerprint, response, applied_at FROM ingress_gateway_applied_operations
             WHERE operation_id = $1")
        .bind(operation.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            tx.commit().await?;
            let applied = operation_from_row(row)?;
            return Ok(if applied.matches(&operation) {
                ApplyOutcome::Replayed(applied)
            } else {
                ApplyOutcome::Conflict
            });
        }
        let integration = sqlx::query(
            "SELECT id, community_host, provider, revision, enabled, disabled_at,
                    created_at, updated_at FROM ingress_gateway_integrations
             WHERE id = $1 FOR UPDATE",
        )
        .bind(operation.integration_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(integration_from_row)
        .transpose()?;
        let mut secrets = load_verifying_secrets(&mut tx, operation.integration_id, now).await?;
        let before: Vec<Uuid> = secrets.iter().map(|s| s.id).collect();
        let next = match transition(integration, &mut secrets, &operation, now) {
            Ok(next) => next,
            Err(outcome) => {
                tx.rollback().await?;
                return Ok(outcome);
            }
        };
        sqlx::query(
            "INSERT INTO ingress_gateway_integrations
                 (id, community_host, provider, revision, enabled, disabled_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET
                 community_host = EXCLUDED.community_host,
                 provider = EXCLUDED.provider,
                 revision = EXCLUDED.revision,
                 enabled = EXCLUDED.enabled,
                 disabled_at = EXCLUDED.disabled_at,
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(next.id)
        .bind(&next.community_host)
        .bind(next.provider.as_str())
        .bind(next.revision)
        .bind(next.enabled)
        .bind(next.disabled_at)
        .bind(next.created_at)
        .bind(next.updated_at)
        .execute(&mut *tx)
        .await?;
        if let Mutation::PutSecret { grace, .. } = &operation.mutation {
            sqlx::query(
                "UPDATE ingress_gateway_integration_secrets
                 SET retired_at = $2, grace_until = $3
                 WHERE integration_id = $1 AND retired_at IS NULL",
            )
            .bind(operation.integration_id)
            .bind(now)
            .bind(now + *grace)
            .execute(&mut *tx)
            .await?;
            for secret in secrets.iter().filter(|s| !before.contains(&s.id)) {
                sqlx::query(
                    "INSERT INTO ingress_gateway_integration_secrets
                         (id, integration_id, key_id, secret_ciphertext, created_at)
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(secret.id)
                .bind(secret.integration_id)
                .bind(&secret.key_id)
                .bind(&secret.ciphertext)
                .bind(secret.created_at)
                .execute(&mut *tx)
                .await?;
            }
        }
        let verifying: Vec<_> = secrets
            .iter()
            .filter(|s| s.verifies_at(now))
            .cloned()
            .collect();
        let response = serde_json::to_value(IntegrationView::new(&next, &verifying, now))
            .map_err(|_| StoreError::Invalid("integration view is not serializable"))?;
        let applied = AppliedOperation {
            operation_id: operation.operation_id,
            integration_id: operation.integration_id,
            revision: operation.revision,
            fingerprint: operation.fingerprint(),
            response,
            applied_at: now,
        };
        // The advisory lock serializes operations per integration only, so a
        // concurrent request reusing this operation_id on another integration
        // can race past the replay check and lose here on the primary key.
        let inserted = sqlx::query(
            "INSERT INTO ingress_gateway_applied_operations
                 (operation_id, integration_id, revision, request_fingerprint, response, applied_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(applied.operation_id)
        .bind(applied.integration_id)
        .bind(applied.revision)
        .bind(&applied.fingerprint)
        .bind(&applied.response)
        .bind(applied.applied_at)
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {}
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                tx.rollback().await?;
                return Ok(ApplyOutcome::Conflict);
            }
            Err(e) => return Err(e.into()),
        }
        tx.commit().await?;
        Ok(ApplyOutcome::Applied(applied))
    }

    async fn insert_delivery(&self, delivery: NewDelivery) -> Result<InsertOutcome, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO ingress_gateway_deliveries
                 (integration_id, delivery_id, event_type, provider_timestamp, headers, body,
                  state, next_attempt_at, received_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'queued', $7, $7)
             ON CONFLICT (integration_id, delivery_id) DO NOTHING",
        )
        .bind(delivery.integration_id)
        .bind(&delivery.delivery_id)
        .bind(&delivery.event_type)
        .bind(delivery.provider_timestamp)
        .bind(&delivery.headers)
        .bind(&delivery.body)
        .bind(delivery.received_at)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(if inserted == 1 {
            InsertOutcome::Inserted
        } else {
            InsertOutcome::Duplicate
        })
    }

    async fn list_deliveries(
        &self,
        integration_id: Uuid,
        page: DeliveryPage,
    ) -> Result<Vec<DeliverySummary>, StoreError> {
        let limit = i64::try_from(page.limit).unwrap_or(i64::MAX);
        sqlx::query(
            "SELECT d.delivery_id, d.event_type, d.state, d.attempts, d.event_id, d.received_at,
                    d.completed_at, d.last_error, m.queued, m.accepted, m.failed
             FROM ingress_gateway_deliveries d
             CROSS JOIN LATERAL (
                 SELECT count(*) FILTER (WHERE state = 'queued') AS queued,
                        count(*) FILTER (WHERE state = 'accepted') AS accepted,
                        count(*) FILTER (WHERE state = 'failed') AS failed
                 FROM ingress_gateway_messages
                 WHERE integration_id = d.integration_id AND delivery_id = d.delivery_id
             ) m
             WHERE d.integration_id = $1
               AND ($2::timestamptz IS NULL OR d.received_at > $2)
               AND ($3::timestamptz IS NULL OR (d.received_at, d.delivery_id) < ($3, $4::text))
               AND ($5::text IS NULL OR d.state = $5)
             ORDER BY d.received_at DESC, d.delivery_id DESC
             LIMIT $6",
        )
        .bind(integration_id)
        .bind(page.since)
        .bind(page.before.as_ref().map(|c| c.received_at))
        .bind(page.before.as_ref().map(|c| c.delivery_id.as_str()))
        .bind(page.state.map(DeliveryState::as_str))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            let state: String = row.try_get("state")?;
            Ok(DeliverySummary {
                delivery_id: row.try_get("delivery_id")?,
                event_type: row.try_get("event_type")?,
                state: DeliveryState::parse(&state)
                    .ok_or(StoreError::Invalid("unknown delivery state stored"))?,
                attempts: row.try_get("attempts")?,
                event_id: row.try_get("event_id")?,
                received_at: row.try_get("received_at")?,
                completed_at: row.try_get("completed_at")?,
                last_error: row.try_get("last_error")?,
                messages: MessageCounts {
                    queued: row.try_get("queued")?,
                    accepted: row.try_get("accepted")?,
                    failed: row.try_get("failed")?,
                },
            })
        })
        .collect()
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedDelivery>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        sqlx::query(
            "WITH due AS (
                 SELECT integration_id, delivery_id
                 FROM ingress_gateway_deliveries
                 WHERE state = 'queued' AND next_attempt_at <= $1
                 ORDER BY next_attempt_at
                 LIMIT $3
                 FOR UPDATE SKIP LOCKED
             ), claimed AS (
                 UPDATE ingress_gateway_deliveries d
                 SET attempts = d.attempts + 1, next_attempt_at = $2
                 FROM due
                 WHERE d.integration_id = due.integration_id
                   AND d.delivery_id = due.delivery_id
                 RETURNING d.integration_id, d.delivery_id, d.event_type, d.body,
                           d.received_at, d.attempts
             )
             SELECT c.integration_id, c.delivery_id, c.event_type, c.body, c.received_at,
                    c.attempts, i.provider
             FROM claimed c
             JOIN ingress_gateway_integrations i ON i.id = c.integration_id",
        )
        .bind(now)
        .bind(now + lease)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            let provider: String = row.try_get("provider")?;
            Ok(ClaimedDelivery {
                integration_id: row.try_get("integration_id")?,
                delivery_id: row.try_get("delivery_id")?,
                provider: Provider::parse(&provider)
                    .ok_or(StoreError::Invalid("unknown provider stored"))?,
                event_type: row.try_get("event_type")?,
                body: row.try_get("body")?,
                received_at: row.try_get("received_at")?,
                attempts: row.try_get("attempts")?,
            })
        })
        .collect()
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
