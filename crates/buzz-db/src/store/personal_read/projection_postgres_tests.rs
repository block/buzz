use super::{postgres_tests::fixture, *};
use crate::Db;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::json;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_sql_eligibility_follows_kind_author_deletion_and_horizon() {
    let (db, pool, community, _, actor, event) = fixture().await;
    let now = chrono::Utc::now().timestamp_millis();
    let horizon = i64::from(DEFAULT_RETENTION_SECONDS) * 1000;
    // Addressed to the actor, so a counted message is also exactly one mention.
    let tags = json!([["p", actor.public_key().to_hex()]]);
    sqlx::query(
        "INSERT INTO event_mentions
            (community_id,pubkey_hex,event_id,event_created_at,channel_id,event_kind)
         SELECT community_id,$2,id,created_at,channel_id,kind FROM events WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_hex())
    .execute(&pool)
    .await
    .unwrap();
    for (kind, eligible_kind) in [
        (9, true),
        (40002, true),
        (45001, true),
        (45003, true),
        (1, false),
        (7, false),
        (39002, false),
        (40008, false),
    ] {
        for own in [false, true] {
            for deleted in [false, true] {
                // Now, then one minute inside and one minute outside the horizon.
                for (age, inside) in [
                    (0, true),
                    (horizon - 60_000, true),
                    (horizon + 60_000, false),
                ] {
                    let created = now - age;
                    sqlx::query("UPDATE events SET kind=$2,pubkey=$3,deleted_at=CASE WHEN $4 THEN now() ELSE NULL END,created_at=to_timestamp($5::double precision/1000),tags=$6 WHERE community_id=$1")
                        .bind(community.as_uuid()).bind(kind)
                        .bind(if own { actor.public_key().to_bytes() } else { event.pubkey.to_bytes() }.as_slice())
                        .bind(deleted).bind(created as f64).bind(&tags).execute(&pool).await.unwrap();
                    sqlx::query("UPDATE event_mentions SET event_created_at=to_timestamp($2::double precision/1000) WHERE community_id=$1")
                        .bind(community.as_uuid()).bind(created as f64).execute(&pool).await.unwrap();
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
                    let expected = eligible_kind && !own && !deleted && inside;
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_ancestry_fact_matches_shared_nip10_parser() {
    let (db, pool, community, _, actor, _) = fixture().await;
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
        sqlx::query("UPDATE events SET tags=$2 WHERE community_id=$1")
            .bind(community.as_uuid())
            .bind(json!(tags))
            .execute(&pool)
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
