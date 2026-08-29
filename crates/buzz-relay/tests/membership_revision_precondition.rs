//! Independent coverage for the prepared-event membership-revision race
//! (finding H1).
//!
//! Unlike the retired in-crate probe, the TEST owns every oracle here:
//!
//! - it recomputes the advisory-lock key string itself and derives the 64-bit
//!   lock id with its own `SELECT hashtextextended($1, 0)`;
//! - it holds the lock and performs the uncommitted membership removal in its
//!   own transaction on its own pool;
//! - it races the publish through the relay's real ingest entry point
//!   ([`buzz_relay::handlers::ingest::ingest_event`] — the same function the
//!   HTTP `POST /events` route and the WebSocket `["EVENT", ...]` handler
//!   call), not through a database helper;
//! - it observes blocking positively via `pg_locks`/`pg_stat_activity`
//!   (a waiter on the exact advisory lock id), not via a wall-clock timeout;
//! - it verifies rejection wording and the absence of the stale event in
//!   `events` storage on its own connection.

use std::sync::Arc;

use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::Row;
use uuid::Uuid;

use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_relay::handlers::ingest::{ingest_event, HttpAuthMethod, IngestAuth, IngestError};
use buzz_relay::state::AppState;

/// The advisory-lock key contract shared by membership writes and prepared
/// publishes. Always runnable under plain `cargo test`: the expected string is
/// recomputed here from the documented format, independently of the
/// production formatter.
#[test]
fn membership_lock_key_matches_documented_contract() {
    let community_uuid = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let community = CommunityId::from_uuid(community_uuid);
    assert_eq!(
        buzz_db::channel::channel_membership_lock_key(community, channel),
        format!("buzz_channel_membership:{community_uuid}:{channel}"),
        "membership advisory-lock key must be buzz_channel_membership:<community>:<channel>",
    );
}

/// Build a real `AppState` against the isolated test database. Redis-backed
/// services are constructed lazily against an unreachable localhost port —
/// none of them are exercised on the rejection path under test (mirrors the
/// in-crate `test_state` helper in `src/state.rs`).
async fn test_app_state(database_url: &str) -> Arc<AppState> {
    let mut config =
        buzz_relay::config::Config::from_env().expect("default relay config must load");
    config.database_url = database_url.to_string();
    config.require_relay_membership = false;
    config.redis_url = "redis://127.0.0.1:1".to_string();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .expect("AppState pool must reach the test database");
    let db = buzz_db::Db::from_pool(pool.clone());
    let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("redis pool");
    let pubsub = Arc::new(
        buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
            .await
            .expect("pubsub manager"),
    );
    let audit = buzz_audit::AuditService::new(pool.clone());
    let auth = buzz_auth::AuthService::new(config.auth.clone());
    let search = buzz_search::SearchService::new(pool.clone());
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
        Keys::generate(),
        media_storage,
    );
    Arc::new(state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit isolated BUZZ_TEST_DATABASE_URL; run with --ignored --exact"]
async fn rejects_dm_publish_after_membership_changes() {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("BUZZ_TEST_DATABASE_URL must name an explicit isolated migrated Postgres");

    // The test's own pool — fixture setup, lock holding, and all assertions
    // run here, on connections the production path never touches.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("test pool must reach the test database");

    // --- Fixture: one community, an owner/agent DM, agent as member. -------
    let db = buzz_db::Db::from_pool(pool.clone());
    let host = format!("prepared-race-{}.example", Uuid::new_v4().simple());
    let community = db
        .ensure_configured_community(&host)
        .await
        .expect("create test community")
        .id;
    let owner = Keys::generate();
    let agent = Keys::generate();
    let owner_bytes = owner.public_key().to_bytes();
    let agent_bytes = agent.public_key().to_bytes();
    buzz_db::user::ensure_user(&pool, community, &owner_bytes)
        .await
        .expect("ensure owner user");
    buzz_db::user::ensure_user(&pool, community, &agent_bytes)
        .await
        .expect("ensure agent user");
    let channel = db
        .create_channel(
            community,
            "prepared-race",
            buzz_db::channel::ChannelType::Dm,
            buzz_db::channel::ChannelVisibility::Private,
            None,
            &owner_bytes,
            None,
        )
        .await
        .expect("create DM channel");
    db.add_member(
        community,
        channel.id,
        &agent_bytes,
        buzz_db::channel::MemberRole::Member,
        Some(&owner_bytes),
    )
    .await
    .expect("add agent to DM");

    // Revision the prepared event will claim: computed from the member rows
    // the TEST reads back, exactly the set the admission path will re-read.
    let member_rows = sqlx::query(
        "SELECT pubkey, role::text AS role FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND removed_at IS NULL \
         ORDER BY pubkey ASC",
    )
    .bind(community.as_uuid())
    .bind(channel.id)
    .fetch_all(&pool)
    .await
    .expect("read membership rows");
    assert_eq!(
        member_rows.len(),
        2,
        "fixture DM must have exactly owner+agent"
    );
    let members: Vec<(Vec<u8>, String)> = member_rows
        .iter()
        .map(|row| {
            (
                row.get::<Vec<u8>, _>("pubkey"),
                row.get::<String, _>("role"),
            )
        })
        .collect();
    let revision = buzz_db::channel::membership_revision(channel.id, &members)
        .expect("compute fixture membership revision");

    let event = EventBuilder::new(Kind::Custom(9), "prepared race reply")
        .tags([
            Tag::parse(["h", channel.id.to_string().as_str()]).expect("h tag"),
            Tag::parse(["buzz_membership_revision", revision.as_str()]).expect("revision tag"),
        ])
        .sign_with_keys(&agent)
        .expect("sign prepared reply");

    // --- Independent lock derivation. --------------------------------------
    // The key string is recomputed here from the documented format (the unit
    // test above pins the production formatter to the same string), and the
    // 64-bit lock id comes from Postgres itself.
    let lock_key = format!(
        "buzz_channel_membership:{}:{}",
        community.as_uuid(),
        channel.id
    );
    let lock_id: i64 = sqlx::query_scalar("SELECT hashtextextended($1, 0)")
        .bind(&lock_key)
        .fetch_one(&pool)
        .await
        .expect("derive advisory lock id");
    let unsigned = lock_id as u64;
    let lock_classid = (unsigned >> 32) as i64;
    let lock_objid = (unsigned & 0xffff_ffff) as i64;

    // --- Removal transaction: take the lock, remove the agent, DO NOT commit.
    let mut removal = pool.begin().await.expect("begin removal transaction");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_id)
        .execute(&mut *removal)
        .await
        .expect("take membership advisory lock");
    let removed = sqlx::query(
        "UPDATE channel_members SET removed_at = NOW(), removed_by = $1 \
         WHERE community_id = $2 AND channel_id = $3 AND pubkey = $4 AND removed_at IS NULL",
    )
    .bind(owner_bytes.as_slice())
    .bind(community.as_uuid())
    .bind(channel.id)
    .bind(agent_bytes.as_slice())
    .execute(&mut *removal)
    .await
    .expect("stage agent removal");
    assert_eq!(
        removed.rows_affected(),
        1,
        "exactly the agent row is removed"
    );

    // --- Race the publish through the relay's real ingest entry point. -----
    let state = test_app_state(&database_url).await;
    let tenant = TenantContext::resolved(community, host.clone());
    let publish_state = Arc::clone(&state);
    let publish_tenant = tenant.clone();
    let publish_event = event.clone();
    let agent_pubkey = agent.public_key();
    let publish = tokio::spawn(async move {
        ingest_event(
            &publish_state,
            &publish_tenant,
            publish_event,
            IngestAuth::Http {
                pubkey: agent_pubkey,
                scopes: vec![buzz_auth::Scope::MessagesWrite],
                auth_method: HttpAuthMethod::DevPubkey,
                nip_oa_owner: None,
            },
        )
        .await
    });

    // --- Positive blocking observation: a backend waiting on OUR lock id. --
    // Not a wall-clock inference — the loop exits only on the observed
    // `pg_locks` waiter row (the deadline is a failsafe against a hung test).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let waiters: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l \
             JOIN pg_stat_activity a ON a.pid = l.pid \
             WHERE l.locktype = 'advisory' AND l.granted = false \
               AND l.classid::bigint = $1 AND l.objid::bigint = $2 \
               AND a.wait_event_type = 'Lock' AND a.wait_event = 'advisory'",
        )
        .bind(lock_classid)
        .bind(lock_objid)
        .fetch_one(&pool)
        .await
        .expect("inspect pg_locks");
        if waiters >= 1 {
            break;
        }
        assert!(
            !publish.is_finished(),
            "the racing publish finished without ever waiting on the membership \
             advisory lock — removal and publish were not serialized",
        );
        assert!(
            std::time::Instant::now() < deadline,
            "no pg_locks waiter appeared on the membership advisory lock \
             (classid={lock_classid}, objid={lock_objid})",
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // --- Commit the removal; the publish must now resolve as a rejection. --
    removal.commit().await.expect("commit removal");
    let publish_result = publish.await.expect("join racing publish");
    match publish_result {
        Err(IngestError::Rejected(ref message)) => assert_eq!(
            message, "restricted: channel membership changed after reply preparation",
            "stale prepared publish must be rejected with the documented wire text",
        ),
        Err(other) => panic!(
            "stale prepared publish must be rejected after membership changes, got error {other:?}"
        ),
        Ok(result) => panic!(
            "stale prepared publish must be rejected after membership changes, but it was \
             accepted (accepted={}, message={:?})",
            result.accepted, result.message
        ),
    }

    // --- Storage proof on the test's own connection. -----------------------
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = $2")
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(&pool)
            .await
            .expect("count stale event rows");
    assert_eq!(
        stored, 0,
        "the rejected stale event must not appear in events storage",
    );

    // Cleanup: children first — the communities FKs do not cascade.
    for statement in [
        "DELETE FROM events WHERE community_id = $1",
        "DELETE FROM channel_members WHERE community_id = $1",
        "DELETE FROM channels WHERE community_id = $1",
        "DELETE FROM users WHERE community_id = $1",
        "DELETE FROM communities WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("cleanup `{statement}` failed: {error}"));
    }
}
