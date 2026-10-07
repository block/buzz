//! Serialized, bounded partition DDL for `events` and `delivery_log`.
//!
//! The manager issues only two kinds of change: creating a month that no
//! partition covers, and replacing an empty right-edge catch-all with
//! dedicated monthlies plus a new catch-all after the horizon. Each table's
//! change runs in one transaction that
//!
//! 1. holds a schema-scoped advisory lock, so concurrent relays produce one
//!    winner and `skipped_locked` losers;
//! 2. locks the parent, then foreign-key counterparts read under the parent
//!    lock, then the catch-all, in the order writers take them, waiting only on
//!    the parent and failing fast on any other contention;
//! 3. re-audits under those locks and refuses if the plan changed; and
//! 4. re-audits the changed catalog and verifies index and constraint
//!    inheritance before commit.
//!
//! A populated catch-all is always refused before any lock is taken: moving
//! rows requires an operator.

use std::time::Duration;

use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use sqlx::{Connection, PgConnection, PgPool, Row};
use tracing::info;

use super::{
    add_months, audit_partition_catalog_at, audit_table_on, month_start, partition_name,
    pin_catalog_rendering, qualified_relation_name, validated_months_ahead, MonthCoverageKind,
    PartitionAudit, PartitionBound, PartitionChildKind, PartitionTableAudit,
    MAX_PARTITION_MONTHS_AHEAD,
};
use crate::error::{DbError, Result};

/// Dedicated monthly runway, after the current month, that the relay maintains.
pub const PARTITION_MANAGER_MONTHS_AHEAD: u32 = 6;

/// Maximum wait for the parent lock. Counterparts and children use `NOWAIT`.
const MAINTENANCE_LOCK_TIMEOUT: &str = "2s";

/// Server-side budget for each statement in the maintenance transaction.
const MAINTENANCE_STATEMENT_TIMEOUT: &str = "5s";

/// Server-side reaping of a maintenance session whose client stalls mid-transaction.
const MAINTENANCE_IDLE_IN_TRANSACTION_TIMEOUT: &str = "5s";

/// Client-side budget for one table's complete maintenance attempt.
const MAINTENANCE_TABLE_DEADLINE: Duration = Duration::from_secs(10);

/// Namespace hashed with `current_schema()` into the maintenance advisory lock.
const MAINTENANCE_LOCK_NAMESPACE: &str = "buzz.partition_maintenance:";

/// Which automatic partition DDL the manager may issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PartitionMaintenancePolicy {
    /// Create months that no partition covers. Off stops all automatic DDL.
    pub create_enabled: bool,
    /// Replace an empty right-edge catch-all with dedicated monthlies.
    /// Takes effect only while `create_enabled` is on.
    pub advance_enabled: bool,
}

/// Result of one table's maintenance attempt, used as the `outcome` metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionMaintenanceOutcome {
    /// Every desired month already has a dedicated leaf.
    Noop,
    /// DDL is needed but the policy forbids it.
    SkippedDisabled,
    /// Uncovered months were created without touching a catch-all.
    Created,
    /// An empty catch-all was replaced by monthlies and a later catch-all.
    Advanced,
    /// Another session holds the maintenance advisory lock.
    SkippedLocked,
    /// Automatic DDL would be unsafe; an operator must repair the catalog.
    OperatorRequired,
    /// A table lock was not available within the lock budget.
    LockTimeout,
    /// The server statement budget or the client deadline expired.
    Deadline,
    /// Any other failure.
    Error,
}

impl PartitionMaintenanceOutcome {
    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::SkippedDisabled => "skipped_disabled",
            Self::Created => "created",
            Self::Advanced => "advanced",
            Self::SkippedLocked => "skipped_locked",
            Self::OperatorRequired => "operator_required",
            Self::LockTimeout => "lock_timeout",
            Self::Deadline => "deadline",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatchAllReplacement {
    schema: String,
    name: String,
    new_lower: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaintenanceChange {
    /// Ascending month starts whose canonical monthly is created.
    create: Vec<DateTime<Utc>>,
    /// The empty catch-all dropped and recreated after the horizon.
    catch_all: Option<CatchAllReplacement>,
}

impl MaintenanceChange {
    fn outcome(&self) -> PartitionMaintenanceOutcome {
        if self.catch_all.is_some() {
            PartitionMaintenanceOutcome::Advanced
        } else {
            PartitionMaintenanceOutcome::Created
        }
    }

    fn replaced(&self) -> Option<(String, String)> {
        self.catch_all
            .as_ref()
            .map(|catch_all| (catch_all.schema.clone(), catch_all.name.clone()))
    }

    fn target_names(&self, table: &str) -> Vec<String> {
        let mut names: Vec<_> = self
            .create
            .iter()
            .map(|month| partition_name(table, *month))
            .collect();
        if self.catch_all.is_some() {
            names.push(catch_all_name(table));
        }
        names
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MaintenancePlan {
    Noop,
    Disabled,
    Refuse(String),
    Apply(MaintenanceChange),
}

enum LockedAttempt {
    Committed(MaintenanceChange),
    SkippedLocked,
    Unchanged,
    Disabled,
    Refused(String),
}

struct TableMaintenance {
    outcome: PartitionMaintenanceOutcome,
    change: Option<MaintenanceChange>,
    error: Option<String>,
}

impl TableMaintenance {
    fn done(outcome: PartitionMaintenanceOutcome) -> Self {
        Self {
            outcome,
            change: None,
            error: None,
        }
    }

    fn failed(
        outcome: PartitionMaintenanceOutcome,
        change: Option<MaintenanceChange>,
        error: String,
    ) -> Self {
        Self {
            outcome,
            change,
            error: Some(error),
        }
    }
}

fn catch_all_name(table: &str) -> String {
    format!("{table}_p_future")
}

fn next_month(start: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let (year, month) = add_months(start.year(), start.month(), 1)?;
    month_start(year, month)
}

/// Decide what DDL, if any, would bring `audit` to the desired layout.
///
/// Pure so the same decision runs before locking, under locks, and in tests.
fn plan_maintenance(
    audit: &PartitionTableAudit,
    policy: PartitionMaintenancePolicy,
) -> MaintenancePlan {
    let uncovered: Vec<_> = audit
        .months
        .iter()
        .filter(|month| month.kind == MonthCoverageKind::Uncovered)
        .map(|month| month.start)
        .collect();
    let catch_all_covered = audit
        .months
        .iter()
        .any(|month| month.kind == MonthCoverageKind::CoveredByCatchAll);
    let create = policy.create_enabled && !uncovered.is_empty();
    // Advancement creates monthlies, so the create kill switch stops it too.
    let advance = policy.create_enabled && policy.advance_enabled && catch_all_covered;
    if !create && !advance {
        return if uncovered.is_empty() && !catch_all_covered {
            MaintenancePlan::Noop
        } else {
            MaintenancePlan::Disabled
        };
    }
    if !audit.structurally_safe_for_creation() {
        return MaintenancePlan::Refuse(
            "catalog is not structurally safe for automatic partition DDL".to_string(),
        );
    }

    let mut months = if create { uncovered } else { Vec::new() };
    let mut catch_all = None;
    if advance {
        let catch_alls: Vec<_> = audit
            .children
            .iter()
            .filter(|child| child.kind == PartitionChildKind::CatchAll)
            .collect();
        let [child] = catch_alls.as_slice() else {
            return MaintenancePlan::Refuse(format!(
                "expected exactly one right-edge catch-all, found {}",
                catch_alls.len()
            ));
        };
        match child.catch_all_nonempty {
            Some(false) => {}
            Some(true) => {
                return MaintenancePlan::Refuse(format!(
                    "catch-all {} contains rows; advancing it requires an operator",
                    child.name
                ))
            }
            None => {
                return MaintenancePlan::Refuse(format!(
                    "catch-all {} occupancy is unknown",
                    child.name
                ))
            }
        }
        // Replacements inherit only parent triggers; dropping the catch-all
        // would silently discard child-specific behavior.
        if !child.extra_triggers.is_empty() {
            return MaintenancePlan::Refuse(format!(
                "catch-all {} has child-only triggers ({}); replacing it would drop them",
                child.name,
                child.extra_triggers.join(", ")
            ));
        }
        let Some(PartitionBound::Finite(lower)) = child.lower else {
            return MaintenancePlan::Refuse(format!(
                "catch-all {} has no finite lower bound",
                child.name
            ));
        };
        if month_start(lower.year(), lower.month()).ok() != Some(lower) {
            return MaintenancePlan::Refuse(format!(
                "catch-all {} lower bound {lower} is not a UTC month start",
                child.name
            ));
        }
        let Some(new_lower) = audit
            .months
            .last()
            .and_then(|month| next_month(month.start).ok())
        else {
            return MaintenancePlan::Refuse("desired horizon is not representable".to_string());
        };
        let mut month = lower;
        while month < new_lower {
            if months.len() >= MAX_PARTITION_MONTHS_AHEAD as usize {
                return MaintenancePlan::Refuse(format!(
                    "catch-all {} would require more than {MAX_PARTITION_MONTHS_AHEAD} monthlies",
                    child.name
                ));
            }
            months.push(month);
            let Ok(next) = next_month(month) else {
                return MaintenancePlan::Refuse("desired horizon is not representable".to_string());
            };
            month = next;
        }
        catch_all = Some(CatchAllReplacement {
            schema: child.schema.clone(),
            name: child.name.clone(),
            new_lower,
        });
    }
    months.sort();
    months.dedup();
    MaintenancePlan::Apply(MaintenanceChange {
        create: months,
        catch_all,
    })
}

/// Audit, then issue only the DDL `policy` permits, one bounded transaction per table.
///
/// A table that is already at the desired layout, or whose required DDL is
/// disabled, takes no lock. One table's failure does not prevent the other
/// table's attempt; failures are aggregated into the returned error.
pub async fn maintain_partitions(
    pool: &PgPool,
    months_ahead: u32,
    policy: PartitionMaintenancePolicy,
) -> Result<PartitionAudit> {
    maintain_partitions_at(pool, months_ahead, policy, Utc::now()).await
}

pub(super) async fn maintain_partitions_at(
    pool: &PgPool,
    months_ahead: u32,
    policy: PartitionMaintenancePolicy,
    now: DateTime<Utc>,
) -> Result<PartitionAudit> {
    maintain_partitions_with_deadline(pool, months_ahead, policy, now, MAINTENANCE_TABLE_DEADLINE)
        .await
}

async fn maintain_partitions_with_deadline(
    pool: &PgPool,
    months_ahead: u32,
    policy: PartitionMaintenancePolicy,
    now: DateTime<Utc>,
    table_deadline: Duration,
) -> Result<PartitionAudit> {
    let audit = audit_partition_catalog_at(pool, months_ahead, now).await?;
    maintain_audited(pool, audit, months_ahead, policy, now, table_deadline).await
}

/// Maintain each table described by `audit`, which may be stale by now.
async fn maintain_audited(
    pool: &PgPool,
    audit: PartitionAudit,
    months_ahead: u32,
    policy: PartitionMaintenancePolicy,
    now: DateTime<Utc>,
    table_deadline: Duration,
) -> Result<PartitionAudit> {
    let horizon = validated_months_ahead(months_ahead)?;
    let mut errors = Vec::new();
    // Any table that planned DDL may have found the catalog changed under the
    // lock, so the opening audit no longer describes it.
    let mut stale = false;
    for table_audit in &audit.tables {
        let table = table_audit.table;
        let plan = plan_maintenance(table_audit, policy);
        stale |= matches!(plan, MaintenancePlan::Apply(_));
        let result = maintain_table(
            pool,
            table_audit,
            plan,
            horizon,
            policy,
            now,
            table_deadline,
        )
        .await;
        emit_maintenance_metrics(table_audit, &result);
        match (&result.error, result.outcome) {
            (Some(error), outcome) => errors.push(format!("{table} {}: {error}", outcome.as_str())),
            (
                None,
                PartitionMaintenanceOutcome::Created | PartitionMaintenanceOutcome::Advanced,
            ) => {
                info!(
                    table,
                    outcome = result.outcome.as_str(),
                    partitions = ?result
                        .change
                        .as_ref()
                        .map(|change| change.target_names(table)),
                    "partition maintenance committed"
                );
            }
            (None, _) => {}
        }
    }

    if !errors.is_empty() {
        Err(DbError::InvalidData(format!(
            "partition maintenance failed: {}",
            errors.join("; ")
        )))
    } else if stale {
        audit_partition_catalog_at(pool, months_ahead, now).await
    } else {
        Ok(audit)
    }
}

async fn maintain_table(
    pool: &PgPool,
    audit: &PartitionTableAudit,
    plan: MaintenancePlan,
    months_ahead: i32,
    policy: PartitionMaintenancePolicy,
    now: DateTime<Utc>,
    table_deadline: Duration,
) -> TableMaintenance {
    let change = match plan {
        MaintenancePlan::Noop => return TableMaintenance::done(PartitionMaintenanceOutcome::Noop),
        MaintenancePlan::Disabled => {
            return TableMaintenance::done(PartitionMaintenanceOutcome::SkippedDisabled)
        }
        MaintenancePlan::Refuse(reason) => {
            // Report a name collision ahead of the structural reason it
            // usually causes. A refusal takes no lock, so no race applies.
            let (names, replaced) = wanted_names(audit, policy);
            let error = match colliding_relation(pool, names, replaced).await {
                Ok(Some(name)) => collision_message(&name),
                Ok(None) => reason,
                Err(error) => {
                    return TableMaintenance::failed(
                        classify_error(&error),
                        None,
                        error.to_string(),
                    )
                }
            };
            return TableMaintenance::failed(
                PartitionMaintenanceOutcome::OperatorRequired,
                None,
                error,
            );
        }
        MaintenancePlan::Apply(change) => change,
    };

    let deadline = tokio::time::Instant::now() + table_deadline;
    let mut connection = match crate::observability::acquire_writer_until(
        pool,
        crate::observability::WriterOperation::Maintenance,
        deadline,
    )
    .await
    {
        Ok(connection) => connection,
        Err(error) => {
            let error = DbError::from(error);
            return TableMaintenance::failed(
                classify_error(&error),
                Some(change),
                error.to_string(),
            );
        }
    };
    let attempt = tokio::time::timeout_at(
        deadline,
        apply_change(&mut connection, audit.table, months_ahead, policy, now),
    )
    .await;
    let Ok(attempt) = attempt else {
        // The abandoned transaction may still be open on the server. Close the
        // session instead of returning it to the pool so PostgreSQL rolls back
        // whatever did not commit. The deadline can fire after COMMIT was sent,
        // so the change may have landed; the next audit reports the truth. The
        // backend keeps its locks until its in-flight statement ends, which
        // `statement_timeout` bounds.
        connection.close_on_drop();
        drop(connection);
        return TableMaintenance::failed(
            PartitionMaintenanceOutcome::Deadline,
            Some(change),
            format!(
                "exceeded the {}ms maintenance deadline; the attempt was abandoned and the \
                 server rolls back any uncommitted work",
                table_deadline.as_millis()
            ),
        );
    };
    match attempt {
        Ok(LockedAttempt::Committed(committed)) => TableMaintenance {
            outcome: committed.outcome(),
            change: Some(committed),
            error: None,
        },
        Ok(LockedAttempt::SkippedLocked) => {
            TableMaintenance::done(PartitionMaintenanceOutcome::SkippedLocked)
        }
        Ok(LockedAttempt::Unchanged) => TableMaintenance::done(PartitionMaintenanceOutcome::Noop),
        Ok(LockedAttempt::Disabled) => {
            TableMaintenance::done(PartitionMaintenanceOutcome::SkippedDisabled)
        }
        Ok(LockedAttempt::Refused(reason)) => TableMaintenance::failed(
            PartitionMaintenanceOutcome::OperatorRequired,
            Some(change),
            reason,
        ),
        Err(error) => {
            TableMaintenance::failed(classify_error(&error), Some(change), error.to_string())
        }
    }
}

async fn apply_change(
    connection: &mut PgConnection,
    table: &'static str,
    months_ahead: i32,
    policy: PartitionMaintenancePolicy,
    now: DateTime<Utc>,
) -> Result<LockedAttempt> {
    let mut transaction = connection.begin().await?;
    configure_maintenance_transaction(&mut transaction).await?;

    let acquired: bool = crate::observability::observe_advisory_lock(
        crate::observability::LockType::PartitionMaintenance,
        sqlx::query_scalar(
            "SELECT pg_catalog.pg_try_advisory_xact_lock(\
             pg_catalog.hashtextextended($1 || pg_catalog.current_schema()::text, 0))",
        )
        .bind(MAINTENANCE_LOCK_NAMESPACE)
        .fetch_one(&mut *transaction),
    )
    .await?;
    if !acquired {
        transaction.rollback().await?;
        return Ok(LockedAttempt::SkippedLocked);
    }

    // Re-plan while serialized against other managers but before any table
    // lock. A runner that lost a race sees the winner's committed layout here
    // and stops, instead of mistaking the winner's monthlies for collisions.
    let serialized = audit_table_on(&mut transaction, table, months_ahead, now).await?;
    let expected = match plan_maintenance(&serialized, policy) {
        MaintenancePlan::Apply(change) => change,
        MaintenancePlan::Noop => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Unchanged);
        }
        MaintenancePlan::Disabled => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Disabled);
        }
        MaintenancePlan::Refuse(reason) => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Refused(reason));
        }
    };
    // Reject misnamed relations before any lock can cost a parent outage.
    if let Some(name) = colliding_relation(
        &mut *transaction,
        expected.target_names(table),
        expected.replaced(),
    )
    .await?
    {
        transaction.rollback().await?;
        return Ok(LockedAttempt::Refused(collision_message(&name)));
    }

    // Refuse an unproven lock set before taking any lock.
    let parent = maintenance_parent(&mut transaction, table).await?;
    if let Some(reason) = parent.refusal {
        transaction.rollback().await?;
        return Ok(LockedAttempt::Refused(reason));
    }
    if let Some(catch_all) = &expected.catch_all {
        if let Some(reason) = catch_all_foreign_key_refusal(&mut transaction, catch_all).await? {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Refused(reason));
        }
    }
    // Parent first, in the order writers lock: an insert takes the parent and
    // then the foreign-key counterparts its constraint check reads. Waiting on
    // a counterpart while holding the parent could deadlock with such a writer.
    // ONLY: without it PostgreSQL locks every historical child in turn.
    lock(
        &mut transaction,
        &format!(
            "LOCK TABLE ONLY {} IN ACCESS EXCLUSIVE MODE",
            qualified_relation_name(&parent.schema, table)
        ),
    )
    .await?;
    // Read the foreign keys again under the parent lock. The read above can
    // miss one committed while this transaction waited on the parent, and
    // attaching would then wait on its table while holding the parent. Adding
    // or dropping a foreign key on or to the parent needs a lock that
    // conflicts with this one, so the set cannot change from here on.
    let parent = maintenance_parent(&mut transaction, table).await?;
    if let Some(reason) = parent.refusal {
        transaction.rollback().await?;
        return Ok(LockedAttempt::Refused(reason));
    }
    // Attaching a partition adds its foreign key under SHARE ROW EXCLUSIVE on
    // each referenced table. Taking that mode up front still admits readers and
    // the `FOR KEY SHARE` checks of other writers, and fails at once rather
    // than queue the counterpart's own writers behind this transaction.
    // Dropping the catch-all removes only its inherited foreign key and check
    // triggers, which lock the catch-all itself, never the referenced table
    // (lock probes on PostgreSQL 16 and 17), so no stronger mode is needed. A
    // catch-all with any other foreign key is refused before and after its lock.
    for counterpart in &parent.counterparts {
        lock(
            &mut transaction,
            &format!("LOCK TABLE ONLY {counterpart} IN SHARE ROW EXCLUSIVE MODE NOWAIT"),
        )
        .await?;
    }
    // Blocks direct child inserts that bypass the parent. Never wait on a
    // child while holding the parent.
    if let Some(catch_all) = &expected.catch_all {
        lock(
            &mut transaction,
            &format!(
                "LOCK TABLE ONLY {} IN ACCESS EXCLUSIVE MODE NOWAIT",
                qualified_relation_name(&catch_all.schema, &catch_all.name)
            ),
        )
        .await?;
        // Foreign-key DDL on or to the catch-all conflicts with this lock but
        // not with the parent's, so only this check is authoritative.
        if let Some(reason) = catch_all_foreign_key_refusal(&mut transaction, catch_all).await? {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Refused(reason));
        }
    }

    // Close the window between the serialized audit and the table locks.
    let locked = audit_table_on(&mut transaction, table, months_ahead, now).await?;
    match plan_maintenance(&locked, policy) {
        MaintenancePlan::Apply(change) if change == expected => {}
        MaintenancePlan::Noop => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Unchanged);
        }
        MaintenancePlan::Disabled => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Disabled);
        }
        MaintenancePlan::Refuse(reason) => {
            transaction.rollback().await?;
            return Ok(LockedAttempt::Refused(reason));
        }
        MaintenancePlan::Apply(_) => {
            transaction.rollback().await?;
            return Err(DbError::InvalidData(
                "partition catalog changed between the audit and the maintenance locks".to_string(),
            ));
        }
    }

    let parent_relation = qualified_relation_name(&parent.schema, table);
    if let Some(catch_all) = &expected.catch_all {
        execute_ddl(
            &mut transaction,
            format!(
                "DROP TABLE {}",
                qualified_relation_name(&catch_all.schema, &catch_all.name)
            ),
        )
        .await?;
    }
    for month in &expected.create {
        let end = next_month(*month)?;
        execute_ddl(
            &mut transaction,
            format!(
                "CREATE TABLE {} PARTITION OF {parent_relation} \
                 FOR VALUES FROM ('{}') TO ('{}')",
                qualified_relation_name(&parent.schema, &partition_name(table, *month)),
                month.format("%Y-%m-%d"),
                end.format("%Y-%m-%d"),
            ),
        )
        .await?;
    }
    if let Some(catch_all) = &expected.catch_all {
        execute_ddl(
            &mut transaction,
            format!(
                "CREATE TABLE {} PARTITION OF {parent_relation} \
                 FOR VALUES FROM ('{}') TO (MAXVALUE)",
                qualified_relation_name(&parent.schema, &catch_all_name(table)),
                catch_all.new_lower.format("%Y-%m-%d"),
            ),
        )
        .await?;
    }

    let after = audit_table_on(&mut transaction, table, months_ahead, now).await?;
    let verified = verify_applied(&locked, &after, &expected).and(
        verify_inheritance(&mut transaction, parent.oid, &expected.target_names(table)).await?,
    );
    if let Err(reason) = verified {
        transaction.rollback().await?;
        return Err(DbError::InvalidData(format!(
            "partition maintenance failed its postcondition and rolled back: {reason}"
        )));
    }
    transaction.commit().await?;
    Ok(LockedAttempt::Committed(expected))
}

async fn configure_maintenance_transaction(connection: &mut PgConnection) -> Result<()> {
    pin_catalog_rendering(connection).await?;
    for (setting, value) in [
        ("lock_timeout", MAINTENANCE_LOCK_TIMEOUT),
        ("statement_timeout", MAINTENANCE_STATEMENT_TIMEOUT),
        (
            "idle_in_transaction_session_timeout",
            MAINTENANCE_IDLE_IN_TRANSACTION_TIMEOUT,
        ),
        ("application_name", "buzz_partition_maintenance"),
    ] {
        sqlx::query("SELECT pg_catalog.set_config($1, $2, true)")
            .bind(setting)
            .bind(value)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

async fn lock(connection: &mut PgConnection, sql: &str) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn execute_ddl(connection: &mut PgConnection, sql: String) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(&mut *connection)
        .await?;
    Ok(())
}

struct MaintenanceParent {
    oid: i64,
    schema: String,
    /// Quoted, schema-qualified referenced tables in ascending OID order.
    counterparts: Vec<String>,
    refusal: Option<String>,
}

async fn maintenance_parent(
    connection: &mut PgConnection,
    table: &str,
) -> Result<MaintenanceParent> {
    let row = sqlx::query(
        r#"
        SELECT parent.oid::bigint AS parent_oid,
               parent_ns.nspname AS parent_schema,
               ARRAY(
                   SELECT pg_catalog.format('%I.%I', referenced_ns.nspname, referenced.relname)
                   FROM pg_catalog.pg_class referenced
                   JOIN pg_catalog.pg_namespace referenced_ns
                     ON referenced_ns.oid = referenced.relnamespace
                   WHERE referenced.oid IN (
                       SELECT foreign_key.confrelid
                       FROM pg_catalog.pg_constraint foreign_key
                       WHERE foreign_key.conrelid = parent.oid
                         AND foreign_key.contype = 'f'
                         AND foreign_key.confrelid <> parent.oid
                   )
                   ORDER BY referenced.oid
               )::text[] AS counterparts,
               EXISTS (
                   SELECT 1
                   FROM pg_catalog.pg_constraint foreign_key
                   JOIN pg_catalog.pg_class referenced ON referenced.oid = foreign_key.confrelid
                   WHERE foreign_key.conrelid = parent.oid
                     AND foreign_key.contype = 'f'
                     AND (referenced.relkind <> 'r' OR referenced.relhassubclass)
               ) AS counterpart_has_children,
               EXISTS (
                   SELECT 1
                   FROM pg_catalog.pg_constraint foreign_key
                   WHERE foreign_key.confrelid = parent.oid
                     AND foreign_key.contype = 'f'
               ) AS referenced_by_foreign_key
        FROM pg_catalog.pg_class parent
        JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
        WHERE parent_ns.nspname = pg_catalog.current_schema()
          AND parent.relname = $1
        "#,
    )
    .bind(table)
    .fetch_one(&mut *connection)
    .await?;
    let refusal = if row.try_get::<bool, _>("counterpart_has_children")? {
        Some(format!(
            "{table} references a partitioned or inherited table; the counterpart lock set is not proven"
        ))
    } else if row.try_get::<bool, _>("referenced_by_foreign_key")? {
        Some(format!(
            "{table} is referenced by a foreign key; the counterpart lock set is not proven"
        ))
    } else {
        None
    };
    Ok(MaintenanceParent {
        oid: row.try_get("parent_oid")?,
        schema: row.try_get("parent_schema")?,
        counterparts: row.try_get("counterparts")?,
        refusal,
    })
}

/// Names the policy would create, and the catch-all it would replace, even
/// when the plan is refused. Used to report collisions ahead of other refusals.
fn wanted_names(
    audit: &PartitionTableAudit,
    policy: PartitionMaintenancePolicy,
) -> (Vec<String>, Option<(String, String)>) {
    let mut names: Vec<_> = audit
        .months
        .iter()
        .filter(|month| {
            (policy.create_enabled && month.kind == MonthCoverageKind::Uncovered)
                || (policy.advance_enabled && month.kind == MonthCoverageKind::CoveredByCatchAll)
        })
        .map(|month| partition_name(audit.table, month.start))
        .collect();
    let catch_alls: Vec<_> = audit
        .children
        .iter()
        .filter(|child| child.kind == PartitionChildKind::CatchAll)
        .collect();
    let advancing = policy.advance_enabled
        && audit
            .months
            .iter()
            .any(|month| month.kind == MonthCoverageKind::CoveredByCatchAll);
    if advancing {
        names.push(catch_all_name(audit.table));
    }
    let replaced = match catch_alls.as_slice() {
        [child] if advancing => Some((child.schema.clone(), child.name.clone())),
        _ => None,
    };
    (names, replaced)
}

/// Return a target name already used by a relation other than the catch-all being replaced.
async fn colliding_relation<'e, E>(
    executor: E,
    names: Vec<String>,
    replaced: Option<(String, String)>,
) -> Result<Option<String>>
where
    E: sqlx::PgExecutor<'e>,
{
    let (replaced_schema, replaced_name) = replaced.unwrap_or_default();
    Ok(sqlx::query_scalar(
        r#"
        SELECT relation.relname::text
        FROM pg_catalog.pg_class relation
        JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
        WHERE namespace.nspname = pg_catalog.current_schema()
          AND relation.relname = ANY($1)
          AND NOT (namespace.nspname = $2 AND relation.relname = $3)
        ORDER BY relation.relname
        LIMIT 1
        "#,
    )
    .bind(names)
    .bind(replaced_schema)
    .bind(replaced_name)
    .fetch_optional(executor)
    .await?)
}

fn collision_message(name: &str) -> String {
    format!("canonical name {name} already exists without the expected attachment and bounds")
}

/// Refuse a catch-all with a foreign key of its own, or one that references it.
///
/// Such a key is invisible to the parent's counterpart set. Dropping the
/// catch-all would silently drop it and lock its other table without `NOWAIT`
/// while holding the parent. The referenced arm also matches the catch-all's
/// clone of a key referencing the parent; the parent refusal runs first, so
/// that arm is a backstop.
async fn catch_all_foreign_key_refusal(
    connection: &mut PgConnection,
    catch_all: &CatchAllReplacement,
) -> Result<Option<String>> {
    let names: Option<String> = sqlx::query_scalar(
        r#"
        SELECT pg_catalog.string_agg(foreign_key.conname::text, ', ' ORDER BY foreign_key.conname)
        FROM pg_catalog.pg_constraint foreign_key
        WHERE foreign_key.contype = 'f'
          AND ((foreign_key.conrelid = $1::regclass AND foreign_key.conparentid = 0)
               OR foreign_key.confrelid = $1::regclass)
        "#,
    )
    .bind(qualified_relation_name(&catch_all.schema, &catch_all.name))
    .fetch_one(&mut *connection)
    .await?;
    Ok(names.map(|names| {
        format!(
            "catch-all {} has foreign keys outside the parent's counterpart set ({names}); \
             the counterpart lock set is not proven",
            catch_all.name
        )
    }))
}

/// Verify the changed catalog, audited inside the transaction, before commit.
fn verify_applied(
    before: &PartitionTableAudit,
    after: &PartitionTableAudit,
    change: &MaintenanceChange,
) -> std::result::Result<(), String> {
    let table = after.table;
    if !after.structurally_safe_for_creation() {
        return Err("catalog is not structurally safe after DDL".to_string());
    }
    if before.serving_safe && !after.serving_safe {
        return Err("current month lost its covering partition".to_string());
    }
    let parity = |name: &str| {
        after
            .children
            .iter()
            .find(|child| child.name == name)
            .filter(|child| child.missing_triggers.is_empty() && child.extra_triggers.is_empty())
    };
    for month in &change.create {
        let name = partition_name(table, *month);
        let end = next_month(*month).map_err(|error| error.to_string())?;
        if !parity(&name).is_some_and(|child| {
            child.kind == PartitionChildKind::CanonicalMonthly
                && child.lower == Some(PartitionBound::Finite(*month))
                && child.upper == Some(PartitionBound::Finite(end))
        }) {
            return Err(format!(
                "{name} is missing, misbounded, or lacks trigger parity"
            ));
        }
    }
    if let Some(catch_all) = &change.catch_all {
        let name = catch_all_name(table);
        if !parity(&name).is_some_and(|child| {
            child.kind == PartitionChildKind::CatchAll
                && child.lower == Some(PartitionBound::Finite(catch_all.new_lower))
                && child.catch_all_nonempty == Some(false)
        }) {
            return Err(format!(
                "{name} is missing, misbounded, or lacks trigger parity"
            ));
        }
        if after
            .children
            .iter()
            .filter(|child| child.kind == PartitionChildKind::CatchAll)
            .count()
            != 1
        {
            return Err("expected exactly one catch-all after DDL".to_string());
        }
    }
    if let Some(month) = after.months.iter().find(|month| {
        change.create.contains(&month.start) && month.kind != MonthCoverageKind::CoveredByMonthly
    }) {
        return Err(format!(
            "{} is not covered by its dedicated monthly",
            month.start.format("%Y-%m")
        ));
    }
    Ok(())
}

/// Verify every new child carries a valid, ready clone of each parent index and
/// a validated clone of each parent primary-key, unique, and foreign-key constraint.
async fn verify_inheritance(
    connection: &mut PgConnection,
    parent_oid: i64,
    names: &[String],
) -> Result<std::result::Result<(), String>> {
    let rows = sqlx::query(
        r#"
        SELECT child.relname::text AS name,
               (
                   SELECT count(*)
                   FROM pg_catalog.pg_index parent_index
                   WHERE parent_index.indrelid = $1::bigint::oid
                     AND NOT EXISTS (
                         SELECT 1
                         FROM pg_catalog.pg_inherits index_edge
                         JOIN pg_catalog.pg_index child_index
                           ON child_index.indexrelid = index_edge.inhrelid
                         WHERE index_edge.inhparent = parent_index.indexrelid
                           AND child_index.indrelid = child.oid
                           AND child_index.indisvalid
                           AND child_index.indisready
                     )
               ) AS missing_indexes,
               (
                   SELECT count(*)
                   FROM pg_catalog.pg_constraint parent_constraint
                   WHERE parent_constraint.conrelid = $1::bigint::oid
                     AND parent_constraint.contype IN ('f', 'p', 'u')
                     AND NOT EXISTS (
                         SELECT 1
                         FROM pg_catalog.pg_constraint child_constraint
                         WHERE child_constraint.conrelid = child.oid
                           AND child_constraint.conparentid = parent_constraint.oid
                           AND child_constraint.convalidated
                     )
               ) AS missing_constraints
        FROM pg_catalog.pg_class child
        JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid = child.relnamespace
        JOIN pg_catalog.pg_inherits edge
          ON edge.inhrelid = child.oid AND edge.inhparent = $1::bigint::oid
        WHERE child_ns.nspname = pg_catalog.current_schema()
          AND child.relname = ANY($2)
        "#,
    )
    .bind(parent_oid)
    .bind(names)
    .fetch_all(&mut *connection)
    .await?;
    if rows.len() != names.len() {
        return Ok(Err(format!(
            "expected {} new children attached to the parent, found {}",
            names.len(),
            rows.len()
        )));
    }
    for row in rows {
        let name: String = row.try_get("name")?;
        let missing_indexes: i64 = row.try_get("missing_indexes")?;
        let missing_constraints: i64 = row.try_get("missing_constraints")?;
        if missing_indexes != 0 || missing_constraints != 0 {
            return Ok(Err(format!(
                "{name} is missing {missing_indexes} inherited indexes and \
                 {missing_constraints} inherited constraints"
            )));
        }
    }
    Ok(Ok(()))
}

fn classify_error(error: &DbError) -> PartitionMaintenanceOutcome {
    match error {
        DbError::Sqlx(sqlx::Error::Database(database)) => match database.code().as_deref() {
            // A deadlock victim lost a lock race; the next pass retries.
            Some("55P03" | "40P01") => PartitionMaintenanceOutcome::LockTimeout,
            Some("57014" | "25P03") => PartitionMaintenanceOutcome::Deadline,
            _ => PartitionMaintenanceOutcome::Error,
        },
        DbError::Sqlx(sqlx::Error::PoolTimedOut) => PartitionMaintenanceOutcome::Deadline,
        _ => PartitionMaintenanceOutcome::Error,
    }
}

fn emit_maintenance_metrics(audit: &PartitionTableAudit, result: &TableMaintenance) {
    let table = audit.table;
    metrics::counter!(
        "buzz_partition_maintenance_runs_total",
        "table" => table,
        "outcome" => result.outcome.as_str()
    )
    .increment(1);
    // Per-month creation accounting, continuous with the pre-advancement manager.
    let changed = result
        .change
        .as_ref()
        .map(|change| change.create.as_slice())
        .unwrap_or_default();
    // `error` means DDL or collision failure. Lock and deadline outcomes are
    // transient and only `buzz_partition_maintenance_runs_total` records them.
    let month_outcome = match result.outcome {
        PartitionMaintenanceOutcome::Error | PartitionMaintenanceOutcome::OperatorRequired => {
            Some("error")
        }
        PartitionMaintenanceOutcome::LockTimeout | PartitionMaintenanceOutcome::Deadline => None,
        _ => Some("created"),
    };
    for month in &audit.months {
        let outcome = if changed.contains(&month.start) {
            let Some(outcome) = month_outcome else {
                continue;
            };
            outcome
        } else if month.kind == MonthCoverageKind::Uncovered {
            continue;
        } else {
            "skipped_covered"
        };
        metrics::counter!(
            "buzz_partition_create_attempts_total",
            "table" => table,
            "outcome" => outcome
        )
        .increment(1);
    }
}

#[cfg(test)]
#[path = "maintenance_tests.rs"]
mod tests;
