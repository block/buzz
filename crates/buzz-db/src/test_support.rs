use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

use crate::Db;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

pub(crate) struct IsolatedPostgres {
    pub(crate) db: Db,
    pub(crate) pool: PgPool,
    admin: PgPool,
    database: String,
}

impl IsolatedPostgres {
    pub(crate) async fn migrated(label: &str) -> Self {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let admin_options = PgConnectOptions::from_str(&database_url)
            .expect("O5 test database URL must be valid PostgreSQL");
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(admin_options.clone())
            .await
            .expect(
                "O5 PostgreSQL gate requires a reachable test database; a green zero-scenario path is prohibited",
            );
        let database = format!("o5_{}_{}", label, Uuid::new_v4().simple());
        assert!(
            database
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "generated test database must be identifier-safe"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{database}\""
        )))
        .execute(&admin)
        .await
        .expect("create isolated O5 test database");
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(admin_options.database(&database))
            .await
            .expect("connect isolated O5 test database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply the exact embedded O5 SQLx chain");
        let versions: Vec<i64> = sqlx::query_scalar(
            "SELECT version FROM _sqlx_migrations WHERE success ORDER BY version",
        )
        .fetch_all(&pool)
        .await
        .expect("read exact embedded migration versions");
        assert_eq!(versions.len(), 50, "O5 embedded migrator count");
        assert_eq!(versions.first(), Some(&1));
        assert_eq!(versions.last(), Some(&50));
        assert!(
            versions.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "O5 migration chain must be gap-free"
        );
        Self {
            db: Db::from_pool(pool.clone()),
            pool,
            admin,
            database,
        }
    }

    pub(crate) async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE \"{}\" WITH (FORCE)",
            self.database
        )))
        .execute(&self.admin)
        .await
        .expect("drop isolated O5 test database");
        self.admin.close().await;
    }
}
