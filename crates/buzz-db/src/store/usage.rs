//! Per-community usage rollup queries for Prometheus gauges.
//!
//! Stock queries (`user_counts`, `channel_counts`, `relay_member_counts`,
//! `workflow_counts`, `git_repo_counts`) use `GROUP BY community_id` against
//! indexed columns — no per-community loops, no full-table scans.
//!
//! Event-derived queries (`message_counts`, `active_user_counts`,
//! `active_channel_counts`) are exact aggregates over the `events` table.
//! At scale these can become recurring partition scans; if that becomes a
//! problem, move them to a maintained rollup table and drop the interval.
//!
//! Returned structs are plain data; the caller (relay poller) maps them
//! to Prometheus labels and calls `metrics::gauge!(...).set(...)`.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgConnection;
use sqlx::{Connection as _, Executor, PgPool, Postgres};
use uuid::Uuid;

use crate::error::Result;
use crate::{observability, Db};

/// Fixed-row fleet adoption snapshot used when per-community telemetry is disabled.
#[derive(Debug, Clone, Copy, Default, sqlx::FromRow)]
pub struct FleetStockSnapshot {
    /// Planner estimate for the number of communities.
    pub communities_estimated: i64,
    /// Active human users.
    pub users_human: i64,
    /// Active agent users.
    pub users_agent: i64,
    /// Non-deleted stream channels.
    pub channels_stream: i64,
    /// Non-deleted forum channels.
    pub channels_forum: i64,
    /// Non-deleted direct-message channels.
    pub channels_dm: i64,
    /// Non-deleted workflow channels.
    pub channels_workflow: i64,
    /// Relay owners.
    pub members_owner: i64,
    /// Relay administrators.
    pub members_admin: i64,
    /// Relay members.
    pub members_member: i64,
    /// Active workflows.
    pub workflows_active: i64,
    /// Disabled workflows.
    pub workflows_disabled: i64,
    /// Archived workflows.
    pub workflows_archived: i64,
    /// Registered Git repositories.
    pub git_repos: i64,
}

/// Fixed-row 1d/7d/30d active-user snapshot derived from one 30-day scan.
#[derive(Debug, Clone, Copy, Default, sqlx::FromRow)]
pub struct FleetActiveUsersSnapshot {
    /// Human publishers active in the last day.
    pub human_1d: i64,
    /// Agent publishers active in the last day.
    pub agent_1d: i64,
    /// Unclassified publishers active in the last day.
    pub unknown_1d: i64,
    /// Human publishers active in the last seven days.
    pub human_7d: i64,
    /// Agent publishers active in the last seven days.
    pub agent_7d: i64,
    /// Unclassified publishers active in the last seven days.
    pub unknown_7d: i64,
    /// Human publishers active in the last thirty days.
    pub human_30d: i64,
    /// Agent publishers active in the last thirty days.
    pub agent_30d: i64,
    /// Unclassified publishers active in the last thirty days.
    pub unknown_30d: i64,
}

/// Collect one fixed-row fleet adoption snapshot on the supplied executor.
pub async fn fleet_stock_snapshot_on<'e, E>(executor: E) -> Result<FleetStockSnapshot>
where
    E: Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as::<_, FleetStockSnapshot>(
        r#"
        WITH user_counts AS (
            SELECT
                COUNT(*) FILTER (WHERE agent_owner_pubkey IS NULL) AS human,
                COUNT(*) FILTER (WHERE agent_owner_pubkey IS NOT NULL) AS agent
            FROM users
            WHERE deactivated_at IS NULL
        ),
        channel_counts AS (
            SELECT
                COUNT(*) FILTER (WHERE channel_type = 'stream') AS stream,
                COUNT(*) FILTER (WHERE channel_type = 'forum') AS forum,
                COUNT(*) FILTER (WHERE channel_type = 'dm') AS dm,
                COUNT(*) FILTER (WHERE channel_type = 'workflow') AS workflow
            FROM channels
            WHERE deleted_at IS NULL
        ),
        member_counts AS (
            SELECT
                COUNT(*) FILTER (WHERE role = 'owner') AS owner,
                COUNT(*) FILTER (WHERE role = 'admin') AS admin,
                COUNT(*) FILTER (WHERE role = 'member') AS member
            FROM relay_members
        ),
        workflow_counts AS (
            SELECT
                COUNT(*) FILTER (WHERE status = 'active') AS active,
                COUNT(*) FILTER (WHERE status = 'disabled') AS disabled,
                COUNT(*) FILTER (WHERE status = 'archived') AS archived
            FROM workflows
        )
        SELECT
            COALESCE((SELECT GREATEST(reltuples, 0)::bigint FROM pg_class WHERE oid = 'communities'::regclass), 0) AS communities_estimated,
            users.human AS users_human,
            users.agent AS users_agent,
            channels.stream AS channels_stream,
            channels.forum AS channels_forum,
            channels.dm AS channels_dm,
            channels.workflow AS channels_workflow,
            members.owner AS members_owner,
            members.admin AS members_admin,
            members.member AS members_member,
            workflows.active AS workflows_active,
            workflows.disabled AS workflows_disabled,
            workflows.archived AS workflows_archived,
            (SELECT COUNT(*) FROM git_repo_names) AS git_repos
        FROM user_counts users
        CROSS JOIN channel_counts channels
        CROSS JOIN member_counts members
        CROSS JOIN workflow_counts workflows
        "#,
    )
    .fetch_one(executor)
    .await?)
}

/// Collect all fleet active-user windows with one bounded 30-day event scan.
pub async fn fleet_active_users_on<'e, E>(
    executor: E,
    observed_at: DateTime<Utc>,
) -> Result<FleetActiveUsersSnapshot>
where
    E: Executor<'e, Database = Postgres>,
{
    let start_30d = observed_at - Duration::days(30);
    let start_7d = observed_at - Duration::days(7);
    let start_1d = observed_at - Duration::days(1);
    Ok(sqlx::query_as::<_, FleetActiveUsersSnapshot>(
        r#"
        WITH publishers AS (
            SELECT
                e.community_id,
                e.pubkey,
                MAX(e.created_at) AS last_active_at,
                CASE
                    WHEN u.pubkey IS NULL THEN 'unknown'
                    WHEN u.agent_owner_pubkey IS NULL THEN 'human'
                    ELSE 'agent'
                END AS author_type
            FROM events e
            LEFT JOIN users u
                ON u.community_id = e.community_id AND u.pubkey = e.pubkey
            WHERE e.created_at >= $1
              AND e.created_at < $2
              AND e.deleted_at IS NULL
            GROUP BY e.community_id, e.pubkey, u.pubkey, u.agent_owner_pubkey
        )
        SELECT
            COUNT(*) FILTER (WHERE last_active_at >= $4 AND author_type = 'human') AS human_1d,
            COUNT(*) FILTER (WHERE last_active_at >= $4 AND author_type = 'agent') AS agent_1d,
            COUNT(*) FILTER (WHERE last_active_at >= $4 AND author_type = 'unknown') AS unknown_1d,
            COUNT(*) FILTER (WHERE last_active_at >= $3 AND author_type = 'human') AS human_7d,
            COUNT(*) FILTER (WHERE last_active_at >= $3 AND author_type = 'agent') AS agent_7d,
            COUNT(*) FILTER (WHERE last_active_at >= $3 AND author_type = 'unknown') AS unknown_7d,
            COUNT(*) FILTER (WHERE author_type = 'human') AS human_30d,
            COUNT(*) FILTER (WHERE author_type = 'agent') AS agent_30d,
            COUNT(*) FILTER (WHERE author_type = 'unknown') AS unknown_30d
        FROM publishers
        "#,
    )
    .bind(start_30d)
    .bind(observed_at)
    .bind(start_7d)
    .bind(start_1d)
    .fetch_one(executor)
    .await?)
}

/// Owns the detached Postgres session holding the relay usage-metrics advisory lock.
///
/// The connection deliberately does not return to the main pool: session advisory
/// locks must remain bound to this exact physical connection, and the poller
/// pings it before each leader-only collection tick.
pub struct UsageMetricsLeader {
    connection: PgConnection,
}

impl UsageMetricsLeader {
    /// Returns whether the lock-owning session is still reachable.
    ///
    /// Bounded to 5 seconds — a blackholed connection (no RST) would otherwise
    /// stall the entire poller tick until the OS TCP timeout.
    pub async fn is_live(&mut self) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.connection.ping())
            .await
            .is_ok_and(|r| r.is_ok())
    }
}

/// Total number of communities registered on this relay.
pub async fn community_count(pool: &PgPool) -> Result<i64> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let row = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM communities")
        .fetch_one(&mut *connection)
        .await?;
    Ok(row)
}

/// Per-community user counts split by human/agent.
#[derive(Debug)]
pub struct CommunityUserCounts {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Number of active human users (no `agent_owner_pubkey`).
    pub human: i64,
    /// Number of active agent users (`agent_owner_pubkey IS NOT NULL`).
    pub agent: i64,
}

/// Return active (non-deactivated) user counts per community, split by type.
///
/// Agent discriminator: `agent_owner_pubkey IS NOT NULL`.
pub async fn user_counts(pool: &PgPool) -> Result<Vec<CommunityUserCounts>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    // Single GROUP BY query; two conditional SUMs avoid two round-trips.
    let rows = sqlx::query_as::<_, (Uuid, i64, i64)>(
        r#"
        SELECT
            community_id,
            COUNT(*) FILTER (WHERE agent_owner_pubkey IS NULL)     AS human,
            COUNT(*) FILTER (WHERE agent_owner_pubkey IS NOT NULL) AS agent
        FROM users
        WHERE deactivated_at IS NULL
        GROUP BY community_id
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, human, agent)| CommunityUserCounts {
            community_id,
            human,
            agent,
        })
        .collect())
}

/// Per-community channel counts by type.
#[derive(Debug)]
pub struct CommunityChannelCount {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Channel type string (e.g. `"stream"`, `"dm"`, `"forum"`, `"workflow"`).
    pub channel_type: String,
    /// Number of non-deleted channels of this type.
    pub count: i64,
}

/// Return non-deleted channel counts per community per type.
pub async fn channel_counts(pool: &PgPool) -> Result<Vec<CommunityChannelCount>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, channel_type::text, COUNT(*) AS count
        FROM channels
        WHERE deleted_at IS NULL
        GROUP BY community_id, channel_type
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(community_id, channel_type, count)| CommunityChannelCount {
                community_id,
                channel_type,
                count,
            },
        )
        .collect())
}

/// Per-community message (kind=9) count.
#[derive(Debug)]
pub struct CommunityMessageCount {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Number of stored non-deleted kind=9 events.
    pub count: i64,
}

/// Return non-deleted kind=9 event counts per community.
pub async fn message_counts(pool: &PgPool) -> Result<Vec<CommunityMessageCount>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"
        SELECT community_id, COUNT(*) AS count
        FROM events
        WHERE kind = 9 AND deleted_at IS NULL
        GROUP BY community_id
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, count)| CommunityMessageCount {
            community_id,
            count,
        })
        .collect())
}

/// Per-community relay-member counts by role.
#[derive(Debug)]
pub struct CommunityMemberCount {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Role string (e.g. `"owner"`, `"admin"`, `"member"`).
    pub role: String,
    /// Number of members with this role.
    pub count: i64,
}

/// Return relay-member counts per community per role.
pub async fn relay_member_counts(pool: &PgPool) -> Result<Vec<CommunityMemberCount>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, role::text, COUNT(*) AS count
        FROM relay_members
        GROUP BY community_id, role
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, role, count)| CommunityMemberCount {
            community_id,
            role,
            count,
        })
        .collect())
}

/// Per-community workflow counts by status.
#[derive(Debug)]
pub struct CommunityWorkflowCount {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Workflow status string (e.g. `"active"`, `"inactive"`).
    pub status: String,
    /// Number of workflows in this status.
    pub count: i64,
}

/// Return workflow counts per community per status.
pub async fn workflow_counts(pool: &PgPool) -> Result<Vec<CommunityWorkflowCount>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, status::text, COUNT(*) AS count
        FROM workflows
        GROUP BY community_id, status
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, status, count)| CommunityWorkflowCount {
            community_id,
            status,
            count,
        })
        .collect())
}

/// Per-community git-repo count.
#[derive(Debug)]
pub struct CommunityGitRepoCount {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Number of git repos registered for this community.
    pub count: i64,
}

/// Return git repo counts per community.
pub async fn git_repo_counts(pool: &PgPool) -> Result<Vec<CommunityGitRepoCount>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"
        SELECT community_id, COUNT(*) AS count
        FROM git_repo_names
        GROUP BY community_id
        "#,
    )
    .fetch_all(&mut *connection)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, count)| CommunityGitRepoCount {
            community_id,
            count,
        })
        .collect())
}

/// Per-community active-user counts for a given window (e.g. 1d, 7d, 30d),
/// split by human/agent.
#[derive(Debug)]
pub struct CommunityActiveUsers {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Distinct human pubkeys that published at least one event in the window.
    /// A pubkey is human when its `users` row exists and `agent_owner_pubkey IS NULL`.
    pub human: i64,
    /// Distinct agent pubkeys that published at least one event in the window.
    /// A pubkey is an agent when its `users` row exists and `agent_owner_pubkey IS NOT NULL`.
    pub agent: i64,
    /// Distinct pubkeys that published at least one event but have no `users` row.
    /// Ingest does not guarantee a `users` row for every pubkey (profileless posters,
    /// agents with missing rows). These are not classified and must not be folded into
    /// `human` to avoid inflating the human count.
    pub unknown: i64,
}

/// Return distinct-publisher counts for events in `[now - interval, now]`
/// per community, split by human/agent/unknown.
///
/// `interval_sql` must be a trusted literal (e.g. `"1 day"`, `"7 days"`) —
/// it is not user-controlled; callers are in the relay process.
pub async fn active_user_counts(
    pool: &PgPool,
    interval_sql: &'static str,
) -> Result<Vec<CommunityActiveUsers>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    // LEFT JOIN users: pubkeys with no row have u.* = NULL.
    // Three-way classification:
    //   human   — row exists (u.pubkey IS NOT NULL) and agent_owner_pubkey IS NULL
    //   agent   — row exists and agent_owner_pubkey IS NOT NULL
    //   unknown — no row (u.pubkey IS NULL); not classified, reported separately
    let sql = format!(
        r#"
        SELECT
            e.community_id,
            COUNT(DISTINCT e.pubkey)
                FILTER (WHERE u.pubkey IS NOT NULL AND u.agent_owner_pubkey IS NULL)     AS human,
            COUNT(DISTINCT e.pubkey)
                FILTER (WHERE u.pubkey IS NOT NULL AND u.agent_owner_pubkey IS NOT NULL) AS agent,
            COUNT(DISTINCT e.pubkey)
                FILTER (WHERE u.pubkey IS NULL)                                          AS unknown
        FROM events e
        LEFT JOIN users u
            ON u.community_id = e.community_id AND u.pubkey = e.pubkey
        WHERE e.created_at >= NOW() - INTERVAL '{interval_sql}'
          AND e.deleted_at IS NULL
        GROUP BY e.community_id
        "#
    );
    let rows = sqlx::query_as::<_, (Uuid, i64, i64, i64)>(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *connection)
        .await?;

    Ok(rows
        .into_iter()
        .map(
            |(community_id, human, agent, unknown)| CommunityActiveUsers {
                community_id,
                human,
                agent,
                unknown,
            },
        )
        .collect())
}

/// Per-community active-channel counts for a given window.
#[derive(Debug)]
pub struct CommunityActiveChannels {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// Distinct channel IDs with ≥1 kind=9 message in the window.
    pub count: i64,
}

/// Return distinct channel IDs with ≥1 kind=9 message in `[now - interval, now]`.
pub async fn active_channel_counts(
    pool: &PgPool,
    interval_sql: &'static str,
) -> Result<Vec<CommunityActiveChannels>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let sql = format!(
        r#"
        SELECT community_id, COUNT(DISTINCT channel_id) AS count
        FROM events
        WHERE kind = 9
          AND channel_id IS NOT NULL
          AND created_at >= NOW() - INTERVAL '{interval_sql}'
          AND deleted_at IS NULL
        GROUP BY community_id
        "#
    );
    let rows = sqlx::query_as::<_, (Uuid, i64)>(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *connection)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, count)| CommunityActiveChannels {
            community_id,
            count,
        })
        .collect())
}

/// Mapping from community UUID to host string, used by the poller to resolve
/// Prometheus label values.
#[derive(Debug)]
pub struct CommunityHost {
    /// The UUID of the community.
    pub id: Uuid,
    /// The canonical host string for this community (used as the Prometheus label value).
    pub host: String,
}

/// Fetch every community id → host mapping in one query, including archived,
/// deleting, and tombstoned communities.
///
/// Usage metrics label per-community series and attribute storage with this
/// map, so it must cover the same rows as the unfiltered usage count queries.
pub async fn community_hosts(pool: &PgPool) -> Result<Vec<CommunityHost>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Maintenance).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>("SELECT id, host FROM communities")
        .fetch_all(&mut *connection)
        .await?;
    Ok(into_community_hosts(rows))
}

/// Fetch the id → host mapping for every active community in one query.
///
/// Archived communities are unreachable through host resolution, and
/// communities anywhere in the deletion lifecycle (`quiescing`, `fenced`,
/// `tombstone`) have their writes DB-fenced, so background workers that write
/// per community must skip them. Tombstone rows stay in place for retention.
async fn active_community_hosts(
    pool: &PgPool,
    operation: observability::WriterOperation,
) -> Result<Vec<CommunityHost>> {
    let mut connection = observability::acquire_writer(pool, operation).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, host FROM communities \
         WHERE archived_at IS NULL \
           AND deleted_at IS NULL \
           AND deletion_state = 'active'",
    )
    .fetch_all(&mut *connection)
    .await?;
    Ok(into_community_hosts(rows))
}

fn into_community_hosts(rows: Vec<(Uuid, String)>) -> Vec<CommunityHost> {
    rows.into_iter()
        .map(|(id, host)| CommunityHost { id, host })
        .collect()
}

impl Db {
    /// Try to acquire the detached session advisory lock for relay usage metrics.
    ///
    /// The returned guard owns the exact connection that acquired the lock. It is
    /// detached from the shared pool so a stable leader neither returns a locked
    /// session to other callers nor permanently consumes a pool slot. Dropping the
    /// guard closes the connection and releases the session-scoped lock.
    #[datastore_span(name = "try_lock_usage_metrics", system = "postgresql")]
    pub async fn try_lock_usage_metrics(
        &self,
        lock_key: i64,
    ) -> Result<Option<UsageMetricsLeader>> {
        let mut connection = observability::acquire_writer_with_legacy_metrics(
            &self.pool,
            observability::WriterOperation::Maintenance,
        )
        .await?;
        let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await?;
        if acquired {
            Ok(Some(UsageMetricsLeader {
                connection: connection.detach(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Collect fleet adoption stocks from a proved read-replica snapshot.
    ///
    /// Returns `None` instead of falling back to the writer when a proved
    /// reader is unavailable. Usage telemetry is allowed to be stale or
    /// skipped and must not add load to the serving database.
    #[datastore_span(name = "usage_fleet_stock_snapshot", system = "postgresql")]
    pub async fn usage_fleet_stock_snapshot(&self) -> Result<Option<FleetStockSnapshot>> {
        let path = "usage_fleet_stock";
        let Some((mut tx, reason)) = self.route_usage_read(path).await else {
            return Ok(None);
        };
        let collected = async {
            sqlx::query("SET LOCAL statement_timeout = '5s'")
                .execute(&mut *tx)
                .await?;
            fleet_stock_snapshot_on(&mut *tx).await
        }
        .await;
        Self::finish_usage_read(path, reason, collected)
    }

    /// Collect all fleet active-user windows from a proved read-replica snapshot.
    ///
    /// Returns `None` instead of falling back to the writer when a proved
    /// reader is unavailable.
    #[datastore_span(name = "usage_fleet_active_users", system = "postgresql")]
    pub async fn usage_fleet_active_users(
        &self,
        observed_at: DateTime<Utc>,
    ) -> Result<Option<FleetActiveUsersSnapshot>> {
        let path = "usage_fleet_active_users";
        let Some((mut tx, reason)) = self.route_usage_read(path).await else {
            return Ok(None);
        };
        let collected = async {
            sqlx::query("SET LOCAL statement_timeout = '15s'")
                .execute(&mut *tx)
                .await?;
            fleet_active_users_on(&mut *tx, observed_at).await
        }
        .await;
        Self::finish_usage_read(path, reason, collected)
    }

    /// Record the route outcome of a proved-reader telemetry query. A query
    /// error is a skipped attempt (`replica_error`), never a writer fallback,
    /// so every attempt that completes appears in `buzz_db_route_decision`.
    fn finish_usage_read<T>(
        path: &'static str,
        reason: &'static str,
        collected: Result<T>,
    ) -> Result<Option<T>> {
        match collected {
            Ok(snapshot) => {
                Self::record_route(path, "replica", reason);
                Ok(Some(snapshot))
            }
            Err(error) => {
                Self::record_route(path, "skipped", "replica_error");
                Err(error)
            }
        }
    }

    /// Return total number of communities on this relay.
    #[datastore_span(name = "usage_community_count", system = "postgresql")]
    pub async fn usage_community_count(&self) -> Result<i64> {
        community_count(&self.pool).await
    }

    /// Return per-community user counts split by human/agent.
    #[datastore_span(name = "usage_user_counts", system = "postgresql")]
    pub async fn usage_user_counts(&self) -> Result<Vec<CommunityUserCounts>> {
        user_counts(&self.pool).await
    }

    /// Return per-community channel counts by type.
    #[datastore_span(name = "usage_channel_counts", system = "postgresql")]
    pub async fn usage_channel_counts(&self) -> Result<Vec<CommunityChannelCount>> {
        channel_counts(&self.pool).await
    }

    /// Return per-community kind=9 message counts.
    #[datastore_span(name = "usage_message_counts", system = "postgresql")]
    pub async fn usage_message_counts(&self) -> Result<Vec<CommunityMessageCount>> {
        message_counts(&self.pool).await
    }

    /// Return per-community relay-member counts by role.
    #[datastore_span(name = "usage_relay_member_counts", system = "postgresql")]
    pub async fn usage_relay_member_counts(&self) -> Result<Vec<CommunityMemberCount>> {
        relay_member_counts(&self.pool).await
    }

    /// Return per-community workflow counts by status.
    #[datastore_span(name = "usage_workflow_counts", system = "postgresql")]
    pub async fn usage_workflow_counts(&self) -> Result<Vec<CommunityWorkflowCount>> {
        workflow_counts(&self.pool).await
    }

    /// Return per-community git-repo counts.
    #[datastore_span(name = "usage_git_repo_counts", system = "postgresql")]
    pub async fn usage_git_repo_counts(&self) -> Result<Vec<CommunityGitRepoCount>> {
        git_repo_counts(&self.pool).await
    }

    /// Return per-community distinct active-user counts for a given SQL interval.
    ///
    /// `interval_sql` must be a trusted literal such as `"1 day"` or `"7 days"`.
    #[datastore_span(name = "usage_active_user_counts", system = "postgresql")]
    pub async fn usage_active_user_counts(
        &self,
        interval_sql: &'static str,
    ) -> Result<Vec<CommunityActiveUsers>> {
        active_user_counts(&self.pool, interval_sql).await
    }

    /// Return per-community active-channel counts for a given SQL interval.
    #[datastore_span(name = "usage_active_channel_counts", system = "postgresql")]
    pub async fn usage_active_channel_counts(
        &self,
        interval_sql: &'static str,
    ) -> Result<Vec<CommunityActiveChannels>> {
        active_channel_counts(&self.pool, interval_sql).await
    }

    /// Return all community id → host mappings, for usage-metrics labels.
    #[datastore_span(name = "usage_community_hosts", system = "postgresql")]
    pub async fn usage_community_hosts(&self) -> Result<Vec<CommunityHost>> {
        community_hosts(&self.pool).await
    }

    /// Return active community host mappings for background maintenance writers.
    #[datastore_span(name = "active_community_hosts", system = "postgresql")]
    pub async fn active_community_hosts(&self) -> Result<Vec<CommunityHost>> {
        active_community_hosts(&self.pool, observability::WriterOperation::Maintenance).await
    }

    /// Return active community host mappings during startup bootstrap work.
    #[datastore_span(name = "bootstrap_community_hosts", system = "postgresql")]
    pub async fn bootstrap_community_hosts(&self) -> Result<Vec<CommunityHost>> {
        active_community_hosts(&self.pool, observability::WriterOperation::Bootstrap).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DbError;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    fn route_decisions(recorder: &DebuggingRecorder) -> Vec<(String, String, String, u64)> {
        let mut decisions: Vec<_> = recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_db_route_decision" {
                    return None;
                }
                let label = |name: &str| {
                    key.key()
                        .labels()
                        .find(|label| label.key() == name)
                        .map(|label| label.value().to_owned())
                        .unwrap_or_default()
                };
                let DebugValue::Counter(count) = value else {
                    return None;
                };
                Some((label("path"), label("decision"), label("reason"), count))
            })
            .collect();
        decisions.sort();
        decisions
    }

    /// Every telemetry attempt on a proved reader lands in the route counter:
    /// a completed query as `replica/<reason>`, a failed one as
    /// `skipped/replica_error` with the error propagated (never a writer
    /// fallback).
    #[test]
    fn finish_usage_read_records_every_attempt_in_route_decisions() {
        let recorder = DebuggingRecorder::new();
        let (ok, err) = metrics::with_local_recorder(&recorder, || {
            (
                Db::finish_usage_read("usage_fleet_stock", "fresh", Ok(7_u8)),
                Db::finish_usage_read::<u8>(
                    "usage_fleet_active_users",
                    "fresh",
                    Err(DbError::AuthEventRejected),
                ),
            )
        });

        assert!(matches!(ok, Ok(Some(7))));
        assert!(matches!(err, Err(DbError::AuthEventRejected)));
        assert_eq!(
            route_decisions(&recorder),
            vec![
                (
                    "usage_fleet_active_users".to_owned(),
                    "skipped".to_owned(),
                    "replica_error".to_owned(),
                    1
                ),
                (
                    "usage_fleet_stock".to_owned(),
                    "replica".to_owned(),
                    "fresh".to_owned(),
                    1
                ),
            ]
        );
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::Keys;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;

    async fn get_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
        let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch db");
        let base = crate::test_support::database_url();
        let idx = base.rfind('/').expect("db url has a path segment");
        let scratch_url = format!("{}/{}", &base[..idx], name);
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch db");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch db");
        (pool, name)
    }

    async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(admin)
        .await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_usage_metrics_lock_has_single_owner_and_releases_on_drop() {
        // Use a private scratch database — not the shared TEST_DATABASE_URL.
        // Postgres advisory locks are per-database; hardcoding the production
        // USAGE_METRICS_LOCK_KEY (0x4255_5A5A_4D45_5452) on the shared test DB
        // races any live buzz-relay on the same database (see #3619).
        let admin_url = crate::test_support::database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin to create scratch db");
        let (pool, scratch_name) = create_scratch_db(&admin, "usage_metrics_lock").await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());
        // Same key as production (`buzz-relay` USAGE_METRICS_LOCK_KEY) — safe here
        // because the scratch DB is empty of other holders.
        let key = 0x4255_5A5A_4D45_5452;

        let mut leader = first
            .try_lock_usage_metrics(key)
            .await
            .expect("first lock attempt")
            .expect("first database handle becomes leader");
        assert!(leader.is_live().await, "lock owner remains reachable");
        assert!(
            second
                .try_lock_usage_metrics(key)
                .await
                .expect("second lock attempt")
                .is_none(),
            "another session cannot become leader while the guard exists"
        );

        drop(leader);
        assert!(
            second
                .try_lock_usage_metrics(key)
                .await
                .expect("lock attempt after leader drop")
                .is_some(),
            "dropping the detached session releases its advisory lock"
        );

        // Release any remaining session state before DROP DATABASE.
        drop(first);
        drop(second);
        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn make_community(pool: &PgPool) -> (Uuid, CommunityId, String) {
        let id = uuid::Uuid::new_v4();
        let host = format!("usage-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert test community");
        (id, CommunityId::from_uuid(id), host)
    }

    async fn insert_user(pool: &PgPool, community_id: Uuid, pubkey: &[u8], is_agent: bool) {
        if is_agent {
            let owner = random_pubkey();
            // Insert owner first (FK constraint).
            sqlx::query(
                "INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(community_id)
            .bind(&owner)
            .execute(pool)
            .await
            .expect("insert owner");
            sqlx::query(
                "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(community_id)
            .bind(pubkey)
            .bind(&owner)
            .execute(pool)
            .await
            .expect("insert agent user");
        } else {
            sqlx::query(
                "INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(community_id)
            .bind(pubkey)
            .execute(pool)
            .await
            .expect("insert human user");
        }
    }

    /// user_counts returns correct human/agent split and is scoped per community.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_user_counts_scoped_per_community() {
        let pool = get_pool().await;
        let (comm_a_uuid, _, _) = make_community(&pool).await;
        let (comm_b_uuid, _, _) = make_community(&pool).await;

        // Community A: insert 2 humans first, then 1 agent whose owner is one
        // of those humans (reuses existing pubkey — no extra human row).
        let human1 = random_pubkey();
        let human2 = random_pubkey();
        let agent_pk = random_pubkey();
        insert_user(&pool, comm_a_uuid, &human1, false).await;
        insert_user(&pool, comm_a_uuid, &human2, false).await;
        // Insert agent with human1 as owner (human1 is already in users).
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(comm_a_uuid)
        .bind(&agent_pk)
        .bind(&human1)
        .execute(&pool)
        .await
        .expect("insert agent user");

        // Community B: 0 human, 1 agent (owner is a fresh human in comm_b).
        let owner_b = random_pubkey();
        insert_user(&pool, comm_b_uuid, &owner_b, false).await;
        let agent_b = random_pubkey();
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(comm_b_uuid)
        .bind(&agent_b)
        .bind(&owner_b)
        .execute(&pool)
        .await
        .expect("insert agent user b");

        let counts = user_counts(&pool).await.expect("user_counts");

        let a = counts.iter().find(|r| r.community_id == comm_a_uuid);
        let b = counts.iter().find(|r| r.community_id == comm_b_uuid);

        let a = a.expect("community A row");
        assert_eq!(a.human, 2, "community A: 2 humans");
        assert_eq!(a.agent, 1, "community A: 1 agent");

        let b = b.expect("community B row");
        assert_eq!(b.human, 1, "community B: 1 human (the agent owner)");
        assert_eq!(b.agent, 1, "community B: 1 agent");
    }

    /// Deactivated users are excluded from user_counts.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_user_counts_excludes_deactivated() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;

        let active_pk = random_pubkey();
        let deactivated_pk = random_pubkey();

        insert_user(&pool, comm_uuid, &active_pk, false).await;
        insert_user(&pool, comm_uuid, &deactivated_pk, false).await;
        // Deactivate the second user.
        sqlx::query(
            "UPDATE users SET deactivated_at = NOW() WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(comm_uuid)
        .bind(&deactivated_pk)
        .execute(&pool)
        .await
        .expect("deactivate user");

        let counts = user_counts(&pool).await.expect("user_counts");
        let row = counts
            .iter()
            .find(|r| r.community_id == comm_uuid)
            .expect("row");
        assert_eq!(row.human, 1, "only active user counted");
    }

    /// channel_counts is scoped per community and excludes deleted channels.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_channel_counts_scoped_and_excludes_deleted() {
        let pool = get_pool().await;
        let (comm_uuid, comm_id, _) = make_community(&pool).await;
        let owner = random_pubkey();
        insert_user(&pool, comm_uuid, &owner, false).await;

        // Insert a stream and a DM channel.
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
             VALUES ($1, $2, 'test-stream', 'stream', 'open', $3)",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(comm_uuid)
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("insert stream channel");

        let dm_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
             VALUES ($1, $2, 'test-dm', 'dm', 'private', $3)",
        )
        .bind(dm_id)
        .bind(comm_uuid)
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("insert dm channel");

        // Soft-delete the DM.
        sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE id = $1")
            .bind(dm_id)
            .execute(&pool)
            .await
            .expect("delete channel");

        // Use comm_id to satisfy unused import warning.
        let _ = comm_id;

        let counts = channel_counts(&pool).await.expect("channel_counts");
        let comm_counts: Vec<_> = counts
            .iter()
            .filter(|r| r.community_id == comm_uuid)
            .collect();

        // Only the stream channel should be counted.
        assert_eq!(comm_counts.len(), 1);
        assert_eq!(comm_counts[0].channel_type, "stream");
        assert_eq!(comm_counts[0].count, 1);
    }

    /// community_hosts returns id → host mapping.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_community_hosts_returns_mapping() {
        let pool = get_pool().await;
        let (id, _, host) = make_community(&pool).await;

        let hosts = community_hosts(&pool).await.expect("community_hosts");
        let found = hosts.iter().find(|h| h.id == id);
        assert!(found.is_some(), "inserted community not found");
        assert_eq!(found.unwrap().host, host);
    }

    /// Regression for #7558: maintenance and bootstrap enumeration return only
    /// active communities. Archived, logically deleted (quiescing/fenced), and
    /// tombstoned rows are skipped on every sweep, and the tombstone row itself
    /// is left intact for retention. The usage-metrics map stays unfiltered so
    /// its per-community labels and storage attribution keep covering every
    /// row the unfiltered usage count queries return.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_community_hosts_excludes_archived_and_deleted_communities() {
        let pool = get_pool().await;
        let (active, _, active_host) = make_community(&pool).await;
        let (archived, _, _) = make_community(&pool).await;
        let (quiescing, _, _) = make_community(&pool).await;
        let (fenced, _, _) = make_community(&pool).await;
        let (tombstone, _, _) = make_community(&pool).await;

        sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
            .bind(archived)
            .execute(&pool)
            .await
            .expect("archive fixture");
        crate::test_support::set_deletion_state(&pool, quiescing, "quiescing").await;
        crate::test_support::set_deletion_state(&pool, fenced, "fenced").await;
        crate::test_support::set_deletion_state(&pool, tombstone, "tombstone").await;

        let db = Db::from_pool(pool.clone());
        for sweep in 0..2 {
            for (caller, hosts) in [
                ("maintenance", db.active_community_hosts().await),
                ("bootstrap", db.bootstrap_community_hosts().await),
            ] {
                let hosts = hosts.unwrap_or_else(|e| panic!("{caller} sweep {sweep}: {e}"));
                let ids: std::collections::HashSet<Uuid> = hosts.iter().map(|h| h.id).collect();
                assert_eq!(
                    hosts
                        .iter()
                        .find(|h| h.id == active)
                        .map(|h| h.host.as_str()),
                    Some(active_host.as_str()),
                    "{caller} sweep {sweep}: active community must be returned"
                );
                for (label, id) in [
                    ("archived", archived),
                    ("quiescing", quiescing),
                    ("fenced", fenced),
                    ("tombstone", tombstone),
                ] {
                    assert!(
                        !ids.contains(&id),
                        "{caller} sweep {sweep}: {label} community must be excluded"
                    );
                }
            }
        }

        let metrics_ids: std::collections::HashSet<Uuid> = db
            .usage_community_hosts()
            .await
            .expect("usage-metrics hosts")
            .iter()
            .map(|h| h.id)
            .collect();
        for (label, id) in [
            ("active", active),
            ("archived", archived),
            ("quiescing", quiescing),
            ("fenced", fenced),
            ("tombstone", tombstone),
        ] {
            assert!(
                metrics_ids.contains(&id),
                "usage-metrics map must still include the {label} community"
            );
        }

        let (state, deleted): (String, bool) = sqlx::query_as(
            "SELECT deletion_state, deleted_at IS NOT NULL FROM communities WHERE id = $1",
        )
        .bind(tombstone)
        .fetch_one(&pool)
        .await
        .expect("tombstone row is retained");
        assert_eq!(state, "tombstone");
        assert!(deleted, "tombstone keeps its deleted_at");
    }

    /// community_count reflects newly inserted communities.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_community_count_increases() {
        let pool = get_pool().await;
        let before = community_count(&pool).await.expect("count before");
        make_community(&pool).await;
        let after = community_count(&pool).await.expect("count after");
        assert!(after > before, "count should increase after insert");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fleet_snapshots_have_fixed_result_shapes() {
        let pool = get_pool().await;
        let stock = fleet_stock_snapshot_on(&pool)
            .await
            .expect("fleet stock snapshot");
        assert!(stock.communities_estimated >= 0);

        let now = chrono::Utc::now();
        let activity = fleet_active_users_on(&pool, now)
            .await
            .expect("fleet activity snapshot");
        assert!(activity.human_1d >= 0);
        assert!(activity.human_7d >= activity.human_1d);
        assert!(activity.human_30d >= activity.human_7d);
    }

    /// git_repo_counts queries git_repo_names (not git_repos) and is scoped per community.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_git_repo_counts_scoped_per_community() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let owner = random_pubkey();
        insert_user(&pool, comm_uuid, &owner, false).await;
        let owner_hex = hex::encode(&owner);

        // Insert two repos for this community.
        for repo_id in &["repo-alpha", "repo-beta"] {
            sqlx::query(
                "INSERT INTO git_repo_names (community_id, repo_id, owner_pubkey)
                 VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
            )
            .bind(comm_uuid)
            .bind(repo_id)
            .bind(&owner_hex)
            .execute(&pool)
            .await
            .expect("insert git repo");
        }

        let counts = git_repo_counts(&pool).await.expect("git_repo_counts");
        let comm_counts: Vec<_> = counts
            .iter()
            .filter(|r| r.community_id == comm_uuid)
            .collect();

        assert_eq!(comm_counts.len(), 1, "one row per community");
        assert_eq!(comm_counts[0].count, 2, "two repos");
    }

    /// active_user_counts classifies pubkeys with no users row as "unknown",
    /// not "human" — the old LEFT JOIN treated NULL.agent_owner_pubkey as human.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_active_user_counts_unknown_bucket_for_profileless_poster() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;

        // One known human (has a users row).
        let human_pk = random_pubkey();
        insert_user(&pool, comm_uuid, &human_pk, false).await;

        // One profileless poster (no users row at all).
        let profileless_pk = random_pubkey();

        // Insert events for both pubkeys in this community.
        let event_id1 = random_pubkey(); // 32-byte id
        let event_id2 = random_pubkey();
        let sig = vec![0u8; 64];
        for (pk, eid) in [(&human_pk, &event_id1), (&profileless_pk, &event_id2)] {
            sqlx::query(
                "INSERT INTO events \
                 (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at) \
                 VALUES ($1, $2, $3, NOW(), 9, '[]', '', $4, NOW()) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(comm_uuid)
            .bind(eid)
            .bind(pk)
            .bind(&sig)
            .execute(&pool)
            .await
            .expect("insert event");
        }

        let counts = active_user_counts(&pool, "1 day")
            .await
            .expect("active_user_counts");
        let row = counts.iter().find(|r| r.community_id == comm_uuid);
        assert!(row.is_some(), "row for community must exist");
        let row = row.unwrap();
        assert_eq!(row.human, 1, "known human poster counts as human");
        assert_eq!(row.agent, 0, "no agents");
        assert_eq!(
            row.unknown, 1,
            "profileless poster must land in unknown, not human"
        );
    }

    /// Regression: channel_counts returns no row for a community once all
    /// channels of a type are soft-deleted.  The poller zero-fills from
    /// host_map, so absence from this query is the correct "zero" signal.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_channel_counts_drops_to_zero_after_last_channel_deleted() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let owner = random_pubkey();
        insert_user(&pool, comm_uuid, &owner, false).await;

        // Insert one stream channel.
        let ch_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
             VALUES ($1, $2, 'only-stream', 'stream', 'open', $3)",
        )
        .bind(ch_id)
        .bind(comm_uuid)
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("insert channel");

        // Sanity: row present before deletion.
        let before = channel_counts(&pool).await.expect("channel_counts before");
        let before_row = before
            .iter()
            .find(|r| r.community_id == comm_uuid && r.channel_type == "stream");
        assert_eq!(
            before_row.map(|r| r.count),
            Some(1),
            "1 stream channel before deletion"
        );

        // Soft-delete the channel.
        sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE id = $1")
            .bind(ch_id)
            .execute(&pool)
            .await
            .expect("soft-delete channel");

        // After deletion: no row for this community+type — query returns nothing.
        let after = channel_counts(&pool).await.expect("channel_counts after");
        let after_row = after
            .iter()
            .find(|r| r.community_id == comm_uuid && r.channel_type == "stream");
        assert!(
            after_row.is_none(),
            "no stream row after last channel deleted — poller will zero-fill"
        );
    }

    async fn insert_metric_event(
        pool: &PgPool,
        community_id: Uuid,
        pubkey: &[u8],
        created_at: DateTime<Utc>,
        deleted: bool,
    ) {
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, deleted_at) \
             VALUES ($1, $2, $3, $4, 9, '[]', '', $5, $4, \
                     CASE WHEN $6 THEN $4 ELSE NULL END)",
        )
        .bind(community_id)
        .bind(random_pubkey())
        .bind(pubkey)
        .bind(created_at)
        .bind(vec![0u8; 64])
        .bind(deleted)
        .execute(pool)
        .await
        .expect("insert metric event");
    }

    /// Fixed-row fleet SQL must preserve every inclusion/exclusion rule while
    /// keeping its result shape independent of community cardinality.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fleet_snapshots_match_seeded_stock_and_activity_exactly() {
        let admin_url = crate::test_support::database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin to create scratch db");
        let (pool, scratch_name) = create_scratch_db(&admin, "usage_snapshot").await;
        let (community_id, _, _) = make_community(&pool).await;

        let human = random_pubkey();
        let agent = random_pubkey();
        let inactive = random_pubkey();
        insert_user(&pool, community_id, &human, false).await;
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
        )
        .bind(community_id)
        .bind(&agent)
        .bind(&human)
        .execute(&pool)
        .await
        .expect("insert agent");
        insert_user(&pool, community_id, &inactive, false).await;
        sqlx::query(
            "UPDATE users SET deactivated_at = NOW() WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community_id)
        .bind(&inactive)
        .execute(&pool)
        .await
        .expect("deactivate user");

        for channel_type in ["stream", "forum", "dm", "workflow"] {
            sqlx::query(
                "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
                 VALUES ($1, $2, $3, $4::channel_type, 'open', $5)",
            )
            .bind(Uuid::new_v4())
            .bind(community_id)
            .bind(format!("metric-{channel_type}"))
            .bind(channel_type)
            .bind(&human)
            .execute(&pool)
            .await
            .expect("insert live channel");
        }
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by, deleted_at) \
             VALUES ($1, $2, 'deleted-stream', 'stream', 'open', $3, NOW())",
        )
        .bind(Uuid::new_v4())
        .bind(community_id)
        .bind(&human)
        .execute(&pool)
        .await
        .expect("insert deleted channel");

        for role in ["owner", "admin", "member"] {
            sqlx::query(
                "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, $3)",
            )
            .bind(community_id)
            .bind(format!("{role}-pubkey"))
            .bind(role)
            .execute(&pool)
            .await
            .expect("insert relay member");
        }
        for status in ["active", "disabled", "archived"] {
            sqlx::query(
                "INSERT INTO workflows \
                 (community_id, id, name, owner_pubkey, definition, definition_hash, status, enabled) \
                 VALUES ($1, $2, $3, $4, '{}', $5, $6::workflow_status, $7)",
            )
            .bind(community_id)
            .bind(Uuid::new_v4())
            .bind(format!("metric-{status}"))
            .bind(&human)
            .bind(vec![0u8; 32])
            .bind(status)
            .bind(status == "active")
            .execute(&pool)
            .await
            .expect("insert workflow");
        }
        sqlx::query(
            "INSERT INTO git_repo_names (community_id, repo_id, owner_pubkey) VALUES ($1, 'metric-repo', $2)",
        )
        .bind(community_id)
        .bind(hex::encode(&human))
        .execute(&pool)
        .await
        .expect("insert git repo");
        sqlx::query("ANALYZE communities")
            .execute(&pool)
            .await
            .expect("refresh planner estimate");

        let observed_at = Utc::now();
        let unknown = random_pubkey();
        insert_metric_event(
            &pool,
            community_id,
            &human,
            observed_at - Duration::hours(12),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &human,
            observed_at - Duration::days(3),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &agent,
            observed_at - Duration::days(3),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &unknown,
            observed_at - Duration::days(20),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &unknown,
            observed_at - Duration::days(31),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &unknown,
            observed_at + Duration::hours(1),
            false,
        )
        .await;
        insert_metric_event(
            &pool,
            community_id,
            &unknown,
            observed_at - Duration::hours(1),
            true,
        )
        .await;

        let stock = fleet_stock_snapshot_on(&pool).await.expect("fleet stock");
        assert_eq!(stock.communities_estimated, 1);
        assert_eq!((stock.users_human, stock.users_agent), (1, 1));
        assert_eq!(
            (
                stock.channels_stream,
                stock.channels_forum,
                stock.channels_dm,
                stock.channels_workflow,
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (
                stock.members_owner,
                stock.members_admin,
                stock.members_member
            ),
            (1, 1, 1)
        );
        assert_eq!(
            (
                stock.workflows_active,
                stock.workflows_disabled,
                stock.workflows_archived,
            ),
            (1, 1, 1)
        );
        assert_eq!(stock.git_repos, 1);

        let activity = fleet_active_users_on(&pool, observed_at)
            .await
            .expect("fleet activity");
        assert_eq!(
            (activity.human_1d, activity.agent_1d, activity.unknown_1d),
            (1, 0, 0)
        );
        assert_eq!(
            (activity.human_7d, activity.agent_7d, activity.unknown_7d),
            (1, 1, 0)
        );
        assert_eq!(
            (activity.human_30d, activity.agent_30d, activity.unknown_30d,),
            (1, 1, 1)
        );

        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    /// Fleet collection must read a proved replica and skip instead of
    /// silently adding load to the writer when no reader is configured.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fleet_collection_is_replica_only_and_skips_without_reader() {
        let admin_url = crate::test_support::database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin to create scratch db");
        let (writer, writer_name) = create_scratch_db(&admin, "usage_writer").await;
        let (reader, reader_name) = create_scratch_db(&admin, "usage_reader").await;
        let (writer_community, _, _) = make_community(&writer).await;
        let (reader_community, _, _) = make_community(&reader).await;
        insert_user(&writer, writer_community, &random_pubkey(), false).await;
        insert_user(&reader, reader_community, &random_pubkey(), false).await;
        insert_user(&reader, reader_community, &random_pubkey(), false).await;

        let without_reader = Db::from_pool(writer.clone());
        assert!(
            without_reader
                .usage_fleet_stock_snapshot()
                .await
                .expect("skip without reader")
                .is_none(),
            "telemetry must not fall back to the writer"
        );

        let db = Db::from_pools(writer.clone(), reader.clone());
        db.fence().force_open_for_tests(Utc::now());
        let snapshot = db
            .usage_fleet_stock_snapshot()
            .await
            .expect("replica collection")
            .expect("fresh proved reader");
        assert_eq!(
            snapshot.users_human, 2,
            "fixture proves the reader served the query"
        );

        drop(db);
        drop_scratch_db(&admin, reader, &reader_name).await;
        drop_scratch_db(&admin, writer, &writer_name).await;
    }

    /// Loopback TCP proxy that can make the reader go dark. While dark, any
    /// session that receives bytes stops relaying for good but keeps both
    /// sockets open, like a replica that stops answering mid-query. Sessions
    /// opened after the proxy comes back relay normally.
    struct DarkeningProxy {
        port: u16,
        dark: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl DarkeningProxy {
        async fn spawn(upstream_host: String, upstream_port: u16) -> Self {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            async fn pump(
                mut from: tokio::net::tcp::OwnedReadHalf,
                mut to: tokio::net::tcp::OwnedWriteHalf,
                dark: Arc<AtomicBool>,
            ) {
                let mut buf = [0u8; 8192];
                loop {
                    let Ok(n) = from.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    if dark.load(Ordering::SeqCst) {
                        // Swallow the bytes and hold both sockets open.
                        let _held = (from, to);
                        std::future::pending::<()>().await;
                        return;
                    }
                    if to.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            }

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind proxy");
            let port = listener.local_addr().expect("proxy addr").port();
            let dark = Arc::new(AtomicBool::new(false));
            let accept_dark = dark.clone();
            tokio::spawn(async move {
                while let Ok((client, _)) = listener.accept().await {
                    let Ok(server) =
                        tokio::net::TcpStream::connect((upstream_host.as_str(), upstream_port))
                            .await
                    else {
                        continue;
                    };
                    let (client_read, client_write) = client.into_split();
                    let (server_read, server_write) = server.into_split();
                    tokio::spawn(pump(client_read, server_write, accept_dark.clone()));
                    tokio::spawn(pump(server_read, client_write, accept_dark.clone()));
                }
            });
            Self { port, dark }
        }

        fn set_dark(&self, dark: bool) {
            self.dark.store(dark, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// A fleet collection abandoned at its relay-side deadline on a reader
    /// that went dark mid-query must release its slot in the shared reader
    /// pool within SQLx's bounded close-on-drop, not hold it until the kernel
    /// gives up on the socket. SQLx's default return-to-pool path pings the
    /// dark connection with no timeout while holding the slot.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn abandoned_fleet_collection_releases_its_reader_slot() {
        use sqlx::postgres::PgConnectOptions;
        use std::time::{Duration, Instant};

        let admin_url = crate::test_support::database_url();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin to create scratch db");
        let (writer, writer_name) = create_scratch_db(&admin, "usage_writer").await;
        let (reader, reader_name) = create_scratch_db(&admin, "usage_reader").await;
        let (reader_community, _, _) = make_community(&reader).await;
        insert_user(&reader, reader_community, &random_pubkey(), false).await;

        let reader_options = reader.connect_options();
        let proxy = DarkeningProxy::spawn(
            reader_options.get_host().to_owned(),
            reader_options.get_port(),
        )
        .await;
        let proxied: PgConnectOptions =
            (*reader_options).clone().host("127.0.0.1").port(proxy.port);
        // One slot, so a stranded slot blocks every later acquire. No
        // before-acquire ping, so the stall lands after the checkout is
        // handed out, where the relay deadline drops it.
        let read_pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .test_before_acquire(false)
            .acquire_timeout(Duration::from_secs(15))
            .connect_with(proxied)
            .await
            .expect("connect reader through proxy");
        let db = Db::from_pools(writer.clone(), read_pool.clone());
        db.fence().force_open_for_tests(Utc::now());
        db.usage_fleet_stock_snapshot()
            .await
            .expect("healthy collection")
            .expect("fresh proved reader");

        // Leave one established, idle connection for the collection to use.
        drop(read_pool.acquire().await.expect("warm reader connection"));
        let warm_deadline = Instant::now() + Duration::from_secs(5);
        while read_pool.num_idle() != 1 {
            assert!(
                Instant::now() < warm_deadline,
                "warm connection never returned to the pool"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // A fresh Db has a cold Aurora capability cache, so the first
        // post-acquire await is the capability probe, as in production after
        // a failed boot ping. This pins close-on-drop ahead of that await.
        let cold = Db::from_pools(writer.clone(), read_pool.clone());
        cold.fence().force_open_for_tests(Utc::now());
        proxy.set_dark(true);
        tokio::time::timeout(Duration::from_secs(1), cold.usage_fleet_stock_snapshot())
            .await
            .expect_err("a dark reader must stall the collection until its deadline");
        proxy.set_dark(false);

        let started = Instant::now();
        let mut replacement = read_pool
            .acquire()
            .await
            .expect("the abandoned collection must release its reader slot");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "slot release must be bounded by close-on-drop, took {:?}",
            started.elapsed()
        );
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *replacement)
            .await
            .expect("replacement reader is usable");
        assert_eq!(one, 1);
        drop(replacement);

        drop(cold);
        drop(db);
        read_pool.close().await;
        drop_scratch_db(&admin, reader, &reader_name).await;
        drop_scratch_db(&admin, writer, &writer_name).await;
    }
}
