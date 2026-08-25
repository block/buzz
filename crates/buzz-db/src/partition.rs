//! Monthly partition manager for `events` and `delivery_log`.
//!
//! Call `ensure_future_partitions` on startup and monthly via cron.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sqlx::{PgPool, Row};
use tracing::info;

use crate::error::{DbError, Result};

/// Tables that may be partition-managed. Allowlist prevents DDL injection.
const PARTITIONED_TABLES: &[&str] = &["events", "delivery_log"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PartitionSpec {
    table: &'static str,
    start_date: String,
    end_date: String,
    suffix: String,
    partition_name: String,
}

/// Ensures monthly partition tables exist for the next `months_ahead` months.
pub async fn ensure_future_partitions(pool: &PgPool, months_ahead: u32) -> Result<()> {
    let specs = partition_specs(Utc::now(), months_ahead)?;
    let missing = missing_partitions(pool, &specs).await?;

    if missing.is_empty() {
        return Ok(());
    }

    info!(
        count = missing.len(),
        "partitions missing, proceeding with DDL"
    );
    let missing: HashSet<&str> = missing.iter().map(String::as_str).collect();
    for spec in specs {
        if missing.contains(spec.partition_name.as_str()) {
            ensure_partition(
                pool,
                spec.table,
                &spec.start_date,
                &spec.end_date,
                &spec.suffix,
            )
            .await?;
        }
    }

    Ok(())
}

fn partition_specs(now: DateTime<Utc>, months_ahead: u32) -> Result<Vec<PartitionSpec>> {
    let mut specs = Vec::with_capacity((months_ahead as usize + 1) * PARTITIONED_TABLES.len());

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
        let start_date = start.format("%Y-%m-%d").to_string();
        let end_date = end.format("%Y-%m-%d").to_string();

        for table in PARTITIONED_TABLES {
            specs.push(PartitionSpec {
                table,
                start_date: start_date.clone(),
                end_date: end_date.clone(),
                suffix: suffix.clone(),
                partition_name: format!("{table}_p{suffix}"),
            });
        }
    }

    Ok(specs)
}

async fn missing_partitions(pool: &PgPool, specs: &[PartitionSpec]) -> Result<Vec<String>> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }

    let candidates: Vec<String> = specs
        .iter()
        .map(|spec| spec.partition_name.clone())
        .collect();
    let expected: HashMap<&str, &PartitionSpec> = specs
        .iter()
        .map(|spec| (spec.partition_name.as_str(), spec))
        .collect();
    let existing = expected_partition_names(pool, &candidates, |name, parent, bound| {
        expected.get(name).is_some_and(|spec| {
            partition_identity_matches(parent, bound, spec.table, &spec.start_date, &spec.end_date)
        })
    })
    .await?;

    Ok(missing_from_existing(&candidates, &existing))
}

async fn expected_partition_names(
    pool: &PgPool,
    candidates: &[String],
    matches_expected: impl Fn(&str, &str, &str) -> bool,
) -> Result<Vec<String>> {
    let rows = sqlx::query(
        r#"
        SELECT c.relname::text AS partition_name,
               p.relname::text AS parent_name,
               pg_get_expr(c.relpartbound, c.oid, true)::text AS partition_bound
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_catalog.pg_inherits i ON i.inhrelid = c.oid
        JOIN pg_catalog.pg_class p ON p.oid = i.inhparent
        JOIN pg_catalog.pg_namespace pn ON pn.oid = p.relnamespace
        WHERE n.nspname = current_schema()
          AND pn.nspname = current_schema()
          AND c.relispartition = true
          AND c.relname = ANY($1)
        "#,
    )
    .bind(candidates)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            let partition_name: String = row.try_get("partition_name")?;
            let parent_name: String = row.try_get("parent_name")?;
            let partition_bound: String = row.try_get("partition_bound")?;
            Ok((partition_name, parent_name, partition_bound))
        })
        .filter_map(|row: Result<(String, String, String)>| match row {
            Ok((partition_name, parent_name, partition_bound))
                if matches_expected(&partition_name, &parent_name, &partition_bound) =>
            {
                Some(Ok(partition_name))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

async fn expected_partition_exists(
    pool: &PgPool,
    partition_name: &str,
    parent_name: &str,
    start_date_str: &str,
    end_date_str: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT p.relname::text AS parent_name,
               pg_get_expr(c.relpartbound, c.oid, true)::text AS partition_bound
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_catalog.pg_inherits i ON i.inhrelid = c.oid
        JOIN pg_catalog.pg_class p ON p.oid = i.inhparent
        JOIN pg_catalog.pg_namespace pn ON pn.oid = p.relnamespace
        WHERE n.nspname = current_schema()
          AND pn.nspname = current_schema()
          AND c.relispartition = true
          AND c.relname = $1
        "#,
    )
    .bind(partition_name)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };
    let actual_parent: String = row.try_get("parent_name")?;
    let partition_bound: String = row.try_get("partition_bound")?;
    Ok(partition_identity_matches(
        &actual_parent,
        &partition_bound,
        parent_name,
        start_date_str,
        end_date_str,
    ))
}

async fn expected_partition_exists_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    partition_name: &str,
    parent_name: &str,
    start_date_str: &str,
    end_date_str: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT p.relname::text AS parent_name,
               pg_get_expr(c.relpartbound, c.oid, true)::text AS partition_bound
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_catalog.pg_inherits i ON i.inhrelid = c.oid
        JOIN pg_catalog.pg_class p ON p.oid = i.inhparent
        JOIN pg_catalog.pg_namespace pn ON pn.oid = p.relnamespace
        WHERE n.nspname = current_schema()
          AND pn.nspname = current_schema()
          AND c.relispartition = true
          AND c.relname = $1
        "#,
    )
    .bind(partition_name)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };
    let actual_parent: String = row.try_get("parent_name")?;
    let partition_bound: String = row.try_get("partition_bound")?;
    Ok(partition_identity_matches(
        &actual_parent,
        &partition_bound,
        parent_name,
        start_date_str,
        end_date_str,
    ))
}

fn partition_identity_matches(
    actual_parent: &str,
    partition_bound: &str,
    expected_parent: &str,
    expected_start: &str,
    expected_end: &str,
) -> bool {
    actual_parent == expected_parent
        && partition_bound.contains(&format!("FROM ('{expected_start}"))
        && partition_bound.contains(&format!("TO ('{expected_end}"))
}

fn missing_from_existing(candidates: &[String], existing: &[String]) -> Vec<String> {
    let existing: HashSet<&str> = existing.iter().map(String::as_str).collect();
    candidates
        .iter()
        .filter(|name| !existing.contains(name.as_str()))
        .cloned()
        .collect()
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

    if expected_partition_exists(
        pool,
        &partition_name,
        table_name,
        start_date_str,
        end_date_str,
    )
    .await?
    {
        return Ok(());
    }

    // SET LOCAL must run on the same transaction/connection as the DDL.
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '2s'")
        .execute(&mut *tx)
        .await?;

    if expected_partition_exists_in_tx(
        &mut tx,
        &partition_name,
        table_name,
        start_date_str,
        end_date_str,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(());
    }

    // DDL identifiers cannot be parameterized -- all inputs are validated above.
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {partition_name} PARTITION OF {table_name} \
         FOR VALUES FROM ('{start_date_str}') TO ('{end_date_str}')"
    );

    match sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(&mut *tx)
        .await
    {
        Ok(_) => {
            tx.commit().await?;
            if expected_partition_exists(
                pool,
                &partition_name,
                table_name,
                start_date_str,
                end_date_str,
            )
            .await?
            {
                info!("added partition {partition_name}");
                Ok(())
            } else {
                Err(DbError::InvalidData(format!(
                    "relation {partition_name:?} exists but is not the expected partition of {table_name:?}"
                )))
            }
        }
        Err(e) if is_duplicate_table(&e) => {
            // A concurrent creator can commit between the in-transaction catalog
            // re-check and this DDL. `IF NOT EXISTS` is not enough for partition
            // DDL on PostgreSQL, and SQLSTATE 42P07 only proves a same-name
            // relation exists. Roll back the aborted transaction, then accept
            // only if the committed relation is the intended partition.
            tx.rollback().await?;
            if expected_partition_exists(
                pool,
                &partition_name,
                table_name,
                start_date_str,
                end_date_str,
            )
            .await?
            {
                info!(partition_name, "partition was created concurrently");
                Ok(())
            } else {
                Err(e.into())
            }
        }
        Err(sqlx::Error::Database(db_err))
            if db_err.code().as_deref() == Some("42P17")
                && db_err.message().contains("would overlap partition") =>
        {
            // Fresh schemas include a right-edge catch-all partition (`*_p_future`).
            // If it already covers this month, the table is still safe for writes;
            // treat the overlap as "ensured" rather than failing startup.
            info!(
                partition_name,
                "partition range already covered by an existing partition"
            );
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn is_duplicate_table(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("42P07")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn partition_specs_include_each_table_for_each_month() {
        let now = Utc.with_ymd_and_hms(2026, 12, 15, 0, 0, 0).unwrap();
        let specs = partition_specs(now, 1).expect("partition specs");

        assert_eq!(
            specs,
            vec![
                PartitionSpec {
                    table: "events",
                    start_date: "2026-12-01".into(),
                    end_date: "2027-01-01".into(),
                    suffix: "2026_12".into(),
                    partition_name: "events_p2026_12".into(),
                },
                PartitionSpec {
                    table: "delivery_log",
                    start_date: "2026-12-01".into(),
                    end_date: "2027-01-01".into(),
                    suffix: "2026_12".into(),
                    partition_name: "delivery_log_p2026_12".into(),
                },
                PartitionSpec {
                    table: "events",
                    start_date: "2027-01-01".into(),
                    end_date: "2027-02-01".into(),
                    suffix: "2027_01".into(),
                    partition_name: "events_p2027_01".into(),
                },
                PartitionSpec {
                    table: "delivery_log",
                    start_date: "2027-01-01".into(),
                    end_date: "2027-02-01".into(),
                    suffix: "2027_01".into(),
                    partition_name: "delivery_log_p2027_01".into(),
                },
            ]
        );
    }

    #[test]
    fn missing_from_existing_returns_empty_when_all_candidates_exist() {
        let candidates = vec!["events_p2026_08".into(), "delivery_log_p2026_08".into()];
        let existing = candidates.clone();

        assert!(missing_from_existing(&candidates, &existing).is_empty());
    }

    #[test]
    fn missing_from_existing_preserves_only_missing_candidates_in_order() {
        let candidates = vec![
            "events_p2026_08".into(),
            "delivery_log_p2026_08".into(),
            "events_p2026_09".into(),
        ];
        let existing = vec!["events_p2026_09".into(), "events_p2026_08".into()];

        assert_eq!(
            missing_from_existing(&candidates, &existing),
            vec!["delivery_log_p2026_08".to_string()]
        );
    }

    async fn admin_pool() -> PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        PgPool::connect(&url).await.expect("connect admin database")
    }

    async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
        let name = format!("{}_{}", prefix, uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch database");

        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let idx = admin_url.rfind('/').expect("database URL has path segment");
        let scratch_url = format!("{}/{}", &admin_url[..idx], name);
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");
        drop_future_partitions(&pool).await;
        (pool, name)
    }

    async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(admin)
        .await
        .expect("drop scratch database");
    }

    async fn drop_future_partitions(pool: &PgPool) {
        sqlx::query("DROP TABLE IF EXISTS events_p_future, delivery_log_p_future")
            .execute(pool)
            .await
            .expect("drop future partitions");
    }

    async fn assert_current_partitions_exist(pool: &PgPool) {
        let specs = partition_specs(Utc::now(), 0).expect("partition specs");
        let missing = missing_partitions(pool, &specs)
            .await
            .expect("query missing partitions");
        assert!(missing.is_empty(), "missing partitions: {missing:?}");
    }

    fn is_lock_timeout(error: &DbError) -> bool {
        matches!(
            error,
            DbError::Sqlx(sqlx::Error::Database(db_err))
                if db_err.code().as_deref() == Some("55P03")
        )
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn ensure_future_partitions_fast_path_does_not_wait_on_parent_lock() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partitions_all_present").await;

        ensure_future_partitions(&pool, 0)
            .await
            .expect("seed current partitions");
        let mut blocker = pool.begin().await.expect("begin blocker transaction");
        sqlx::query("SELECT 1 FROM events LIMIT 0")
            .execute(&mut *blocker)
            .await
            .expect("hold ACCESS SHARE lock on parent");

        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            ensure_future_partitions(&pool, 0),
        )
        .await
        .expect("all-present preflight should not wait for DDL locks")
        .expect("all-present ensure succeeds");

        blocker.rollback().await.expect("rollback blocker");
        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn same_name_regular_table_collision_is_not_accepted() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partitions_bad_collision").await;
        let specs = partition_specs(Utc::now(), 0).expect("partition specs");
        let events = specs
            .iter()
            .find(|spec| spec.table == "events")
            .expect("events spec");

        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE {} (id integer)",
            events.partition_name
        )))
        .execute(&pool)
        .await
        .expect("create unsafe same-name regular table");

        let error = ensure_future_partitions(&pool, 0)
            .await
            .expect_err("regular table collision must not be treated as ensured");
        assert!(
            error.to_string().contains("is not the expected partition"),
            "unexpected error: {error}"
        );
        assert!(
            !expected_partition_exists(
                &pool,
                &events.partition_name,
                events.table,
                &events.start_date,
                &events.end_date,
            )
            .await
            .expect("query partition identity"),
            "regular table must not verify as the expected partition"
        );

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn missing_partition_lock_timeout_is_bounded_and_recovers() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partitions_lock_timeout").await;
        let mut blocker = pool.begin().await.expect("begin blocker transaction");
        sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
            .execute(&mut *blocker)
            .await
            .expect("hold parent lock that conflicts with partition DDL");

        let started = tokio::time::Instant::now();
        let error = ensure_future_partitions(&pool, 0)
            .await
            .expect_err("blocked partition DDL must hit lock_timeout");
        let elapsed = started.elapsed();

        assert!(is_lock_timeout(&error), "unexpected error: {error}");
        assert!(
            elapsed >= std::time::Duration::from_millis(1500)
                && elapsed < std::time::Duration::from_secs(8),
            "lock_timeout should be near 2s, elapsed {elapsed:?}"
        );

        blocker.rollback().await.expect("release parent lock");
        ensure_future_partitions(&pool, 0)
            .await
            .expect("partition ensure recovers after timeout transaction rolls back");
        assert_current_partitions_exist(&pool).await;

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn ensure_future_partitions_creates_only_subset_missing() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partitions_subset_missing").await;
        let specs = partition_specs(Utc::now(), 0).expect("partition specs");
        let events = specs
            .iter()
            .find(|spec| spec.table == "events")
            .expect("events spec");

        ensure_partition(
            &pool,
            events.table,
            &events.start_date,
            &events.end_date,
            &events.suffix,
        )
        .await
        .expect("pre-create events partition only");

        ensure_future_partitions(&pool, 0)
            .await
            .expect("create missing delivery_log partition");
        assert_current_partitions_exist(&pool).await;

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_creation_race_is_idempotent() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partitions_race").await;

        let (left, right) = tokio::join!(
            ensure_future_partitions(&pool, 0),
            ensure_future_partitions(&pool, 0)
        );
        left.expect("left ensure succeeds");
        right.expect("right ensure succeeds");
        assert_current_partitions_exist(&pool).await;

        drop_scratch_db(&admin, pool, &name).await;
    }
}
