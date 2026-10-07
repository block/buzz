//! Thread membership, whole-channel reads, thread summaries and targeted refresh.
use super::postgres_tests::{
    apply, channel_read, fixture, mark, mention, now, post, post_kind, reply, sidebar, start,
    states, status,
};
use super::*;
use crate::channel::{ChannelType, ChannelVisibility};
use buzz_core::{channel::MemberRole, CommunityId};
use nostr::Keys;
use sqlx::PgPool;
use uuid::Uuid;

async fn thread_rows(pool: &PgPool, community: CommunityId, actor: &Keys) -> Vec<Vec<u8>> {
    sqlx::query_scalar(
        "SELECT root_id FROM personal_read_frontiers
         WHERE community_id=$1 AND actor=$2 AND root_id<>''::bytea ORDER BY root_id",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn replying_mentioning_and_starting_a_thread_make_members() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let base = now();
    let starter = Keys::generate();
    let bystander = Keys::generate();
    for who in [&starter, &bystander] {
        db.add_member(
            community,
            channel,
            &who.public_key().to_bytes(),
            MemberRole::Member,
            None,
        )
        .await
        .unwrap();
        start(&pool, community, who).await;
    }
    let root = post(&db, community, channel, &starter, base, vec![]).await;
    assert!(
        thread_rows(&pool, community, &starter).await.is_empty(),
        "a root alone is no thread"
    );

    // The actor replies: a member, with its own reply read. The root's author
    // becomes a member with the reply unread.
    let own = reply(
        &db,
        community,
        channel,
        &root,
        &actor,
        base + 1,
        vec![],
        false,
    )
    .await;
    let root_row = vec![root.id.as_bytes().to_vec()];
    assert_eq!(thread_rows(&pool, community, &actor).await, root_row);
    assert_eq!(thread_rows(&pool, community, &starter).await, root_row);
    assert!(thread_rows(&pool, community, &bystander).await.is_empty());
    let row = sidebar(&db, community, &starter).await;
    // Only the reply is unread: attention, but no ordinary timeline backlog.
    assert_eq!(
        (row.unread, row.attention, row.threads.len()),
        (false, true, 1)
    );
    assert_eq!(row.threads[0].latest_reply_id, own.id.to_hex());
    assert_eq!(
        states(&db, community, &starter, channel, Some(&root), &[&own]).await,
        [serde_json::json!({"status":"unread","reason":"conversation"})]
    );
    let row = sidebar(&db, community, &actor).await;
    assert!(row.threads.is_empty());

    // Someone else replies: the actor, now a member, has it unread, beside
    // the fixture's message and the root on the timeline.
    let other = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 2,
        vec![],
        false,
    )
    .await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        (row.unread, row.attention, row.threads.len()),
        (true, true, 1)
    );
    assert_eq!(row.threads[0].latest_reply_id, other.id.to_hex());

    // A mention makes a channel member a thread member; a non-member gets nothing.
    let outsider = Keys::generate();
    let mut tags = mention(&bystander);
    tags.extend(mention(&outsider));
    let mentioned = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 3,
        tags,
        false,
    )
    .await;
    assert_eq!(thread_rows(&pool, community, &bystander).await, root_row);
    assert!(thread_rows(&pool, community, &outsider).await.is_empty());
    let row = sidebar(&db, community, &bystander).await;
    // The mention, and the root on the timeline.
    assert_eq!(
        (row.unread, row.attention, row.threads.len()),
        (true, true, 1)
    );
    assert_eq!(
        states(
            &db,
            community,
            &bystander,
            channel,
            Some(&root),
            &[&other, &mentioned]
        )
        .await,
        [
            serde_json::json!({"status":"read"}),
            serde_json::json!({"status":"unread","reason":"mention"})
        ]
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reading_a_thread_moves_a_members_row_and_never_joins() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = now();
    let first = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base,
        vec![],
        false,
    )
    .await;
    let reader = Keys::generate();
    // Non-members may mark (the outcome is the same), but get no row.
    assert_eq!(
        apply(&db, community, &reader, mark(channel, Some(&root), &first)).await,
        IntentOutcome::Applied
    );
    assert!(thread_rows(&pool, community, &reader).await.is_empty());
    let later = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 1,
        vec![],
        false,
    )
    .await;
    assert_eq!(
        status(&db, community, &reader, channel, Some(&root), &[&later]).await,
        ["not_counted"]
    );

    // A member's row moves through the anchor, which may be the root itself.
    reply(
        &db,
        community,
        channel,
        &root,
        &actor,
        base + 2,
        vec![],
        false,
    )
    .await;
    let newest = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 3,
        vec![],
        false,
    )
    .await;
    assert_eq!(sidebar(&db, community, &actor).await.threads.len(), 1);
    assert_eq!(
        apply(&db, community, &actor, mark(channel, Some(&root), &root)).await,
        IntentOutcome::Applied
    );
    assert_eq!(
        sidebar(&db, community, &actor).await.threads.len(),
        1,
        "never backwards"
    );
    assert_eq!(
        apply(&db, community, &actor, mark(channel, Some(&root), &newest)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        (row.unread, row.threads.len()),
        (true, 0),
        "the root itself is on the timeline"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_replies_make_every_member_a_thread_member() {
    let (db, pool, community, _, actor, _) = fixture().await;
    let peer = Keys::generate();
    let dm = db
        .create_channel(
            community,
            "dm",
            ChannelType::Dm,
            ChannelVisibility::Private,
            None,
            &peer.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let inviter = peer.public_key().to_bytes();
    db.add_member(
        community,
        dm,
        &actor.public_key().to_bytes(),
        MemberRole::Member,
        Some(&inviter),
    )
    .await
    .unwrap();
    let root = post(&db, community, dm, &Keys::generate(), now(), vec![]).await;
    reply(&db, community, dm, &root, &peer, now(), vec![], false).await;
    assert_eq!(
        thread_rows(&pool, community, &actor).await,
        vec![root.id.as_bytes().to_vec()]
    );
    let row = db
        .personal_read_sidebar_channels(community, &actor.public_key(), &[dm])
        .await
        .unwrap()
        .channels
        .remove(0);
    assert_eq!(
        (row.unread, row.attention, row.threads.len()),
        (false, true, 1)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_channel_read_covers_every_thread_through_a_reply_anchor() {
    let (db, _, community, channel, actor, root) = fixture().await;
    let base = now();
    reply(&db, community, channel, &root, &actor, base, vec![], false).await;
    let first = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 10,
        vec![],
        false,
    )
    .await;
    post(
        &db,
        community,
        channel,
        &Keys::generate(),
        base + 20,
        vec![],
    )
    .await;
    let second = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 30,
        vec![],
        false,
    )
    .await;

    // A plain reply is no timeline anchor, but anchors a whole-channel read.
    assert_eq!(
        apply(&db, community, &actor, mark(channel, None, &first)).await,
        IntentOutcome::Blocked
    );
    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &first)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        (row.unread, row.attention),
        (true, true),
        "top at +20 and reply at +30 remain"
    );
    assert_eq!(row.threads.len(), 1);
    assert_eq!(row.threads[0].latest_reply_id, second.id.to_hex());

    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &second)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        (row.unread, row.attention, row.threads.len()),
        (false, false, 0),
        "every thread is covered"
    );

    // A reply that arrives after the cut is unread, though backdated.
    let late = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 5,
        vec![],
        false,
    )
    .await;
    let newer = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 40,
        vec![],
        false,
    )
    .await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.attention), (false, true));
    assert_eq!(row.threads[0].latest_reply_id, newer.id.to_hex());
    assert_eq!(
        status(
            &db,
            community,
            &actor,
            channel,
            Some(&root),
            &[&second, &late, &newer]
        )
        .await,
        ["read", "unread", "unread"]
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_channel_read_validates_its_anchor() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    for (message_id, outcome) in [
        ("cd".repeat(32), IntentOutcome::Blocked),
        ("zz".to_owned(), IntentOutcome::Invalid),
    ] {
        let intent = ReadIntent::MarkChannelRead {
            channel_id: channel,
            message_id,
        };
        assert_eq!(apply(&db, community, &actor, intent).await, outcome);
    }
    let reaction = post_kind(&db, community, channel, &Keys::generate(), 7, now(), vec![]).await;
    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &reaction)).await,
        IntentOutcome::Blocked
    );
    // Ordinary channel marks never set the whole-channel cut.
    assert_eq!(
        apply(&db, community, &actor, mark(channel, None, &root)).await,
        IntentOutcome::Applied
    );
    let cuts: Vec<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
        "SELECT threads_through_timestamp FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(cuts, vec![None]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_summaries_order_cap_and_count_every_unread_thread() {
    let (db, _, community, channel, actor, _) = fixture().await;
    let base = now();
    let mut roots = Vec::new();
    for i in 0..6 {
        let root = post(&db, community, channel, &actor, base + i, vec![]).await;
        roots.push(root);
    }
    // Threads 0 and 1 tie on newest reply time; thread 2 has three replies.
    let mut anchors = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        let at = base + 100 + [50, 50, 40, 30, 20, 10][i];
        anchors.push(
            reply(
                &db,
                community,
                channel,
                root,
                &Keys::generate(),
                at,
                vec![],
                false,
            )
            .await,
        );
    }
    // Arriving last, with older author times: the anchor, not the display time.
    let mut last = None;
    for at in [base + 101, base + 102] {
        last = Some(
            reply(
                &db,
                community,
                channel,
                &roots[2],
                &Keys::generate(),
                at,
                vec![],
                false,
            )
            .await,
        );
    }
    let row = sidebar(&db, community, &actor).await;
    // The fixture's message is one ordinary timeline unread.
    assert_eq!((row.unread, row.attention, row.threads.len()), (true, true, 5));
    let mut tied = [roots[0].id.to_hex(), roots[1].id.to_hex()];
    tied.sort();
    let order: Vec<_> = row.threads.iter().map(|t| t.root_id.clone()).collect();
    assert_eq!(
        order,
        [
            tied[0].clone(),
            tied[1].clone(),
            roots[2].id.to_hex(),
            roots[3].id.to_hex(),
            roots[4].id.to_hex()
        ]
    );
    assert_eq!(row.threads[2].latest_reply_id, last.unwrap().id.to_hex());
    assert_eq!(row.threads[2].latest_reply_at, (base + 140) as i64);
    // The row's activity is the greatest author time among the newest messages.
    assert_eq!(row.latest_message_at, Some((base + 150) as i64));

    // Reading a listed thread through its anchor promotes the sixth.
    let first = &row.threads[0];
    let intent = ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: Some(first.root_id.clone()),
        },
        message_id: first.latest_reply_id.clone(),
    };
    assert_eq!(
        apply(&db, community, &actor, intent).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.threads.len(), 5);
    assert!(row.threads.iter().all(|t| t.root_id != first.root_id));
    // Plus the fixture's timeline message.
    assert!(row.unread);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_on_a_never_unread_root_is_selectable_and_readable_by_itself() {
    let (db, _, community, channel, actor, root) = fixture().await;
    let base = now();
    // A diff is never unread, not even one addressed to the actor.
    let diff = post_kind(
        &db,
        community,
        channel,
        &Keys::generate(),
        40008,
        base,
        mention(&actor),
    )
    .await;
    let on_diff = reply(
        &db,
        community,
        channel,
        &diff,
        &Keys::generate(),
        base + 10,
        mention(&actor),
        false,
    )
    .await;
    let elsewhere = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base + 20,
        mention(&actor),
        false,
    )
    .await;

    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.attention), (true, true), "the root and two replies");
    assert_eq!(row.threads.len(), 2);
    let listed = &row.threads[1];
    assert_eq!(listed.root_id, diff.id.to_hex());
    assert_eq!(listed.latest_reply_id, on_diff.id.to_hex());
    assert_eq!(
        states(&db, community, &actor, channel, Some(&diff), &[&on_diff]).await,
        [serde_json::json!({"status":"unread","reason":"mention"})]
    );
    assert_eq!(
        status(&db, community, &actor, channel, None, &[&diff]).await,
        ["not_counted"]
    );
    // The diff is no anchor; the listed reply reads its thread alone.
    assert_eq!(
        apply(&db, community, &actor, mark(channel, Some(&diff), &diff)).await,
        IntentOutcome::Blocked
    );
    assert_eq!(
        apply(&db, community, &actor, mark(channel, Some(&diff), &on_diff)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.attention), (true, true));
    assert_eq!(row.threads.len(), 1);
    assert_eq!(row.threads[0].latest_reply_id, elsewhere.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn targeted_sidebar_returns_only_requested_joined_channels() {
    let (db, _pool, community, channel, actor, _) = fixture().await;
    let create = |name: &'static str, owner: Keys| {
        let db = db.clone();
        async move {
            db.create_channel(
                community,
                name,
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &owner.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap()
            .id
        }
    };
    let joined = create("joined", actor.clone()).await;
    let foreign = create("not joined", Keys::generate()).await;
    let _unrequested = create("unrequested", actor.clone()).await;
    let page = db
        .personal_read_sidebar_channels(community, &actor.public_key(), &[joined, foreign, channel])
        .await
        .unwrap();
    let mut expected = vec![channel, joined];
    expected.sort();
    let ids: Vec<_> = page.channels.iter().map(|c| c.channel_id).collect();
    assert_eq!(ids, expected);
    assert!(page.next_cursor.is_none());
    for bad in [
        vec![],
        vec![channel, channel],
        (0..21).map(|_| Uuid::new_v4()).collect(),
    ] {
        assert!(db
            .personal_read_sidebar_channels(community, &actor.public_key(), &bad)
            .await
            .is_err());
    }
}
