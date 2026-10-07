const DEFAULT_DATABASE_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

/// Resolve the database URL shared by PostgreSQL-backed unit tests.
pub(crate) fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_owned())
}

/// Move a community fixture into a deletion lifecycle state the way the
/// executor does: the tombstone trigger only accepts lifecycle changes from a
/// transaction carrying the executor GUCs for that community, including its
/// current fence generation.
pub(crate) async fn set_deletion_state(pool: &sqlx::PgPool, id: uuid::Uuid, state: &str) {
    let mut tx = pool.begin().await.expect("begin lifecycle fixture");
    sqlx::query(
        "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                set_config('buzz.deletion_fence_generation', \
                    (SELECT deletion_fence_generation::text FROM communities WHERE id = $2), true)",
    )
    .bind(id.to_string())
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("authorize lifecycle fixture");
    sqlx::query(
        "UPDATE communities SET deletion_state = $2, \
                deleted_at = CASE WHEN $2 = 'tombstone' THEN now() END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(state)
    .execute(&mut *tx)
    .await
    .expect("set lifecycle state");
    tx.commit().await.expect("commit lifecycle fixture");
}

/// Move a disposable test community into `quiescing`, the state in which
/// admission must reject new serving writes.
pub(crate) async fn quiesce_community_for_tests(
    pool: &sqlx::PgPool,
    community: buzz_core::CommunityId,
) {
    set_deletion_state(pool, *community.as_uuid(), "quiescing").await;
}

/// Whether `error` is the community-admission rejection taken at transaction
/// entry, as opposed to a later commit-time trigger rejection.
pub(crate) fn is_admission_rejection(error: &crate::DbError) -> bool {
    matches!(error, crate::DbError::AccessDenied(message) if message.contains("write-fenced (quiescing)"))
}
