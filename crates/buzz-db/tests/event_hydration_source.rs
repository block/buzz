//! Cross-crate source-policy checks supplement the real PostgreSQL seam tests.

#[test]
fn every_search_hydrator_preserves_the_exact_hit_timestamp() {
    for (source, path) in [
        (
            include_str!("../../buzz-relay/src/handlers/req.rs"),
            "req_search_hydrate",
        ),
        (
            include_str!("../../buzz-relay/src/api/bridge.rs"),
            "bridge_search_hydrate",
        ),
        (
            include_str!("../../buzz-relay/src/api/profiles.rs"),
            "profile_search_hydrate",
        ),
    ] {
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains(&format!(".get_events_by_ids_at_routed(\"{path}\"")),
            "{path}: must use exact-key hydration"
        );
        assert!(
            source.contains("(h.event_id, h.created_at)")
                || source.contains("(hit.event_id, hit.created_at)"),
            "{path}: retain timestamp"
        );
        assert!(
            !source.contains(".get_events_by_ids_routed("),
            "{path}: must not discard timestamps"
        );
    }
}

#[test]
fn exact_key_hydration_keeps_operation_attribution_and_bounded_proof() {
    let source = include_str!("../src/store/event_hydration.rs");
    assert!(source.contains("ReaderOperation::SubscriptionHistory"));
    assert!(source.contains("WriterOperation::SubscriptionHistory"));
    assert!(source.contains("RoutePredicate::Bounded,"));
    assert!(!source.contains("RoutePredicate::Covered"));
    assert!(source.contains("observability::acquire_writer("));
    assert!(source.contains("is_statement_cancelled()"));
    assert!(source.contains("\"statement_cancelled\""));
    assert!(source.contains("\"replica_error\""));
}
