//! Monthly partition manager for `events` and `delivery_log`.
//!
//! `ensure_future_partitions` runs on relay startup and on a periodic tick.
//! Besides creating upcoming monthly partitions, it rolls the right-edge
//! catch-all (`*_p_future`) forward: without that, every month covered by the
//! catch-all is silently absorbed by it (issue #2396) — the overlap makes
//! plain `CREATE TABLE ... PARTITION OF` impossible, writes pile into one
//! ever-growing partition, and range pruning stops working.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use sqlx::{Connection, PgPool, Row};
use tracing::{info, warn};

use crate::error::{DbError, Result};

/// Tables that may be partition-managed, with the timestamp column each is
/// range-partitioned on. Allowlist prevents DDL injection.
const PARTITIONED_TABLES: &[(&str, &str)] =
    &[("events", "created_at"), ("delivery_log", "delivered_at")];

/// The partition-key column for an allowlisted table, or `None` when the
/// table is not partition-managed.
fn partition_column(table: &str) -> Option<&'static str> {
    PARTITIONED_TABLES
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, col)| *col)
}

/// Suffix of the right-edge catch-all partition created by the initial schema.
const CATCH_ALL_SUFFIX: &str = "p_future";

/// Ensures monthly partition tables exist for the current month plus the next
/// `months_ahead` months, rolling the catch-all partition forward when it
/// still covers any of those months.
///
/// Safe to call concurrently from multiple relay instances: the catch-all
/// roll is serialized by a transaction-scoped advisory lock, and losing the
/// race is treated as success (the winner does the work).
pub async fn ensure_future_partitions(pool: &PgPool, months_ahead: u32) -> Result<()> {
    let now = Utc::now();
    let mut first_err: Option<DbError> = None;
    for (table, _) in PARTITIONED_TABLES {
        if let Err(e) = roll_table_partitions(pool, table, now, months_ahead).await {
            warn!(table, "partition roll-forward failed: {e}");
            first_err.get_or_insert(e);
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Ensure the target months for one table, advancing the catch-all if it
/// still covers any of them.
async fn roll_table_partitions(
    pool: &PgPool,
    table: &str,
    now: DateTime<Utc>,
    months_ahead: u32,
) -> Result<()> {
    let mut conn = pool.acquire().await?;

    let mut any_covered = false;
    for i in 0..=(months_ahead as i32) {
        let (year, month) = add_months(now.year(), now.month(), i)?;
        match ensure_partition(&mut conn, table, year, month).await? {
            EnsureOutcome::Exists | EnsureOutcome::Created => {}
            EnsureOutcome::CoveredByOther => any_covered = true,
        }
    }
    if !any_covered {
        return Ok(());
    }

    // A target month is still absorbed by an overlapping partition — in the
    // shipped schema that is the `*_p_future` catch-all. Advance it so writes
    // land in real monthly partitions again.
    let (bound_year, bound_month) = add_months(now.year(), now.month(), months_ahead as i32 + 1)?;
    let new_lower_bound = month_start(bound_year, bound_month)?;
    drop(conn);
    advance_catch_all(pool, table, new_lower_bound).await
}

/// Outcome of a single-month partition ensure.
enum EnsureOutcome {
    /// The monthly partition already exists.
    Exists,
    /// The monthly partition was created.
    Created,
    /// Another partition's range covers this month (catch-all overlap) — the
    /// month has no dedicated partition and creating one is impossible until
    /// the covering partition is rolled forward.
    CoveredByOther,
}

/// Roll the `{table}_p_future` catch-all forward so it starts at
/// `new_lower_bound`, creating real monthly partitions for the range it
/// used to cover.
///
/// Runs as one transaction (writers block for its duration — DDL only, plus
/// one validation scan of the catch-all when it holds rows):
///
/// 1. `DETACH` the catch-all.
/// 2. If it is empty → drop it. If all its rows fall in one calendar month →
///    re-attach it as that month's partition (no rows are copied; the attach
///    scan validates the bounds). If rows span multiple months → roll back
///    and report — that layout needs the operator procedure documented in
///    issue #2396.
/// 3. Create the remaining monthly partitions from the old lower bound
///    through `new_lower_bound`.
/// 4. Recreate the catch-all as `FROM (new_lower_bound) TO (MAXVALUE)`.
async fn advance_catch_all(
    pool: &PgPool,
    table: &str,
    new_lower_bound: DateTime<Utc>,
) -> Result<()> {
    let Some(part_col) = partition_column(table) else {
        return Err(DbError::InvalidData(format!(
            "table not in partition allowlist: {table:?}"
        )));
    };
    let catch_all = format!("{table}_{CATCH_ALL_SUFFIX}");

    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;

    // Transaction-scoped advisory lock: serializes the roll across relay
    // instances and releases automatically on commit/rollback. Scoped to the
    // current schema so isolated test schemas don't contend with each other.
    let locked: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtextextended(current_schema() || ':partition_roll:' || $1, 0))",
    )
    .bind(table)
    .fetch_one(&mut *tx)
    .await?;
    if !locked {
        info!(table, "another instance is rolling partitions; skipping");
        return Ok(());
    }

    // Resolve the catch-all's lower bound from the catalog.
    let Some(bound_expr) = partition_bound_expr(&mut tx, &catch_all).await? else {
        warn!(
            table,
            "target month covered by an overlapping partition but no {catch_all} exists; \
             leaving the manual layout untouched"
        );
        return Ok(());
    };
    let Some(old_lower_bound) = parse_range_lower_bound(&bound_expr) else {
        return Err(DbError::InvalidData(format!(
            "cannot parse lower bound of {catch_all}: {bound_expr:?}"
        )));
    };
    if old_lower_bound >= new_lower_bound {
        // Someone already rolled it past our horizon.
        return Ok(());
    }

    // 1. Detach — from here to commit, the parent takes ACCESS EXCLUSIVE and
    // writers queue behind us.
    execute_ddl(
        &mut tx,
        format!("ALTER TABLE {table} DETACH PARTITION {catch_all}"),
    )
    .await?;

    // 2. Where do the absorbed rows go?
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT min({part_col}) AS min_at, max({part_col}) AS max_at FROM {catch_all}"
    )))
    .fetch_one(&mut *tx)
    .await?;
    let min_at: Option<DateTime<Utc>> = row.try_get("min_at")?;
    let max_at: Option<DateTime<Utc>> = row.try_get("max_at")?;

    let mut absorbed_month: Option<(i32, u32)> = None;
    match (min_at, max_at) {
        (None, None) => {
            execute_ddl(&mut tx, format!("DROP TABLE {catch_all}")).await?;
        }
        (Some(min_at), Some(max_at)) => {
            if max_at >= new_lower_bound {
                // The ingest fence bounds created_at near now, so this means
                // the roll horizon is somehow behind the data. Bail out; the
                // rollback re-attaches the catch-all untouched.
                return Err(DbError::InvalidData(format!(
                    "{catch_all} holds rows at {max_at} beyond the roll horizon {new_lower_bound}"
                )));
            }
            if (min_at.year(), min_at.month()) != (max_at.year(), max_at.month()) {
                return Err(DbError::InvalidData(format!(
                    "{catch_all} holds rows spanning {} through {} — more than one month; \
                     run the operator partition-surgery procedure from issue #2396",
                    min_at.format("%Y-%m"),
                    max_at.format("%Y-%m"),
                )));
            }
            // All rows in one calendar month: re-attach the old catch-all as
            // that month's partition. No rows move; the attach scan validates.
            let (year, month) = (min_at.year(), min_at.month());
            let name = partition_name(table, year, month)?;
            let (start, end) = month_bounds(year, month)?;
            execute_ddl(&mut tx, format!("ALTER TABLE {catch_all} RENAME TO {name}")).await?;
            execute_ddl(
                &mut tx,
                format!(
                    "ALTER TABLE {table} ATTACH PARTITION {name} \
                     FOR VALUES FROM ('{start}') TO ('{end}')"
                ),
            )
            .await?;
            absorbed_month = Some((year, month));
        }
        _ => {
            return Err(DbError::InvalidData(format!(
                "inconsistent min/max created_at for {catch_all}"
            )));
        }
    }

    // 3. Real monthly partitions for everything the catch-all used to cover.
    let (mut year, mut month) = (old_lower_bound.year(), old_lower_bound.month());
    while month_start(year, month)? < new_lower_bound {
        if absorbed_month != Some((year, month)) {
            create_month_partition_if_absent(&mut tx, table, year, month).await?;
        }
        (year, month) = add_months(year, month, 1)?;
    }

    // 4. Fresh catch-all so far-future writes keep a home.
    let bound_str = new_lower_bound.format("%Y-%m-%d").to_string();
    execute_ddl(
        &mut tx,
        format!(
            "CREATE TABLE {catch_all} PARTITION OF {table} \
             FOR VALUES FROM ('{bound_str}') TO (MAXVALUE)"
        ),
    )
    .await?;

    tx.commit().await?;
    info!(
        table,
        new_lower_bound = %bound_str,
        "rolled catch-all partition forward"
    );
    Ok(())
}

/// Ensure one month's partition exists, distinguishing "created"/"exists"
/// from "its range is absorbed by another partition".
async fn ensure_partition(
    conn: &mut sqlx::PgConnection,
    table: &str,
    year: i32,
    month: u32,
) -> Result<EnsureOutcome> {
    let name = partition_name(table, year, month)?;
    if partition_exists(conn, &name).await? {
        return Ok(EnsureOutcome::Exists);
    }
    match try_create_month_partition(conn, table, year, month).await {
        Ok(()) => {
            info!("added partition {name}");
            Ok(EnsureOutcome::Created)
        }
        Err(DbError::Sqlx(sqlx::Error::Database(db_err)))
            if db_err.code().as_deref() == Some("42P17")
                && db_err.message().contains("would overlap partition") =>
        {
            Ok(EnsureOutcome::CoveredByOther)
        }
        Err(e) => Err(e),
    }
}

/// Create one month's partition inside the roll transaction, skipping months
/// that already have one (e.g. re-runs after a partial manual roll).
async fn create_month_partition_if_absent(
    conn: &mut sqlx::PgConnection,
    table: &str,
    year: i32,
    month: u32,
) -> Result<()> {
    let name = partition_name(table, year, month)?;
    if partition_exists(conn, &name).await? {
        return Ok(());
    }
    try_create_month_partition(conn, table, year, month).await?;
    info!("added partition {name}");
    Ok(())
}

async fn try_create_month_partition(
    conn: &mut sqlx::PgConnection,
    table: &str,
    year: i32,
    month: u32,
) -> Result<()> {
    let name = partition_name(table, year, month)?;
    let (start, end) = month_bounds(year, month)?;
    execute_ddl(
        conn,
        format!(
            "CREATE TABLE IF NOT EXISTS {name} PARTITION OF {table} \
             FOR VALUES FROM ('{start}') TO ('{end}')"
        ),
    )
    .await
}

/// Whether a partition with this name exists in the current schema.
async fn partition_exists(conn: &mut sqlx::PgConnection, name: &str) -> Result<bool> {
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
    .bind(name)
    .fetch_one(&mut *conn)
    .await?;
    let cnt: i64 = row.try_get("cnt")?;
    Ok(cnt > 0)
}

/// The partition bound expression (`FOR VALUES FROM (...) TO (...)`) of a
/// partition in the current schema, or `None` if it does not exist.
async fn partition_bound_expr(conn: &mut sqlx::PgConnection, name: &str) -> Result<Option<String>> {
    let row = sqlx::query(
        r#"
        SELECT pg_catalog.pg_get_expr(c.relpartbound, c.oid) AS bound
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = current_schema()
          AND c.relname = $1
          AND c.relispartition = true
        "#,
    )
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(row) => Ok(row.try_get("bound")?),
        None => Ok(None),
    }
}

/// Parse the lower bound out of a range partition bound expression like
/// `FOR VALUES FROM ('2026-07-01 00:00:00+00') TO (MAXVALUE)`.
///
/// Returns `None` for `MINVALUE` lower bounds or unrecognized shapes.
fn parse_range_lower_bound(expr: &str) -> Option<DateTime<Utc>> {
    let after_from = expr.split("FROM ('").nth(1)?;
    let literal = after_from.split('\'').next()?;
    // pg_get_expr renders timestamptz literals as `YYYY-MM-DD HH:MM:SS+TZ`
    // (optionally with fractional seconds); date-only literals as `YYYY-MM-DD`.
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"] {
        if let Ok(dt) = DateTime::parse_from_str(literal, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(literal, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?));
    }
    None
}

/// Validated `{table}_pYYYY_MM` partition name.
fn partition_name(table: &str, year: i32, month: u32) -> Result<String> {
    if partition_column(table).is_none() {
        return Err(DbError::InvalidData(format!(
            "table not in partition allowlist: {table:?}"
        )));
    }
    let suffix = format!("{year:04}_{month:02}");
    if !validate_partition_suffix(&suffix) {
        return Err(DbError::InvalidData(format!(
            "partition suffix contains invalid characters: {suffix:?}"
        )));
    }
    Ok(format!("{table}_p{suffix}"))
}

/// `[first day of month, first day of next month)` as `YYYY-MM-DD` strings.
fn month_bounds(year: i32, month: u32) -> Result<(String, String)> {
    let start = month_start(year, month)?;
    let (next_year, next_month) = add_months(year, month, 1)?;
    let end = month_start(next_year, next_month)?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();
    if !validate_date_str(&start_str) || !validate_date_str(&end_str) {
        return Err(DbError::InvalidData(format!(
            "month bounds are not YYYY-MM-DD: {start_str:?}..{end_str:?}"
        )));
    }
    Ok((start_str, end_str))
}

/// Midnight UTC on the first day of the month.
fn month_start(year: i32, month: u32) -> Result<DateTime<Utc>> {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| DbError::InvalidData(format!("invalid date: {year}-{month:02}-01")))
}

/// `(year, month)` shifted by `offset` months, handling year rollover.
fn add_months(year: i32, month: u32, offset: i32) -> Result<(i32, u32)> {
    if !(1..=12).contains(&month) {
        return Err(DbError::InvalidData(format!("invalid month: {month}")));
    }
    let zero_based = year
        .checked_mul(12)
        .and_then(|y| y.checked_add(month as i32 - 1))
        .and_then(|m| m.checked_add(offset))
        .ok_or_else(|| {
            DbError::InvalidData(format!("month arithmetic overflow: {year}-{month}"))
        })?;
    Ok((
        zero_based.div_euclid(12),
        (zero_based.rem_euclid(12) + 1) as u32,
    ))
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

/// Run a DDL statement built from allowlisted identifiers and validated
/// date strings. DDL identifiers cannot be parameterized.
async fn execute_ddl(conn: &mut sqlx::PgConnection, sql: String) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(sql)).execute(conn).await?;
    Ok(())
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
        assert_eq!(partition_column("events"), Some("created_at"));
        assert_eq!(partition_column("delivery_log"), Some("delivered_at"));
        assert_eq!(partition_column("api_tokens"), None);
        assert_eq!(partition_column("users"), None);
    }

    #[test]
    fn add_months_rolls_over_years() {
        assert_eq!(add_months(2026, 7, 0).unwrap(), (2026, 7));
        assert_eq!(add_months(2026, 7, 5).unwrap(), (2026, 12));
        assert_eq!(add_months(2026, 7, 6).unwrap(), (2027, 1));
        assert_eq!(add_months(2026, 12, 1).unwrap(), (2027, 1));
        assert_eq!(add_months(2026, 12, 25).unwrap(), (2029, 1));
        assert_eq!(add_months(2026, 1, -1).unwrap(), (2025, 12));
        assert!(add_months(2026, 0, 1).is_err());
        assert!(add_months(2026, 13, 1).is_err());
    }

    #[test]
    fn month_bounds_cover_year_rollover() {
        assert_eq!(
            month_bounds(2026, 12).unwrap(),
            ("2026-12-01".to_string(), "2027-01-01".to_string())
        );
        assert_eq!(
            month_bounds(2026, 7).unwrap(),
            ("2026-07-01".to_string(), "2026-08-01".to_string())
        );
    }

    #[test]
    fn parse_lower_bound_timestamptz_literal() {
        let expr = "FOR VALUES FROM ('2026-07-01 00:00:00+00') TO (MAXVALUE)";
        assert_eq!(
            parse_range_lower_bound(expr),
            Some(Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn parse_lower_bound_date_literal() {
        let expr = "FOR VALUES FROM ('2026-07-01') TO ('2026-08-01')";
        assert_eq!(
            parse_range_lower_bound(expr),
            Some(Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn parse_lower_bound_rejects_minvalue_and_garbage() {
        assert_eq!(
            parse_range_lower_bound("FOR VALUES FROM (MINVALUE) TO ('2026-01-01')"),
            None
        );
        assert_eq!(parse_range_lower_bound("DEFAULT"), None);
        assert_eq!(
            parse_range_lower_bound("FOR VALUES FROM ('nonsense') TO (MAXVALUE)"),
            None
        );
    }

    #[test]
    fn partition_name_is_allowlisted_and_validated() {
        assert_eq!(
            partition_name("events", 2026, 7).unwrap(),
            "events_p2026_07"
        );
        assert_eq!(
            partition_name("delivery_log", 2027, 1).unwrap(),
            "delivery_log_p2027_01"
        );
        assert!(partition_name("api_tokens", 2026, 7).is_err());
    }

    // ─── Postgres-backed roll-forward tests ──────────────────────────────
    //
    // Each test builds `events` + `delivery_log` shaped partitioned tables in
    // a throwaway schema and pins the pool's search_path there, so the roll
    // logic (which resolves everything against current_schema()) runs fully
    // isolated from the dev database's real tables.

    use sqlx::postgres::PgPoolOptions;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn scratch_pool() -> (PgPool, PgPool, String) {
        let url =
            std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
        let schema = format!("partition_roll_test_{}", uuid::Uuid::new_v4().simple());
        let admin = PgPool::connect(&url).await.expect("connect admin pool");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create scratch schema");
        let search_path_schema = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |conn, _meta| {
                let schema = search_path_schema.clone();
                Box::pin(async move {
                    sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO {schema}")))
                        .execute(conn)
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

    /// Mimic the shipped schema shape: monthly partitions in the past, and a
    /// catch-all absorbing everything from `catchall_from` on.
    async fn seed_table(pool: &PgPool, table: &str, catchall_from: DateTime<Utc>) {
        let col = partition_column(table).expect("allowlisted table");
        let from = catchall_from.format("%Y-%m-%d").to_string();
        for sql in [
            format!(
                "CREATE TABLE {table} (id BIGSERIAL, {col} TIMESTAMPTZ NOT NULL, \
                 PRIMARY KEY ({col}, id)) PARTITION BY RANGE ({col})"
            ),
            format!(
                "CREATE TABLE {table}_p_past PARTITION OF {table} \
                 FOR VALUES FROM (MINVALUE) TO ('{from}')"
            ),
            format!(
                "CREATE TABLE {table}_p_future PARTITION OF {table} \
                 FOR VALUES FROM ('{from}') TO (MAXVALUE)"
            ),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .expect("seed scratch table");
        }
    }

    async fn partition_of_row(pool: &PgPool, table: &str, id: i64) -> String {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT tableoid::regclass::text FROM {table} WHERE id = $1"
        )))
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("resolve row partition")
    }

    async fn catchall_bound(pool: &PgPool, table: &str) -> Option<String> {
        let mut conn = pool.acquire().await.expect("acquire");
        partition_bound_expr(&mut conn, &format!("{table}_p_future"))
            .await
            .expect("bound expr")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn roll_forward_with_empty_catchall_creates_monthly_partitions() {
        let (pool, admin, schema) = scratch_pool().await;
        let now = Utc::now();
        let this_month = month_start(now.year(), now.month()).unwrap();
        for (table, _) in PARTITIONED_TABLES {
            seed_table(&pool, table, this_month).await;
        }

        ensure_future_partitions(&pool, 3).await.expect("roll");

        for (table, _) in PARTITIONED_TABLES {
            for i in 0..=3 {
                let (y, m) = add_months(now.year(), now.month(), i).unwrap();
                let name = partition_name(table, y, m).unwrap();
                let mut conn = pool.acquire().await.expect("acquire");
                assert!(
                    partition_exists(&mut conn, &name).await.expect("exists"),
                    "expected {name} after roll"
                );
            }
            let bound = catchall_bound(&pool, table)
                .await
                .expect("catch-all exists");
            let (y, m) = add_months(now.year(), now.month(), 4).unwrap();
            let expected = month_start(y, m).unwrap().format("%Y-%m-01").to_string();
            assert!(
                bound.contains(&expected),
                "catch-all should start at {expected}, got {bound}"
            );
        }

        // A fresh write must land in the real monthly partition, not the catch-all.
        let id: i64 =
            sqlx::query_scalar("INSERT INTO events (created_at) VALUES (now()) RETURNING id")
                .fetch_one(&pool)
                .await
                .expect("insert");
        let part = partition_of_row(&pool, "events", id).await;
        let expected = partition_name("events", now.year(), now.month()).unwrap();
        assert_eq!(part, expected);

        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn roll_forward_reattaches_single_month_catchall_rows() {
        let (pool, admin, schema) = scratch_pool().await;
        let now = Utc::now();
        let this_month = month_start(now.year(), now.month()).unwrap();
        for (table, _) in PARTITIONED_TABLES {
            seed_table(&pool, table, this_month).await;
        }

        // Rows already absorbed by the catch-all (the live-deployment state).
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id: i64 =
                sqlx::query_scalar("INSERT INTO events (created_at) VALUES (now()) RETURNING id")
                    .fetch_one(&pool)
                    .await
                    .expect("insert");
            assert_eq!(
                partition_of_row(&pool, "events", id).await,
                "events_p_future"
            );
            ids.push(id);
        }

        ensure_future_partitions(&pool, 3).await.expect("roll");

        // Same rows, now served from the real monthly partition.
        let expected = partition_name("events", now.year(), now.month()).unwrap();
        for id in ids {
            assert_eq!(partition_of_row(&pool, "events", id).await, expected);
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 3, "no rows may be lost by the roll");
        let in_catchall: i64 = sqlx::query_scalar("SELECT count(*) FROM events_p_future")
            .fetch_one(&pool)
            .await
            .expect("catch-all count");
        assert_eq!(in_catchall, 0);

        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn roll_forward_bails_on_multi_month_catchall_rows() {
        let (pool, admin, schema) = scratch_pool().await;
        let now = Utc::now();
        let (prev_y, prev_m) = add_months(now.year(), now.month(), -1).unwrap();
        let prev_month = month_start(prev_y, prev_m).unwrap();
        for (table, _) in PARTITIONED_TABLES {
            seed_table(&pool, table, prev_month).await;
        }

        // Rows spanning two months inside the catch-all — the layout the
        // automated roll must refuse (operator surgery per issue #2396).
        sqlx::query("INSERT INTO events (created_at) VALUES ($1), (now())")
            .bind(prev_month + chrono::Duration::days(1))
            .execute(&pool)
            .await
            .expect("insert spanning rows");

        let result = ensure_future_partitions(&pool, 3).await;
        assert!(
            result.is_err(),
            "multi-month catch-all must not be auto-rolled"
        );

        // Rollback must leave the catch-all attached and every row readable.
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 2);
        assert!(
            catchall_bound(&pool, "events").await.is_some(),
            "catch-all must still be attached after rollback"
        );

        drop_schema(&admin, &schema).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn roll_forward_is_idempotent() {
        let (pool, admin, schema) = scratch_pool().await;
        let now = Utc::now();
        let this_month = month_start(now.year(), now.month()).unwrap();
        for (table, _) in PARTITIONED_TABLES {
            seed_table(&pool, table, this_month).await;
        }

        ensure_future_partitions(&pool, 3)
            .await
            .expect("first roll");
        let bound_after_first = catchall_bound(&pool, "events").await;
        ensure_future_partitions(&pool, 3)
            .await
            .expect("second roll");
        assert_eq!(
            catchall_bound(&pool, "events").await,
            bound_after_first,
            "a second roll in the same month must be a no-op"
        );

        drop_schema(&admin, &schema).await;
    }
}
