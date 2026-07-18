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

use crate::error::Result;
use sqlx::PgPool;
use uuid::Uuid;

/// Total number of communities registered on this relay.
pub async fn community_count(pool: &PgPool) -> Result<i64> {
    let row = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM communities")
        .fetch_one(pool)
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
    .fetch_all(pool)
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
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, channel_type::text, COUNT(*) AS count
        FROM channels
        WHERE deleted_at IS NULL
        GROUP BY community_id, channel_type
        "#,
    )
    .fetch_all(pool)
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
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"
        SELECT community_id, COUNT(*) AS count
        FROM events
        WHERE kind = 9 AND deleted_at IS NULL
        GROUP BY community_id
        "#,
    )
    .fetch_all(pool)
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
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, role::text, COUNT(*) AS count
        FROM relay_members
        GROUP BY community_id, role
        "#,
    )
    .fetch_all(pool)
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
    let rows = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"
        SELECT community_id, status::text, COUNT(*) AS count
        FROM workflows
        GROUP BY community_id, status
        "#,
    )
    .fetch_all(pool)
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
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"
        SELECT community_id, COUNT(*) AS count
        FROM git_repo_names
        GROUP BY community_id
        "#,
    )
    .fetch_all(pool)
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
        .fetch_all(pool)
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
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(community_id, count)| CommunityActiveChannels {
            community_id,
            count,
        })
        .collect())
}

/// Per-user storage usage derived from the audit log (logical attribution).
///
/// "Logical attribution" means one full-size charge per uploader per distinct
/// blob (`object_id`).  Re-uploading the same SHA is idempotent and does not
/// double-count.
#[derive(Debug)]
pub struct UserStorageCounts {
    /// The UUID of the community.
    pub community_id: Uuid,
    /// 32-byte pubkey of the uploading actor.
    pub actor_pubkey: Vec<u8>,
    /// Sum of bytes across distinct blobs owned by this user in this community.
    pub total_bytes: i64,
    /// Number of distinct blobs (unique `object_id` values).
    pub object_count: i64,
}

/// Return per-user logical storage, deduped by `(community_id, actor_pubkey,
/// object_id)`.  Each distinct blob is charged once at its full size (the
/// largest *valid* recorded size wins when multiple audit rows exist for the
/// same triple, so a malformed row never shadows a good one).  Rows with
/// NULL `actor_pubkey` or NULL `object_id` are excluded.
///
/// `detail->>'size'` is historical, operator-written JSONB — never trust it
/// to be a well-formed non-negative integer. A raw `::bigint` cast throws on
/// anything that isn't, which would abort the whole poller over one bad row.
/// Instead: validate with a digits-only regex, then bound-check against
/// `i64::MAX` lexicographically (strip leading zeros with `ltrim`, reject
/// if the stripped length exceeds 19 digits or if it equals 19 and exceeds
/// `'9223372036854775807'`). All predicates are non-throwing on arbitrary
/// text; the `::bigint` cast is inside `THEN`, which CASE only evaluates
/// after both guards pass. Fall back to 0 — the object is still counted
/// once, just at zero bytes.
///
/// Each individual `blob_bytes` is bounded to `i64::MAX`, but the per-user
/// `SUM` of many valid blobs can still exceed it — `SUM(bigint)` returns
/// `numeric` (wider precision) precisely so that doesn't overflow
/// mid-aggregation. The final cast to `BIGINT` is what would throw, so
/// the sum is clamped into `[0, i64::MAX]` *before* that cast — an
/// extreme (or adversarial) total degrades to a saturated gauge reading,
/// not an aborted poller.
pub async fn storage_byte_counts(pool: &PgPool) -> Result<Vec<UserStorageCounts>> {
    let rows = sqlx::query_as::<_, (Uuid, Vec<u8>, i64, i64)>(
        r#"
        SELECT
            community_id,
            actor_pubkey,
            CAST(
                LEAST(
                    GREATEST(COALESCE(SUM(blob_bytes), 0), 0::numeric),
                    9223372036854775807::numeric
                ) AS BIGINT
            )                    AS total_bytes,
            COUNT(*)             AS object_count
        FROM (
            SELECT DISTINCT ON (community_id, actor_pubkey, object_id)
                community_id,
                actor_pubkey,
                CASE
                    WHEN detail->>'size' ~ '^[0-9]+$'
                         AND (length(ltrim(detail->>'size', '0')) < 19
                              OR (length(ltrim(detail->>'size', '0')) = 19
                                  AND ltrim(detail->>'size', '0') <= '9223372036854775807'))
                    THEN (detail->>'size')::bigint
                    ELSE 0::bigint
                END AS blob_bytes
            FROM audit_log
            WHERE action = 'media_uploaded'
              AND actor_pubkey IS NOT NULL
              AND object_id IS NOT NULL
            ORDER BY community_id, actor_pubkey, object_id, blob_bytes DESC
        ) deduped
        GROUP BY community_id, actor_pubkey
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(community_id, actor_pubkey, total_bytes, object_count)| UserStorageCounts {
                community_id,
                actor_pubkey,
                total_bytes,
                object_count,
            },
        )
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

/// Fetch all community id → host mappings in one query.
pub async fn community_hosts(pool: &PgPool) -> Result<Vec<CommunityHost>> {
    let rows = sqlx::query_as::<_, (Uuid, String)>("SELECT id, host FROM communities")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, host)| CommunityHost { id, host })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use nostr::Keys;
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn get_pool() -> PgPool {
        PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB")
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

    /// Insert a media_uploaded audit entry for `object_id` with a raw JSON
    /// `size` value (as text, so malformed values can be exercised — e.g.
    /// `"not-a-number"`, `"-5"`, or omission via `None`).
    async fn insert_media_audit_raw(
        pool: &PgPool,
        community_id: Uuid,
        actor: Option<&[u8]>,
        object_id: Option<&str>,
        seq: i64,
        size: Option<&str>,
    ) {
        let hash = random_pubkey(); // unique 32 bytes
        let mut detail = serde_json::Map::new();
        if let Some(s) = size {
            detail.insert("size".to_string(), serde_json::Value::String(s.to_string()));
        }
        sqlx::query(
            "INSERT INTO audit_log (community_id, seq, hash, action, actor_pubkey, object_id, detail)
             VALUES ($1, $2, $3, 'media_uploaded', $4, $5, $6)",
        )
        .bind(community_id)
        .bind(seq)
        .bind(&hash)
        .bind(actor)
        .bind(object_id)
        .bind(serde_json::Value::Object(detail))
        .execute(pool)
        .await
        .expect("insert media audit entry");
    }

    /// Insert a media_uploaded audit entry with a well-formed numeric size.
    async fn insert_media_audit(
        pool: &PgPool,
        community_id: Uuid,
        actor: &[u8],
        object_id: &str,
        seq: i64,
        size: u64,
    ) {
        insert_media_audit_raw(
            pool,
            community_id,
            Some(actor),
            Some(object_id),
            seq,
            Some(&size.to_string()),
        )
        .await;
    }

    /// Two users in the same community get independent per-user storage rows.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_two_users_same_community_independent() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;

        let user_a = random_pubkey();
        let user_b = random_pubkey();

        // User A: 2 distinct blobs totalling 300 bytes.
        insert_media_audit(&pool, comm_uuid, &user_a, "blob-a1", 1, 100).await;
        insert_media_audit(&pool, comm_uuid, &user_a, "blob-a2", 2, 200).await;
        // User B: 1 blob of 500 bytes.
        insert_media_audit(&pool, comm_uuid, &user_b, "blob-b1", 3, 500).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");

        let a_row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user_a);
        let b_row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user_b);

        let a = a_row.expect("user A row");
        assert_eq!(a.total_bytes, 300, "user A: 100 + 200");
        assert_eq!(a.object_count, 2, "user A: 2 distinct blobs");

        let b = b_row.expect("user B row");
        assert_eq!(b.total_bytes, 500, "user B: 500");
        assert_eq!(b.object_count, 1, "user B: 1 blob");
    }

    /// Storage counts are scoped per-community — same pubkey in two communities
    /// gets separate rows.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_cross_community_isolation() {
        let pool = get_pool().await;
        let (comm_a, _, _) = make_community(&pool).await;
        let (comm_b, _, _) = make_community(&pool).await;

        let user = random_pubkey();

        insert_media_audit(&pool, comm_a, &user, "blob-1", 1, 100).await;
        insert_media_audit(&pool, comm_b, &user, "blob-1", 1, 900).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");

        let in_a = rows
            .iter()
            .find(|r| r.community_id == comm_a && r.actor_pubkey == user)
            .expect("row in community A");
        let in_b = rows
            .iter()
            .find(|r| r.community_id == comm_b && r.actor_pubkey == user)
            .expect("row in community B");

        assert_eq!(in_a.total_bytes, 100, "community A: 100 bytes");
        assert_eq!(in_b.total_bytes, 900, "community B: 900 bytes");
    }

    /// Multiple distinct blobs by the same user accumulate correctly.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_multiple_uploads_same_user_accumulate() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        for seq in 1..=5 {
            insert_media_audit(&pool, comm_uuid, &user, &format!("blob-{seq}"), seq, 10).await;
        }

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(row.total_bytes, 50, "5 × 10 bytes");
        assert_eq!(row.object_count, 5, "5 distinct blobs");
    }

    /// Rows with missing `size` key in detail JSONB contribute 0 bytes but
    /// still count the blob once (fail-safe, not fail-closed).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_missing_size_contributes_zero_bytes() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        // One normal blob + one with missing size.
        insert_media_audit(&pool, comm_uuid, &user, "blob-1", 1, 100).await;
        insert_media_audit_raw(&pool, comm_uuid, Some(&user), Some("blob-2"), 2, None).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes, 100,
            "only the well-formed size contributes"
        );
        assert_eq!(row.object_count, 2, "both blobs counted");
    }

    /// Malformed (non-numeric, negative, or otherwise unparseable) `size`
    /// values must not abort the whole aggregate — they fall back to 0 bytes
    /// while the blob is still counted once. Proves the poller survives
    /// historical junk in `detail->>'size'` instead of erroring the query.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_malformed_size_fails_safe_to_zero() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-text"),
            1,
            Some("not-a-number"),
        )
        .await;
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-neg"),
            2,
            Some("-5"),
        )
        .await;
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-float"),
            3,
            Some("3.5"),
        )
        .await;
        insert_media_audit(&pool, comm_uuid, &user, "blob-good", 4, 42).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes, 42,
            "malformed sizes contribute 0 bytes; only the valid blob's 42 counts"
        );
        assert_eq!(row.object_count, 4, "all four blobs are still counted");
    }

    /// A `detail->>'size'` value exceeding `i64::MAX` or with an extreme
    /// digit count must not abort the poller. The lexicographic bound check
    /// (`ltrim` + length/comparison) rejects these without throwing, so the
    /// blob falls back to 0 bytes and the poller survives.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_oversized_digits_fails_safe_to_zero() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        let oversized = "9".repeat(131_073);
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-oversized"),
            1,
            Some(&oversized),
        )
        .await;
        insert_media_audit(&pool, comm_uuid, &user, "blob-good", 2, 42).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts must not abort on oversized digit string");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes, 42,
            "oversized-digit value contributes 0 bytes; only the valid blob's 42 counts"
        );
        assert_eq!(row.object_count, 2, "both blobs are still counted");
    }

    /// Boundary battery for the lexicographic bigint bound check: i64::MAX
    /// accepted, i64::MAX+1 → 0, 131,073 digits → 0, leading-zero forms
    /// ('09223372036854775807' → MAX, '0000' → 0), mixed together with a
    /// valid 42-byte blob. All must resolve without error.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_bigint_boundary_battery() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        // i64::MAX — must be accepted.
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-i64max"),
            1,
            Some("9223372036854775807"),
        )
        .await;
        // i64::MAX + 1 — rejected, falls back to 0.
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-i64max-plus-1"),
            2,
            Some("9223372036854775808"),
        )
        .await;
        // 131,073 digits — rejected, falls back to 0.
        let oversized = "9".repeat(131_073);
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-oversized-digits"),
            3,
            Some(&oversized),
        )
        .await;
        // Leading-zero i64::MAX — accepted as MAX.
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-leading-zero-max"),
            4,
            Some("09223372036854775807"),
        )
        .await;
        // All zeros — accepted as 0.
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            Some(&user),
            Some("blob-all-zeros"),
            5,
            Some("0000"),
        )
        .await;
        // Valid reference blob.
        insert_media_audit(&pool, comm_uuid, &user, "blob-valid", 6, 42).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("boundary battery must not abort");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        // i64::MAX + leading-zero-MAX + 42 = 2 * 9223372036854775807 + 42
        // exceeds i64::MAX → saturates to i64::MAX.
        assert_eq!(
            row.total_bytes,
            i64::MAX,
            "i64::MAX + leading-zero-MAX + 42 saturates to i64::MAX"
        );
        assert_eq!(
            row.object_count, 6,
            "all six blobs counted (including rejected-size ones)"
        );
    }

    /// Rows with a NULL `actor_pubkey` or NULL `object_id` are excluded from
    /// per-user attribution entirely — they must not collapse into a fake
    /// shared "unknown" user and must not be miscounted against a real user.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_null_actor_or_object_id_excluded() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        // Real attributable upload.
        insert_media_audit(&pool, comm_uuid, &user, "blob-real", 1, 100).await;
        // NULL actor_pubkey (e.g. system-initiated action) — excluded.
        insert_media_audit_raw(
            &pool,
            comm_uuid,
            None,
            Some("blob-no-actor"),
            2,
            Some("999"),
        )
        .await;
        // NULL object_id — excluded.
        insert_media_audit_raw(&pool, comm_uuid, Some(&user), None, 3, Some("999")).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes, 100,
            "only the real, fully-attributed upload counts"
        );
        assert_eq!(row.object_count, 1, "only one attributable blob");

        // No row should exist with an empty/placeholder pubkey standing in
        // for the excluded NULL-actor entry.
        assert_eq!(
            rows.iter().filter(|r| r.community_id == comm_uuid).count(),
            1,
            "excluded rows must not create a second, fake user row"
        );
    }

    /// Re-uploading the same blob (same `object_id`) twice for one user is
    /// charged once — idempotent re-upload must not double-count logical
    /// storage even though the audit log records both attempts.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_same_user_same_blob_reuploaded_charged_once() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        // Same object_id uploaded twice (idempotent re-upload).
        insert_media_audit(&pool, comm_uuid, &user, "blob-dup", 1, 250).await;
        insert_media_audit(&pool, comm_uuid, &user, "blob-dup", 2, 250).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes, 250,
            "re-upload of the same blob charged once"
        );
        assert_eq!(row.object_count, 1, "one distinct blob, not two");
    }

    /// Two different users uploading the same blob (`object_id`, e.g. same
    /// file contents hashing to the same SHA) are each charged once — the
    /// dedup key includes actor_pubkey, so blob sharing across users does not
    /// collapse their independent attribution.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_two_users_same_blob_each_charged_once() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user_a = random_pubkey();
        let user_b = random_pubkey();

        insert_media_audit(&pool, comm_uuid, &user_a, "shared-blob", 1, 400).await;
        insert_media_audit(&pool, comm_uuid, &user_b, "shared-blob", 2, 400).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts");
        let a = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user_a)
            .expect("user A row");
        let b = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user_b)
            .expect("user B row");

        assert_eq!(
            a.total_bytes, 400,
            "user A charged once for the shared blob"
        );
        assert_eq!(a.object_count, 1);
        assert_eq!(
            b.total_bytes, 400,
            "user B charged once for the shared blob"
        );
        assert_eq!(b.object_count, 1);
    }

    /// Two distinct blobs whose sizes are each individually valid (≤
    /// `i64::MAX`) but sum past it must not abort the query — Postgres
    /// `SUM(bigint)` returns `numeric` (wider precision than `bigint`), so
    /// the accumulation itself won't overflow for any feasible row count,
    /// but a naive final `CAST(... AS BIGINT)` throws `bigint out of range`
    /// on the result. The aggregate must clamp to `i64::MAX` instead, and
    /// the object is still counted (both blobs are individually valid, so
    /// nothing here is malformed input to fail-safe on).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_storage_aggregate_overflow_saturates_to_i64_max() {
        let pool = get_pool().await;
        let (comm_uuid, _, _) = make_community(&pool).await;
        let user = random_pubkey();

        insert_media_audit(&pool, comm_uuid, &user, "blob-huge-1", 1, i64::MAX as u64).await;
        insert_media_audit(&pool, comm_uuid, &user, "blob-huge-2", 2, 1).await;

        let rows = storage_byte_counts(&pool)
            .await
            .expect("storage_byte_counts must not abort on aggregate overflow");
        let row = rows
            .iter()
            .find(|r| r.community_id == comm_uuid && r.actor_pubkey == user)
            .expect("user row");

        assert_eq!(
            row.total_bytes,
            i64::MAX,
            "aggregate exceeding i64::MAX saturates instead of erroring"
        );
        assert_eq!(
            row.object_count, 2,
            "both individually-valid blobs are still counted"
        );
    }
}
