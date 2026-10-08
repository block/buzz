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

    /// The wire state of one message in the channel timeline or `root`'s thread.
    async fn state(
        &self,
        channel: Uuid,
        root: Option<&nostr::Event>,
        message: &nostr::Event,
    ) -> Value {
        let page = self
            .db
            .personal_read_contexts(
                self.community,
                &self.actor.public_key(),
                DEFAULT_RETENTION_SECONDS,
                &[ContextQuery {
                    target: ReadTarget {
                        channel_id: channel,
                        root_id: root.map(|root| root.id.to_hex()),
                    },
                    message_ids: vec![message.id.to_hex()],
                }],
            )
            .await
            .unwrap();
        let mut state = wire(&page)["contexts"][0]["messages"][0].take();
        state.as_object_mut().unwrap().remove("message_id");
        state
    }

    fn mention(&self) -> Tag {
        Tag::parse(["p", &self.actor.public_key().to_hex()]).unwrap()
    }
}

fn wire<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

fn exact(value: u32) -> Value {
    json!({"status":"exact","value":value})
}

/// Unread with a reason; `Value::Null` for an ordinary top-level message.
fn unread(reason: impl Into<Value>) -> Value {
    json!({"status":"unread","reason":reason.into()})
}

fn status(status: &str) -> Value {
    json!({ "status": status })
}

fn broadcast() -> Tag {
    Tag::parse(["broadcast", "1"]).unwrap()
}

fn item(root: &nostr::Event, unread: u32, latest: &nostr::Event) -> Value {
    json!({"root_id":root.id.to_hex(),"unread":exact(unread),
        "latest_reply_id":latest.id.to_hex(),"latest_reply_at":latest.created_at.as_secs()})
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
    assert_eq!(w.state(c, None, &root).await, status("not_counted"));
    assert_eq!(
        w.state(c, Some(&root), &answer).await,
        unread("conversation")
    );
    assert_eq!(
        w.state(c, Some(&root), &nested).await,
        status("not_counted")
    );
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(1));
    assert_eq!(wire(&row.attention), exact(1));
    // The newer reply that does not count is not the thread's preview.
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 1, &answer)],"complete":true})
    );

    // Joining the nested conversation makes its earlier reply count. The
    // actor's own reply adds nothing.
    w.reply(c, &w.actor, &root, Some(&answer), w.now + 3, vec![])
        .await;
    assert_eq!(
        w.state(c, Some(&root), &nested).await,
        unread("conversation")
    );
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(2));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 2, &nested)],"complete":true})
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
    let elsewhere = w
        .reply(c, &w.peer, &root, Some(&other), w.now + 2, vec![])
        .await;
    assert_eq!(
        w.state(c, Some(&root), &sibling).await,
        unread("conversation")
    );
    assert_eq!(
        w.state(c, Some(&root), &elsewhere).await,
        status("not_counted")
    );
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(1));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 1, &sibling)],"complete":true})
    );

    // A conversation the actor never joined: one unread top-level message, and
    // a reply that is not unread even in its own thread.
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let reply = w
        .reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
        .await;
    assert_eq!(w.state(c, None, &root).await, unread(Value::Null));
    assert_eq!(w.state(c, Some(&root), &reply).await, status("not_counted"));
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(1));
    assert_eq!(wire(&row.attention), exact(0));
    assert_eq!(wire(&row.threads), json!({"items":[],"complete":true}));

    // A reply whose parent was never recorded is undecided, not absent.
    let orphan = w.reply(c, &w.peer, &root, None, w.now + 2, vec![]).await;
    assert_eq!(w.state(c, Some(&root), &orphan).await, status("unknown"));
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), json!({"status":"at_least","value":1}));
    assert_eq!(wire(&row.attention), status("unknown"));
    assert_eq!(wire(&row.threads), json!({"items":[],"complete":false}));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_directed_reply_counts_outside_the_actors_conversations() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let parent = w.reply(c, &w.peer, &root, Some(&root), w.now, vec![]).await;
    let own = w
        .reply(c, &w.actor, &root, Some(&root), w.now + 1, vec![])
        .await;
    let mention = w
        .reply(
            c,
            &w.peer,
            &root,
            Some(&parent),
            w.now + 2,
            vec![w.mention()],
        )
        .await;
    let shout = w
        .reply(
            c,
            &w.peer,
            &root,
            Some(&parent),
            w.now + 3,
            vec![broadcast()],
        )
        .await;
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
    assert_eq!(w.state(c, Some(&root), &own).await, status("not_counted"));
    assert_eq!(w.state(c, Some(&root), &mention).await, unread("mention"));
    assert_eq!(w.state(c, Some(&root), &shout).await, unread("broadcast"));
    assert_eq!(w.state(c, Some(&root), &both).await, unread("mention"));
    // The root, the actor's sibling `parent`, and the three directed replies.
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(5));
    assert_eq!(wire(&row.attention), exact(4));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 4, &both)],"complete":true})
    );

    // Conversation outranks broadcast, never mention. Counts do not move.
    w.reply(c, &w.actor, &root, Some(&parent), w.now + 5, vec![])
        .await;
    assert_eq!(w.state(c, Some(&root), &mention).await, unread("mention"));
    assert_eq!(
        w.state(c, Some(&root), &shout).await,
        unread("conversation")
    );
    assert_eq!(wire(&w.row(c).await.unread), exact(5));

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
    assert_eq!(w.state(dm, None, &root).await, unread("direct"));
    assert_eq!(w.state(dm, Some(&root), &reply).await, unread("direct"));
    let row = w.row(dm).await;
    assert_eq!(wire(&row.unread), exact(3));
    assert_eq!(wire(&row.attention), exact(3));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn eligibility_then_the_read_frontier_are_reported_before_membership() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let parent = w.reply(c, &w.peer, &root, Some(&root), w.now, vec![]).await;
    let reply = |author, at, tags| w.reply(c, author, &root, Some(&parent), w.now + at, tags);
    // Peer replies to a parent the actor neither wrote nor replied to.
    let outside = reply(&w.peer, 1, vec![]).await;
    let deleted = reply(&w.peer, 2, vec![]).await;
    w.delete(&deleted).await;
    let own = w
        .reply(c, &w.actor, &root, Some(&root), w.now + 3, vec![])
        .await;
    let anchor = reply(&w.peer, 4, vec![w.mention()]).await;
    let later = reply(&w.peer, 5, vec![]).await;
    assert_eq!(
        w.state(c, Some(&root), &outside).await,
        status("not_counted")
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
    for (message, expected) in [
        (&outside, "read"),
        (&deleted, "not_counted"),
        (&own, "not_counted"),
        (&anchor, "read"),
        (&later, "not_counted"),
    ] {
        assert_eq!(w.state(c, Some(&root), message).await, status(expected));
    }
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
    let to_wrote = reply(&w.peer, &wrote, 10).await;
    // The actor's only reply to a peer's parent.
    let once = reply(&w.peer, &root, 0).await;
    let only = reply(&w.actor, &once, 1).await;
    let to_once = reply(&w.peer, &once, 11).await;
    // Two replies by the actor to a peer's parent.
    let twice = reply(&w.peer, &root, 0).await;
    let first = reply(&w.actor, &twice, 1).await;
    reply(&w.actor, &twice, 2).await;
    let to_twice = reply(&w.peer, &twice, 12).await;
    // The actor wrote the parent and also replied under it.
    let both = reply(&w.actor, &root, 0).await;
    let under = reply(&w.actor, &both, 1).await;
    let to_both = reply(&w.peer, &both, 13).await;

    // Whether each counts: `unread` in a conversation, or else `not_counted`.
    let counted = || async {
        let mut counted = Vec::new();
        for message in [&to_wrote, &to_once, &to_twice, &to_both] {
            let state = w.state(c, Some(&root), message).await;
            let yes = state == unread("conversation");
            assert!(yes || state == status("not_counted"), "{state}");
            counted.push(yes);
        }
        counted
    };
    let (yes, no) = (true, false);
    // `once` and `twice` are peer replies to the root, which the actor has
    // replied to (`wrote`, `both`), so they count as well.
    assert_eq!(counted().await, [yes, yes, yes, yes]);
    assert_eq!(wire(&w.row(c).await.unread), exact(6));

    // Each deletion changes only its own parent's conversation.
    w.delete(&wrote).await;
    assert_eq!(counted().await, [no, yes, yes, yes]);
    w.delete(&only).await;
    assert_eq!(counted().await, [no, no, yes, yes]);
    w.delete(&first).await;
    assert_eq!(counted().await, [no, no, yes, yes]);
    w.delete(&both).await;
    assert_eq!(counted().await, [no, no, yes, yes]);
    assert_eq!(wire(&w.row(c).await.unread), exact(2));
    w.delete(&under).await;
    assert_eq!(counted().await, [no, no, yes, no]);
    // With `wrote` and `both` gone the actor has no reply to the root either.
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(1));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 1, &to_twice)],"complete":true})
    );
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
    assert_eq!(
        w.state(a, Some(&theirs), &in_a).await,
        status("not_counted")
    );
    assert_eq!(wire(&w.row(a).await.unread), exact(0));

    // The actor wrote a parent in A. A peer reply in B claims it as its parent.
    let mine = w.post(a, &w.actor, old, vec![]).await;
    w.reply(b, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    let row = w.row(b).await;
    assert_eq!(wire(&row.unread), exact(0));
    assert_eq!(wire(&row.threads), json!({"items":[],"complete":true}));

    // The same parents count once the actor is a member in the reply's channel.
    w.reply(a, &w.actor, &theirs, Some(&theirs), old, vec![])
        .await;
    let to_mine = w.reply(a, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    assert_eq!(
        w.state(a, Some(&theirs), &in_a).await,
        unread("conversation")
    );
    assert_eq!(
        w.state(a, Some(&mine), &to_mine).await,
        unread("conversation")
    );
    assert_eq!(wire(&w.row(a).await.unread), exact(2));
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
    assert_eq!(w.state(c, Some(&root), &last).await, unread("conversation"));
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(258));
    assert_eq!(wire(&row.attention), exact(258));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&root, 258, &last)],"complete":true})
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn threads_are_capped_after_replies_that_do_not_count_are_removed() {
    let w = World::new().await;
    let c = w.channel().await;
    let old = w.now - 40 * DAY;
    // The actor's thread has the oldest unread reply. Five unjoined threads
    // are newer and would fill the list if the cap came first.
    let mine = w.post(c, &w.actor, old, vec![]).await;
    let answer = w.reply(c, &w.peer, &mine, Some(&mine), w.now, vec![]).await;
    for i in 1..=5 {
        let root = w.post(c, &w.peer, old, vec![]).await;
        w.reply(c, &w.peer, &root, Some(&root), w.now + i, vec![])
            .await;
    }
    let row = w.row(c).await;
    assert_eq!(wire(&row.unread), exact(1));
    assert_eq!(
        wire(&row.threads),
        json!({"items":[item(&mine, 1, &answer)],"complete":true})
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_parent_budget_leaves_replies_undecided_and_fabricates_no_absence() {
    let w = World::new().await;
    let c = w.channel().await;
    // The actor is in the first conversation. Whether it is inside the budget
    // depends on ID order, so only the bounds are asserted.
    for i in 0..1025 {
        let author = if i == 0 { &w.actor } else { &w.peer };
        let root = w.post(c, author, w.now, vec![]).await;
        w.reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
            .await;
        if i == 1023 {
            // 1024 parents fit: 1023 peer roots, and the reply to the actor's.
            // The independent SQL deadline may still withhold the answer.
            let row = w.row(c).await;
            assert!(
                [exact(1024), json!({"status":"at_least","value":1023})]
                    .contains(&wire(&row.unread)),
                "{:?}",
                row.unread
            );
            assert!(
                [exact(1), status("unknown")].contains(&wire(&row.attention)),
                "{:?}",
                row.attention
            );
        }
    }
    // 1025 parents do not: one reply is undecided, so nothing is exact.
    let row = w.row(c).await;
    assert!(
        [1024, 1025]
            .map(|value| json!({"status":"at_least","value":value}))
            .contains(&wire(&row.unread)),
        "{:?}",
        row.unread
    );
    assert!(
        [json!({"status":"at_least","value":1}), status("unknown")].contains(&wire(&row.attention)),
        "{:?}",
        row.attention
    );
    assert!(!row.threads.complete);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_lookup_timeout_decides_nothing_and_preserves_the_callers_transaction() {
    let w = World::new().await;
    let c = w.channel().await;
    let root = w.post(c, &w.peer, w.now, vec![]).await;
    let reply = w
        .reply(c, &w.peer, &root, Some(&root), w.now + 1, vec![])
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
    assert_eq!(w.state(c, Some(&root), &reply).await, status("not_counted"));
}
