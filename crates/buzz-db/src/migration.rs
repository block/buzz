//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in SQL files under `migrations/`.
//! Existing pre-SQLx deployments are baselined when core Buzz tables already
//! exist but `_sqlx_migrations` does not, so startup will not try to replay the
//! initial schema over a live database.

use sqlx::PgPool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[cfg(test)]
static SCHEMA_SQL: &str = include_str!("../../../schema/schema.sql");

const BASELINE_MIGRATION_VERSIONS: &[i64] = &[1, 2];

/// Run all pending Buzz database migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    baseline_existing_database(pool).await?;
    MIGRATOR.run(pool).await?;
    Ok(())
}

async fn baseline_existing_database(pool: &PgPool) -> Result<()> {
    if migrations_table_exists(pool).await? || !pre_sqlx_schema_exists(pool).await? {
        return Ok(());
    }

    ensure_migrations_table(pool).await?;

    for version in BASELINE_MIGRATION_VERSIONS {
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == *version)
            .expect("baseline migration version must exist in embedded migrator");

        sqlx::query(
            r#"
            INSERT INTO _sqlx_migrations
                (version, description, success, checksum, execution_time)
            VALUES ($1, $2, TRUE, $3, 0)
            ON CONFLICT (version) DO NOTHING
            "#,
        )
        .bind(migration.version)
        .bind(&*migration.description)
        .bind(&*migration.checksum)
        .execute(pool)
        .await?;
    }

    tracing::info!(
        versions = ?BASELINE_MIGRATION_VERSIONS,
        "Baselined existing Buzz database for SQLx migrations"
    );

    Ok(())
}

async fn migrations_table_exists(pool: &PgPool) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = '_sqlx_migrations'
        )
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

async fn pre_sqlx_schema_exists(pool: &PgPool) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = 'events'
        ) AND EXISTS (
            SELECT 1
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name = 'channels'
        )
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

async fn ensure_migrations_table(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
            success BOOLEAN NOT NULL,
            checksum BYTEA NOT NULL,
            execution_time BIGINT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    #[test]
    fn embedded_migrator_contains_all_schema_migrations() {
        let migrations: Vec<_> = MIGRATOR.iter().collect();

        assert_eq!(migrations.len(), 4);
        assert_eq!(migrations[0].version, 1);
        assert_eq!(&*migrations[0].description, "initial schema");
        assert!(
            migrations[0].sql.as_str().contains("CREATE TABLE channels"),
            "initial schema migration should include Buzz core tables"
        );
        assert!(
            migrations[0]
                .sql
                .as_str()
                .contains("CREATE TABLE IF NOT EXISTS relay_members"),
            "initial schema migration should include relay_members"
        );

        assert_eq!(migrations[1].version, 2);
        assert_eq!(&*migrations[1].description, "backfill d tag");
        assert!(
            migrations[1].sql.as_str().contains("UPDATE events"),
            "second migration should backfill existing event rows"
        );

        assert_eq!(migrations[2].version, 3);
        assert_eq!(&*migrations[2].description, "event reminders");
        assert!(
            migrations[2]
                .sql
                .as_str()
                .contains("ADD COLUMN not_before BIGINT")
                && migrations[2].sql.as_str().contains("idx_events_not_before"),
            "third migration should add the NIP-ER reminder columns and index"
        );

        assert_eq!(migrations[3].version, 4);
        assert_eq!(&*migrations[3].description, "user git email");
        assert!(
            migrations[3]
                .sql
                .as_str()
                .contains("ADD COLUMN git_email TEXT"),
            "fourth migration should add the git_email column to users"
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

    /// Returns `schema/schema.sql` with the NIP-ER reminder DDL removed, so it
    /// models a pre-stack deployment whose `events` table lacks the reminder
    /// columns and index. The strip is asserted: if the snapshot text drifts so
    /// these fragments no longer match, the test fails loudly rather than
    /// silently loading a snapshot that already carries the reminder columns
    /// (which would make migration 0003 collide on re-add).
    fn pre_reminder_schema_snapshot() -> String {
        const REMINDER_COLUMNS: &str = "    not_before  BIGINT,\n    delivered_at BIGINT,\n";
        const REMINDER_INDEX: &str = "CREATE INDEX idx_events_not_before ON events (not_before)\n    WHERE not_before IS NOT NULL AND deleted_at IS NULL AND delivered_at IS NULL;\n";

        assert!(
            SCHEMA_SQL.contains(REMINDER_COLUMNS) && SCHEMA_SQL.contains(REMINDER_INDEX),
            "schema.sql reminder DDL drifted; update pre_reminder_schema_snapshot to match"
        );

        SCHEMA_SQL
            .replace(REMINDER_COLUMNS, "")
            .replace(REMINDER_INDEX, "")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_migrations_applies_embedded_versions_on_fresh_database() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;

        run_migrations(&pool).await.expect("run migrations");

        assert_eq!(applied_versions(&pool).await, vec![1, 2, 3]);
        let events_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'events')",
        )
        .fetch_one(&pool)
        .await
        .expect("check events table");
        assert!(events_exists);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn run_migrations_baselines_existing_schema_and_preserves_allowlist_backfill_path() {
        let pool = connect_test_pool().await;
        reset_public_schema(&pool).await;
        // Load a pre-stack snapshot (without the NIP-ER reminder DDL) so the
        // events table matches a real pre-SQLx deployment, which never had the
        // reminder columns. Migration 0003 must then add them — proving the
        // genuine prod-upgrade path, not a snapshot that already carries them.
        sqlx::raw_sql(sqlx::AssertSqlSafe(pre_reminder_schema_snapshot()))
            .execute(&pool)
            .await
            .expect("load pre-SQLx schema snapshot");
        sqlx::query(
            "INSERT INTO pubkey_allowlist (pubkey, added_at) VALUES (decode($1, 'hex'), now())",
        )
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .execute(&pool)
        .await
        .expect("seed legacy allowlist row");

        run_migrations(&pool).await.expect("baseline migrations");

        assert_eq!(applied_versions(&pool).await, vec![1, 2, 3]);
        let allowlist_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pubkey_allowlist")
            .fetch_one(&pool)
            .await
            .expect("count allowlist rows");
        assert_eq!(
            allowlist_count, 1,
            "baseline must not drop legacy allowlist rows before relay startup backfills them"
        );

        let inserted = crate::relay_members::backfill_from_allowlist(&pool)
            .await
            .expect("backfill legacy allowlist rows");
        assert_eq!(inserted, 1);
        let relay_member_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM relay_members WHERE pubkey = $1 AND role = 'member'",
        )
        .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .fetch_one(&pool)
        .await
        .expect("count backfilled relay member");
        assert_eq!(relay_member_count, 1);
    }
}
