//! Which replies count: the direct-parent conversation rule, its deletion and
//! channel edges, and the bounds on the membership lookup.
use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const DAY: u64 = 86_400;

/// One community with the reading actor and a peer.
struct World {
    db: Db,
    pool: PgPool,
    community: CommunityId,
    actor: Keys,
    peer: Keys,
    now: u64,
}

impl World {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = db
            .ensure_configured_community(&format!("conversation-{}.local", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
        let actor = Keys::generate();
        super::postgres_tests::start_before_everything(&pool, community, &actor.public_key()).await;
        Self {
            db,
            pool,
            community,
            actor,
            peer: Keys::generate(),
            now: Timestamp::now().as_secs(),
        }
    }

    /// A channel the actor has joined.
    async fn channel(&self) -> Uuid {
        self.db
            .create_channel(
                self.community,
                &Uuid::new_v4().to_string(),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &self.actor.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// A top-level message.
    async fn post(&self, channel: Uuid, author: &Keys, at: u64, tags: Vec<Tag>) -> nostr::Event {
        let event = EventBuilder::new(Kind::Custom(9), Uuid::new_v4().to_string())
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(author)
            .unwrap();
        self.db
            .insert_event(self.community, &event, Some(channel))
            .await
            .unwrap();
        event
    }

    /// A canonical reply to `parent` in `root`'s thread. `parent` need not be
    /// stored in `channel`: thread metadata records what the reply claims.
    async fn reply(
        &self,
        channel: Uuid,
        author: &Keys,
        root: &nostr::Event,
        parent: Option<&nostr::Event>,
        at: u64,
        tags: Vec<Tag>,
    ) -> nostr::Event {
        let event = self.post(channel, author, at, tags).await;
        sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
            VALUES ($1,$2,to_timestamp($3),$4,$5,$6,1)")
            .bind(self.community.as_uuid()).bind(event.id.as_bytes().as_slice()).bind(at as f64)
            .bind(channel).bind(root.id.as_bytes().as_slice())
            .bind(parent.map(|parent| parent.id.as_bytes().as_slice()))
            .execute(&self.pool).await.unwrap();
        event
    }

    /// The storage write that author and staff deletions share.
    async fn delete(&self, event: &nostr::Event) {
        assert!(self
            .db
            .soft_delete_event_and_update_thread(self.community, event.id.as_bytes(), None, None)
            .await
            .unwrap());
    }

    async fn row(&self, channel: Uuid) -> ChannelReadSummary {
        self.db
            .personal_read_sidebar_channels(
                self.community,
                &self.actor.public_key(),
                DEFAULT_RETENTION_SECONDS,
                &[channel],
            )
            .await
            .unwrap()
            .channels
            .remove(0)
    }

    /// The row's counts and thread rows: what the sidebar says counts.
    async fn counts(&self, channel: Uuid) -> Value {
        let row = self.row(channel).await;
        json!({"unread":row.unread,"mentions":row.mentions,"threads":wire(&row.threads)})
    }

    fn mention(&self) -> Tag {
        Tag::parse(["p", &self.actor.public_key().to_hex()]).unwrap()
    }
}

fn wire<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

/// Expected `counts`: whether a top-level message is unread, how many of those
/// are directed, and the listed thread rows.
fn counts(unread: bool, mentions: u32, threads: Vec<Value>) -> Value {
    json!({"unread":unread,"mentions":mentions,"threads":threads})
}

fn broadcast() -> Tag {
    Tag::parse(["broadcast", "1"]).unwrap()
}

/// A listed, never-marked thread with `mentions` counted unread replies.
fn item(root: &nostr::Event, mentions: u32, latest: &nostr::Event) -> Value {
    json!({"root_id":root.id.to_hex(),"unread":true,"mentions":mentions,
        "read_through_id":null,"latest_id":latest.id.to_hex()})
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_reply_counts_only_in_a_conversation_the_actor_wrote_or_replied_to() {
    let w = World::new().await;
    // Every witness predates the unread horizon: membership does not expire.
    let old = w.now - 40 * DAY;

    // The actor wrote the root. A peer answers it; another peer reply continues
    // under that answer, where the actor has written nothing.
    let c = w.channel().await;
    let root = w.post(c, &w.actor, old, vec![]).await;
    let answer = w
        .reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
        .await;
    let nested = w
        .reply(c, &w.peer, &root, Some(&answer), w.now + 2, vec![])
        .await;
    // Only `answer` counts. The newer reply that does not count is not the
    // thread's latest.
    assert_eq!(
        w.counts(c).await,
        counts(false, 0, vec![item(&root, 1, &answer)])
    );

    // Joining the nested conversation makes its earlier reply count. The
    // actor's own reply adds nothing.
    w.reply(c, &w.actor, &root, Some(&answer), w.now + 3, vec![])
        .await;
    assert_eq!(
        w.counts(c).await,
        counts(false, 0, vec![item(&root, 2, &nested)])
    );

    // A peer's thread. The actor replied to one of two parents, long ago.
    let c = w.channel().await;
    let root = w.post(c, &w.peer, old, vec![]).await;
    let joined = w.reply(c, &w.peer, &root, Some(&root), old, vec![]).await;
    let other = w.reply(c, &w.peer, &root, Some(&root), old, vec![]).await;
    w.reply(c, &w.actor, &root, Some(&joined), old, vec![])
        .await;
    let sibling = w
        .reply(c, &w.peer, &root, Some(&joined), w.now + 1, vec![])
        .await;
    w.reply(c, &w.peer, &root, Some(&other), w.now + 2, vec![])
        .await;
    assert_eq!(
        w.counts(c).await,
        counts(false, 0, vec![item(&root, 1, &sibling)])
    );

    // A conversation the actor never joined: one unread top-level message, and
    // a reply that is not unread even in its own thread.
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    w.reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
        .await;
    assert_eq!(w.counts(c).await, counts(true, 0, vec![]));

    // A reply whose parent was never recorded is undecided: left out.
    w.reply(c, &w.peer, &root, None, w.now + 2, vec![]).await;
    assert_eq!(w.counts(c).await, counts(true, 0, vec![]));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_directed_reply_counts_outside_the_actors_conversations() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let parent = w.reply(c, &w.peer, &root, Some(&root), w.now, vec![]).await;
    w.reply(c, &w.actor, &root, Some(&root), w.now + 1, vec![])
        .await;
    for (at, tags) in [(2, vec![w.mention()]), (3, vec![broadcast()])] {
        w.reply(c, &w.peer, &root, Some(&parent), w.now + at, tags)
            .await;
    }
    let both = w
        .reply(
            c,
            &w.peer,
            &root,
            Some(&parent),
            w.now + 4,
            vec![broadcast(), w.mention()],
        )
        .await;
    // The root is an undirected top-level message. The thread counts the
    // actor's sibling `parent` and the three directed replies, not the
    // actor's own.
    let expected = counts(true, 0, vec![item(&root, 4, &both)]);
    assert_eq!(w.counts(c).await, expected);

    // Joining the directed replies' conversation changes no count.
    w.reply(c, &w.actor, &root, Some(&parent), w.now + 5, vec![])
        .await;
    assert_eq!(w.counts(c).await, expected);

    // In a DM every peer message is direct, tagged or not, joined or not.
    let dm = w.channel().await;
    sqlx::query("UPDATE channels SET channel_type='dm' WHERE community_id=$1 AND id=$2")
        .bind(w.community.as_uuid())
        .bind(dm)
        .execute(&w.pool)
        .await
        .unwrap();
    let root = w.post(dm, &w.peer, w.now, vec![w.mention()]).await;
    let parent = w
        .reply(dm, &w.peer, &root, Some(&root), w.now, vec![])
        .await;
    let reply = w
        .reply(dm, &w.peer, &root, Some(&parent), w.now + 1, vec![])
        .await;
    assert_eq!(
        w.counts(dm).await,
        counts(true, 1, vec![item(&root, 2, &reply)])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn eligibility_frontier_and_membership_each_leave_replies_out() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let parent = w.reply(c, &w.peer, &root, Some(&root), w.now, vec![]).await;
    let reply = |author, at, tags| w.reply(c, author, &root, Some(&parent), w.now + at, tags);
    // Peer replies to a parent the actor neither wrote nor replied to.
    reply(&w.peer, 1, vec![]).await;
    let deleted = reply(&w.peer, 2, vec![w.mention()]).await;
    w.delete(&deleted).await;
    w.reply(c, &w.actor, &root, Some(&root), w.now + 3, vec![])
        .await;
    let anchor = reply(&w.peer, 4, vec![w.mention()]).await;
    // Undirected, outside the actor's conversations: never counts.
    reply(&w.peer, 5, vec![]).await;
    // `parent` (a reply to the root, which the actor answered) and `anchor`
    // count: not the deleted, own or undirected replies.
    assert_eq!(
        w.counts(c).await,
        counts(true, 0, vec![item(&root, 2, &anchor)])
    );

    assert_eq!(
        w.db.apply_personal_read_intent(
            w.community,
            &w.actor.public_key(),
            &ReadIntent::MarkThrough {
                target: ReadTarget {
                    channel_id: c,
                    root_id: Some(root.id.to_hex()),
                },
                message_id: anchor.id.to_hex(),
            },
        )
        .await
        .unwrap(),
        IntentOutcome::Applied
    );
    assert_eq!(w.counts(c).await, counts(true, 0, vec![]));
    // A later directed reply is unread past the thread's anchor; the channel
    // timeline was never marked.
    let after = reply(&w.peer, 6, vec![w.mention()]).await;
    let mut thread = item(&root, 1, &after);
    thread["read_through_id"] = json!(anchor.id.to_hex());
    assert_eq!(w.counts(c).await, counts(true, 0, vec![thread]));
    assert_eq!(w.row(c).await.read_through_id, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_deleted_message_is_no_witness_and_a_surviving_reply_still_is() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now - 40 * DAY, vec![]).await;
    let reply = |author, parent, at| w.reply(c, author, &root, Some(parent), w.now + at, vec![]);

    // The actor wrote the parent and never replied under it.
    let wrote = reply(&w.actor, &root, 0).await;
    reply(&w.peer, &wrote, 10).await;
    // The actor's only reply to a peer's parent.
    let once = reply(&w.peer, &root, 0).await;
    let only = reply(&w.actor, &once, 1).await;
    reply(&w.peer, &once, 11).await;
    // Two replies by the actor to a peer's parent.
    let twice = reply(&w.peer, &root, 0).await;
    let first = reply(&w.actor, &twice, 1).await;
    reply(&w.actor, &twice, 2).await;
    let to_twice = reply(&w.peer, &twice, 12).await;
    // The actor wrote the parent and also replied under it.
    let both = reply(&w.actor, &root, 0).await;
    let under = reply(&w.actor, &both, 1).await;
    let to_both = reply(&w.peer, &both, 13).await;

    // How many replies the thread counts, the latest of them `to_both` until
    // it stops counting.
    let w = &w;
    let thread = |mentions: u32| async move {
        let row = w.row(c).await;
        assert!(!row.unread, "the root predates the horizon");
        assert_eq!(row.threads.len(), 1);
        assert_eq!(row.threads[0].mentions, mentions);
        row.threads[0].latest_id.clone()
    };
    // The four peer replies to the actor's conversations count. So do `once`
    // and `twice`: peer replies to the root, which the actor has replied to
    // (`wrote`, `both`).
    assert_eq!(thread(6).await, to_both.id.to_hex());

    // Each deletion changes only its own parent's conversation.
    w.delete(&wrote).await;
    assert_eq!(thread(5).await, to_both.id.to_hex());
    w.delete(&only).await;
    assert_eq!(thread(4).await, to_both.id.to_hex());
    w.delete(&first).await;
    assert_eq!(thread(4).await, to_both.id.to_hex());
    // With `wrote` and `both` gone the actor has no reply to the root either.
    w.delete(&both).await;
    assert_eq!(thread(2).await, to_both.id.to_hex());
    w.delete(&under).await;
    assert_eq!(thread(1).await, to_twice.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_is_scoped_to_the_replys_own_channel() {
    let w = World::new().await;
    let (a, b) = (w.channel().await, w.channel().await);
    let old = w.now - 40 * DAY;

    // The actor replied to a peer's parent, but that reply is stored in B.
    let theirs = w.post(a, &w.peer, old, vec![]).await;
    w.reply(b, &w.actor, &theirs, Some(&theirs), old, vec![])
        .await;
    let in_a = w
        .reply(a, &w.peer, &theirs, Some(&theirs), w.now, vec![])
        .await;
    assert_eq!(w.counts(a).await, counts(false, 0, vec![]));

    // The actor wrote a parent in A. A peer reply in B claims it as its parent.
    let mine = w.post(a, &w.actor, old, vec![]).await;
    w.reply(b, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    assert_eq!(w.counts(b).await, counts(false, 0, vec![]));

    // The same parents count once the actor is a member in the reply's channel.
    w.reply(a, &w.actor, &theirs, Some(&theirs), old, vec![])
        .await;
    let to_mine = w.reply(a, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    // Equal reply times: root ID order.
    let mut threads = [item(&theirs, 1, &in_a), item(&mine, 1, &to_mine)];
    threads.sort_by(|x, y| x["root_id"].as_str().cmp(&y["root_id"].as_str()));
    assert_eq!(w.counts(a).await, counts(false, 0, threads.to_vec()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_is_exact_behind_a_busy_parent() {
    let w = World::new().await;
    let c = w.channel().await;
    let old = w.now - 40 * DAY;
    let root = w.post(c, &w.peer, old, vec![]).await;
    // The actor's reply is the oldest of 259 under one parent: a 256-row
    // window over the parent's replies would never reach it.
    w.reply(c, &w.actor, &root, Some(&root), old, vec![]).await;
    let mut last = root.clone();
    for i in 0..258 {
        last = w
            .reply(c, &w.peer, &root, Some(&root), w.now + i, vec![])
            .await;
    }
    assert_eq!(
        w.counts(c).await,
        counts(false, 0, vec![item(&root, 258, &last)])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn threads_are_capped_after_replies_that_do_not_count_are_removed() {
    let w = World::new().await;
    let c = w.channel().await;
    let old = w.now - 40 * DAY;
    // The actor's thread has the oldest unread reply. A cap's worth of
    // unjoined threads are newer and would fill the list if the cap came
    // first.
    let mine = w.post(c, &w.actor, old, vec![]).await;
    let answer = w.reply(c, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    for i in 1..=MAX_THREAD_SUMMARIES as u64 {
        let root = w.post(c, &w.peer, old, vec![]).await;
        w.reply(c, &w.peer, &root, Some(&root), w.now + i, vec![])
            .await;
    }
    assert_eq!(
        w.counts(c).await,
        counts(false, 0, vec![item(&mine, 1, &answer)])
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_parent_budget_leaves_undecided_replies_out() {
    let w = World::new().await;
    let c = w.channel().await;
    // The actor is in the first conversation. Whether it is inside the budget
    // depends on ID order, so only the bounds are asserted.
    let mut mine = None;
    for i in 0..1025 {
        let author = if i == 0 { &w.actor } else { &w.peer };
        let root = w.post(c, author, w.now, vec![]).await;
        let reply = w
            .reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
            .await;
        if i == 0 {
            mine = Some(item(&root, 1, &reply));
        }
        // 1024 parents fit: 1023 peer roots, and the reply to the actor's.
        // The independent SQL deadline may still withhold the answer. Past
        // 1024, one reply is undecided. An undecided reply is left out.
        if i >= 1023 {
            let counted = w.counts(c).await;
            assert!(
                [vec![], vec![mine.clone().unwrap()]]
                    .map(|threads| counts(true, 0, threads))
                    .contains(&counted),
                "{i}: {counted}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_lookup_timeout_decides_nothing_and_preserves_the_callers_transaction() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    w.reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
        .await;
    let mut held = w.pool.begin().await.unwrap();
    // Force the real resolver to time out, then check that its outer snapshot
    // and original statement budget remain usable.
    sqlx::query("LOCK TABLE thread_metadata IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .unwrap();
    let mut reader = w.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL statement_timeout='2000ms'")
        .execute(&mut *reader)
        .await
        .unwrap();
    for lock_timeout in ["0", "10ms"] {
        sqlx::query("SELECT set_config('lock_timeout',$1,true)")
            .bind(lock_timeout)
            .execute(&mut *reader)
            .await
            .unwrap();
        sqlx::query("SET LOCAL jit=on")
            .execute(&mut *reader)
            .await
            .unwrap();
        let result = participation::resolve(
            &mut reader,
            w.community,
            &w.actor.public_key().to_bytes(),
            &[(c, root.id.as_bytes().to_vec())],
        )
        .await
        .unwrap();
        assert!(result.is_empty(), "timeout provides no negative evidence");
        let setting: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&mut *reader)
            .await
            .unwrap();
        assert_eq!(setting, "2s");
        let jit: String = sqlx::query_scalar("SHOW jit")
            .fetch_one(&mut *reader)
            .await
            .unwrap();
        assert_eq!(jit, "on", "optional inference restores caller settings");
    }
    reader.rollback().await.unwrap();
    held.rollback().await.unwrap();
    assert_eq!(w.counts(c).await, counts(true, 0, vec![]));
}
