//! Bounded, tenant-scoped authorization snapshots for already-open sessions.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use sqlx::Row as _;
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// One admitted principal and captured owner whose live authorization should be revalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LiveAuthorizationTarget {
    /// The community resolved for the connection.
    pub community_id: CommunityId,
    /// The NIP-42-authenticated principal.
    pub pubkey: [u8; 32],
    /// The owner captured when this socket was admitted, if any.
    pub session_owner_pubkey: Option<[u8; 32]>,
}

/// Authoritative access state for one live principal/owner pair in one community.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveAuthorizationState {
    /// The community used to scope every part of this lookup.
    pub community_id: CommunityId,
    /// The NIP-42-authenticated principal.
    pub pubkey: [u8; 32],
    /// The owner captured when this socket was admitted, if any.
    pub session_owner_pubkey: Option<[u8; 32]>,
    /// Whether that captured owner currently belongs to the community roster.
    pub session_owner_is_relay_member: bool,
    /// The principal's currently recorded agent owner, if any.
    pub agent_owner_pubkey: Option<[u8; 32]>,
    /// Whether the principal is currently in the community relay roster.
    pub relay_member: bool,
    /// Whether its current owner is currently in the community relay roster.
    pub owner_is_relay_member: bool,
    /// Whether the principal, captured owner, or current owner has an active community ban.
    pub banned: bool,
}

/// Return one writer-authoritative access snapshot for each requested target.
///
/// Membership, current agent ownership, and active bans are read by one SQL
/// statement so a relay can revalidate many live sessions without a query per
/// connection. Every input is an explicit `(community_id, pubkey, captured
/// owner)` tuple; the lookup never expands an owner or ban across tenant
/// boundaries. Timeouts are intentionally omitted because they restrict writes
/// rather than live access.
async fn live_authorization_state_batch(
    pool: &sqlx::PgPool,
    targets: &[LiveAuthorizationTarget],
) -> Result<Vec<LiveAuthorizationState>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let community_ids = targets
        .iter()
        .map(|target| *target.community_id.as_uuid())
        .collect::<Vec<_>>();
    let pubkeys = targets
        .iter()
        .map(|target| target.pubkey.to_vec())
        .collect::<Vec<_>>();
    let session_owner_pubkeys = targets
        .iter()
        .map(|target| target.session_owner_pubkey.map(|owner| owner.to_vec()))
        .collect::<Vec<_>>();
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    let rows = sqlx::query(
        r#"
        WITH requested AS (
            SELECT DISTINCT community_id, pubkey, session_owner_pubkey
            FROM unnest($1::uuid[], $2::bytea[], $3::bytea[])
                 AS target(community_id, pubkey, session_owner_pubkey)
        )
        SELECT requested.community_id,
               requested.pubkey,
               requested.session_owner_pubkey,
               EXISTS (
                   SELECT 1
                   FROM relay_members
                   WHERE relay_members.community_id = requested.community_id
                     AND relay_members.pubkey = encode(requested.session_owner_pubkey, 'hex')
               ) AS session_owner_is_relay_member,
               users.agent_owner_pubkey,
               EXISTS (
                   SELECT 1
                   FROM relay_members
                   WHERE relay_members.community_id = requested.community_id
                     AND relay_members.pubkey = encode(requested.pubkey, 'hex')
               ) AS relay_member,
               EXISTS (
                   SELECT 1
                   FROM relay_members
                   WHERE relay_members.community_id = requested.community_id
                     AND relay_members.pubkey = encode(users.agent_owner_pubkey, 'hex')
               ) AS owner_is_relay_member,
               EXISTS (
                   SELECT 1
                   FROM community_bans
                   WHERE community_bans.community_id = requested.community_id
                     AND community_bans.pubkey IN
                         (requested.pubkey, requested.session_owner_pubkey, users.agent_owner_pubkey)
                     AND community_bans.banned
                     AND (community_bans.ban_expires_at IS NULL
                          OR community_bans.ban_expires_at > now())
               ) AS banned
        FROM requested
        LEFT JOIN users
          ON users.community_id = requested.community_id
         AND users.pubkey = requested.pubkey
        "#,
    )
    .bind(community_ids)
    .bind(pubkeys)
    .bind(session_owner_pubkeys)
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(|row| {
            let community_id: Uuid = row.try_get("community_id")?;
            let pubkey = decode_pubkey(row.try_get("pubkey")?, "requested pubkey")?;
            let session_owner_pubkey = row
                .try_get::<Option<Vec<u8>>, _>("session_owner_pubkey")?
                .map(|owner| decode_pubkey(owner, "session owner pubkey"))
                .transpose()?;
            let agent_owner_pubkey = row
                .try_get::<Option<Vec<u8>>, _>("agent_owner_pubkey")?
                .map(|owner| decode_pubkey(owner, "agent owner pubkey"))
                .transpose()?;

            Ok(LiveAuthorizationState {
                community_id: CommunityId::from_uuid(community_id),
                pubkey,
                session_owner_pubkey,
                session_owner_is_relay_member: row.try_get("session_owner_is_relay_member")?,
                agent_owner_pubkey,
                relay_member: row.try_get("relay_member")?,
                owner_is_relay_member: row.try_get("owner_is_relay_member")?,
                banned: row.try_get("banned")?,
            })
        })
        .collect()
}

fn decode_pubkey(bytes: Vec<u8>, field: &'static str) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| DbError::InvalidData(format!("{field} must contain exactly 32 bytes")))
}

impl Db {
    /// Read current membership, agent ownership, and ban state for a bounded
    /// batch of live connections from the authoritative writer.
    ///
    /// The caller is responsible for bounding the batch size and total work.
    /// Each result is scoped to the exact community/principal/captured-owner tuple supplied.
    #[datastore_span(name = "live_authorization_state_batch", system = "postgresql")]
    pub async fn live_authorization_state_batch(
        &self,
        targets: &[LiveAuthorizationTarget],
    ) -> Result<Vec<LiveAuthorizationState>> {
        live_authorization_state_batch(&self.pool, targets).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use chrono::{Duration, Utc};
    use nostr::Keys;
    use sqlx::PgPool;

    async fn setup_db() -> Db {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to disposable test database");
        Db::from_pool(pool)
    }

    async fn make_community(db: &Db) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("live-auth-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(&db.pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    fn pubkey() -> [u8; 32] {
        Keys::generate().public_key().to_bytes()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn batch_is_tenant_scoped_reads_owner_bans_ignores_timeouts_and_scales_to_10k() {
        let db = setup_db().await;
        let community_a = make_community(&db).await;
        let community_b = make_community(&db).await;
        let owner = pubkey();
        let current_owner = pubkey();
        let agent = pubkey();
        let late_link_agent = pubkey();
        let direct = pubkey();
        let timed_out = pubkey();
        let actor = pubkey();

        for key in [
            owner,
            current_owner,
            agent,
            late_link_agent,
            direct,
            timed_out,
            actor,
        ] {
            db.ensure_user_for_authorization(community_a, &key)
                .await
                .expect("create community A user");
        }
        db.ensure_user_for_authorization(community_b, &direct)
            .await
            .expect("create community B user");
        db.ensure_user_for_authorization(community_b, &late_link_agent)
            .await
            .expect("create community B agent");
        db.ensure_user_for_authorization(community_b, &current_owner)
            .await
            .expect("create community B owner");
        db.set_agent_owner_for_authorization(community_a, &agent, &current_owner)
            .await
            .expect("record agent owner");
        db.add_relay_member(community_a, &hex::encode(owner), "member", None)
            .await
            .expect("add owner to community A");
        db.add_relay_member(community_a, &hex::encode(current_owner), "member", None)
            .await
            .expect("add current owner to community A");
        db.add_relay_member(community_b, &hex::encode(direct), "member", None)
            .await
            .expect("add principal to community B");
        db.add_relay_member(community_b, &hex::encode(current_owner), "member", None)
            .await
            .expect("add current owner to community B");
        // Capture the session as ownerless, then commit its first owner link.
        // This is the supported None -> Some ownership transition.
        db.set_agent_owner_for_authorization(community_a, &late_link_agent, &current_owner)
            .await
            .expect("record late owner link in community A");
        db.set_agent_owner_for_authorization(community_b, &late_link_agent, &current_owner)
            .await
            .expect("record same owner link in community B");
        db.ban_community_member(community_a, &owner, &actor, None, None)
            .await
            .expect("ban agent owner in community A");
        db.ban_community_member(community_a, &current_owner, &actor, None, None)
            .await
            .expect("ban current agent owner in community A");
        db.timeout_community_member(
            community_a,
            &timed_out,
            &actor,
            Utc::now() + Duration::minutes(5),
            None,
        )
        .await
        .expect("time out principal in community A");

        let states = db
            .live_authorization_state_batch(&[
                LiveAuthorizationTarget {
                    community_id: community_a,
                    pubkey: agent,
                    session_owner_pubkey: Some(owner),
                },
                LiveAuthorizationTarget {
                    community_id: community_a,
                    pubkey: late_link_agent,
                    session_owner_pubkey: None,
                },
                LiveAuthorizationTarget {
                    community_id: community_b,
                    pubkey: late_link_agent,
                    session_owner_pubkey: None,
                },
                LiveAuthorizationTarget {
                    community_id: community_b,
                    pubkey: agent,
                    session_owner_pubkey: Some(owner),
                },
                LiveAuthorizationTarget {
                    community_id: community_a,
                    pubkey: direct,
                    session_owner_pubkey: None,
                },
                LiveAuthorizationTarget {
                    community_id: community_b,
                    pubkey: direct,
                    session_owner_pubkey: None,
                },
                LiveAuthorizationTarget {
                    community_id: community_a,
                    pubkey: timed_out,
                    session_owner_pubkey: None,
                },
            ])
            .await
            .expect("batch live authorization state");
        let state = |community_id, pubkey| {
            states
                .iter()
                .find(|state| state.community_id == community_id && state.pubkey == pubkey)
                .expect("state for requested tenant/principal")
        };

        let agent_state = state(community_a, agent);
        assert_eq!(agent_state.agent_owner_pubkey, Some(current_owner));
        assert_eq!(agent_state.session_owner_pubkey, Some(owner));
        assert!(agent_state.session_owner_is_relay_member);
        assert!(!agent_state.relay_member);
        assert!(
            agent_state.owner_is_relay_member,
            "active bans are reported independently of roster membership"
        );
        assert!(
            agent_state.banned,
            "captured owner ban applies after owner change"
        );
        let late_link_state = state(community_a, late_link_agent);
        assert_eq!(
            late_link_state.agent_owner_pubkey,
            Some(current_owner),
            "the lookup reads a first owner link added after admission"
        );
        assert!(
            late_link_state.banned,
            "a ban on a newly recorded current owner is returned for an ownerless session"
        );
        assert!(
            !state(community_b, late_link_agent).banned,
            "a ban on the current owner does not cross communities"
        );
        assert!(
            !state(community_b, agent).banned,
            "a ban on the same captured owner does not cross communities"
        );
        assert!(
            !state(community_b, agent).session_owner_is_relay_member,
            "captured-owner membership is community scoped"
        );

        assert!(!state(community_a, direct).relay_member);
        assert!(state(community_b, direct).relay_member);
        let timeout_state = state(community_a, timed_out);
        assert!(!timeout_state.banned, "timeouts do not revoke live access");

        // Exercise the relay's actual 500-principal batches and four-query
        // concurrency at the default 10,000-socket limit. This is a disposable
        // local fixture; no user or moderation rows are needed for absent keys.
        const SUPPORTED_PRINCIPALS: usize = 10_000;
        const BATCH_SIZE: usize = 500;
        const QUERY_CONCURRENCY: usize = 4;
        let targets = (0..SUPPORTED_PRINCIPALS)
            .map(|index| {
                let mut pubkey = [0; 32];
                pubkey[..8].copy_from_slice(&(index as u64).to_be_bytes());
                LiveAuthorizationTarget {
                    community_id: community_a,
                    pubkey,
                    session_owner_pubkey: None,
                }
            })
            .collect::<Vec<_>>();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(QUERY_CONCURRENCY));
        let mut queries = tokio::task::JoinSet::new();
        let started_at = std::time::Instant::now();
        for batch in targets.chunks(BATCH_SIZE).map(|batch| batch.to_vec()) {
            let db = db.clone();
            let semaphore = std::sync::Arc::clone(&semaphore);
            queries.spawn(async move {
                let _permit = semaphore.acquire_owned().await.expect("query permit");
                db.live_authorization_state_batch(&batch)
                    .await
                    .map(|states| states.len())
            });
        }
        let mut rows = 0;
        while let Some(result) = queries.join_next().await {
            rows += result.expect("batch task").expect("batch query");
        }
        let elapsed = started_at.elapsed();
        assert_eq!(rows, SUPPORTED_PRINCIPALS);
        eprintln!(
            "10k-principal PostgreSQL scan: {rows} rows, batches of {BATCH_SIZE}, \
             concurrency {QUERY_CONCURRENCY}, {} ms",
            elapsed.as_millis(),
        );
    }
}
