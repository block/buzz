use super::workflow_deletion::delete_in_transaction;
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    pool: PgPool,
    community: CommunityId,
    keys: Keys,
    id: Uuid,
    timestamp: i64,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community.as_uuid())
            .bind(format!("deletion-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .unwrap();
        let keys = Keys::generate();
        let f = Self {
            pool,
            community,
            keys,
            id: Uuid::new_v4(),
            timestamp: Timestamp::now().as_secs() as i64,
        };
        f.seed(f.community).await;
        f
    }

    async fn seed(&self, community: CommunityId) {
        crate::user::ensure_user(&self.pool, community, &self.keys.public_key().to_bytes())
            .await
            .unwrap();
        crate::workflow::upsert_workflow(
            &self.pool,
            community,
            self.id,
            None,
            &self.keys.public_key().to_bytes(),
            "deletion-test",
            "{}",
            &[0; 32],
        )
        .await
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(30620), "definition")
            .tags([Tag::parse(["d", &self.id.to_string()]).unwrap()])
            .custom_created_at(Timestamp::from(self.timestamp as u64))
            .sign_with_keys(&self.keys)
            .unwrap();
        crate::event::insert_event(&self.pool, community, &event, None)
            .await
            .unwrap();
    }

    async fn counts(&self, community: CommunityId) -> (i64, i64) {
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM workflows WHERE community_id = $1 AND id = $2), \
             (SELECT count(*) FROM events WHERE community_id = $1 AND kind = 30620 \
              AND d_tag = $3 AND deleted_at IS NULL)",
        )
        .bind(community.as_uuid())
        .bind(self.id)
        .bind(self.id.to_string())
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn delete(&self, owner: &[u8], timestamp: i64, commit: bool) {
        let mut tx = self.pool.begin().await.unwrap();
        delete_in_transaction(
            &mut tx,
            self.community,
            owner,
            self.id,
            &self.id.to_string(),
            timestamp,
        )
        .await
        .unwrap();
        if commit {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rollback_preserves_both_rows_and_commit_retires_both() {
    let f = Fixture::new().await;
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp, false)
        .await;
    assert_eq!(f.counts(f.community).await, (1, 1));
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp, true)
        .await;
    assert_eq!(f.counts(f.community).await, (0, 0));
    assert!(
        crate::workflow::claim_scheduled_workflow_fire(
            &f.pool,
            f.community,
            f.id,
            chrono::Utc::now()
        )
        .await
        .unwrap()
        .is_none(),
        "deleted workflow cannot acquire a scheduled execution claim"
    );
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp, true)
        .await;
    assert_eq!(f.counts(f.community).await, (0, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn missing_runtime_still_retires_orphan_definition() {
    let f = Fixture::new().await;
    sqlx::query("DELETE FROM workflows WHERE community_id = $1 AND id = $2")
        .bind(f.community.as_uuid())
        .bind(f.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(f.counts(f.community).await, (0, 1));
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp, true)
        .await;
    assert_eq!(f.counts(f.community).await, (0, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_timestamp_and_community_boundaries_preserve_unrelated_rows() {
    let f = Fixture::new().await;
    let other = Fixture::new().await;
    f.seed(other.community).await;
    f.delete(&other.keys.public_key().to_bytes(), f.timestamp, true)
        .await;
    assert_eq!(f.counts(f.community).await, (1, 1));
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp - 1, true)
        .await;
    assert_eq!(f.counts(f.community).await, (1, 1));
    f.delete(&f.keys.public_key().to_bytes(), f.timestamp, true)
        .await;
    assert_eq!(f.counts(f.community).await, (0, 0));
    assert_eq!(f.counts(other.community).await, (1, 1));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deletion_waits_for_the_definition_writer_coordinate_lock() {
    let f = Fixture::new().await;
    let mut writer = f.pool.begin().await.unwrap();
    let lock = super::replaceable::event_replacement_lock_key(
        f.community,
        30620,
        &f.keys.public_key().to_bytes(),
        Some(f.id.to_string().as_bytes()),
    );
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock)
        .execute(&mut *writer)
        .await
        .unwrap();
    let owner = f.keys.public_key().to_bytes();
    let coordinate = f.id.to_string();
    let mut tx = f.pool.begin().await.unwrap();
    {
        let deletion =
            delete_in_transaction(&mut tx, f.community, &owner, f.id, &coordinate, f.timestamp);
        tokio::pin!(deletion);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut deletion)
                .await
                .is_err()
        );
        writer.commit().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), deletion)
            .await
            .unwrap()
            .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(f.counts(f.community).await, (0, 0));
}
