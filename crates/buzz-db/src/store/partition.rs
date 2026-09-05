//! Monthly partition manager for `events` and `delivery_log`.
//!
//! Call `ensure_future_partitions` on startup and monthly via cron.

use buzz_datastore_tracing::datastore_span;
use chrono::{Datelike, TimeZone, Utc};
use sqlx::{Connection, PgPool};
use tracing::info;

use crate::error::{DbError, Result};
use crate::Db;

/// Tables that may be partition-managed. Allowlist prevents DDL injection.
const PARTITIONED_TABLES: &[&str] = &["events", "delivery_log"];

/// Ensures monthly partition tables exist for the next `months_ahead` months.
pub async fn ensure_future_partitions(pool: &PgPool, months_ahead: u32) -> Result<()> {
    let now = Utc::now();
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Bootstrap,
    )
    .await?;

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
            ensure_partition(&mut connection, table, &start_str, &end_str, &suffix).await?;
        }
    }

    Ok(())
}

impl Db {
    /// Ensures monthly partitions exist for the next N months.
    #[datastore_span(name = "ensure_future_partitions", system = "postgresql")]
    pub async fn ensure_future_partitions(&self, months_ahead: u32) -> Result<()> {
        ensure_future_partitions(&self.pool, months_ahead).await
    }
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
    connection: &mut sqlx::PgConnection,
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

    if partition_range_covered(connection, table_name, start_date_str, end_date_str).await? {
        return Ok(());
    }

    // DDL identifiers cannot be parameterized -- all inputs are validated above.
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {partition_name} PARTITION OF {table_name} \
         FOR VALUES FROM ('{start_date_str}') TO ('{end_date_str}')"
    );

    // SET LOCAL applies only to this transaction, and the DDL must use the same
    // connection for the timeout to bound its parent-table lock wait.
    let mut tx = connection.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '2s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL TIME ZONE 'UTC'")
        .execute(&mut *tx)
        .await?;
    let result = sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(&mut *tx)
        .await;

    match result {
        Ok(_) => {
            tx.commit().await?;
            info!("added partition {partition_name}");
            Ok(())
        }
        Err(error) => {
            let is_overlap = matches!(
                &error,
                sqlx::Error::Database(db_error)
                    if db_error.code().as_deref() == Some("42P17")
                        && db_error.message().contains("would overlap partition")
            );
            tx.rollback().await?;
            if is_overlap
                && partition_range_covered(connection, table_name, start_date_str, end_date_str)
                    .await?
            {
                // A concurrent creator can cover the range between the catalog
                // pre-check and this DDL.
                info!(
                    partition_name,
                    "partition range already covered by an existing partition"
                );
                Ok(())
            } else {
                Err(error.into())
            }
        }
    }
}

async fn partition_range_covered(
    connection: &mut sqlx::PgConnection,
    table_name: &str,
    start_date_str: &str,
    end_date_str: &str,
) -> Result<bool> {
    let (all_bounds_recognized, covered): (bool, bool) = sqlx::query_as(
        r#"
        WITH partition_nodes AS (
            SELECT
                tree.level,
                tree.isleaf,
                pg_get_expr(c.relpartbound, c.oid, false) AS bound
            FROM pg_catalog.pg_partition_tree(to_regclass($1)) AS tree
            JOIN pg_catalog.pg_class c ON c.oid = tree.relid
            WHERE tree.level > 0
        ),
        partition_bounds AS (
            SELECT bound
            FROM partition_nodes
            WHERE level = 1 AND isleaf
        ),
        extracted_bounds AS (
            SELECT
                bound,
                regexp_match(
                    bound,
                    $partition_bound$^FOR VALUES FROM \((MINVALUE|'[^']+')\) TO \((MAXVALUE|'[^']+')\)$$partition_bound$
                ) AS parts
            FROM partition_bounds
        ),
        parsed_bounds AS (
            SELECT
                bound,
                CASE
                    WHEN parts[1] = 'MINVALUE'
                        THEN '-infinity'::timestamptz
                    WHEN parts IS NOT NULL
                        THEN trim(both '''' from parts[1])::timestamptz
                END AS start_at,
                CASE
                    WHEN parts[2] = 'MAXVALUE'
                        THEN 'infinity'::timestamptz
                    WHEN parts IS NOT NULL
                        THEN trim(both '''' from parts[2])::timestamptz
                END AS end_at
            FROM extracted_bounds
        )
        SELECT
            COALESCE(
                (SELECT bool_and(level = 1 AND isleaf) FROM partition_nodes),
                true
            )
            AND COALESCE(bool_and(bound = 'DEFAULT' OR parts IS NOT NULL), true),
            EXISTS (
                SELECT 1
                FROM parsed_bounds
                WHERE bound = 'DEFAULT'
                   OR (
                       start_at <= to_date($2, 'YYYY-MM-DD')::timestamp AT TIME ZONE 'UTC'
                       AND end_at >= to_date($3, 'YYYY-MM-DD')::timestamp AT TIME ZONE 'UTC'
                   )
            )
        FROM extracted_bounds
        "#,
    )
    .bind(table_name)
    .bind(start_date_str)
    .bind(end_date_str)
    .fetch_one(&mut *connection)
    .await?;

    if !all_bounds_recognized {
        return Err(DbError::InvalidData(format!(
            "unsupported partition topology or bound for table {table_name:?}"
        )));
    }

    Ok(covered)
}

#[cfg(test)]
mod postgres_tests {
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

    async fn admin_pool() -> PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL must point to a database where tests can create databases");
        PgPool::connect(&url).await.expect("connect admin database")
    }

    async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
        create_scratch_db_with_timezone(admin, prefix, "UTC").await
    }

    async fn create_scratch_db_with_timezone(
        admin: &PgPool,
        prefix: &str,
        timezone: &str,
    ) -> (PgPool, String) {
        let name = format!("{prefix}_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch database");

        let admin_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let path_start = admin_url
            .rfind('/')
            .expect("database URL has a path segment");
        let scratch_url = format!("{}/{}", &admin_url[..path_start], name);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&scratch_url)
            .await
            .expect("connect scratch database");
        let mut first = pool.acquire().await.expect("acquire first connection");
        let mut second = pool.acquire().await.expect("acquire second connection");
        for connection in [&mut *first, &mut *second] {
            sqlx::query("SELECT set_config('TimeZone', $1, false)")
                .bind(timezone)
                .execute(connection)
                .await
                .expect("set test connection timezone");
        }
        drop(first);
        drop(second);
        create_partitioned_tables(&pool).await;
        (pool, name)
    }

    async fn create_partitioned_tables(pool: &PgPool) {
        for statement in [
            "CREATE TABLE events (created_at TIMESTAMPTZ NOT NULL) PARTITION BY RANGE (created_at)",
            "CREATE TABLE events_p_past PARTITION OF events FOR VALUES FROM (MINVALUE) TO ('2000-01-01')",
            "CREATE TABLE events_p_future PARTITION OF events FOR VALUES FROM ('2000-01-01') TO (MAXVALUE)",
            "CREATE TABLE delivery_log (delivered_at TIMESTAMPTZ NOT NULL) PARTITION BY RANGE (delivered_at)",
            "CREATE TABLE delivery_log_p_past PARTITION OF delivery_log FOR VALUES FROM (MINVALUE) TO ('2000-01-01')",
            "CREATE TABLE delivery_log_p_future PARTITION OF delivery_log FOR VALUES FROM ('2000-01-01') TO (MAXVALUE)",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .expect("create partitioned test table");
        }
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn existing_catch_all_bounds_avoid_partition_ddl() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_coverage").await;
        let mut blocker = pool.begin().await.expect("begin blocker");
        sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
            .execute(&mut *blocker)
            .await
            .expect("lock partition parent");

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            ensure_future_partitions(&pool, 0),
        )
        .await;

        blocker.rollback().await.expect("release parent lock");
        drop_scratch_db(&admin, pool, &name).await;
        result
            .expect("coverage pre-check must return without waiting for a DDL lock")
            .expect("covered partition range is already ensured");
    }

    fn is_lock_timeout(error: &DbError) -> bool {
        matches!(
            error,
            DbError::Sqlx(sqlx::Error::Database(db_error))
                if db_error.code().as_deref() == Some("55P03")
        )
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn blocked_partition_ddl_stops_at_lock_timeout() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_lock_timeout").await;
        sqlx::query("DROP TABLE events_p_future")
            .execute(&pool)
            .await
            .expect("remove partition coverage");
        let mut blocker = pool.begin().await.expect("begin blocker");
        sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
            .execute(&mut *blocker)
            .await
            .expect("lock partition parent");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            ensure_future_partitions(&pool, 0),
        )
        .await;

        blocker.rollback().await.expect("release parent lock");
        drop_scratch_db(&admin, pool, &name).await;
        let error = result
            .expect("partition DDL must stop at the transaction-local lock timeout")
            .expect_err("blocked partition DDL must return a lock timeout");
        assert!(is_lock_timeout(&error), "unexpected error: {error}");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn missing_partition_is_created_with_production_bounds_in_non_utc_session() {
        let admin = admin_pool().await;
        let (pool, name) =
            create_scratch_db_with_timezone(&admin, "partition_missing", "America/Los_Angeles")
                .await;
        sqlx::query("DROP TABLE events_p_future, delivery_log_p_future")
            .execute(&pool)
            .await
            .expect("remove catch-all partitions");

        ensure_future_partitions(&pool, 0)
            .await
            .expect("create genuinely missing monthly partitions");

        let now = Utc::now();
        let suffix = format!("{:04}_{:02}", now.year(), now.month());
        let start_str = format!("{:04}-{:02}-01", now.year(), now.month());
        let (end_year, end_month) = if now.month() == 12 {
            (now.year() + 1, 1)
        } else {
            (now.year(), now.month() + 1)
        };
        let end_str = format!("{end_year:04}-{end_month:02}-01");
        let mut connection = pool.acquire().await.expect("acquire test connection");
        for table in PARTITIONED_TABLES {
            let partition_name = format!("{table}_p{suffix}");
            let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
                .bind(&partition_name)
                .fetch_one(&pool)
                .await
                .expect("check created partition");
            assert!(exists, "missing partition {partition_name}");
            let covered = partition_range_covered(&mut connection, table, &start_str, &end_str)
                .await
                .expect("inspect created partition bounds");
            assert!(covered, "partition {partition_name} must use UTC bounds");
        }

        drop(connection);
        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn partial_overlap_does_not_count_as_coverage() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_partial_overlap").await;
        sqlx::query("DROP TABLE events_p_future")
            .execute(&pool)
            .await
            .expect("remove events catch-all partition");

        let now = Utc::now();
        let start = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .expect("current month starts at a valid date");
        let middle = Utc
            .with_ymd_and_hms(now.year(), now.month(), 15, 0, 0, 0)
            .single()
            .expect("current month has a fifteenth day");
        let sql = format!(
            "CREATE TABLE events_partial PARTITION OF events \
             FOR VALUES FROM ('{}') TO ('{}')",
            start.to_rfc3339(),
            middle.to_rfc3339()
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .expect("create partially overlapping partition");

        let error = ensure_future_partitions(&pool, 0)
            .await
            .expect_err("partial overlap must not be accepted as full coverage");
        assert!(
            matches!(
                error,
                DbError::Sqlx(sqlx::Error::Database(ref db_error))
                    if db_error.code().as_deref() == Some("42P17")
            ),
            "unexpected error: {error}"
        );

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn default_partition_avoids_partition_ddl() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_default").await;
        sqlx::query("DROP TABLE events_p_future, delivery_log_p_future")
            .execute(&pool)
            .await
            .expect("remove catch-all partitions");
        for statement in [
            "CREATE TABLE events_p_default PARTITION OF events DEFAULT",
            "CREATE TABLE delivery_log_p_default PARTITION OF delivery_log DEFAULT",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create default partition");
        }
        let mut blocker = pool.begin().await.expect("begin blocker");
        sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
            .execute(&mut *blocker)
            .await
            .expect("lock partition parent");

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            ensure_future_partitions(&pool, 0),
        )
        .await;

        blocker.rollback().await.expect("release parent lock");
        drop_scratch_db(&admin, pool, &name).await;
        result
            .expect("default coverage must return without waiting for a DDL lock")
            .expect("default partitions cover the target range");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unrecognized_partition_bounds_fail_closed() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_unrecognized").await;
        sqlx::query("DROP TABLE events CASCADE")
            .execute(&pool)
            .await
            .expect("replace events test table");
        for statement in [
            "CREATE TABLE events (created_at TIMESTAMPTZ NOT NULL, sequence INT NOT NULL) \
             PARTITION BY RANGE (created_at, sequence)",
            "CREATE TABLE events_multicolumn PARTITION OF events \
             FOR VALUES FROM (MINVALUE, MINVALUE) TO (MAXVALUE, MAXVALUE)",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create unsupported partition topology");
        }

        let error = ensure_future_partitions(&pool, 0)
            .await
            .expect_err("unsupported bounds must fail closed before DDL");
        assert!(
            matches!(
                error,
                DbError::InvalidData(ref message)
                    if message.contains("unsupported partition")
            ),
            "unexpected error: {error}"
        );

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn nested_partition_bounds_fail_closed() {
        let admin = admin_pool().await;
        let (pool, name) = create_scratch_db(&admin, "partition_nested").await;
        sqlx::query("DROP TABLE events CASCADE")
            .execute(&pool)
            .await
            .expect("replace events test table");
        for statement in [
            "CREATE TABLE events (created_at TIMESTAMPTZ NOT NULL, sequence INT NOT NULL) \
             PARTITION BY RANGE (created_at)",
            "CREATE TABLE events_all PARTITION OF events \
             FOR VALUES FROM (MINVALUE) TO (MAXVALUE) PARTITION BY RANGE (sequence)",
            "CREATE TABLE events_nested_default PARTITION OF events_all DEFAULT",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create nested partition topology");
        }

        let error = ensure_future_partitions(&pool, 0)
            .await
            .expect_err("nested bounds must fail closed before DDL");
        assert!(
            matches!(
                error,
                DbError::InvalidData(ref message)
                    if message.contains("unsupported partition")
            ),
            "unexpected error: {error}"
        );

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn exact_utc_bounds_are_covered_in_non_utc_session() {
        let admin = admin_pool().await;
        let (pool, name) =
            create_scratch_db_with_timezone(&admin, "partition_non_utc", "America/Los_Angeles")
                .await;
        let timezone: String = sqlx::query_scalar("SHOW timezone")
            .fetch_one(&pool)
            .await
            .expect("read scratch database timezone");
        assert_eq!(timezone, "America/Los_Angeles");
        sqlx::query("DROP TABLE events_p_future, delivery_log_p_future")
            .execute(&pool)
            .await
            .expect("remove catch-all partitions");

        let now = Utc::now();
        let start = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .expect("current month starts at a valid date");
        let (end_year, end_month) = if now.month() == 12 {
            (now.year() + 1, 1)
        } else {
            (now.year(), now.month() + 1)
        };
        let end = Utc
            .with_ymd_and_hms(end_year, end_month, 1, 0, 0, 0)
            .single()
            .expect("next month starts at a valid date");
        for statement in [
            format!(
                "CREATE TABLE events_existing_range PARTITION OF events \
                 FOR VALUES FROM ('{}') TO ('{}')",
                start.to_rfc3339(),
                end.to_rfc3339()
            ),
            format!(
                "CREATE TABLE delivery_log_existing_range PARTITION OF delivery_log \
                 FOR VALUES FROM ('{}') TO ('{}')",
                start.to_rfc3339(),
                end.to_rfc3339()
            ),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&pool)
                .await
                .expect("create exact UTC monthly partition");
        }

        let mut blocker = pool.begin().await.expect("begin blocker");
        sqlx::query("LOCK TABLE events IN ACCESS SHARE MODE")
            .execute(&mut *blocker)
            .await
            .expect("lock partition parent");
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            ensure_future_partitions(&pool, 0),
        )
        .await;

        blocker.rollback().await.expect("release parent lock");
        drop_scratch_db(&admin, pool, &name).await;
        result
            .expect("UTC partition coverage must not depend on the session timezone")
            .expect("exact UTC monthly partitions are already ensured");
    }
}
