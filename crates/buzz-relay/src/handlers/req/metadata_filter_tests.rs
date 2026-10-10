//! The generic WS/HTTP query builder must push metadata predicates before LIMIT.

use super::*;
use buzz_core::CommunityId;
use serde_json::json;

#[test]
fn metadata_discovery_pushes_all_tag_fields_for_query_and_count() {
    let channel = uuid::Uuid::new_v4();
    let filter: Filter = serde_json::from_value(json!({
        "kinds": [39000], "#L": ["nip-cl"], "#l": ["a", "b"],
        "#t": ["stream", "forum"], "#d": [channel.to_string()],
        "#p": ["not-a-pubkey"], "#e": ["not-an-event-id"], "#x": ["custom"],
        "limit": 1
    }))
    .unwrap();
    let query = filter_to_query_params(&filter, None, CommunityId::from_uuid(uuid::Uuid::new_v4()));
    assert!(filter_fully_pushable(&filter));
    assert_eq!(query.limit, Some(1));
    assert_eq!(query.tag_filters.len(), filter.generic_tags.len());
    for (key, values) in &filter.generic_tags {
        let pushed = query
            .tag_filters
            .iter()
            .find(|(name, _)| name == &key.to_string())
            .unwrap();
        assert_eq!(
            pushed.1.iter().collect::<std::collections::BTreeSet<_>>(),
            values.iter().collect()
        );
    }
    assert!(query.p_tag_hex.is_none());
    assert!(query.e_tags.is_none());
    assert!(query.d_tag.is_none());
}

#[test]
fn metadata_empty_constraints_stay_match_nothing_and_h_stays_scoped() {
    let community = CommunityId::from_uuid(uuid::Uuid::new_v4());
    let channel = uuid::Uuid::new_v4();
    for field in ["#l", "#L", "#t", "#d", "#p", "#e", "#x"] {
        let filter: Filter = serde_json::from_value(json!({"kinds": [39000], field: []})).unwrap();
        let query = filter_to_query_params(&filter, None, community);
        assert_eq!(query.tag_filters, vec![(field[1..].to_string(), vec![])]);
        assert!(filter_fully_pushable(&filter));
    }
    for field in ["ids", "authors"] {
        let filter: Filter = serde_json::from_value(json!({"kinds": [39000], field: []})).unwrap();
        let query = filter_to_query_params(&filter, None, community);
        assert!(
            query.ids.as_ref().is_some_and(Vec::is_empty)
                || query.authors.as_ref().is_some_and(Vec::is_empty)
        );
    }
    let filter: Filter =
        serde_json::from_value(json!({"kinds": [39000], "#h": [], "#l": ["a"]})).unwrap();
    let mut query = filter_to_query_params(&filter, None, community);
    apply_channel_scope_to_query(&mut query, &filter, None, &[channel]);
    assert_eq!(query.channel_ids, Some(vec![]));
    assert!(!query.channel_ids_include_global);
    assert_eq!(
        query.tag_filters,
        vec![("l".to_string(), vec!["a".to_string()])]
    );
}

#[test]
fn other_kind_and_search_paths_do_not_claim_metadata_pushability() {
    for kinds in [json!([1]), json!([])] {
        let filter: Filter = serde_json::from_value(json!({"kinds": kinds, "#l": ["a"]})).unwrap();
        assert!(!filter_fully_pushable(&filter));
        let query =
            filter_to_query_params(&filter, None, CommunityId::from_uuid(uuid::Uuid::new_v4()));
        assert!(query.tag_filters.is_empty());
    }
    let filter: Filter = serde_json::from_value(json!({"kinds": [39000], "search": "a"})).unwrap();
    assert!(!filter_fully_pushable(&filter));
}

#[test]
fn mixed_and_id_only_metadata_reads_push_tags_before_limit() {
    for mut value in [
        json!({"kinds":[39000, 1]}),
        json!({"ids":["11".repeat(32)]}),
    ] {
        value["#l"] = json!(["retained"]);
        value["limit"] = json!(1);
        let filter: Filter = serde_json::from_value(value).unwrap();
        let query =
            filter_to_query_params(&filter, None, CommunityId::from_uuid(uuid::Uuid::new_v4()));
        assert_eq!(
            query.tag_filters,
            vec![("l".into(), vec!["retained".into()])]
        );
        assert_eq!(query.limit, Some(1));
    }
}
