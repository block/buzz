//! The sidebar's SQL and the per-message selector apply the same rules.
use super::{classification, postgres_tests::fixture, *};
use serde_json::json;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_sql_eligibility_matches_selector() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    let query = [ContextQuery {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_ids: vec![event.id.to_hex()],
    }];
    for kind in [9, 40002, 45001, 45003, 1, 7, 39002, 40008] {
        for own in [false, true] {
            for deleted in [false, true] {
                sqlx::query("UPDATE events SET kind=$2,pubkey=$3,deleted_at=CASE WHEN $4 THEN now() ELSE NULL END WHERE community_id=$1")
                    .bind(community.as_uuid()).bind(kind)
                    .bind(if own { actor.public_key().to_bytes() } else { event.pubkey.to_bytes() }.as_slice())
                    .bind(deleted).execute(&pool).await.unwrap();
                let page = db
                    .personal_read_sidebar(community, &actor.public_key(), 20, None)
                    .await
                    .unwrap();
                let expected = u32::from(ELIGIBLE_KINDS.contains(&kind) && !own && !deleted);
                assert_eq!(
                    u32::from(page.channels[0].unread), expected,
                    "kind={kind} own={own} deleted={deleted}"
                );
                let contexts = db
                    .personal_read_contexts(community, &actor.public_key(), &query)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(&contexts).unwrap()["contexts"][0]["messages"][0]
                        ["status"],
                    ["not_counted", "unread"][expected as usize],
                    "kind={kind} own={own} deleted={deleted}"
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_directed_fact_matches_selector_classifier() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    let actor_hex = actor.public_key().to_hex();
    let fullwidth: String = actor_hex
        .chars()
        .map(|c| if c.is_ascii_alphabetic() { 'Ａ' } else { c })
        .collect();
    assert_ne!(fullwidth, actor_hex);
    let cases: Vec<Vec<Vec<String>>> = serde_json::from_value(json!([
        [],
        [["p", actor_hex]],
        [["p", actor_hex.to_uppercase()]],
        [["p", actor_hex, "relay", "petname"]],
        [["p", fullwidth]],
        [["p"]],
        [["broadcast", "1"]],
        [["broadcast", "1", "extra"]],
        [["broadcast", "true"]],
        [["p", "00".repeat(32)]],
        [["broadcast", "0"], ["p", actor_hex.to_uppercase()]]
    ]))
    .unwrap();
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
        for tags in &cases {
            // A top-level message: `broadcast` tags alone direct nothing.
            let reason = classification::reason(channel_type, &actor_hex, tags, false);
            let expected = (channel_type != "dm" && reason.is_none(), reason.is_some());
            sqlx::query("UPDATE events SET tags=$2 WHERE community_id=$1")
                .bind(community.as_uuid())
                .bind(json!(tags))
                .execute(&pool)
                .await
                .unwrap();
            // Mirror ingest's mention index (`runtime::insert_mentions`).
            sqlx::query("DELETE FROM event_mentions WHERE community_id=$1")
                .bind(community.as_uuid())
                .execute(&pool)
                .await
                .unwrap();
            let mentioned = tags.iter().any(|t| {
                t.len() >= 2 && t[0] == "p" && t[1].eq_ignore_ascii_case(&actor_hex)
            });
            if mentioned {
                sqlx::query(
                    "INSERT INTO event_mentions (community_id,pubkey_hex,event_id,event_created_at,channel_id,event_kind)
                     SELECT community_id,$2,id,created_at,channel_id,kind FROM events WHERE community_id=$1 AND id=$3",
                )
                .bind(community.as_uuid())
                .bind(&actor_hex)
                .bind(event.id.as_bytes().as_slice())
                .execute(&pool)
                .await
                .unwrap();
            }
            let row = db
                .personal_read_sidebar(community, &actor.public_key(), 20, None)
                .await
                .unwrap()
                .channels
                .remove(0);
            assert_eq!(
                (row.unread, row.attention),
                expected,
                "channel_type={channel_type} tags={tags:?}"
            );
        }
    }
}
