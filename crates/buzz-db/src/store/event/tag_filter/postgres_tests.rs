//! Real SQL binds and NIP-01 matching, including adversarial tag positions.

use buzz_core::{filter::filters_match, CommunityId};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use sqlx::PgPool;
use uuid::Uuid;

use crate::event::{count_events, insert_event, query_events, EventQuery};

async fn community(pool: &PgPool) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("tag-filter-{id}.example"))
        .execute(pool)
        .await
        .unwrap();
    CommunityId::from_uuid(id)
}

async fn channel(pool: &PgPool, community: CommunityId) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1, $2, 'tags', $3)",
    )
    .bind(id)
    .bind(community.as_uuid())
    .bind(vec![7_u8; 32])
    .execute(pool)
    .await
    .unwrap();
    id
}

fn event(keys: &Keys, channel: Uuid, time: u64, tags: &[&[&str]]) -> Event {
    let mut parsed = vec![Tag::parse(["d", &channel.to_string()]).unwrap()];
    parsed.extend(
        tags.iter()
            .map(|parts| Tag::parse(parts.iter().copied()).unwrap()),
    );
    EventBuilder::new(Kind::Custom(39000), "")
        .tags(parsed)
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn exact_tag_predicates_precede_limit_and_match_count_with_tenant_and_acl_scope() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let a = community(&pool).await;
    let b = community(&pool).await;
    let visible = channel(&pool, a).await;
    let hidden = channel(&pool, a).await;
    let foreign = channel(&pool, b).await;
    let keys = Keys::generate();
    let now = Timestamp::now().as_secs();
    let matching_tags: &[&[&str]] = &[&["L", "nip-cl"], &["l", "a", "nip-cl"], &["t", "stream"]];
    let older = event(&keys, visible, now - 100, matching_tags);
    insert_event(&pool, a, &older, Some(visible)).await.unwrap();
    for (scope, ch) in [(a, hidden), (b, foreign)] {
        let event = event(&keys, ch, now, matching_tags);
        insert_event(&pool, scope, &event, Some(ch)).await.unwrap();
    }
    // Every newer row must be eliminated in SQL, not after the one-row limit.
    for (offset, tags) in [
        vec![
            vec!["L", "nip-cl"],
            vec!["l", "other", "a"],
            vec!["t", "stream"],
        ],
        vec![
            vec!["L", "other", "nip-cl"],
            vec!["l", "a", "nip-cl"],
            vec!["t", "stream"],
        ],
        vec![vec!["L", "nip-cl"], vec!["a", "l"], vec!["t", "stream"]],
        vec![
            vec!["L", "nip-cl"],
            vec!["l", "a", "nip-cl"],
            vec!["t", "other", "stream"],
        ],
        vec![
            vec!["L", "nip-cl"],
            vec!["l", "other", "nip-cl"],
            vec!["t", "stream"],
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let refs: Vec<_> = tags.iter().map(Vec::as_slice).collect();
        let candidate = event(&keys, visible, now - offset as u64, &refs);
        insert_event(&pool, a, &candidate, Some(visible))
            .await
            .unwrap();
    }
    let filter: Filter = serde_json::from_value(serde_json::json!({
        "kinds": [39000], "#L": ["nip-cl"], "#l": ["a", "b"], "#t": ["stream"]
    }))
    .unwrap();
    let mut query = EventQuery::for_community(a);
    query.kinds = Some(vec![39000]);
    query.channel_ids = Some(vec![visible]);
    query.channel_ids_include_global = false;
    query.tag_filters = filter
        .generic_tags
        .iter()
        .map(|(key, values)| {
            (
                key.to_string(),
                values.iter().map(ToString::to_string).collect(),
            )
        })
        .collect();
    query.limit = Some(1);
    let rows = query_events(&pool, &query).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event.id, older.id);
    assert!(filters_match(&[filter], &rows[0]));
    assert_eq!(count_events(&pool, &query).await.unwrap(), 1);

    query.tag_filters.push(("l".into(), vec![]));
    assert!(query_events(&pool, &query).await.unwrap().is_empty());
    assert_eq!(count_events(&pool, &query).await.unwrap(), 0);
}
