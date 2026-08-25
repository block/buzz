fn count(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

#[test]
fn replaceable_store_has_single_ownership() {
    let root = include_str!("../src/lib.rs");
    let store = include_str!("../src/replaceable.rs");

    assert_eq!(count(root, "pub async fn replace_addressable_event("), 0);
    assert_eq!(count(store, "pub async fn replace_addressable_event("), 1);
    assert_eq!(
        count(store, "datastore_span(name = \"replace_addressable_event\""),
        1
    );

    for test_name in [
        "addressable_replacement_rolls_back_when_mention_indexing_fails",
        "stale_legacy_roster_cannot_replace_new_locked_snapshot",
        "nip_rs_replacement_hard_deletes_payload_and_watermark_rejects_replay",
        "nip_rs_transaction_operation_restores_hard_delete_opt_in",
        "parameterized_replacement_in_existing_transaction_honors_revision_and_rollback",
        "parameterized_replacement_rolls_back_when_mention_indexing_fails",
        "parameterized_duplicate_restores_live_head_inside_caller_transaction",
        "concurrent_parameterized_replacement_keeps_deterministic_head",
        "mesh_status_replacement_keeps_one_physical_row",
        "duplicate_nip_rs_discriminator_tags_keep_legacy_retention",
        "nip_rs_hard_delete_fence_fails_closed_and_scopes_opt_in_to_transaction",
    ] {
        assert_eq!(count(root, test_name), 0, "{test_name} remains in lib.rs");
        assert_eq!(
            count(store, test_name),
            1,
            "{test_name} is not singly owned"
        );
    }
}
