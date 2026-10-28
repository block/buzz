use super::{
    postgres_tests::{fixture, insert_reply},
    *,
};
use crate::Db;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

/// Eligibility is classified at ingest (kind) and at read (author, deletion);
/// author time plays no part. Each case is stored through ingest, then
/// rewritten in place for the read-time facts.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_sql_eligibility_follows_kind_author_and_deletion() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let now = chrono::Utc::now().timestamp_millis();
    let horizon = i64::from(DEFAULT_RETENTION_SECONDS) * 1000;
    for (kind, eligible_kind) in [
        (9, true),
        (40002, true),
        (45001, true),
        (45003, true),
        (1, false),
        (7, false),
        (40008, false),
    ] {
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        // Addressed to the actor, so a counted message is also exactly one mention.
        let author = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(kind), "case")
            .tags([Tag::public_key(actor.public_key())])
            .sign_with_keys(&author)
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        for own in [false, true] {
            for deleted in [false, true] {
                // Now, and one minute past the default follow window.
                for age in [0, horizon + 60_000] {
                    let created = now - age;
                    sqlx::query("UPDATE events SET pubkey=$3,deleted_at=CASE WHEN $4 THEN now() ELSE NULL END,created_at=to_timestamp($5::double precision/1000) WHERE community_id=$1 AND id=$2")
                        .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
                        .bind(if own { actor.public_key() } else { author.public_key() }.to_bytes().as_slice())
                        .bind(deleted).bind(created as f64).execute(&pool).await.unwrap();
                    sqlx::query("UPDATE event_mentions SET event_created_at=to_timestamp($3::double precision/1000) WHERE community_id=$1 AND event_id=$2")
                        .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
                        .bind(created as f64).execute(&pool).await.unwrap();
                    let page = db
                        .personal_read_sidebar(
                            community,
                            &actor.public_key(),
                            DEFAULT_RETENTION_SECONDS,
                            20,
                            None,
                        )
                        .await
                        .unwrap();
                    let expected = eligible_kind && !own && !deleted;
                    let row = &page.channels[0];
                    assert_eq!(
                        (row.unread, row.mentions),
                        (expected, u32::from(expected)),
                        "kind={kind} own={own} deleted={deleted} age={age}"
                    );
                }
            }
        }
    }
}

/// A message with a NIP-10 reply marker but no recorded ancestry is on
/// neither read path, by the shared parser's definition of a reply.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_ancestry_fact_matches_shared_nip10_parser() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let ids = [
        "a".repeat(64),
        "A".repeat(64),
        "aB09".repeat(16),
        "a".repeat(63),
        "a".repeat(65),
        "g".repeat(64),
        "١".repeat(64),
        "Ａ".repeat(64),
        format!("{}\n", "a".repeat(64)),
    ];
    let mut cases: Vec<Vec<Vec<String>>> = Vec::new();
    for id in ids {
        for marker in ["reply", "Reply", "root", "mention"] {
            cases.push(vec![vec!["e".into(), id.clone(), "".into(), marker.into()]]);
        }
    }
    cases.push(vec![vec!["e".into(), "a".repeat(64), "reply".into()]]);
    cases.push(vec![
        vec!["e".into(), "g".repeat(64), "".into(), "reply".into()],
        vec!["e".into(), "a".repeat(64), "".into(), "reply".into()],
    ]);
    cases.push(vec![
        vec!["e".into(), "a".repeat(64), "".into(), "reply".into()],
        vec!["e".into(), "g".repeat(64), "".into(), "reply".into()],
    ]);
    for tags in cases {
        let reply =
            buzz_core::nip10::parse_thread_markers_from_parts(tags.iter().map(Vec::as_slice))
                .resolve()
                .is_some();
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        let parsed: Vec<Tag> = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().map(String::as_str)).unwrap())
            .collect();
        let event = EventBuilder::new(Kind::Custom(9), "case")
            .tags(parsed)
            .sign_with_keys(&Keys::generate())
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        let page = db
            .personal_read_sidebar(
                community,
                &actor.public_key(),
                DEFAULT_RETENTION_SECONDS,
                20,
                None,
            )
            .await
            .unwrap();
        // A reply marker without recorded ancestry leaves the message out.
        assert_eq!(page.channels[0].unread, !reply, "tags={tags:?}");
        // ...and it is never the timeline's anchor.
        assert_eq!(
            page.channels[0].latest_id.is_some(),
            !reply,
            "tags={tags:?}"
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_mentions_are_p_tags_in_a_stream_and_everything_in_a_dm() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let actor_hex = actor.public_key().to_hex();
    let fullwidth: String = actor_hex
        .chars()
        .map(|c| if c.is_ascii_alphabetic() { 'Ａ' } else { c })
        .collect();
    assert_ne!(fullwidth, actor_hex);
    let tag = |parts: &[&str]| Tag::parse(parts.iter().copied()).unwrap();
    // Whether each message mentions the actor in a stream. A top-level
    // broadcast is not a mention.
    let cases = [
        (vec![], false),
        (vec![tag(&["p", &actor_hex])], true),
        (vec![tag(&["p", &actor_hex.to_uppercase()])], true),
        (vec![tag(&["p", &fullwidth])], false),
        (vec![tag(&["broadcast", "1"])], false),
        (vec![tag(&["p", &"00".repeat(32)])], false),
        (
            vec![tag(&["broadcast", "0"]), tag(&["p", &actor_hex])],
            true,
        ),
    ];
    let mut expected = 0;
    for (tags, mentioned) in &cases {
        let event = EventBuilder::new(Kind::Custom(9), "case")
            .tags(tags.clone())
            .sign_with_keys(&Keys::generate())
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        expected += u32::from(*mentioned);
        assert_eq!(
            mentions(&db, community, &actor).await,
            expected,
            "tags={tags:?}"
        );
    }
    sqlx::query("UPDATE channels SET channel_type='dm' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    // Every message by someone else, the fixture's included.
    assert_eq!(
        mentions(&db, community, &actor).await,
        cases.len() as u32 + 1
    );
}

async fn mentions(db: &Db, community: buzz_core::CommunityId, actor: &Keys) -> u32 {
    db.personal_read_sidebar(
        community,
        &actor.public_key(),
        DEFAULT_RETENTION_SECONDS,
        20,
        None,
    )
    .await
    .unwrap()
    .channels[0]
        .mentions
}

/// THREAD_MENTIONS before the root-first rewrite, kept as the oracle: it
/// walked the actor's mentions in the channel and filtered them by root. `$4`
/// was the author-time horizon.
const CHANNEL_FIRST_THREAD_MENTIONS: &str = "SELECT count(*) AS n FROM (
        SELECT 1 FROM thread_metadata tm JOIN events e ON e.community_id=$1
            AND e.created_at=tm.event_created_at AND e.id=tm.event_id
        WHERE t.unread AND t.dm AND tm.community_id=$1 AND tm.root_event_id=t.root_id
            AND tm.received_at > t.position AND tm.channel_id=t.channel_id
            AND e.deleted_at IS NULL AND e.pubkey<>$2 AND e.created_at >= $4
        UNION ALL
        SELECT 1 FROM event_mentions em JOIN thread_metadata tm ON tm.community_id=$1
            AND tm.event_created_at=em.event_created_at AND tm.event_id=em.event_id
        JOIN events e ON e.community_id=$1 AND e.created_at=em.event_created_at AND e.id=em.event_id
        WHERE t.unread AND NOT t.dm AND em.community_id=$1 AND em.pubkey_hex=$5 AND em.channel_id=t.channel_id
            AND em.event_created_at >= GREATEST(t.position-$6, $4)
            AND tm.root_event_id=t.root_id AND tm.received_at > t.position
            AND e.deleted_at IS NULL AND e.pubkey<>$2
        LIMIT $7
    ) x";

/// Each of the actor's thread frontiers and its mention count by `query`.
async fn thread_mentions(
    pool: &sqlx::PgPool,
    community: buzz_core::CommunityId,
    actor: &Keys,
    query: &str,
) -> Vec<(String, i64)> {
    let sql = format!(
        "SELECT encode(t.root_id,'hex') AS root, tc.n FROM (
            SELECT tf.root_id, tf.channel_id, c.channel_type='dm' AS dm, true AS unread,
                GREATEST(tf.through_timestamp, COALESCE($3::timestamptz, 'infinity')) AS position,
                GREATEST(tf.through_timestamp, COALESCE($3::timestamptz, 'infinity')) AS walk_from
            FROM personal_read_frontiers tf JOIN channels c
                ON c.community_id=tf.community_id AND c.id=tf.channel_id
            WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.root_id<>''::bytea
                AND $4::timestamptz IS NOT NULL
         ) t CROSS JOIN LATERAL ({query}) tc ORDER BY 1"
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(community.as_uuid())
        .bind(actor.public_key().to_bytes().as_slice())
        .bind(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .bind(chrono::DateTime::from_timestamp(0, 0).unwrap())
        .bind(actor.public_key().to_hex())
        .bind(chrono::Duration::seconds(i64::from(
            MAX_ARRIVAL_SKEW_SECONDS,
        )))
        .bind(MAX_UNREAD_COUNT as i64)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// The root-first thread mention count matches the channel-first query it
/// replaced, across positions, own and deleted replies, mentions in sibling
/// threads and channels, and DMs.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn root_first_thread_mentions_match_the_channel_first_query() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let tag = || vec![Tag::public_key(actor.public_key())];
    let (bee, wasp) = (Keys::generate(), Keys::generate());
    let keys = [&actor, &bee, &wasp].map(|k| k.public_key().to_bytes());
    let participants: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
    let dm = db
        .create_dm(community, &participants, &keys[1])
        .await
        .unwrap()
        .id;
    let other = db
        .create_channel(
            community,
            "sibling channel",
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let mut expected = Vec::new();
    for (place, mentions) in [
        (channel, 0),
        (channel, 1),
        (channel, 3),
        (other, 2),
        (dm, 2),
    ] {
        let author = if place == dm {
            bee.clone()
        } else {
            Keys::generate()
        };
        let root = send(&db, community, place, None, &author, vec![]).await;
        // A mention before the actor's own reply is read.
        send(&db, community, place, Some(&root), &Keys::generate(), tag()).await;
        db.apply_personal_read_intent(
            community,
            &actor.public_key(),
            &ReadIntent::Follow {
                target: ReadTarget {
                    channel_id: place,
                    root_id: Some(root.id.to_hex()),
                },
            },
        )
        .await
        .unwrap();
        // Posting reads through the post; own and deleted mentions never count.
        send(&db, community, place, Some(&root), &actor, tag()).await;
        for _ in 0..mentions {
            send(&db, community, place, Some(&root), &wasp, tag()).await;
        }
        send(
            &db,
            community,
            place,
            Some(&root),
            &Keys::generate(),
            vec![],
        )
        .await;
        let gone = send(&db, community, place, Some(&root), &Keys::generate(), tag()).await;
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(gone.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        // In a DM every reply is directed at the actor.
        let n = if place == dm { mentions + 1 } else { mentions };
        expected.push((root.id.to_hex(), n));
    }
    expected.sort();
    let before = thread_mentions(&pool, community, &actor, CHANNEL_FIRST_THREAD_MENTIONS).await;
    let after = thread_mentions(&pool, community, &actor, super::projection::THREAD_MENTIONS).await;
    assert_eq!(before, expected);
    assert_eq!(after, expected);
}

async fn send(
    db: &Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    root: Option<&nostr::Event>,
    author: &Keys,
    tags: Vec<Tag>,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(9), Uuid::new_v4().to_string())
        .tags(tags)
        .sign_with_keys(author)
        .unwrap();
    match root {
        Some(root) => insert_reply(db, community, channel, root, &event).await,
        None => {
            db.insert_event(community, &event, Some(channel))
                .await
                .unwrap();
        }
    }
    event
}
