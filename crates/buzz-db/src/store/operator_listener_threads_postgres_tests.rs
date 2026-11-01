//! Database-backed thread listener behavior through the event transaction seam.
use super::*;
use crate::event::{insert_event_with_thread_metadata, ThreadMetadataParams};
use crate::operator_listener::{
    claim_deliveries, load_deliverable_event, register_pubkeys, DeliveryAdmission,
};
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;

struct Fixture {
    db: Db,
    pool: PgPool,
    community: CommunityId,
    channel: Uuid,
    listener: Keys,
    target: Keys,
    author: Keys,
    root: Event,
}
impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let listener = Keys::generate();
        let target = Keys::generate();
        let author = Keys::generate();
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("thread-watch-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(target.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'shared',$3)",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(author.public_key().as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,$2,$3)")
            .bind(community.as_uuid())
            .bind(channel)
            .bind(target.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        register_pubkeys(
            &pool,
            listener.public_key().as_bytes(),
            &[target.public_key().to_bytes().to_vec()],
        )
        .await
        .unwrap();
        let root = EventBuilder::new(Kind::Custom(9), "thread root")
            .tag(Tag::custom(
                nostr::TagKind::custom("h"),
                [channel.to_string()],
            ))
            .sign_with_keys(&author)
            .unwrap();
        insert_event_with_thread_metadata(&pool, community, &root, Some(channel), None)
            .await
            .unwrap();
        Self {
            db,
            pool,
            community,
            channel,
            listener,
            target,
            author,
            root,
        }
    }
    async fn register(&self) -> bool {
        self.db
            .register_operator_listener_thread(
                self.community,
                self.listener.public_key().as_bytes(),
                self.target.public_key().as_bytes(),
                self.channel,
                self.root.id.as_bytes(),
            )
            .await
            .unwrap()
    }
    async fn reply(&self, content: &str, root: &Event, channel: Uuid, mention: bool) -> Event {
        self.reply_as(content, root, channel, mention, &self.author)
            .await
    }
    async fn reply_as(
        &self,
        content: &str,
        root: &Event,
        channel: Uuid,
        mention: bool,
        author: &Keys,
    ) -> Event {
        let mut tags = vec![
            Tag::custom(nostr::TagKind::custom("h"), [channel.to_string()]),
            Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap(),
        ];
        if mention {
            tags.push(Tag::public_key(self.target.public_key()));
        }
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags(tags)
            .sign_with_keys(author)
            .unwrap();
        let created =
            chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap();
        let root_created =
            chrono::DateTime::from_timestamp(root.created_at.as_secs() as i64, 0).unwrap();
        insert_event_with_thread_metadata(
            &self.pool,
            self.community,
            &event,
            Some(channel),
            Some(ThreadMetadataParams {
                event_id: event.id.as_bytes(),
                event_created_at: created,
                channel_id: channel,
                parent_event_id: Some(root.id.as_bytes()),
                parent_event_created_at: Some(root_created),
                root_event_id: Some(root.id.as_bytes()),
                root_event_created_at: Some(root_created),
                depth: 1,
                broadcast: false,
            }),
        )
        .await
        .unwrap();
        event
    }
    async fn count(&self, event: &Event) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM operator_listener_outbox WHERE community_id=$1 AND event_id=$2",
        )
        .bind(self.community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_watch_matches_only_registered_root_channel_and_member_and_deduplicates_mentions() {
    let f = Fixture::new().await;
    assert!(f.register().await);
    assert!(f.register().await); // renewal is idempotent
    assert_eq!(f.count(&f.root).await, 0); // subscribing does not deliver the root
    let reply = f.reply("plain followup", &f.root, f.channel, false).await;
    assert_eq!(f.count(&reply).await, 1);
    let own_reply = f
        .reply_as("agent output", &f.root, f.channel, false, &f.target)
        .await;
    assert_eq!(
        f.count(&own_reply).await,
        0,
        "watch must not deliver the target's own output"
    );

    let explicit = f
        .reply("also explicitly mentioned", &f.root, f.channel, true)
        .await;
    assert_eq!(f.count(&explicit).await, 1); // p-tag + watch share one outbox identity
    let metadata:Option<Vec<u8>>=sqlx::query_scalar("SELECT thread_root_event_id FROM operator_listener_outbox WHERE community_id=$1 AND event_id=$2").bind(f.community.as_uuid()).bind(explicit.id.as_bytes().as_slice()).fetch_one(&f.pool).await.unwrap();
    assert!(
        metadata.is_none(),
        "explicit mention keeps its original delivery contract"
    );
    let other = EventBuilder::new(Kind::Custom(9), "other root")
        .sign_with_keys(&f.author)
        .unwrap();
    insert_event_with_thread_metadata(&f.pool, f.community, &other, Some(f.channel), None)
        .await
        .unwrap();
    let wrong = f.reply("wrong root", &other, f.channel, false).await;
    assert_eq!(f.count(&wrong).await, 0);
    assert!(
        !f.db
            .register_operator_listener_thread(
                f.community,
                f.listener.public_key().as_bytes(),
                f.target.public_key().as_bytes(),
                f.channel,
                reply.id.as_bytes()
            )
            .await
            .unwrap(),
        "a reply cannot be registered as a canonical root"
    );
    let other_channel = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'other',$3)")
        .bind(f.community.as_uuid())
        .bind(other_channel)
        .bind(f.author.public_key().as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!f
        .db
        .register_operator_listener_thread(
            f.community,
            f.listener.public_key().as_bytes(),
            f.target.public_key().as_bytes(),
            other_channel,
            f.root.id.as_bytes()
        )
        .await
        .unwrap());
    let wrong_channel = f
        .reply("wrong channel", &f.root, other_channel, false)
        .await;
    assert_eq!(f.count(&wrong_channel).await, 0);
    let neighbor = Fixture::new().await;
    assert!(!neighbor
        .db
        .register_operator_listener_thread(
            neighbor.community,
            f.listener.public_key().as_bytes(),
            f.target.public_key().as_bytes(),
            neighbor.channel,
            f.root.id.as_bytes()
        )
        .await
        .unwrap());
    let deliveries = claim_deliveries(&f.pool, 10, Utc::now() + TimeDelta::seconds(30))
        .await
        .unwrap();
    let plain = deliveries
        .iter()
        .find(|row| row.event_id == reply.id.as_bytes())
        .unwrap();
    assert_eq!(
        plain.thread_root_event_id.as_deref(),
        Some(f.root.id.as_bytes().as_slice())
    );
    let admission = DeliveryAdmission {
        require_membership: false,
        allow_owner_membership: false,
        require_allowlist: false,
    };
    assert!(load_deliverable_event(&f.pool, plain, admission)
        .await
        .unwrap()
        .is_some());

    sqlx::query("UPDATE channels SET archived_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(load_deliverable_event(&f.pool, plain, admission)
        .await
        .unwrap()
        .is_none());
    assert!(!f.register().await);
    sqlx::query("UPDATE channels SET archived_at=NULL WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deactivated_at=now() WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(load_deliverable_event(&f.pool, plain, admission)
        .await
        .unwrap()
        .is_none());
    assert!(!f.register().await);
    sqlx::query("UPDATE users SET deactivated_at=NULL WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.root.id.as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(load_deliverable_event(&f.pool, plain, admission)
        .await
        .unwrap()
        .is_none());
    assert!(!f.register().await);
    sqlx::query("UPDATE events SET deleted_at=NULL WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(f.root.id.as_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operator_listener_threads SET expires_at=now()-interval '1 second' WHERE community_id=$1").bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    assert!(load_deliverable_event(&f.pool, plain, admission)
        .await
        .unwrap()
        .is_none());
    assert_eq!(f.db.reap_operator_listener_threads().await.unwrap(), 1);
    assert!(f.register().await);
    sqlx::query(
        "UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(!f.register().await);
    assert!(
        load_deliverable_event(&f.pool, plain, admission)
            .await
            .unwrap()
            .is_none(),
        "membership is rechecked after enqueue"
    );
    let removed = f.reply("removed member", &f.root, f.channel, false).await;
    assert_eq!(f.count(&removed).await, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_watch_expiry_capacity_removal_and_community_fence_fail_closed() {
    let f = Fixture::new().await;
    assert!(f.register().await);
    sqlx::query("UPDATE operator_listener_threads SET expires_at=now()-interval '1 second' WHERE community_id=$1").bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    let expired = f.reply("expired", &f.root, f.channel, false).await;
    assert_eq!(f.count(&expired).await, 0);
    assert!(f.register().await);
    for n in 0..127_i32 {
        sqlx::query("INSERT INTO operator_listener_threads(community_id,listener_pubkey,target_pubkey,channel_id,root_event_id,expires_at) VALUES($1,$2,$3,$4,decode(md5($5::text)||md5($5::text),'hex'),now()+interval '1 hour') ON CONFLICT DO NOTHING")
            .bind(f.community.as_uuid()).bind(f.listener.public_key().as_bytes().as_slice()).bind(f.target.public_key().as_bytes().as_slice()).bind(f.channel).bind(n.to_string()).execute(&f.pool).await.unwrap();
    }
    assert!(
        f.register().await,
        "full capacity permits renewal of the same tuple"
    );
    let other = EventBuilder::new(Kind::Custom(9), "second root")
        .sign_with_keys(&f.author)
        .unwrap();
    insert_event_with_thread_metadata(&f.pool, f.community, &other, Some(f.channel), None)
        .await
        .unwrap();
    assert!(!f
        .db
        .register_operator_listener_thread(
            f.community,
            f.listener.public_key().as_bytes(),
            f.target.public_key().as_bytes(),
            f.channel,
            other.id.as_bytes()
        )
        .await
        .unwrap());
    f.db.remove_operator_listener_thread(
        f.community,
        f.listener.public_key().as_bytes(),
        f.target.public_key().as_bytes(),
        f.channel,
        f.root.id.as_bytes(),
    )
    .await
    .unwrap();
    let removed = f.reply("removed watch", &f.root, f.channel, false).await;
    assert_eq!(f.count(&removed).await, 0);
    crate::test_support::set_deletion_state(&f.pool, *f.community.as_uuid(), "quiescing").await;
    assert!(f
        .db
        .register_operator_listener_thread(
            f.community,
            f.listener.public_key().as_bytes(),
            f.target.public_key().as_bytes(),
            f.channel,
            f.root.id.as_bytes()
        )
        .await
        .is_err());
}
