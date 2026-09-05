//! Read-only catalog audit and monthly partition manager for `events` and
//! `delivery_log`.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc};
use serde::Serialize;
use sqlx::{Connection, PgConnection, PgPool, Row};
use tracing::info;

use crate::error::{DbError, Result};
use crate::Db;

/// Tables that may be partition-managed. The allowlist prevents DDL injection.
const PARTITIONED_TABLES: &[&str] = &["events", "delivery_log"];

/// Maximum future-month horizon accepted by catalog audits and creation.
///
/// The relay uses three months. Keeping the public API bounded prevents an
/// operator-supplied `buzz-admin partition-audit --months-ahead` value from
/// allocating or iterating an effectively unbounded report.
pub const MAX_PARTITION_MONTHS_AHEAD: u32 = 120;

fn expected_partition_key(table: &str) -> Option<&'static str> {
    match table {
        "events" => Some("RANGE (created_at)"),
        "delivery_log" => Some("RANGE (delivered_at)"),
        _ => None,
    }
}

/// A parsed endpoint from a PostgreSQL range-partition bound.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionBound {
    /// The range has no lower limit.
    MinValue,
    /// A finite UTC timestamp.
    Finite(DateTime<Utc>),
    /// The range has no upper limit.
    MaxValue,
}

/// The catalog classification of one attached child partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionChildKind {
    /// The canonical `{parent}_pYYYY_MM` name agrees with exact month bounds.
    CanonicalMonthly,
    /// A finite lower bound extends through `MAXVALUE`.
    CatchAll,
    /// A well-formed monthly leaf uses a non-canonical name.
    LegacyLeaf,
    /// The canonical `{parent}_p_past` left-edge partition.
    Past,
    /// Bounds were unparseable, invalid, overlapping, or disagreed with the name.
    Anomalous,
}

/// How one target month is covered by the current partition catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonthCoverageKind {
    /// A bounded child covers the complete month.
    CoveredByMonthly,
    /// A right-edge catch-all covers the complete month.
    CoveredByCatchAll,
    /// No parseable child covers the complete month.
    Uncovered,
}

/// Coverage for one target month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonthCoverage {
    /// Inclusive month start.
    pub start: DateTime<Utc>,
    /// Catalog coverage classification.
    pub kind: MonthCoverageKind,
}

/// Audit details for one attached child partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionChildAudit {
    /// Child relation name.
    pub name: String,
    /// PostgreSQL `pg_class.relkind` for the immediate child.
    pub relation_kind: String,
    /// Parsed inclusive lower range endpoint, when parsing succeeded.
    pub lower: Option<PartitionBound>,
    /// Parsed exclusive upper range endpoint, when parsing succeeded.
    pub upper: Option<PartitionBound>,
    /// Catalog classification.
    pub kind: PartitionChildKind,
    /// Whether this child or one of its descendant edges is pending detach.
    pub pending_detach: bool,
    /// Whether a catch-all leaf currently contains rows. `None` for non-catch-alls.
    pub catch_all_nonempty: Option<bool>,
    /// Parent trigger names absent from this child or a routable descendant leaf.
    /// Nested-leaf entries are qualified as `{leaf}:{trigger}`.
    pub missing_triggers: Vec<String>,
    /// Child-only row trigger names absent from the parent.
    /// Nested-leaf entries are qualified as `{leaf}:{trigger}`.
    pub extra_triggers: Vec<String>,
}

/// Effective routable range for one leaf in the partition tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionLeafAudit {
    /// Leaf relation name.
    pub name: String,
    /// Immediate child of the managed parent that owns this leaf.
    pub root_child: String,
    /// Effective inclusive lower bound after intersecting the ancestor path.
    pub lower: PartitionBound,
    /// Effective exclusive upper bound after intersecting the ancestor path.
    pub upper: PartitionBound,
    /// Whether this leaf is below an immediate partitioned child.
    pub nested: bool,
}

/// Read-only audit result for one managed parent table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionTableAudit {
    /// Parent relation name.
    pub table: &'static str,
    /// Partition key rendered by PostgreSQL for the managed parent.
    pub partition_key: Option<String>,
    /// Partition key required by the manager for this table.
    pub expected_partition_key: &'static str,
    /// Whether the managed parent's key is exactly the expected key.
    pub partition_key_valid: bool,
    /// All attached children found via `pg_inherits`.
    pub children: Vec<PartitionChildAudit>,
    /// Cached effective bounds for every routable leaf in the catalog tree.
    pub coverage_leaves: Vec<PartitionLeafAudit>,
    /// Coverage of the current month and the requested future months.
    pub months: Vec<MonthCoverage>,
    /// Whether a parseable routable leaf covers the audit timestamp.
    pub serving_safe: bool,
}

impl PartitionTableAudit {
    /// Number of children with anomalous catalog state.
    pub fn anomalous_children(&self) -> usize {
        self.children
            .iter()
            .filter(|child| child.kind == PartitionChildKind::Anomalous)
            .count()
    }

    /// Number of parent triggers missing across all children.
    pub fn missing_trigger_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| child.missing_triggers.len())
            .sum()
    }

    /// Number of child-only row triggers across all children.
    pub fn extra_trigger_count(&self) -> usize {
        self.children
            .iter()
            .map(|child| child.extra_triggers.len())
            .sum()
    }

    /// Number of catch-all children that currently contain at least one row.
    pub fn nonempty_catch_all_count(&self) -> usize {
        self.children
            .iter()
            .filter(|child| child.catch_all_nonempty == Some(true))
            .count()
    }

    /// Whether the table is serving but has state requiring operator attention.
    pub fn degraded(&self) -> bool {
        !self.partition_key_valid
            || self.children.iter().any(|child| {
                matches!(
                    child.kind,
                    PartitionChildKind::LegacyLeaf | PartitionChildKind::Anomalous
                ) || !child.missing_triggers.is_empty()
                    || !child.extra_triggers.is_empty()
                    || child.catch_all_nonempty == Some(true)
            })
            || self
                .months
                .iter()
                .any(|month| month.kind != MonthCoverageKind::CoveredByMonthly)
    }
}

/// Read-only audit result for every managed parent table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionAudit {
    /// Timestamp whose serving coverage was checked.
    pub audited_at: DateTime<Utc>,
    /// Per-parent audit details.
    pub tables: Vec<PartitionTableAudit>,
}

/// A per-table catalog audit failure retained for operator-facing reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionAuditError {
    /// Managed table whose audit failed.
    pub table: &'static str,
    /// Stable human-readable failure detail.
    pub error: String,
}

/// Best-effort read-only report that preserves successful table audits and failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionAuditReport {
    /// Timestamp whose serving coverage was checked.
    pub audited_at: DateTime<Utc>,
    /// Successful per-parent audit details.
    pub tables: Vec<PartitionTableAudit>,
    /// Per-parent failures that prevented a complete audit.
    pub errors: Vec<PartitionAuditError>,
}

impl PartitionAuditReport {
    /// Whether the report is complete and every managed parent is serving safely.
    pub fn serving_safe(&self) -> bool {
        self.errors.is_empty()
            && self.tables.len() == PARTITIONED_TABLES.len()
            && self.tables.iter().all(|table| table.serving_safe)
    }

    fn into_complete_audit(self) -> Result<PartitionAudit> {
        if self.errors.is_empty() {
            Ok(PartitionAudit {
                audited_at: self.audited_at,
                tables: self.tables,
            })
        } else {
            Err(DbError::InvalidData(format!(
                "partition catalog audit failed: {}",
                self.errors
                    .iter()
                    .map(|error| format!("{}: {}", error.table, error.error))
                    .collect::<Vec<_>>()
                    .join("; ")
            )))
        }
    }
}

impl PartitionAudit {
    /// Whether every managed parent can accept a row timestamped at `audited_at`.
    pub fn serving_safe(&self) -> bool {
        self.serving_safe_at(self.audited_at)
    }

    /// Whether the cached catalog proves every managed parent covers `timestamp`.
    pub fn serving_safe_at(&self, timestamp: DateTime<Utc>) -> bool {
        self.tables.iter().all(|table| {
            table
                .coverage_leaves
                .iter()
                .any(|leaf| leaf_covers_timestamp(leaf, &timestamp))
        })
    }
}

#[derive(Debug)]
struct CatalogChild {
    name: String,
    relation_kind: String,
    lower: Option<PartitionBound>,
    upper: Option<PartitionBound>,
    kind: PartitionChildKind,
    pending_detach: bool,
}

/// Audit the managed partition catalogs without making any writes.
pub async fn audit_partition_catalog(pool: &PgPool, months_ahead: u32) -> Result<PartitionAudit> {
    audit_partition_catalog_at(pool, months_ahead, Utc::now()).await
}

/// Audit every managed catalog while retaining per-table failures for operators.
pub async fn audit_partition_catalog_report(
    pool: &PgPool,
    months_ahead: u32,
) -> PartitionAuditReport {
    audit_partition_catalog_report_at(pool, months_ahead, Utc::now()).await
}

async fn audit_partition_catalog_at(
    pool: &PgPool,
    months_ahead: u32,
    now: DateTime<Utc>,
) -> Result<PartitionAudit> {
    audit_partition_catalog_report_at(pool, months_ahead, now)
        .await
        .into_complete_audit()
}

async fn audit_partition_catalog_report_at(
    pool: &PgPool,
    months_ahead: u32,
    now: DateTime<Utc>,
) -> PartitionAuditReport {
    let mut tables = Vec::with_capacity(PARTITIONED_TABLES.len());
    let mut errors = Vec::new();

    for &table in PARTITIONED_TABLES {
        let started = Instant::now();
        match audit_table(pool, table, months_ahead, now).await {
            Ok(audit) => {
                emit_audit_metrics(&audit, started.elapsed().as_secs_f64(), now);
                tables.push(audit);
            }
            Err(error) => {
                metrics::counter!(
                    "buzz_partition_audit_runs_total",
                    "table" => table,
                    "outcome" => "error"
                )
                .increment(1);
                metrics::histogram!(
                    "buzz_partition_audit_duration_seconds",
                    "table" => table
                )
                .record(started.elapsed().as_secs_f64());
                errors.push(PartitionAuditError {
                    table,
                    error: error.to_string(),
                });
            }
        }
    }

    PartitionAuditReport {
        audited_at: now,
        tables,
        errors,
    }
}

/// Audit first, then create only months proven to be uncovered.
///
/// Covered ranges are never probed with DDL. Creation failures are collected
/// across all managed parents and months before an aggregate error is returned.
pub async fn ensure_future_partitions(
    pool: &PgPool,
    months_ahead: u32,
    create_enabled: bool,
) -> Result<PartitionAudit> {
    ensure_future_partitions_at(pool, months_ahead, create_enabled, Utc::now()).await
}

async fn ensure_future_partitions_at(
    pool: &PgPool,
    months_ahead: u32,
    create_enabled: bool,
    now: DateTime<Utc>,
) -> Result<PartitionAudit> {
    let audit = audit_partition_catalog_at(pool, months_ahead, now).await?;
    let mut errors = Vec::new();
    let mut created_any = false;

    for table in &audit.tables {
        for month in &table.months {
            match month.kind {
                MonthCoverageKind::CoveredByMonthly | MonthCoverageKind::CoveredByCatchAll => {
                    metrics::counter!(
                        "buzz_partition_create_attempts_total",
                        "table" => table.table,
                        "outcome" => "skipped_covered"
                    )
                    .increment(1);
                }
                MonthCoverageKind::Uncovered if !create_enabled => {}
                MonthCoverageKind::Uncovered => {
                    let expected_name = partition_name(table.table, month.start);
                    let collision = match relation_name_exists(pool, &expected_name).await {
                        Ok(collision) => collision,
                        Err(error) => {
                            metrics::counter!(
                                "buzz_partition_create_attempts_total",
                                "table" => table.table,
                                "outcome" => "error"
                            )
                            .increment(1);
                            errors.push(format!(
                                "{} {}: failed to check canonical name {expected_name}: {error}",
                                table.table,
                                month.start.format("%Y-%m")
                            ));
                            continue;
                        }
                    };
                    if collision {
                        metrics::counter!(
                            "buzz_partition_create_attempts_total",
                            "table" => table.table,
                            "outcome" => "error"
                        )
                        .increment(1);
                        errors.push(format!(
                            "{} {}: canonical name {expected_name} already exists without the expected attachment and bounds",
                            table.table,
                            month.start.format("%Y-%m")
                        ));
                        continue;
                    }
                    match create_month_partition(pool, table.table, month.start).await {
                        Ok(name) => {
                            created_any = true;
                            metrics::counter!(
                                "buzz_partition_create_attempts_total",
                                "table" => table.table,
                                "outcome" => "created"
                            )
                            .increment(1);
                            info!(table = table.table, partition = name, "added partition");
                        }
                        Err(error) => {
                            metrics::counter!(
                                "buzz_partition_create_attempts_total",
                                "table" => table.table,
                                "outcome" => "error"
                            )
                            .increment(1);
                            errors.push(format!(
                                "{} {}: {error}",
                                table.table,
                                month.start.format("%Y-%m")
                            ));
                        }
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        if created_any {
            audit_partition_catalog_at(pool, months_ahead, now).await
        } else {
            Ok(audit)
        }
    } else {
        Err(DbError::InvalidData(format!(
            "partition creation failed: {}",
            errors.join("; ")
        )))
    }
}

async fn audit_table(
    pool: &PgPool,
    table: &'static str,
    months_ahead: u32,
    now: DateTime<Utc>,
) -> Result<PartitionTableAudit> {
    let months_ahead = validated_months_ahead(months_ahead)?;
    let mut transaction = pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await?;
    pin_catalog_rendering(&mut transaction).await?;
    let audit = audit_table_on(&mut transaction, table, months_ahead, now).await?;
    transaction.commit().await?;
    Ok(audit)
}

async fn pin_catalog_rendering(connection: &mut PgConnection) -> Result<()> {
    sqlx::query("SET LOCAL DateStyle TO 'ISO, YMD'")
        .execute(&mut *connection)
        .await?;
    sqlx::query("SET LOCAL TimeZone TO 'UTC'")
        .execute(&mut *connection)
        .await?;
    Ok(())
}

impl Db {
    /// Audits the managed partition catalogs without making writes.
    #[datastore_span(name = "audit_partitions", system = "postgresql")]
    pub async fn audit_partitions(&self, months_ahead: u32) -> Result<PartitionAudit> {
        audit_partition_catalog(&self.pool, months_ahead).await
    }

    /// Audits every managed catalog and retains per-table failures.
    #[datastore_span(name = "audit_partitions_report", system = "postgresql")]
    pub async fn audit_partitions_report(&self, months_ahead: u32) -> PartitionAuditReport {
        audit_partition_catalog_report(&self.pool, months_ahead).await
    }

    /// Ensures monthly partitions exist for the next N months when creation is enabled.
    #[datastore_span(name = "ensure_future_partitions", system = "postgresql")]
    pub async fn ensure_future_partitions(
        &self,
        months_ahead: u32,
        create_enabled: bool,
    ) -> Result<PartitionAudit> {
        ensure_future_partitions(&self.pool, months_ahead, create_enabled).await
    }
}

async fn audit_table_on(
    connection: &mut PgConnection,
    table: &'static str,
    months_ahead: i32,
    now: DateTime<Utc>,
) -> Result<PartitionTableAudit> {
    let expected_partition_key = expected_partition_key(table).ok_or_else(|| {
        DbError::InvalidData(format!("table not in partition allowlist: {table:?}"))
    })?;
    let partition_key: Option<String> = sqlx::query_scalar(
        r#"
        SELECT pg_catalog.pg_get_partkeydef(parent.oid)
        FROM pg_catalog.pg_class parent
        JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
        WHERE parent_ns.nspname = current_schema()
          AND parent.relname = $1
        "#,
    )
    .bind(table)
    .fetch_optional(&mut *connection)
    .await?
    .flatten();
    let partition_key_valid = partition_key.as_deref() == Some(expected_partition_key);

    let rows = sqlx::query(
        r#"
        WITH RECURSIVE partition_tree AS (
            SELECT child.oid AS relation_oid,
                   parent.oid AS parent_oid,
                   child.relname,
                   child.relkind,
                   child.relpartbound,
                   child.relname AS root_child,
                   pg_catalog.pg_get_partkeydef(parent.oid) AS bound_partition_key,
                   inherited.inhdetachpending AS detach_pending,
                   0 AS depth
            FROM pg_catalog.pg_inherits inherited
            JOIN pg_catalog.pg_class parent ON parent.oid = inherited.inhparent
            JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
            JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
            WHERE parent_ns.nspname = current_schema()
              AND parent.relname = $1

            UNION ALL

            SELECT descendant.oid,
                   tree.relation_oid,
                   descendant.relname,
                   descendant.relkind,
                   descendant.relpartbound,
                   tree.root_child,
                   pg_catalog.pg_get_partkeydef(tree.relation_oid),
                   nested.inhdetachpending,
                   tree.depth + 1
            FROM partition_tree tree
            JOIN pg_catalog.pg_inherits nested
              ON nested.inhparent = tree.relation_oid
            JOIN pg_catalog.pg_class descendant ON descendant.oid = nested.inhrelid
        )
        SELECT tree.relation_oid::bigint AS relation_oid,
               tree.parent_oid::bigint AS parent_oid,
               tree.relname AS relation_name,
               tree.relkind::text AS relation_kind,
               tree.root_child,
               tree.depth,
               tree.bound_partition_key,
               tree.detach_pending,
               pg_catalog.pg_get_expr(tree.relpartbound, tree.relation_oid) AS bound,
               NOT EXISTS (
                   SELECT 1
                   FROM pg_catalog.pg_inherits child_edge
                   WHERE child_edge.inhparent = tree.relation_oid
               ) AS is_leaf
        FROM partition_tree tree
        ORDER BY tree.depth, tree.relname
        "#,
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await?;

    let mut children = Vec::new();
    let mut coverage_leaves = Vec::new();
    let mut effective_ranges = HashMap::<i64, Option<(PartitionBound, PartitionBound)>>::new();
    let mut pending_roots = HashSet::new();
    for row in rows {
        let relation_oid: i64 = row.try_get("relation_oid")?;
        let parent_oid: i64 = row.try_get("parent_oid")?;
        let name: String = row.try_get("relation_name")?;
        let relation_kind: String = row.try_get("relation_kind")?;
        let root_child: String = row.try_get("root_child")?;
        let depth: i32 = row.try_get("depth")?;
        let is_leaf: bool = row.try_get("is_leaf")?;
        let bound_partition_key: Option<String> = row.try_get("bound_partition_key")?;
        let detach_pending: bool = row.try_get("detach_pending")?;
        if detach_pending {
            pending_roots.insert(root_child.clone());
        }
        let partition_key_compatible = partition_key_valid
            && bound_partition_key.as_deref() == Some(expected_partition_key)
            && !detach_pending;
        let expression: String = row.try_get("bound")?;
        let own_range = parse_range_bounds(&expression);
        let effective_range = if !partition_key_compatible {
            None
        } else if depth == 0 {
            own_range.clone()
        } else {
            effective_ranges
                .get(&parent_oid)
                .and_then(|parent| parent.as_ref())
                .and_then(|parent| {
                    own_range
                        .as_ref()
                        .and_then(|own| intersect_ranges(parent, own))
                })
        };
        effective_ranges.insert(relation_oid, effective_range.clone());

        if depth == 0 {
            let (lower, upper) = match own_range {
                Some(bounds) => (Some(bounds.0), Some(bounds.1)),
                None => (None, None),
            };
            let kind = if partition_key_compatible {
                classify_child(&relation_kind, table, &name, lower.as_ref(), upper.as_ref())
            } else {
                PartitionChildKind::Anomalous
            };
            children.push(CatalogChild {
                name: name.clone(),
                relation_kind: relation_kind.clone(),
                lower,
                upper,
                kind,
                pending_detach: detach_pending,
            });
        }

        let routable_leaf =
            (depth == 0 && relation_kind == "r") || (depth > 0 && is_leaf && relation_kind != "p");
        if routable_leaf {
            if let Some((lower, upper)) = effective_range {
                coverage_leaves.push(PartitionLeafAudit {
                    name,
                    root_child,
                    lower,
                    upper,
                    nested: depth > 0,
                });
            }
        }
    }
    for child in &mut children {
        if pending_roots.contains(&child.name) {
            child.pending_detach = true;
            child.kind = PartitionChildKind::Anomalous;
        }
    }
    mark_overlaps_anomalous(&mut children);

    let parent_triggers = trigger_metadata_for_parent(connection, table).await?;
    let descendant_triggers = trigger_metadata_for_descendants(connection, table).await?;
    let mut child_audits = Vec::with_capacity(children.len());
    for child in children {
        let mut missing_triggers = Vec::new();
        let routable_leaves: Vec<_> = descendant_triggers
            .values()
            .filter(|relation| {
                relation.root_child == child.name
                    && ((relation.depth == 0 && relation.relation_kind == "r")
                        || (relation.depth > 0
                            && relation.is_leaf
                            && relation.relation_kind != "p"))
            })
            .collect();
        for leaf in &routable_leaves {
            for (name, parent_oid) in &parent_triggers {
                let present =
                    trigger_lineage_reaches_parent(leaf, name, *parent_oid, &descendant_triggers)
                        && leaf
                            .triggers
                            .get(name)
                            .is_some_and(|metadata| matches!(metadata.enabled.as_str(), "O" | "A"));
                if !present {
                    if leaf.depth == 0 {
                        missing_triggers.push(name.clone());
                    } else {
                        missing_triggers.push(format!("{}:{name}", leaf.name));
                    }
                }
            }
        }
        missing_triggers.sort();
        let mut extra_triggers = Vec::new();
        for leaf in routable_leaves {
            for name in leaf
                .triggers
                .keys()
                .filter(|name| !parent_triggers.contains_key(*name))
            {
                if leaf.depth == 0 {
                    extra_triggers.push(name.clone());
                } else {
                    extra_triggers.push(format!("{}:{name}", leaf.name));
                }
            }
        }
        extra_triggers.sort();
        let catch_all_nonempty = if child.kind == PartitionChildKind::CatchAll {
            Some(relation_contains_rows(connection, &child.name).await?)
        } else {
            None
        };
        child_audits.push(PartitionChildAudit {
            name: child.name,
            relation_kind: child.relation_kind,
            lower: child.lower,
            upper: child.upper,
            kind: child.kind,
            pending_detach: child.pending_detach,
            catch_all_nonempty,
            missing_triggers,
            extra_triggers,
        });
    }

    let mut months = Vec::with_capacity(months_ahead as usize + 1);
    for offset in 0..=months_ahead {
        let (year, month) = add_months(now.year(), now.month(), offset)?;
        let start = month_start(year, month)?;
        let (end_year, end_month) = add_months(year, month, 1)?;
        let end = month_start(end_year, end_month)?;
        months.push(MonthCoverage {
            start,
            kind: coverage_for_range(&coverage_leaves, &start, &end),
        });
    }

    let serving_safe = coverage_leaves
        .iter()
        .any(|leaf| leaf_covers_timestamp(leaf, &now));

    Ok(PartitionTableAudit {
        table,
        partition_key,
        expected_partition_key,
        partition_key_valid,
        children: child_audits,
        coverage_leaves,
        months,
        serving_safe,
    })
}

fn validated_months_ahead(months_ahead: u32) -> Result<i32> {
    if months_ahead > MAX_PARTITION_MONTHS_AHEAD {
        return Err(DbError::InvalidData(format!(
            "partition audit months_ahead must be at most {MAX_PARTITION_MONTHS_AHEAD}, got {months_ahead}"
        )));
    }
    Ok(months_ahead as i32)
}

async fn relation_contains_rows(connection: &mut PgConnection, relation: &str) -> Result<bool> {
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&mut *connection)
        .await?;
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM ONLY {}.{} LIMIT 1)",
        quote_identifier(&schema),
        quote_identifier(relation)
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(&mut *connection)
        .await?)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn emit_audit_metrics(audit: &PartitionTableAudit, duration_seconds: f64, now: DateTime<Utc>) {
    let outcome = if audit.degraded() { "degraded" } else { "ok" };
    let uncovered = audit
        .months
        .iter()
        .filter(|month| month.kind == MonthCoverageKind::Uncovered)
        .count();
    let catch_all = audit
        .months
        .iter()
        .filter(|month| month.kind == MonthCoverageKind::CoveredByCatchAll)
        .count();

    metrics::counter!(
        "buzz_partition_audit_runs_total",
        "table" => audit.table,
        "outcome" => outcome
    )
    .increment(1);
    metrics::gauge!("buzz_partition_serving_safe", "table" => audit.table)
        .set(if audit.serving_safe { 1.0 } else { 0.0 });
    metrics::gauge!("buzz_partition_uncovered_months", "table" => audit.table)
        .set(uncovered as f64);
    metrics::gauge!(
        "buzz_partition_catch_all_covered_months",
        "table" => audit.table
    )
    .set(catch_all as f64);
    metrics::gauge!("buzz_partition_anomalous_children", "table" => audit.table)
        .set(audit.anomalous_children() as f64);
    metrics::gauge!("buzz_partition_catch_all_nonempty", "table" => audit.table)
        .set(audit.nonempty_catch_all_count() as f64);
    metrics::gauge!(
        "buzz_partition_trigger_parity_missing",
        "table" => audit.table
    )
    .set(audit.missing_trigger_count() as f64);
    metrics::gauge!(
        "buzz_partition_trigger_parity_extra",
        "table" => audit.table
    )
    .set(audit.extra_trigger_count() as f64);
    metrics::histogram!(
        "buzz_partition_audit_duration_seconds",
        "table" => audit.table
    )
    .record(duration_seconds);
    metrics::gauge!(
        "buzz_partition_audit_last_success_timestamp_seconds",
        "table" => audit.table
    )
    .set(now.timestamp() as f64);
}

#[derive(Debug, Clone)]
struct ChildTriggerMetadata {
    oid: i64,
    parent_oid: i64,
    enabled: String,
}

#[derive(Debug, Clone)]
struct DescendantTriggerMetadata {
    name: String,
    relation_oid: i64,
    parent_relation_oid: i64,
    root_child: String,
    relation_kind: String,
    depth: i32,
    is_leaf: bool,
    triggers: HashMap<String, ChildTriggerMetadata>,
}

async fn trigger_metadata_for_parent(
    connection: &mut PgConnection,
    table: &str,
) -> Result<HashMap<String, i64>> {
    let rows = sqlx::query(
        r#"
        SELECT trigger.tgname, trigger.oid::bigint AS trigger_oid
        FROM pg_catalog.pg_class parent
        JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
        JOIN pg_catalog.pg_trigger trigger ON trigger.tgrelid = parent.oid
        WHERE parent_ns.nspname = current_schema()
          AND parent.relname = $1
          AND NOT trigger.tgisinternal
          AND (trigger.tgtype & 1) = 1
        "#,
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter()
        .map(|row| Ok((row.try_get("tgname")?, row.try_get("trigger_oid")?)))
        .collect()
}

async fn trigger_metadata_for_descendants(
    connection: &mut PgConnection,
    table: &str,
) -> Result<HashMap<String, DescendantTriggerMetadata>> {
    let rows = sqlx::query(
        r#"
        WITH RECURSIVE partition_tree AS (
            SELECT child.oid AS relation_oid,
                   parent.oid AS parent_relation_oid,
                   child.relname,
                   child.relkind,
                   child.relname AS root_child,
                   0 AS depth
            FROM pg_catalog.pg_inherits inherited
            JOIN pg_catalog.pg_class parent ON parent.oid = inherited.inhparent
            JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
            JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
            WHERE parent_ns.nspname = current_schema()
              AND parent.relname = $1

            UNION ALL

            SELECT descendant.oid,
                   tree.relation_oid,
                   descendant.relname,
                   descendant.relkind,
                   tree.root_child,
                   tree.depth + 1
            FROM partition_tree tree
            JOIN pg_catalog.pg_inherits nested
              ON nested.inhparent = tree.relation_oid
            JOIN pg_catalog.pg_class descendant ON descendant.oid = nested.inhrelid
        )
        SELECT tree.relname AS relation_name,
               tree.relation_oid::bigint AS relation_oid,
               tree.parent_relation_oid::bigint AS parent_relation_oid,
               tree.root_child,
               tree.relkind::text AS relation_kind,
               tree.depth,
               NOT EXISTS (
                   SELECT 1
                   FROM pg_catalog.pg_inherits child_edge
                   WHERE child_edge.inhparent = tree.relation_oid
               ) AS is_leaf,
               trigger.tgname,
               trigger.oid::bigint AS trigger_oid,
               trigger.tgparentid::bigint AS trigger_parent_oid,
               trigger.tgenabled::text AS trigger_enabled
        FROM partition_tree tree
        LEFT JOIN pg_catalog.pg_trigger trigger
          ON trigger.tgrelid = tree.relation_oid
         AND NOT trigger.tgisinternal
         AND (trigger.tgtype & 1) = 1
        ORDER BY tree.depth, tree.relname, trigger.tgname
        "#,
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await?;
    let mut descendants = HashMap::<String, DescendantTriggerMetadata>::new();
    for row in rows {
        let name: String = row.try_get("relation_name")?;
        let trigger: Option<String> = row.try_get("tgname")?;
        let entry = descendants
            .entry(name.clone())
            .or_insert(DescendantTriggerMetadata {
                name,
                relation_oid: row.try_get("relation_oid")?,
                parent_relation_oid: row.try_get("parent_relation_oid")?,
                root_child: row.try_get("root_child")?,
                relation_kind: row.try_get("relation_kind")?,
                depth: row.try_get("depth")?,
                is_leaf: row.try_get("is_leaf")?,
                triggers: HashMap::new(),
            });
        if let Some(trigger) = trigger {
            entry.triggers.insert(
                trigger,
                ChildTriggerMetadata {
                    oid: row.try_get("trigger_oid")?,
                    parent_oid: row.try_get("trigger_parent_oid")?,
                    enabled: row.try_get("trigger_enabled")?,
                },
            );
        }
    }
    Ok(descendants)
}

fn trigger_lineage_reaches_parent(
    leaf: &DescendantTriggerMetadata,
    trigger_name: &str,
    parent_trigger_oid: i64,
    descendants: &HashMap<String, DescendantTriggerMetadata>,
) -> bool {
    let mut relation = leaf;
    let Some(mut trigger) = relation.triggers.get(trigger_name) else {
        return false;
    };
    while relation.depth > 0 {
        let Some(parent_relation) = descendants
            .values()
            .find(|candidate| candidate.relation_oid == relation.parent_relation_oid)
        else {
            return false;
        };
        let Some(parent_trigger) = parent_relation.triggers.get(trigger_name) else {
            return false;
        };
        if trigger.parent_oid != parent_trigger.oid {
            return false;
        }
        relation = parent_relation;
        trigger = parent_trigger;
    }
    trigger.parent_oid == parent_trigger_oid
}

fn classify_child(
    relation_kind: &str,
    table: &str,
    name: &str,
    lower: Option<&PartitionBound>,
    upper: Option<&PartitionBound>,
) -> PartitionChildKind {
    if relation_kind != "r" {
        return PartitionChildKind::Anomalous;
    }
    match (lower, upper) {
        (Some(PartitionBound::Finite(_)), Some(PartitionBound::MaxValue)) => {
            PartitionChildKind::CatchAll
        }
        (Some(PartitionBound::MinValue), Some(PartitionBound::Finite(_)))
            if name == format!("{table}_p_past") =>
        {
            PartitionChildKind::Past
        }
        (Some(PartitionBound::Finite(lower)), Some(PartitionBound::Finite(upper)))
            if lower < upper =>
        {
            let canonical = format!("{table}_p{:04}_{:02}", lower.year(), lower.month());
            if name == canonical && is_exact_month(lower, upper) {
                PartitionChildKind::CanonicalMonthly
            } else if canonical_month_name(table, name).is_some() {
                PartitionChildKind::Anomalous
            } else {
                PartitionChildKind::LegacyLeaf
            }
        }
        _ => PartitionChildKind::Anomalous,
    }
}

fn canonical_month_name(table: &str, name: &str) -> Option<(i32, u32)> {
    let suffix = name.strip_prefix(&format!("{table}_p"))?;
    if suffix.len() != 7 || suffix.as_bytes().get(4) != Some(&b'_') {
        return None;
    }
    let year = suffix[..4].parse::<i32>().ok()?;
    let month = suffix[5..].parse::<u32>().ok()?;
    (1..=12).contains(&month).then_some((year, month))
}

fn is_exact_month(lower: &DateTime<Utc>, upper: &DateTime<Utc>) -> bool {
    if lower.day() != 1
        || lower.hour() != 0
        || lower.minute() != 0
        || lower.second() != 0
        || lower.nanosecond() != 0
    {
        return false;
    }
    let Ok((year, month)) = add_months(lower.year(), lower.month(), 1) else {
        return false;
    };
    month_start(year, month).is_ok_and(|expected| expected == *upper)
}

fn mark_overlaps_anomalous(children: &mut [CatalogChild]) {
    let mut overlapping = HashSet::new();
    for left in 0..children.len() {
        for right in (left + 1)..children.len() {
            if ranges_overlap(&children[left], &children[right]) {
                overlapping.insert(left);
                overlapping.insert(right);
            }
        }
    }
    for index in overlapping {
        children[index].kind = PartitionChildKind::Anomalous;
    }
}

fn ranges_overlap(left: &CatalogChild, right: &CatalogChild) -> bool {
    let (Some(left_lower), Some(left_upper), Some(right_lower), Some(right_upper)) = (
        left.lower.as_ref(),
        left.upper.as_ref(),
        right.lower.as_ref(),
        right.upper.as_ref(),
    ) else {
        return false;
    };
    left_lower < right_upper && right_lower < left_upper
}

fn coverage_for_range(
    leaves: &[PartitionLeafAudit],
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
) -> MonthCoverageKind {
    let start = PartitionBound::Finite(*start);
    let end = PartitionBound::Finite(*end);
    let mut ranges = leaves
        .iter()
        .filter(|leaf| leaf.upper > start && leaf.lower < end)
        .collect::<Vec<_>>();
    ranges.sort_by(|left, right| left.lower.cmp(&right.lower));

    let mut cursor = start;
    let mut catch_all_contributed = false;
    for leaf in ranges {
        if leaf.lower > cursor {
            return MonthCoverageKind::Uncovered;
        }
        if leaf.upper > cursor {
            catch_all_contributed |= leaf.upper == PartitionBound::MaxValue;
            cursor = leaf.upper.clone();
        }
        if cursor >= end {
            return if catch_all_contributed {
                MonthCoverageKind::CoveredByCatchAll
            } else {
                MonthCoverageKind::CoveredByMonthly
            };
        }
    }
    MonthCoverageKind::Uncovered
}

fn leaf_covers_timestamp(leaf: &PartitionLeafAudit, timestamp: &DateTime<Utc>) -> bool {
    leaf.lower <= PartitionBound::Finite(*timestamp)
        && leaf.upper > PartitionBound::Finite(*timestamp)
}

fn intersect_ranges(
    parent: &(PartitionBound, PartitionBound),
    child: &(PartitionBound, PartitionBound),
) -> Option<(PartitionBound, PartitionBound)> {
    let lower = std::cmp::max(parent.0.clone(), child.0.clone());
    let upper = std::cmp::min(parent.1.clone(), child.1.clone());
    (lower < upper).then_some((lower, upper))
}

fn parse_range_bounds(expression: &str) -> Option<(PartitionBound, PartitionBound)> {
    let remainder = expression.strip_prefix("FOR VALUES FROM (")?;
    let (lower, upper_with_suffix) = remainder.split_once(") TO (")?;
    let upper = upper_with_suffix.strip_suffix(')')?;
    Some((parse_bound(lower)?, parse_bound(upper)?))
}

fn parse_bound(input: &str) -> Option<PartitionBound> {
    let input = input.trim();
    match input {
        "MINVALUE" => Some(PartitionBound::MinValue),
        "MAXVALUE" => Some(PartitionBound::MaxValue),
        _ => {
            let first_quote = input.find('\'')?;
            let literal = &input[first_quote + 1..];
            let last_quote = literal.find('\'')?;
            parse_timestamp_literal(&literal[..last_quote]).map(PartitionBound::Finite)
        }
    }
}

fn parse_timestamp_literal(literal: &str) -> Option<DateTime<Utc>> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(literal) {
        return Some(timestamp.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"] {
        if let Ok(timestamp) = DateTime::parse_from_str(literal, format) {
            return Some(timestamp.with_timezone(&Utc));
        }
    }
    let date = NaiveDate::parse_from_str(literal, "%Y-%m-%d").ok()?;
    Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
}

async fn create_month_partition(
    pool: &PgPool,
    table: &str,
    start: DateTime<Utc>,
) -> Result<String> {
    if !PARTITIONED_TABLES.contains(&table) {
        return Err(DbError::InvalidData(format!(
            "table not in partition allowlist: {table:?}"
        )));
    }
    let (end_year, end_month) = add_months(start.year(), start.month(), 1)?;
    let end = month_start(end_year, end_month)?;
    let partition_name = partition_name(table, start);
    let start_date = start.format("%Y-%m-%d");
    let end_date = end.format("%Y-%m-%d");
    let sql = format!(
        "CREATE TABLE {} PARTITION OF {} \
         FOR VALUES FROM ('{start_date}') TO ('{end_date}')",
        quote_identifier(&partition_name),
        quote_identifier(table)
    );
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Bootstrap,
    )
    .await?;
    let mut transaction = connection.begin().await?;
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(&mut *transaction)
        .await?;
    let row = sqlx::query(
        r#"
        SELECT child.relkind::text AS relation_kind,
               inherited.inhdetachpending AS detach_pending,
               pg_catalog.pg_get_partkeydef(parent.oid) AS partition_key,
               pg_catalog.pg_get_expr(child.relpartbound, child.oid) AS bound
        FROM pg_catalog.pg_inherits inherited
        JOIN pg_catalog.pg_class parent ON parent.oid = inherited.inhparent
        JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
        JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
        JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid = child.relnamespace
        WHERE parent_ns.nspname = current_schema()
          AND child_ns.nspname = current_schema()
          AND parent.relname = $1
          AND child.relname = $2
        "#,
    )
    .bind(table)
    .bind(&partition_name)
    .fetch_optional(&mut *transaction)
    .await?;
    let exact = row.is_some_and(|row| {
        let relation_kind = row.try_get::<String, _>("relation_kind").ok();
        let detach_pending = row.try_get::<bool, _>("detach_pending").ok();
        let partition_key = row
            .try_get::<Option<String>, _>("partition_key")
            .ok()
            .flatten();
        let bounds = row
            .try_get::<String, _>("bound")
            .ok()
            .and_then(|bound| parse_range_bounds(&bound));
        relation_kind.as_deref() == Some("r")
            && detach_pending == Some(false)
            && partition_key.as_deref() == expected_partition_key(table)
            && bounds == Some((PartitionBound::Finite(start), PartitionBound::Finite(end)))
    });
    if !exact {
        transaction.rollback().await?;
        return Err(DbError::InvalidData(format!(
            "created partition {partition_name} failed exact catalog postcondition"
        )));
    }
    transaction.commit().await?;
    Ok(partition_name)
}

async fn relation_name_exists(pool: &PgPool, relation: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM pg_catalog.pg_class relation
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname = current_schema()
              AND relation.relname = $1
        )
        "#,
    )
    .bind(relation)
    .fetch_one(pool)
    .await?)
}

fn partition_name(table: &str, start: DateTime<Utc>) -> String {
    format!("{table}_p{:04}_{:02}", start.year(), start.month())
}

fn month_start(year: i32, month: u32) -> Result<DateTime<Utc>> {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| DbError::InvalidData(format!("invalid date: {year}-{month:02}-01")))
}

fn add_months(year: i32, month: u32, offset: i32) -> Result<(i32, u32)> {
    if !(1..=12).contains(&month) {
        return Err(DbError::InvalidData(format!("invalid month: {month}")));
    }
    let zero_based = year
        .checked_mul(12)
        .and_then(|value| value.checked_add(month as i32 - 1))
        .and_then(|value| value.checked_add(offset))
        .ok_or_else(|| {
            DbError::InvalidData(format!("month arithmetic overflow: {year}-{month}"))
        })?;
    Ok((
        zero_based.div_euclid(12),
        (zero_based.rem_euclid(12) + 1) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unbounded_partition_audit_horizon_before_connecting() {
        assert_eq!(
            validated_months_ahead(MAX_PARTITION_MONTHS_AHEAD).expect("maximum supported horizon"),
            MAX_PARTITION_MONTHS_AHEAD as i32
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://127.0.0.1:1/buzz")
            .expect("lazy test pool");

        let report = audit_partition_catalog_report_at(
            &pool,
            MAX_PARTITION_MONTHS_AHEAD + 1,
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        )
        .await;

        assert!(report.tables.is_empty());
        assert_eq!(report.errors.len(), PARTITIONED_TABLES.len());
        assert!(report
            .errors
            .iter()
            .all(|error| error.error.contains("months_ahead must be at most 120")));
    }

    #[test]
    fn parses_pg16_range_bound_formats() {
        let expected = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
        for expression in [
            "FOR VALUES FROM ('2026-07-01 00:00:00+00') TO (MAXVALUE)",
            "FOR VALUES FROM ('2026-07-01 02:00:00+02') TO (MAXVALUE)",
            "FOR VALUES FROM ('2026-07-01 00:00:00.000000+00'::timestamp with time zone) TO (MAXVALUE)",
            "FOR VALUES FROM ('2026-07-01') TO (MAXVALUE)",
        ] {
            assert_eq!(
                parse_range_bounds(expression),
                Some((PartitionBound::Finite(expected), PartitionBound::MaxValue)),
                "failed to parse {expression}"
            );
        }
        assert_eq!(
            parse_range_bounds("FOR VALUES FROM (MINVALUE) TO ('2026-07-01')"),
            Some((PartitionBound::MinValue, PartitionBound::Finite(expected)))
        );
        assert!(parse_range_bounds("DEFAULT").is_none());
    }

    #[test]
    fn classifies_names_and_bounds() {
        let july = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
        let august = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let lower = PartitionBound::Finite(july);
        let upper = PartitionBound::Finite(august);
        assert_eq!(
            classify_child("r", "events", "events_p2026_07", Some(&lower), Some(&upper)),
            PartitionChildKind::CanonicalMonthly
        );
        assert_eq!(
            classify_child(
                "r",
                "events",
                "events_july_repair",
                Some(&lower),
                Some(&upper)
            ),
            PartitionChildKind::LegacyLeaf
        );
        assert_eq!(
            classify_child("r", "events", "events_p2026_08", Some(&lower), Some(&upper)),
            PartitionChildKind::Anomalous
        );
        assert_eq!(
            classify_child(
                "r",
                "events",
                "events_p_future_next",
                Some(&lower),
                Some(&PartitionBound::MaxValue)
            ),
            PartitionChildKind::CatchAll
        );
        assert_eq!(
            classify_child("p", "events", "events_p2026_07", Some(&lower), Some(&upper)),
            PartitionChildKind::Anomalous
        );
        let september = PartitionBound::Finite(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap());
        assert_eq!(
            classify_child(
                "r",
                "events",
                "events_p2026_07_08_legacy",
                Some(&lower),
                Some(&september)
            ),
            PartitionChildKind::LegacyLeaf
        );
    }

    #[test]
    fn overlapping_ranges_are_anomalous() {
        let july = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
        let august = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let september = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let mut children = vec![
            CatalogChild {
                name: "one".to_string(),
                relation_kind: "r".to_string(),
                lower: Some(PartitionBound::Finite(july)),
                upper: Some(PartitionBound::Finite(september)),
                kind: PartitionChildKind::LegacyLeaf,
                pending_detach: false,
            },
            CatalogChild {
                name: "two".to_string(),
                relation_kind: "r".to_string(),
                lower: Some(PartitionBound::Finite(august)),
                upper: Some(PartitionBound::MaxValue),
                kind: PartitionChildKind::CatchAll,
                pending_detach: false,
            },
        ];
        mark_overlaps_anomalous(&mut children);
        assert!(children
            .iter()
            .all(|child| child.kind == PartitionChildKind::Anomalous));
    }

    #[test]
    fn month_arithmetic_crosses_year_boundary() {
        assert_eq!(add_months(2026, 12, 1).unwrap(), (2027, 1));
        assert_eq!(add_months(2026, 1, -1).unwrap(), (2025, 12));
        assert!(add_months(2026, 0, 1).is_err());
    }

    #[test]
    fn month_coverage_can_span_multiple_nested_leaves() {
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let midpoint = Utc.with_ymd_and_hms(2026, 9, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let leaves = vec![
            PartitionLeafAudit {
                name: "first".to_string(),
                root_child: "nested".to_string(),
                lower: PartitionBound::Finite(start),
                upper: PartitionBound::Finite(midpoint),
                nested: true,
            },
            PartitionLeafAudit {
                name: "second".to_string(),
                root_child: "nested".to_string(),
                lower: PartitionBound::Finite(midpoint),
                upper: PartitionBound::Finite(end),
                nested: true,
            },
        ];
        assert_eq!(
            coverage_for_range(&leaves, &start, &end),
            MonthCoverageKind::CoveredByMonthly
        );
    }

    #[test]
    fn extra_trigger_degradation_has_a_metric() {
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let audit = PartitionTableAudit {
            table: "events",
            partition_key: Some("RANGE (created_at)".to_string()),
            expected_partition_key: "RANGE (created_at)",
            partition_key_valid: true,
            children: vec![PartitionChildAudit {
                name: "events_p2026_09".to_string(),
                relation_kind: "r".to_string(),
                lower: Some(PartitionBound::Finite(start)),
                upper: Some(PartitionBound::Finite(end)),
                kind: PartitionChildKind::CanonicalMonthly,
                pending_detach: false,
                catch_all_nonempty: None,
                missing_triggers: Vec::new(),
                extra_triggers: vec!["child_only_probe".to_string()],
            }],
            coverage_leaves: vec![PartitionLeafAudit {
                name: "events_p2026_09".to_string(),
                root_child: "events_p2026_09".to_string(),
                lower: PartitionBound::Finite(start),
                upper: PartitionBound::Finite(end),
                nested: false,
            }],
            months: vec![MonthCoverage {
                start,
                kind: MonthCoverageKind::CoveredByMonthly,
            }],
            serving_safe: true,
        };
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || emit_audit_metrics(&audit, 0.01, now));

        let extra = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .find_map(|(key, _, _, value)| {
                (key.key().name() == "buzz_partition_trigger_parity_extra").then(|| {
                    let metrics_util::debugging::DebugValue::Gauge(value) = value else {
                        panic!("extra-trigger metric must be a gauge");
                    };
                    value.into_inner()
                })
            });
        assert_eq!(extra, Some(1.0));
    }

    mod postgres_tests {
        use sqlx::postgres::PgPoolOptions;
        use uuid::Uuid;

        use super::*;

        async fn scratch_pool() -> (PgPool, PgPool, String) {
            scratch_pool_with_max_connections(4).await
        }

        async fn scratch_pool_with_max_connections(
            max_connections: u32,
        ) -> (PgPool, PgPool, String) {
            let url = crate::test_support::database_url();
            let schema = format!("partition_audit_test_{}", Uuid::new_v4().simple());
            let admin = PgPool::connect(&url).await.expect("connect admin pool");
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
                .execute(&admin)
                .await
                .expect("create scratch schema");
            let search_path_schema = schema.clone();
            let pool = PgPoolOptions::new()
                .max_connections(max_connections)
                .after_connect(move |connection, _| {
                    let schema = search_path_schema.clone();
                    Box::pin(async move {
                        sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                            .execute(connection)
                            .await?;
                        Ok(())
                    })
                })
                .connect(&url)
                .await
                .expect("connect scratch pool");
            (pool, admin, schema)
        }

        async fn drop_schema(admin: &PgPool, schema: &str) {
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE"
            )))
            .execute(admin)
            .await;
        }

        async fn seed_parents(pool: &PgPool) {
            seed_parents_with_keys(pool, "created_at", "delivered_at").await;
        }

        async fn seed_parents_with_keys(pool: &PgPool, events_key: &str, delivery_log_key: &str) {
            sqlx::query(
                "CREATE FUNCTION partition_test_trigger() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$",
            )
            .execute(pool)
            .await
            .expect("create trigger function");
            for (table, column, partition_key) in [
                ("events", "created_at", events_key),
                ("delivery_log", "delivered_at", delivery_log_key),
            ] {
                let create = format!(
                    "CREATE TABLE {table} (id BIGSERIAL, {column} TIMESTAMPTZ NOT NULL, \
                     alternate_at TIMESTAMPTZ NOT NULL, \
                     PRIMARY KEY ({column}, alternate_at, id)) PARTITION BY RANGE ({partition_key})"
                );
                sqlx::query(sqlx::AssertSqlSafe(create))
                    .execute(pool)
                    .await
                    .expect("create partitioned parent");
                let trigger = format!(
                    "CREATE TRIGGER partition_probe BEFORE INSERT ON {table} \
                     FOR EACH ROW EXECUTE FUNCTION partition_test_trigger()"
                );
                sqlx::query(sqlx::AssertSqlSafe(trigger))
                    .execute(pool)
                    .await
                    .expect("create parent trigger");
            }
        }

        async fn create_child(pool: &PgPool, table: &str, name: &str, lower: &str, upper: &str) {
            let sql = format!(
                "CREATE TABLE {name} PARTITION OF {table} FOR VALUES FROM ({lower}) TO ({upper})"
            );
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .expect("create child");
        }

        async fn create_nested_child(
            pool: &PgPool,
            table: &str,
            name: &str,
            column: &str,
            lower: &str,
            upper: &str,
        ) {
            let sql = format!(
                "CREATE TABLE {name} PARTITION OF {table} \
                 FOR VALUES FROM ({lower}) TO ({upper}) PARTITION BY RANGE ({column})"
            );
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .expect("create nested child");
        }

        async fn catalog_snapshot(pool: &PgPool) -> Vec<(String, String, String, i64)> {
            let mut transaction = pool.begin().await.expect("begin catalog snapshot");
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *transaction)
                .await
                .expect("make catalog snapshot read only");
            pin_catalog_rendering(&mut transaction)
                .await
                .expect("pin catalog rendering");
            let snapshot = sqlx::query_as(
                r#"
                SELECT child.relname,
                       child.relkind::text,
                       pg_catalog.pg_get_expr(child.relpartbound, child.oid),
                       count(trigger.oid)
                FROM pg_catalog.pg_inherits inherited
                JOIN pg_catalog.pg_class parent ON parent.oid = inherited.inhparent
                JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
                JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
                LEFT JOIN pg_catalog.pg_trigger trigger
                  ON trigger.tgrelid = child.oid AND NOT trigger.tgisinternal
                WHERE parent_ns.nspname = current_schema()
                  AND child.relispartition
                  AND child.relkind IN ('r', 'p', 'f')
                GROUP BY child.relname, child.relkind, child.relpartbound, child.oid
                ORDER BY child.relname
                "#,
            )
            .fetch_all(&mut *transaction)
            .await
            .expect("catalog snapshot");
            transaction.commit().await.expect("commit catalog snapshot");
            snapshot
        }

        fn fixed_now() -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap()
        }

        async fn seed_fresh_layout(pool: &PgPool) {
            seed_parents(pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
                create_child(
                    pool,
                    table,
                    &format!("{table}_p_future"),
                    "'2026-09-01'",
                    "MAXVALUE",
                )
                .await;
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn fresh_layout_is_serving_safe_and_audit_is_read_only() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            let before = catalog_snapshot(&pool).await;
            let audit = audit_partition_catalog_at(&pool, 3, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            assert!(audit.tables.iter().all(|table| {
                table.anomalous_children() == 0
                    && table.missing_trigger_count() == 0
                    && table
                        .months
                        .iter()
                        .all(|month| month.kind == MonthCoverageKind::CoveredByCatchAll)
            }));
            assert_eq!(catalog_snapshot(&pool).await, before);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn wrong_root_partition_key_never_proves_coverage() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents_with_keys(&pool, "alternate_at", "delivered_at").await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_future"),
                    "'2026-09-01'",
                    "MAXVALUE",
                )
                .await;
            }

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert_eq!(
                events.partition_key.as_deref(),
                Some("RANGE (alternate_at)")
            );
            assert!(!events.partition_key_valid);
            assert!(!events.serving_safe);
            assert!(events.coverage_leaves.is_empty());
            assert_eq!(events.children[0].kind, PartitionChildKind::Anomalous);
            let delivery_log = audit
                .tables
                .iter()
                .find(|table| table.table == "delivery_log")
                .expect("delivery_log audit");
            assert!(delivery_log.partition_key_valid);
            assert!(delivery_log.serving_safe);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn occupied_catch_all_is_reported_as_degraded() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            sqlx::query(
                "INSERT INTO events (created_at, alternate_at) \
                 VALUES ('2026-09-15', '2026-09-15')",
            )
            .execute(&pool)
            .await
            .expect("insert catch-all row");

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert!(events.serving_safe);
            assert!(events.degraded());
            assert_eq!(events.nonempty_catch_all_count(), 1);
            assert!(events.children.iter().any(|child| {
                child.name == "events_p_future" && child.catch_all_nonempty == Some(true)
            }));
            let delivery_log = audit
                .tables
                .iter()
                .find(|table| table.table == "delivery_log")
                .expect("delivery_log audit");
            assert_eq!(delivery_log.nonempty_catch_all_count(), 0);
            assert!(delivery_log.children.iter().any(|child| {
                child.name == "delivery_log_p_future" && child.catch_all_nonempty == Some(false)
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn report_retains_successful_table_when_another_table_errors() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            let role = format!("partition_audit_reader_{}", Uuid::new_v4().simple());
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE ROLE {role} NOLOGIN")))
                .execute(&admin)
                .await
                .expect("create restricted audit role");
            for statement in [
                format!("GRANT USAGE ON SCHEMA {schema} TO {role}"),
                format!("GRANT SELECT ON {schema}.events_p_future TO {role}"),
            ] {
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&admin)
                    .await
                    .expect("grant restricted audit access");
            }
            let url = crate::test_support::database_url();
            let role_schema = schema.clone();
            let role_name = role.clone();
            let restricted = PgPoolOptions::new()
                .max_connections(1)
                .after_connect(move |connection, _| {
                    let schema = role_schema.clone();
                    let role = role_name.clone();
                    Box::pin(async move {
                        sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                            .execute(&mut *connection)
                            .await?;
                        sqlx::query(sqlx::AssertSqlSafe(format!("SET ROLE {role}")))
                            .execute(&mut *connection)
                            .await?;
                        Ok(())
                    })
                })
                .connect(&url)
                .await
                .expect("connect restricted audit pool");

            let report = audit_partition_catalog_report_at(&restricted, 0, fixed_now()).await;
            assert_eq!(report.tables.len(), 1);
            assert_eq!(report.tables[0].table, "events");
            assert_eq!(report.errors.len(), 1);
            assert_eq!(report.errors[0].table, "delivery_log");
            assert!(report.errors[0].error.contains("permission denied"));
            assert!(!report.serving_safe());

            restricted.close().await;
            drop_schema(&admin, &schema).await;
            sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {role}")))
                .execute(&admin)
                .await
                .expect("drop restricted audit role");
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn pending_detach_never_proves_coverage() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;

            let mut blocker = pool.acquire().await.expect("acquire blocker");
            sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut *blocker)
                .await
                .expect("begin blocker");
            sqlx::query("SELECT count(*) FROM ONLY events")
                .execute(&mut *blocker)
                .await
                .expect("establish old parent-only snapshot");

            let detach_pool = pool.clone();
            let detach = tokio::spawn(async move {
                sqlx::query("ALTER TABLE events DETACH PARTITION events_p_future CONCURRENTLY")
                    .execute(&detach_pool)
                    .await
            });
            let pending_observed =
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        let pending: bool = sqlx::query_scalar(
                            r#"
                        SELECT inherited.inhdetachpending
                        FROM pg_catalog.pg_inherits inherited
                        JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
                        JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid = child.relnamespace
                        WHERE child.relname = 'events_p_future'
                          AND child_ns.nspname = current_schema()
                        "#,
                        )
                        .fetch_one(&pool)
                        .await
                        .expect("read pending-detach state");
                        if pending {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                })
                .await
                .is_ok();

            let audit = if pending_observed {
                Some(
                    audit_partition_catalog_at(&pool, 0, fixed_now())
                        .await
                        .expect("audit pending detach"),
                )
            } else {
                None
            };
            sqlx::query("ROLLBACK")
                .execute(&mut *blocker)
                .await
                .expect("release blocker");
            tokio::time::timeout(std::time::Duration::from_secs(10), detach)
                .await
                .expect("detach completion timeout")
                .expect("detach task")
                .expect("detach partition");

            assert!(
                pending_observed,
                "detach never reached pending catalog state"
            );
            let audit = audit.expect("audit captured");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert!(!events.serving_safe);
            assert!(events
                .coverage_leaves
                .iter()
                .all(|leaf| leaf.name != "events_p_future"));
            assert!(events.children.iter().any(|child| {
                child.name == "events_p_future"
                    && child.pending_detach
                    && child.kind == PartitionChildKind::Anomalous
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn repaired_layout_recognizes_bounds_not_catch_all_name() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-07-01'",
                )
                .await;
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p2026_07_08_legacy"),
                    "'2026-07-01'",
                    "'2026-09-01'",
                )
                .await;
                for month in 9..=12 {
                    let (end_year, end_month) = add_months(2026, month, 1).unwrap();
                    create_child(
                        &pool,
                        table,
                        &format!("{table}_p2026_{month:02}"),
                        &format!("'2026-{month:02}-01'"),
                        &format!("'{end_year}-{end_month:02}-01'"),
                    )
                    .await;
                }
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_future_next"),
                    "'2027-01-01'",
                    "MAXVALUE",
                )
                .await;
            }
            let before = catalog_snapshot(&pool).await;
            let audit = audit_partition_catalog_at(&pool, 3, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            for table in &audit.tables {
                assert_eq!(
                    table
                        .children
                        .iter()
                        .filter(|child| child.kind == PartitionChildKind::LegacyLeaf)
                        .count(),
                    1
                );
                assert!(table
                    .children
                    .iter()
                    .any(|child| child.name.ends_with("p_future_next")
                        && child.kind == PartitionChildKind::CatchAll));
                assert!(table
                    .months
                    .iter()
                    .all(|month| month.kind == MonthCoverageKind::CoveredByMonthly));
            }
            assert_eq!(catalog_snapshot(&pool).await, before);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn uncovered_months_are_created_with_trigger_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
            }
            ensure_future_partitions_at(&pool, 1, true, fixed_now())
                .await
                .expect("create gaps");
            let audit = audit_partition_catalog_at(&pool, 1, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            assert!(audit.tables.iter().all(|table| table
                .months
                .iter()
                .all(|month| month.kind == MonthCoverageKind::CoveredByMonthly)
                && table.missing_trigger_count() == 0));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn disabled_child_trigger_degrades_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            sqlx::query("ALTER TABLE events_p_future DISABLE TRIGGER partition_probe")
                .execute(&pool)
                .await
                .expect("disable child trigger");

            let audit = audit_partition_catalog_at(&pool, 3, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert!(events.degraded());
            assert!(events.children.iter().any(|child| {
                child.name == "events_p_future" && child.missing_triggers == ["partition_probe"]
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn always_enabled_parent_trigger_preserves_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            sqlx::query("ALTER TABLE events ENABLE ALWAYS TRIGGER partition_probe")
                .execute(&pool)
                .await
                .expect("always-enable parent trigger");

            let audit = audit_partition_catalog_at(&pool, 3, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert_eq!(events.missing_trigger_count(), 0);
            assert!(events
                .children
                .iter()
                .all(|child| child.missing_triggers.is_empty()));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn disabled_nested_leaf_trigger_degrades_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for (table, column) in [("events", "created_at"), ("delivery_log", "delivered_at")] {
                let nested = format!("{table}_nested");
                create_nested_child(
                    &pool,
                    table,
                    &nested,
                    column,
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
                create_child(
                    &pool,
                    &nested,
                    &format!("{table}_nested_first"),
                    "'2026-09-01'",
                    "'2026-09-15'",
                )
                .await;
                create_child(
                    &pool,
                    &nested,
                    &format!("{table}_nested_second"),
                    "'2026-09-15'",
                    "'2026-10-01'",
                )
                .await;
            }

            let healthy = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("healthy nested audit");
            assert!(healthy
                .tables
                .iter()
                .all(|table| table.missing_trigger_count() == 0));

            sqlx::query("ALTER TABLE ONLY events_nested DISABLE TRIGGER partition_probe")
                .execute(&pool)
                .await
                .expect("disable intermediate partitioned trigger");
            let intermediate_disabled = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("intermediate-disabled nested audit");
            assert!(intermediate_disabled
                .tables
                .iter()
                .all(|table| table.missing_trigger_count() == 0));

            sqlx::query("ALTER TABLE ONLY events_nested_first DISABLE TRIGGER partition_probe")
                .execute(&pool)
                .await
                .expect("disable nested leaf trigger");

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("degraded nested audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert_eq!(events.missing_trigger_count(), 1);
            assert!(events.children.iter().any(|child| {
                child.name == "events_nested"
                    && child.missing_triggers == ["events_nested_first:partition_probe"]
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn child_only_nested_leaf_trigger_degrades_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            create_nested_child(
                &pool,
                "events",
                "events_nested",
                "created_at",
                "'2026-09-01'",
                "'2026-10-01'",
            )
            .await;
            create_child(
                &pool,
                "events_nested",
                "events_nested_first",
                "'2026-09-01'",
                "'2026-09-15'",
            )
            .await;
            create_child(
                &pool,
                "events_nested",
                "events_nested_second",
                "'2026-09-15'",
                "'2026-10-01'",
            )
            .await;
            sqlx::query(
                "CREATE TRIGGER child_only_probe BEFORE INSERT ON events_nested_first \
                 FOR EACH ROW EXECUTE FUNCTION partition_test_trigger()",
            )
            .execute(&pool)
            .await
            .expect("create nested child-only trigger");

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert_eq!(events.extra_trigger_count(), 1);
            assert!(events.children.iter().any(|child| {
                child.name == "events_nested"
                    && child.extra_triggers == ["events_nested_first:child_only_probe"]
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn catch_all_skips_create_without_catalog_mutation() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_fresh_layout(&pool).await;
            let before = catalog_snapshot(&pool).await;
            ensure_future_partitions_at(&pool, 3, true, fixed_now())
                .await
                .expect("covered no-op");
            assert_eq!(catalog_snapshot(&pool).await, before);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn kill_switch_audits_but_does_not_create() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
            }
            let before = catalog_snapshot(&pool).await;
            ensure_future_partitions_at(&pool, 1, false, fixed_now())
                .await
                .expect("disabled create");
            assert_eq!(catalog_snapshot(&pool).await, before);
            let audit = audit_partition_catalog_at(&pool, 1, fixed_now())
                .await
                .expect("audit");
            assert!(audit.tables.iter().all(|table| !table.serving_safe
                && table
                    .months
                    .iter()
                    .all(|month| month.kind == MonthCoverageKind::Uncovered)));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn nested_children_without_leaves_do_not_prove_coverage() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for (table, column) in [("events", "created_at"), ("delivery_log", "delivered_at")] {
                create_nested_child(
                    &pool,
                    table,
                    &format!("{table}_nested"),
                    column,
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
            }

            let before = catalog_snapshot(&pool).await;
            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            assert!(!audit.serving_safe());
            assert!(audit.tables.iter().all(|table| {
                table.degraded()
                    && table.anomalous_children() == 1
                    && table.coverage_leaves.is_empty()
                    && table.months[0].kind == MonthCoverageKind::Uncovered
            }));
            assert_eq!(catalog_snapshot(&pool).await, before);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn nested_descendant_leaves_prove_coverage_but_remain_degraded() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for (table, column) in [("events", "created_at"), ("delivery_log", "delivered_at")] {
                let nested = format!("{table}_nested");
                create_nested_child(
                    &pool,
                    table,
                    &nested,
                    column,
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
                create_child(
                    &pool,
                    &nested,
                    &format!("{table}_nested_leaf"),
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
            }

            let before = catalog_snapshot(&pool).await;
            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            assert!(audit.tables.iter().all(|table| {
                table.degraded()
                    && table.anomalous_children() == 1
                    && table.coverage_leaves.len() == 1
                    && table.coverage_leaves[0].nested
                    && table.months[0].kind == MonthCoverageKind::CoveredByMonthly
            }));
            assert_eq!(catalog_snapshot(&pool).await, before);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn nested_different_partition_key_does_not_prove_coverage() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                let nested = format!("{table}_nested");
                create_nested_child(
                    &pool,
                    table,
                    &nested,
                    "alternate_at",
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
                create_child(
                    &pool,
                    &nested,
                    &format!("{table}_nested_leaf"),
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
            }

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            assert!(!audit.serving_safe());
            assert!(audit.tables.iter().all(|table| {
                table.degraded()
                    && table.anomalous_children() == 1
                    && table.coverage_leaves.is_empty()
                    && table.months[0].kind == MonthCoverageKind::Uncovered
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn audit_pins_non_iso_session_rendering() {
            let (pool, admin, schema) = scratch_pool_with_max_connections(1).await;
            seed_fresh_layout(&pool).await;
            sqlx::query("SET DateStyle TO 'SQL, DMY'")
                .execute(&pool)
                .await
                .expect("set non-ISO DateStyle");
            sqlx::query("SET TimeZone TO 'America/Los_Angeles'")
                .execute(&pool)
                .await
                .expect("set non-UTC TimeZone");

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            assert!(audit
                .tables
                .iter()
                .all(|table| !table.coverage_leaves.is_empty()));
            let date_style: String = sqlx::query_scalar("SHOW DateStyle")
                .fetch_one(&pool)
                .await
                .expect("show DateStyle");
            let time_zone: String = sqlx::query_scalar("SHOW TimeZone")
                .fetch_one(&pool)
                .await
                .expect("show TimeZone");
            assert_eq!(date_style, "SQL, DMY");
            assert_eq!(time_zone, "America/Los_Angeles");
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn statement_triggers_are_not_part_of_leaf_parity() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "CREATE TRIGGER statement_probe BEFORE INSERT ON {table} \
                     FOR EACH STATEMENT EXECUTE FUNCTION partition_test_trigger()"
                )))
                .execute(&pool)
                .await
                .expect("create parent statement trigger");
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p2026_09"),
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
            }

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            assert!(audit.tables.iter().all(|table| {
                !table.degraded()
                    && table.missing_trigger_count() == 0
                    && table.extra_trigger_count() == 0
            }));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn canonical_name_with_wrong_bounds_is_a_real_creation_error() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
            }
            create_child(
                &pool,
                "events",
                "events_p2026_09",
                "'2026-10-01'",
                "'2026-11-01'",
            )
            .await;

            let result = ensure_future_partitions_at(&pool, 0, true, fixed_now()).await;
            assert!(
                matches!(result, Err(DbError::InvalidData(ref message)) if message.contains("without the expected attachment and bounds")),
                "wrong-bound canonical name must not be counted as created: {result:?}"
            );

            let audit = audit_partition_catalog_at(&pool, 0, fixed_now())
                .await
                .expect("audit");
            let events = audit
                .tables
                .iter()
                .find(|table| table.table == "events")
                .expect("events audit");
            assert_eq!(events.months[0].kind, MonthCoverageKind::Uncovered);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn canonical_name_attached_to_wrong_parent_is_a_real_creation_error() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
            }
            sqlx::query(
                "CREATE TABLE unrelated_events (created_at TIMESTAMPTZ NOT NULL) \
                 PARTITION BY RANGE (created_at)",
            )
            .execute(&pool)
            .await
            .expect("create unrelated parent");
            create_child(
                &pool,
                "unrelated_events",
                "events_p2026_09",
                "'2026-09-01'",
                "'2026-10-01'",
            )
            .await;

            let result = ensure_future_partitions_at(&pool, 0, true, fixed_now()).await;
            assert!(
                matches!(result, Err(DbError::InvalidData(ref message)) if message.contains("events_p2026_09 already exists without the expected attachment and bounds")),
                "wrong-parent canonical name must not be counted as created: {result:?}"
            );
            let attached_to_events: bool = sqlx::query_scalar(
                r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM pg_catalog.pg_inherits inherited
                    JOIN pg_catalog.pg_class parent ON parent.oid = inherited.inhparent
                    JOIN pg_catalog.pg_class child ON child.oid = inherited.inhrelid
                    JOIN pg_catalog.pg_namespace parent_namespace
                      ON parent_namespace.oid = parent.relnamespace
                    JOIN pg_catalog.pg_namespace child_namespace
                      ON child_namespace.oid = child.relnamespace
                    WHERE parent_namespace.nspname = $1
                      AND child_namespace.nspname = $1
                      AND parent.relname = 'events'
                      AND child.relname = 'events_p2026_09'
                )
                "#,
            )
            .bind(&schema)
            .fetch_one(&pool)
            .await
            .expect("check attachment");
            assert!(!attached_to_events);
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn unrelated_canonical_relation_is_a_real_creation_error() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
            }
            sqlx::query("CREATE TABLE events_p2026_09 (unrelated BOOLEAN NOT NULL)")
                .execute(&pool)
                .await
                .expect("create unrelated canonical relation");

            let result = ensure_future_partitions_at(&pool, 0, true, fixed_now()).await;
            assert!(
                matches!(result, Err(DbError::InvalidData(ref message)) if message.contains("events_p2026_09 already exists without the expected attachment and bounds")),
                "unrelated canonical relation must not be counted as created: {result:?}"
            );
            assert!(relation_name_exists(&pool, "events_p2026_09")
                .await
                .expect("query unrelated relation"));
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn anomalous_child_and_trigger_mismatch_degrade_without_aborting() {
            let (pool, admin, schema) = scratch_pool().await;
            seed_parents(&pool).await;
            for table in PARTITIONED_TABLES {
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_past"),
                    "MINVALUE",
                    "'2026-09-01'",
                )
                .await;
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p2099_01"),
                    "'2026-09-01'",
                    "'2026-10-01'",
                )
                .await;
                create_child(
                    &pool,
                    table,
                    &format!("{table}_p_future"),
                    "'2026-10-01'",
                    "MAXVALUE",
                )
                .await;
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "CREATE TRIGGER child_only_probe BEFORE INSERT ON {table}_p2099_01 \
                     FOR EACH ROW EXECUTE FUNCTION partition_test_trigger()"
                )))
                .execute(&pool)
                .await
                .expect("create child-only trigger");
            }
            let audit = audit_partition_catalog_at(&pool, 1, fixed_now())
                .await
                .expect("audit");
            assert!(audit.serving_safe());
            assert!(audit.tables.iter().all(|table| {
                table.degraded()
                    && table.anomalous_children() == 1
                    && table.missing_trigger_count() == 0
                    && table
                        .children
                        .iter()
                        .any(|child| child.extra_triggers == ["child_only_probe"])
            }));
            drop_schema(&admin, &schema).await;
        }
    }
}
