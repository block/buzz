use super::user_deletion_fences::{self as fences, FenceIdentity};
use crate::{event, Db, DbError};
use buzz_core::CommunityId;
use nostr::{Event, EventBuilder, Keys, Kind};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

struct Fixture {
    pool: PgPool,
    db: Db,
    community: CommunityId,
    lease: crate::entity_deletion::EntityLease,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = db
            .ensure_configured_community(&format!("message-fences-{}.test", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
        let request = db
            .user_deletion_store()
            .submit(&format!("demo:{}", Uuid::new_v4()), "operator", None)
            .await
            .unwrap();
        let store = db.entity_deletion_store();
        store
            .approve(
                request.id,
                "reviewer",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        let lease = store
            .claim(request.id, "worker", Duration::from_secs(300))
            .await
            .unwrap()
            .unwrap();
        Self {
            pool,
            db,
            community,
            lease,
        }
    }

    async fn fence(&self, keys: &Keys, owner: Option<&Keys>) {
        let mut tx = self.pool.begin().await.unwrap();
        self.fence_in(&mut tx, keys, owner).await;
        tx.commit().await.unwrap();
    }

    async fn fence_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &Keys,
        owner: Option<&Keys>,
    ) {
        let identity = if owner.is_some() {
            FenceIdentity::agent(self.community, keys.public_key())
        } else {
            FenceIdentity::user(keys.public_key())
        };
        fences::insert(tx, identity, &self.lease, owner.map(Keys::public_key))
            .await
            .unwrap();
    }

    async fn agent(&self, owner: &Keys) -> Keys {
        self.db
            .ensure_user(self.community, owner.public_key().as_bytes())
            .await
            .unwrap();
        let agent = Keys::generate();
        self.db
            .ensure_user(self.community, agent.public_key().as_bytes())
            .await
            .unwrap();
        self.db
            .set_agent_owner(
                self.community,
                agent.public_key().as_bytes(),
                owner.public_key().as_bytes(),
            )
            .await
            .unwrap();
        agent
    }
}

fn message(keys: &Keys, kind: u16) -> Event {
    EventBuilder::new(Kind::Custom(kind), Uuid::new_v4().to_string())
        .sign_with_keys(keys)
        .unwrap()
}

fn restricted(result: crate::Result<(buzz_core::StoredEvent, bool)>) {
    assert!(
        matches!(result, Err(DbError::UserDeletionRestricted)),
        "{result:?}"
    );
}

async fn wait_for_waiter(pool: &PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND locktype = 'advisory' AND NOT granted)").bind(pid).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("production writer must wait for the identity lock");
}

async fn writer(
    f: &Fixture,
    event: Event,
) -> (
    i32,
    tokio::task::JoinHandle<crate::Result<(buzz_core::StoredEvent, bool)>>,
) {
    let mut tx = crate::begin_community_event_write_transaction(
        &f.pool,
        f.community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(tx.conn())
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        let result = event::insert_event_in_transaction(&mut tx, &event, None).await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
        result
    });
    (pid, task)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn message_writers_enforce_global_and_scoped_fences_even_without_profiles() {
    let f = Fixture::new().await;
    let user = Keys::generate();
    let agent = f.agent(&user).await;
    let other = Keys::generate();
    let other_community =
        f.db.ensure_configured_community(&format!("unrestricted-{}.test", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
    for keys in [&user, &agent, &other] {
        f.db.insert_event_with_thread_metadata(f.community, &message(keys, 9), None, None)
            .await
            .unwrap();
    }
    f.fence(&user, None).await;
    f.fence(&agent, Some(&user)).await;
    for kind in [9, 40002, 45001, 45003] {
        for keys in [&user, &agent] {
            restricted(
                f.db.insert_event(f.community, &message(keys, kind), None)
                    .await,
            );
            restricted(
                f.db.insert_event_with_thread_metadata(
                    f.community,
                    &message(keys, kind),
                    None,
                    None,
                )
                .await,
            );
        }
        f.db.insert_event(f.community, &message(&other, kind), None)
            .await
            .unwrap();
        f.db.insert_event(other_community, &message(&agent, kind), None)
            .await
            .unwrap();
        restricted(
            f.db.insert_event(other_community, &message(&user, kind), None)
                .await,
        );
    }
    sqlx::query("DELETE FROM users WHERE community_id = $1 AND pubkey = $2")
        .bind(f.community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    restricted(
        f.db.insert_event(f.community, &message(&agent, 9), None)
            .await,
    );
    f.db.ensure_user(f.community, agent.public_key().as_bytes())
        .await
        .unwrap();
    restricted(
        f.db.insert_event_with_thread_metadata(f.community, &message(&agent, 9), None, None)
            .await,
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn already_started_message_commits_before_fence_then_new_message_is_rejected() {
    let f = Fixture::new().await;
    let user = Keys::generate();
    let event = message(&user, 9);
    let mut tx = crate::begin_community_event_write_transaction(
        &f.pool,
        f.community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    event::insert_event_in_transaction(&mut tx, &event, None)
        .await
        .unwrap();
    let mut gate = f.pool.begin().await.unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let lease = f.lease.clone();
    let identity = FenceIdentity::user(user.public_key());
    let gating = tokio::spawn(async move {
        fences::insert(&mut gate, identity, &lease, None)
            .await
            .unwrap();
        gate.commit().await.unwrap();
    });
    wait_for_waiter(&f.pool, pid).await;
    assert!(f
        .db
        .get_event_by_id(f.community, event.id.as_bytes())
        .await
        .unwrap()
        .is_none());
    tx.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), gating)
        .await
        .unwrap()
        .unwrap();
    assert!(f
        .db
        .get_event_by_id(f.community, event.id.as_bytes())
        .await
        .unwrap()
        .is_some());
    restricted(
        f.db.insert_event(f.community, &message(&user, 9), None)
            .await,
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn message_waits_for_human_or_agent_owner_fence_and_reads_after_commit() {
    let f = Fixture::new().await;
    for is_agent in [false, true] {
        let user = Keys::generate();
        let sender = if is_agent {
            f.agent(&user).await
        } else {
            user.clone()
        };
        let event = message(&sender, 9);
        let mut gate = f.pool.begin().await.unwrap();
        fences::lock_request(&mut gate, &f.lease).await.unwrap();
        fences::lock_exclusive(&mut gate, &[FenceIdentity::user(user.public_key())])
            .await
            .unwrap();
        let (pid, sending) = writer(&f, event.clone()).await;
        wait_for_waiter(&f.pool, pid).await;
        f.fence_in(&mut gate, &user, None).await;
        if is_agent {
            f.fence_in(&mut gate, &sender, Some(&user)).await;
        }
        gate.commit().await.unwrap();
        restricted(
            tokio::time::timeout(Duration::from_secs(5), sending)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(f
            .db
            .get_event_by_id(f.community, event.id.as_bytes())
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn ownership_change_while_waiting_rejects_and_retry_uses_current_owner() {
    let f = Fixture::new().await;
    let agent = Keys::generate();
    let owner = Keys::generate();
    f.db.ensure_user(f.community, owner.public_key().as_bytes())
        .await
        .unwrap();
    f.db.ensure_user(f.community, agent.public_key().as_bytes())
        .await
        .unwrap();
    let mut assignment = f.pool.begin().await.unwrap();
    fences::lock_shared(
        &mut assignment,
        &[
            FenceIdentity::user(agent.public_key()),
            FenceIdentity::user(owner.public_key()),
        ],
    )
    .await
    .unwrap();
    fences::lock_exclusive(
        &mut assignment,
        &[FenceIdentity::agent(f.community, agent.public_key())],
    )
    .await
    .unwrap();
    let event = message(&agent, 9);
    let (pid, sending) = writer(&f, event.clone()).await;
    wait_for_waiter(&f.pool, pid).await;
    sqlx::query("UPDATE users SET agent_owner_pubkey = $1 WHERE community_id = $2 AND pubkey = $3")
        .bind(owner.public_key().to_bytes().as_slice())
        .bind(f.community.as_uuid())
        .bind(agent.public_key().to_bytes().as_slice())
        .execute(&mut *assignment)
        .await
        .unwrap();
    assignment.commit().await.unwrap();
    assert!(
        matches!(sending.await.unwrap(), Err(DbError::AccessDenied(reason)) if reason.contains("ownership changed"))
    );
    assert!(f
        .db
        .get_event_by_id(f.community, event.id.as_bytes())
        .await
        .unwrap()
        .is_none());
    f.db.insert_event(f.community, &event, None).await.unwrap();
    f.fence(&agent, Some(&owner)).await;
    restricted(
        f.db.insert_event(f.community, &message(&agent, 9), None)
            .await,
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn first_owner_assignment_waits_for_messages_and_rejects_restricted_owners() {
    let f = Fixture::new().await;
    let agent = Keys::generate();
    let owner = Keys::generate();
    f.db.ensure_user(f.community, owner.public_key().as_bytes())
        .await
        .unwrap();
    f.db.ensure_user(f.community, agent.public_key().as_bytes())
        .await
        .unwrap();
    let mut tx = crate::begin_community_event_write_transaction(
        &f.pool,
        f.community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    event::insert_event_in_transaction(&mut tx, &message(&agent, 9), None)
        .await
        .unwrap();
    // A dedicated single-connection pool exposes the actual setter's backend.
    let assign_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&assign_pool)
        .await
        .unwrap();
    let assignment = tokio::spawn({
        let (agent, owner, community) = (agent.public_key(), owner.public_key(), f.community);
        async move {
            super::user::set_agent_owner(
                &assign_pool,
                community,
                agent.as_bytes(),
                owner.as_bytes(),
            )
            .await
        }
    });
    wait_for_waiter(&f.pool, pid).await;
    tx.commit().await.unwrap();
    assert!(assignment.await.unwrap().unwrap());
    f.fence(&owner, None).await;
    let another = Keys::generate();
    f.db.ensure_user(f.community, another.public_key().as_bytes())
        .await
        .unwrap();
    assert!(matches!(
        f.db.set_agent_owner(
            f.community,
            another.public_key().as_bytes(),
            owner.public_key().as_bytes()
        )
        .await,
        Err(DbError::UserDeletionRestricted)
    ));
    let saved: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(f.community.as_uuid())
    .bind(another.public_key().to_bytes().as_slice())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(saved.is_none());
}
