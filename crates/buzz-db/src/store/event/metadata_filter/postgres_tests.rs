use super::ChannelMetadataRead;
use crate::event::{count_events, insert_event, query_events, EventQuery};
use buzz_core::CommunityId;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::PgPool;
use uuid::Uuid;

async fn community(pool: &PgPool) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("metadata-{id}.example"))
        .execute(pool)
        .await
        .unwrap();
    CommunityId::from_uuid(id)
}

async fn channel(pool: &PgPool, community: CommunityId, id: Uuid, private: bool) {
    sqlx::query("INSERT INTO channels (id, community_id, name, visibility, created_by) VALUES ($1, $2, 'metadata', $3::channel_visibility, $4)")
        .bind(id).bind(community.as_uuid()).bind(if private { "private" } else { "open" })
        .bind(vec![7_u8; 32]).execute(pool).await.unwrap();
}

fn event(keys: &Keys, channel: Uuid, time: u64, label: &str) -> Event {
    EventBuilder::new(Kind::Custom(39000), "")
        .tags([
            Tag::parse(["d", &channel.to_string()]).unwrap(),
            Tag::parse(["t", "stream"]).unwrap(),
            Tag::parse(["L", "nip-cl"]).unwrap(),
            Tag::parse(["l", label, "nip-cl"]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn current_metadata_acl_signer_and_head_precede_filters_and_limit() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let a = community(&pool).await;
    let b = community(&pool).await;
    let relay = Keys::generate();
    let outsider = Keys::generate();
    let reader = Keys::generate();
    let visible = Uuid::new_v4();
    let superseded = Uuid::new_v4();
    let hidden = Uuid::new_v4();
    let deleted = Uuid::new_v4();
    let forged = Uuid::new_v4();
    for id in [visible, superseded, hidden, deleted, forged] {
        channel(&pool, a, id, id == hidden).await;
    }
    channel(&pool, b, visible, false).await;
    let now = Timestamp::now().as_secs();
    let oldest_match = event(&relay, visible, now - 100, "a");
    for (scope, candidate, id) in [
        (a, oldest_match.clone(), visible),
        (b, event(&relay, visible, now + 10, "a"), visible),
        (a, event(&relay, superseded, now - 50, "a"), superseded),
        (a, event(&relay, superseded, now - 40, "b"), superseded),
        (a, event(&relay, hidden, now - 30, "a"), hidden),
        (a, event(&relay, deleted, now - 20, "a"), deleted),
        (a, event(&outsider, forged, now - 10, "a"), forged),
    ] {
        insert_event(&pool, scope, &candidate, Some(id))
            .await
            .unwrap();
    }
    sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(a.as_uuid())
        .bind(deleted)
        .execute(&pool)
        .await
        .unwrap();
    let mut query = EventQuery::for_community(a);
    query.kinds = Some(vec![39000]);
    query.limit = Some(1);
    // Deliberately stale catalog: the SQL ACL must reject hidden/deleted rows.
    query.channel_ids = Some(vec![visible, superseded, hidden, deleted, forged]);
    query.channel_metadata_read = Some(ChannelMetadataRead {
        reader: reader.public_key().to_bytes().to_vec(),
        relay: relay.public_key().to_bytes().to_vec(),
    });
    query.tag_filters = vec![
        ("L".into(), vec!["nip-cl".into()]),
        ("l".into(), vec!["a".into()]),
    ];
    let rows = query_events(&pool, &query).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event.id, oldest_match.id);
    assert_eq!(count_events(&pool, &query).await.unwrap(), 1);

    // An until bound or explicit event ID cannot resurrect an old matching head.
    query.channel_id = Some(superseded);
    query.until = Some(chrono::DateTime::from_timestamp((now - 45) as i64, 0).unwrap());
    assert!(query_events(&pool, &query).await.unwrap().is_empty());
    query.until = None;
    query.channel_id = None;

    sqlx::query("INSERT INTO channel_members (community_id, channel_id, pubkey, role) VALUES ($1, $2, $3, 'member')")
        .bind(a.as_uuid()).bind(hidden).bind(reader.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
    assert_eq!(
        query_events(&pool, &query).await.unwrap()[0].channel_id,
        Some(hidden)
    );
    assert_eq!(count_events(&pool, &query).await.unwrap(), 2);
    sqlx::query(
        "UPDATE channel_members SET removed_at = NOW() WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(a.as_uuid())
    .bind(hidden)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        query_events(&pool, &query).await.unwrap()[0].event.id,
        oldest_match.id
    );
    assert_eq!(count_events(&pool, &query).await.unwrap(), 1);

    // Archive preserves readability; a tied replacement selects the lowest ID.
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(a.as_uuid())
        .bind(visible)
        .execute(&pool)
        .await
        .unwrap();
    let tied = event(&relay, visible, now - 100, "b");
    insert_event(&pool, a, &tied, Some(visible)).await.unwrap();
    let (winner, loser) = if tied.id < oldest_match.id {
        (&tied, &oldest_match)
    } else {
        (&oldest_match, &tied)
    };
    query.tag_filters.clear();
    query.channel_id = Some(visible);
    assert_eq!(
        query_events(&pool, &query).await.unwrap()[0].event.id,
        winner.id
    );
    assert_eq!(count_events(&pool, &query).await.unwrap(), 1);
    query.ids = Some(vec![loser.id.as_bytes().to_vec()]);
    assert!(query_events(&pool, &query).await.unwrap().is_empty());
    assert_eq!(count_events(&pool, &query).await.unwrap(), 0);
}
