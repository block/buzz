use super::{postgres_tests::fixture, *};
use serde_json::json;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_sql_eligibility_follows_kind_author_deletion_and_horizon() {
    let (db, pool, community, _, actor, event) = fixture().await;
    let now = chrono::Utc::now().timestamp_millis();
    let horizon = i64::from(DEFAULT_RETENTION_SECONDS) * 1000;
    // Addressed to the actor, so a counted message is also exactly one mention.
    let tags = json!([["p", actor.public_key().to_hex()]]);
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
async fn sidebar_compacted_tags_preserve_directed_and_corruption_rules() {
    let (db, pool, community, _, actor, _) = fixture().await;
    let actor_hex = actor.public_key().to_hex().to_uppercase();
    // Unusable tags prove nothing: that message is left out, neither unread
    // nor a mention.
    for (tags, unread, mentions) in [
        (json!([]), true, 0),
        (json!(["p"]), false, 0),
        (json!(["e"]), false, 0),
        (json!(["broadcast"]), false, 0),
        (json!([["p", actor_hex, "relay", "petname"]]), true, 1),
        (json!([["p", "00".repeat(32)]]), true, 0),
        (json!([["broadcast", "1", "extra"]]), true, 1),
        (json!([["broadcast", "0"]]), true, 0),
        (json!([["p", "00".repeat(32), 42]]), false, 0),
        (json!([["broadcast", "0", 42]]), false, 0),
        (json!([["e", "00".repeat(32), "", "root", 42]]), false, 0),
        (json!([["p", actor_hex], ["p", "other", 42]]), false, 0),
        (json!([["e", "00".repeat(32), "", "reply"]]), false, 0),
        (json!([["x", 42]]), true, 0),
        (json!({"p":actor_hex}), false, 0),
        (json!([["p", "x".repeat(8193)]]), false, 0),
    ] {
        sqlx::query("UPDATE events SET tags=$2 WHERE community_id=$1")
            .bind(community.as_uuid())
            .bind(&tags)
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
        let row = &page.channels[0];
        assert_eq!(
            (row.unread, row.mentions),
            (unread, mentions),
            "tags={tags}"
        );
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
async fn sidebar_directed_fact_follows_dm_mention_and_broadcast_rules() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let actor_hex = actor.public_key().to_hex();
    let fullwidth: String = actor_hex
        .chars()
        .map(|c| if c.is_ascii_alphabetic() { 'Ａ' } else { c })
        .collect();
    assert_ne!(fullwidth, actor_hex);
    // Whether each tag set is directed in a stream; in a DM every one is.
    let cases = [
        (json!([]), false),
        (json!([["p", actor_hex]]), true),
        (json!([["p", actor_hex.to_uppercase()]]), true),
        (json!([["p", fullwidth]]), false),
        (json!([["p"]]), false),
        (json!([["broadcast", "1"]]), true),
        (json!([["broadcast", "true"]]), false),
        (json!([["p", "00".repeat(32)]]), false),
        (
            json!([["broadcast", "0"], ["p", actor_hex.to_uppercase()]]),
            true,
        ),
    ];
    for channel_type in ["stream", "dm"] {
        sqlx::query(
            "UPDATE channels SET channel_type=$3::channel_type WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(channel_type)
        .execute(&pool)
        .await
        .unwrap();
        for (tags, in_stream) in &cases {
            let expected = u32::from(channel_type == "dm" || *in_stream);
            sqlx::query("UPDATE events SET tags=$2 WHERE community_id=$1")
                .bind(community.as_uuid())
                .bind(tags)
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
            assert!(page.channels[0].unread);
            assert_eq!(
                page.channels[0].mentions, expected,
                "channel_type={channel_type} tags={tags}"
            );
        }
    }
}
