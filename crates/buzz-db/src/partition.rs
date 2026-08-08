//! Monthly partition manager for `events` and `delivery_log`.
//!
//! Call `ensure_future_partitions` on startup and monthly via cron.

use chrono::{Datelike, TimeZone, Utc};
use sqlx::{PgPool, Row};
use tracing::{info, warn};

use crate::error::{DbError, Result};

/// True when a `sqlx::Error` is a Postgres `42P17` ("would overlap partition")
/// raised by a `CREATE PARTITION` whose range is already covered. Fresh schemas
/// include a right-edge catch-all (`*_p_future`), so a fresh install collides
/// on the "current month" boundary it tries to add. Split out of
/// `ensure_partition` so the classification is unit-testable without Postgres.
fn is_partition_overlap_error(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => {
            db.code().as_deref() == Some("42P17")
                && db.message().contains("would overlap partition")
        }
        _ => false,
    }
}

/// Record #4033: a monthly partition attempt was absorbed by the `*_p_future`
/// catch-all (monthly partition NOT created). Metric so operators can alert
/// independent of log level.
fn catch_all_coverage_metric(table: &'static str) {
    metrics::counter!(
        "buzz_db_partition_catchall_coverage",
        "table" => table,
    )
    .increment(1);
}

/// Resolve a validated table name to its static allowlist entry so the metric
/// label escapes the caller's stack. `ensure_partition` already validated the
/// name against `PARTITIONED_TABLES`; `.expect` is unreachable for valid input.
fn static_table_name(table: &str) -> &'static str {
    PARTITIONED_TABLES
        .iter()
        .copied()
        .find(|t| *t == table)
        .expect("table name validated against PARTITIONED_TABLES")
}

/// Tables that may be partition-managed. Allowlist prevents DDL injection.
const PARTITIONED_TABLES: &[&str] = &["events", "delivery_log"];

/// Ensures monthly partition tables exist for the next `months_ahead` months.
pub async fn ensure_future_partitions(pool: &PgPool, months_ahead: u32) -> Result<()> {
    let now = Utc::now();

    for i in 0..=(months_ahead as i32) {
        let year = now.year();
        let month = now.month() as i32 + i;
        let (target_year, target_month) = if month > 12 {
            (year + (month - 1) / 12, ((month - 1) % 12 + 1) as u32)
        } else {
            (year, month as u32)
        };

        let (end_year, end_month) = if target_month == 12 {
            (target_year + 1, 1u32)
        } else {
            (target_year, target_month + 1)
        };

        let start = Utc
            .with_ymd_and_hms(target_year, target_month, 1, 0, 0, 0)
            .single()
            .ok_or_else(|| {
                DbError::InvalidData(format!("invalid date: {target_year}-{target_month:02}-01"))
            })?;
        let end = Utc
            .with_ymd_and_hms(end_year, end_month, 1, 0, 0, 0)
            .single()
            .ok_or_else(|| {
                DbError::InvalidData(format!("invalid date: {end_year}-{end_month:02}-01"))
            })?;

        let suffix = format!("{:04}_{:02}", target_year, target_month);
        let start_str = start.format("%Y-%m-%d").to_string();
        let end_str = end.format("%Y-%m-%d").to_string();

        for table in PARTITIONED_TABLES {
            ensure_partition(pool, table, &start_str, &end_str, &suffix).await?;
        }
    }

    Ok(())
}

/// Validate that a partition suffix is digits and underscores only.
fn validate_partition_suffix(suffix: &str) -> bool {
    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit() || c == '_')
}

/// Validate that a date string matches YYYY-MM-DD format.
fn validate_date_str(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..].iter().all(|b| b.is_ascii_digit())
}

async fn ensure_partition(
    pool: &PgPool,
    table_name: &str,
    start_date_str: &str,
    end_date_str: &str,
    suffix: &str,
) -> Result<()> {
    // Allowlist check -- parameterized queries cannot be used for DDL identifiers.
    if !PARTITIONED_TABLES.contains(&table_name) {
        return Err(DbError::InvalidData(format!(
            "table not in partition allowlist: {table_name:?}"
        )));
    }
    if !validate_partition_suffix(suffix) {
        return Err(DbError::InvalidData(format!(
            "partition suffix contains invalid characters: {suffix:?}"
        )));
    }
    if !validate_date_str(start_date_str) {
        return Err(DbError::InvalidData(format!(
            "start_date_str is not YYYY-MM-DD: {start_date_str:?}"
        )));
    }
    if !validate_date_str(end_date_str) {
        return Err(DbError::InvalidData(format!(
            "end_date_str is not YYYY-MM-DD: {end_date_str:?}"
        )));
    }

    let partition_name = format!("{table_name}_p{suffix}");

    let row = sqlx::query(
        r#"
        SELECT COUNT(*) as cnt
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = current_schema()
          AND c.relname = $1
          AND c.relispartition = true
        "#,
    )
    .bind(&partition_name)
    .fetch_one(pool)
    .await?;

    let cnt: i64 = row.try_get("cnt")?;
    if cnt > 0 {
        return Ok(());
    }

    // DDL identifiers cannot be parameterized -- all inputs are validated above.
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {partition_name} PARTITION OF {table_name} \
         FOR VALUES FROM ('{start_date_str}') TO ('{end_date_str}')"
    );

    match sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await {
        Ok(_) => {
            info!("added partition {partition_name}");
            Ok(())
        }
        Err(e) if is_partition_overlap_error(&e) => {
            // #4033: the `*_p_future` catch-all already covers this month, so the
            // monthly partition was NOT created and never will be — every row for
            // this range keeps landing in the catch-all. Postgres only logs an
            // unsuppressable server-side ERROR; the app must NOT surface this as a
            // silent `info!` success. Escalate to `warn!` and emit a metric so the
            // growing catch-all is alertable, but still return Ok: writes are safe
            // and startup must not fail.
            warn!(
                partition = %partition_name,
                table = %table_name,
                "monthly partition was NOT created: range already covered by the `*_p_future` catch-all (Postgres 42P17); rows for this range keep accumulating in the catch-all. Re-base or split the catch-all to restore monthly pruning/archival."
            );
            catch_all_coverage_metric(static_table_name(table_name));
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_predicate_classifies_42p17() {
        // We cannot easily construct a real sqlx::Error::Database without a
        // Postgres connection, so this documents the contract: the predicate
        // only matches sqlx::Error::Database variants whose code is 42P17 and
        // whose message contains "would overlap partition" (covered by the
        // 42P17-catch-all arm in `ensure_partition`); every other sqlx::Error
        // variant (PoolClosed here) must NOT match.
        let non_db = sqlx::Error::PoolClosed;
        assert!(!is_partition_overlap_error(&non_db));
    }

    #[test]
    fn catchall_metric_emits_under_local_recorder() {
        use metrics_util::debugging::DebuggingRecorder;

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let guard = metrics::set_default_local_recorder(&recorder);
        catch_all_coverage_metric("events");
        drop(guard);

        let hit = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_db_partition_catchall_coverage")
            .map(|(key, _, _, value)| {
                let metrics_util::debugging::DebugValue::Counter(n) = value else {
                    panic!("must be a counter");
                };
                let labels: Vec<_> = key.key().labels().collect();
                let table = labels
                    .iter()
                    .find(|l| l.key() == "table")
                    .map(|l| l.value().to_owned())
                    .unwrap_or_default();
                (table, n)
            })
            .collect::<Vec<_>>();

        assert_eq!(hit, vec![("events".to_owned(), 1)]);
    }

    #[test]
    fn suffix_validation() {
        assert!(validate_partition_suffix("2026_03"));
        assert!(validate_partition_suffix("9999_12"));
        assert!(!validate_partition_suffix(""));
        assert!(!validate_partition_suffix("2026-03"));
        assert!(!validate_partition_suffix("2026_03; DROP TABLE events--"));
    }

    #[test]
    fn date_str_validation() {
        assert!(validate_date_str("2026-03-01"));
        assert!(validate_date_str("9999-12-31"));
        assert!(!validate_date_str("2026-3-01"));
        assert!(!validate_date_str("2026/03/01"));
        assert!(!validate_date_str("20260301"));
        assert!(!validate_date_str("2026-03-01; DROP TABLE events--"));
    }

    #[test]
    fn table_allowlist() {
        assert!(PARTITIONED_TABLES.contains(&"events"));
        assert!(PARTITIONED_TABLES.contains(&"delivery_log"));
        assert!(!PARTITIONED_TABLES.contains(&"api_tokens"));
        assert!(!PARTITIONED_TABLES.contains(&"users"));
    }
}
