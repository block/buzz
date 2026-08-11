//! Shared relay test state for backend-conformance tests.

use std::sync::Arc;

use crate::config::Config;
use crate::state::AppState;

const SQLITE_TEST_URL: &str = "sqlite::memory:";
const TEST_COMMUNITY_HOST: &str = "relay.example";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestBackend {
    Postgres,
    Sqlite,
}

impl TestBackend {
    fn from_env() -> Self {
        match std::env::var("BUZZ_TEST_BACKEND") {
            Err(_) => Self::Postgres,
            Ok(value) if value.eq_ignore_ascii_case("postgres") => Self::Postgres,
            Ok(value) if value.eq_ignore_ascii_case("sqlite") => Self::Sqlite,
            Ok(value) => panic!("BUZZ_TEST_BACKEND must be 'postgres' or 'sqlite', got {value:?}"),
        }
    }
}

/// Build the standard relay test state for `BUZZ_TEST_BACKEND`.
pub(crate) async fn test_state() -> Arc<AppState> {
    test_state_with_config(|_| {}).await
}

/// Build relay test state after applying test-specific configuration.
pub(crate) async fn test_state_with_config(configure: impl FnOnce(&mut Config)) -> Arc<AppState> {
    let mut config = Config::from_env().expect("default config loads");
    config.require_relay_membership = false;
    config.redis_url = "redis://127.0.0.1:1".to_string();
    configure(&mut config);

    let auxiliary_pg_pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
    let db = match TestBackend::from_env() {
        TestBackend::Postgres => buzz_db::Db::from_pool(auxiliary_pg_pool.clone()),
        TestBackend::Sqlite => {
            let db = buzz_db::Db::new_sqlite(SQLITE_TEST_URL)
                .await
                .expect("in-memory sqlite database");
            db.migrate().await.expect("sqlite migrations");
            db.ensure_configured_community(TEST_COMMUNITY_HOST)
                .await
                .expect("configured sqlite test community");
            db
        }
    };

    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("redis pool");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("pubsub manager"),
    );
    let audit = buzz_audit::AuditService::new(auxiliary_pg_pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(auxiliary_pg_pool);
    let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
        db.clone(),
        buzz_workflow::WorkflowConfig::default(),
    ));
    let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
    let (state, _audit_shutdown) = AppState::new(
        config,
        db,
        redis_pool,
        audit,
        pubsub,
        auth,
        search,
        workflow_engine,
        nostr::Keys::generate(),
        media_storage,
    );
    Arc::new(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_are_explicit() {
        assert_eq!(TestBackend::Postgres, TestBackend::Postgres);
        assert_ne!(TestBackend::Postgres, TestBackend::Sqlite);
    }
}
