use super::*;

#[test]
fn search_messages_filter_requests_prefix_mode_for_topbar_typeahead() {
    let filter = build_search_messages_filter("  pro  ", 12, Some("channel-1"), None, None, None);

    assert_eq!(filter["search"], serde_json::json!("pro"));
    assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
    assert_eq!(filter["limit"], serde_json::json!(12));
    assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
    assert!(filter.get("authors").is_none());
    assert!(filter.get("since").is_none());
    assert!(filter.get("until").is_none());
}

#[test]
fn search_messages_filter_emits_operator_fields() {
    let authors =
        vec!["aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899".to_string()];
    let filter = build_search_messages_filter(
        "deploy",
        20,
        Some("channel-uuid"),
        Some(&authors),
        Some(1_700_000_000),
        Some(1_700_086_400),
    );

    assert_eq!(filter["search"], serde_json::json!("deploy"));
    assert_eq!(filter["#h"], serde_json::json!(["channel-uuid"]));
    assert_eq!(
        filter["authors"],
        serde_json::json!(["aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899"])
    );
    assert_eq!(filter["since"], serde_json::json!(1_700_000_000));
    assert_eq!(filter["until"], serde_json::json!(1_700_086_400));
}
