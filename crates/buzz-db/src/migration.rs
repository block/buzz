//! Embedded SQLx migrations for Buzz.
//!
//! Fresh deployments apply the checked-in SQL files under `migrations/`.
//! Existing pre-SQLx deployments are baselined when core Buzz tables already
//! exist but `_sqlx_migrations` does not, so startup will not try to replay the
//! initial schema over a live database.

use sqlx::PgPool;

use crate::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

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
