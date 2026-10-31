use super::user_deletion_fences::{self as fences, FenceIdentity};
use crate::{Db, DbError};
use buzz_core::CommunityId;
use nostr::{Event, EventBuilder, Keys, Kind};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

pub(super) struct Fixture {
    pub(super) pool: PgPool,
    pub(super) db: Db,
    pub(super) community: CommunityId,
    pub(super) lease: crate::entity_deletion::EntityLease,
}

impl Fixture {
    pub(super) async fn new() -> Self {
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

    pub(super) async fn fence(&self, keys: &Keys, owner: Option<&Keys>) {
        let mut tx = self.pool.begin().await.unwrap();
        self.fence_in(&mut tx, keys, owner).await;
        tx.commit().await.unwrap();
    }

    pub(super) async fn fence_in(
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

    pub(super) async fn agent(&self, owner: &Keys) -> Keys {
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

pub(super) fn message(keys: &Keys, kind: u16) -> Event {
    EventBuilder::new(Kind::Custom(kind), Uuid::new_v4().to_string())
        .sign_with_keys(keys)
        .unwrap()
}

pub(super) fn restricted(result: crate::Result<(buzz_core::StoredEvent, bool)>) {
    assert!(
        matches!(result, Err(DbError::UserDeletionRestricted)),
        "{result:?}"
    );
}

pub(super) async fn wait_for_waiter(pool: &PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_locks WHERE pid = $1 AND locktype = 'advisory' AND NOT granted)").bind(pid).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("production writer must wait for the identity lock");
}
