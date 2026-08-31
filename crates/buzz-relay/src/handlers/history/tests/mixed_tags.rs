//! The shared SQL predicate must agree with the core matcher, even when a
//! host-capable filter also requests derived-channel reactions/deletions.
use super::*;
use buzz_core::filter::filters_match;
use buzz_db::EventQuery;
use nostr::{EventBuilder, Filter, Kind, Tag};

#[tokio::test]
#[ignore = "requires isolated migrated Postgres: MULTIVERSE_TEST_DATABASE_URL"]
async fn mixed_host_h_filter_preserves_derived_channels_and_explicit_tag_authority() {
    let url = std::env::var("MULTIVERSE_TEST_DATABASE_URL").expect("isolated DB required");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community(&format!("mixed-tags-{}.test", Uuid::new_v4()))
        .await
        .unwrap();
    let owner = Keys::generate();
    let author = Keys::generate();
    let channel = Uuid::new_v4();
    let other_channel = Uuid::new_v4().to_string();
    let channel_text = channel.to_string();
    let owner_text = owner.public_key().to_hex();
    let mut expected = vec![];
    for (index, h_tags) in [
        vec![],
        vec![vec!["h", channel_text.as_str()]],
        vec![vec!["h", other_channel.as_str()]],
        vec![vec!["h"]],
        vec![vec!["h", other_channel.as_str(), channel_text.as_str()]],
    ]
    .into_iter()
    .enumerate()
    {
        let mut tags = vec![Tag::parse(["p", &owner_text]).unwrap()];
        tags.extend(h_tags.into_iter().map(|t| Tag::parse(t).unwrap()));
        let event = EventBuilder::new(Kind::Reaction, index.to_string())
            .tags(tags)
            .sign_with_keys(&author)
            .unwrap();
        buzz_db::event::insert_event(&pool, community.id, &event, Some(channel))
            .await
            .unwrap();
        if index < 2 {
            expected.push(event.id);
        }
    }
    for kinds in [json!([7]), json!([7, 50000])] {
        for values in [
            json!([channel_text]),
            json!([Uuid::new_v4().to_string(), channel_text]),
            json!([]),
        ] {
            let filter: Filter = serde_json::from_value(json!({
                "kinds": kinds, "#h": values, "#p": [owner_text], "limit": 1000
            }))
            .unwrap();
            let mut params = EventQuery::for_community(community.id);
            params.kinds = Some(if kinds == json!([7]) {
                vec![7]
            } else {
                vec![7, 50000]
            });
            params.exact_tags = filter
                .generic_tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().cloned().collect()))
                .collect();
            // No event_mentions index: exact p predicates use primary tags.
            let rows =
                crate::handlers::history::query(&db, "mixed-tags-test", &filter, params.clone())
                    .await
                    .unwrap();
            let count = buzz_db::event::count_events(&pool, &params).await.unwrap();
            assert_eq!(
                count as usize,
                rows.len(),
                "COUNT shares the query predicate"
            );
            assert!(rows
                .iter()
                .all(|row| filters_match(std::slice::from_ref(&filter), row)));
            let mut actual: Vec<_> = rows.iter().map(|row| row.event.id).collect();
            actual.sort();
            let mut want = if values == json!([]) {
                vec![]
            } else {
                expected.clone()
            };
            want.sort();
            assert_eq!(actual, want, "kinds={kinds}, h={values}");
        }
    }
    pool.close().await;
}
