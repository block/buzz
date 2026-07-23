use super::*;

#[test]
fn marker_author_scope_validates_scope_and_required_pubkey() {
    assert_eq!(
        marker_author_for_scope(None, Some("agent")),
        Ok(Some("agent"))
    );
    assert_eq!(
        marker_author_for_scope(Some("agent"), Some("agent")),
        Ok(Some("agent"))
    );
    assert_eq!(marker_author_for_scope(Some("channel"), None), Ok(None));
    assert_eq!(
        marker_author_for_scope(Some("agent"), None),
        Err("agent pubkey is required for agent-scoped markers".to_string())
    );
    assert_eq!(
        marker_author_for_scope(None, None),
        Err("agent pubkey is required for agent-scoped markers".to_string())
    );
    assert_eq!(
        marker_author_for_scope(Some("unexpected"), Some("agent")),
        Err("unsupported marker scope: unexpected".to_string())
    );
}

#[test]
fn managed_agent_message_builder_adds_mentions_and_client_marker() {
    let pubkey = Keys::generate().public_key().to_hex();
    let event = build_managed_agent_channel_message(
        uuid::Uuid::new_v4(),
        "Welcome!",
        None,
        std::slice::from_ref(&pubkey),
        &[vec!["client".to_string(), "welcome-v1".to_string()]],
    )
    .expect("message should build")
    .sign_with_keys(&Keys::generate())
    .expect("message should sign");

    assert!(event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "p" && parts[1] == pubkey
    }));
    assert!(event_has_client_marker(&event, "welcome-v1"));
}

#[test]
fn managed_agent_message_builder_can_carry_multiple_client_markers() {
    let event = build_managed_agent_channel_message(
        uuid::Uuid::new_v4(),
        "Welcome!",
        None,
        &[],
        &[
            vec!["client".to_string(), "opener-v1".to_string()],
            vec!["client".to_string(), "closer-v1".to_string()],
        ],
    )
    .expect("message should build")
    .sign_with_keys(&Keys::generate())
    .expect("message should sign");

    assert!(event_has_client_marker(&event, "opener-v1"));
    assert!(event_has_client_marker(&event, "closer-v1"));
}

#[test]
fn managed_agent_message_builder_rejects_invalid_mentions() {
    let error = build_managed_agent_channel_message(
        uuid::Uuid::new_v4(),
        "Welcome!",
        None,
        &["not-a-pubkey".to_string()],
        &[],
    )
    .expect_err("invalid mentions should fail");
    assert!(error.contains("pubkey must be a 64-character hex string"));
}
#[test]
fn search_messages_filter_requests_prefix_mode_for_topbar_typeahead() {
    let filter = build_search_messages_filter("  pro  ", 12, Some("channel-1"));

    assert_eq!(filter["search"], serde_json::json!("pro"));
    assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
    assert_eq!(filter["limit"], serde_json::json!(12));
    assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
}

#[test]
fn channel_messages_before_filter_sends_before_id_the_relay_reads() {
    // The relay bridge's `extract_before_id` reads the composite tiebreak
    // from `before_id`. If this filter sent the id under any other key (an
    // earlier cut used `n`), the relay would silently drop the tiebreak and
    // the dense-second keyset would degrade to a bare inclusive `until` —
    // re-returning the same page forever. Pin the field name here so the
    // client/relay contract can't drift without a red test (the Playwright
    // mock reimplements the keyset in JS and cannot catch this).
    let filter = build_channel_messages_before_filter("channel-1", 1_700_000_000, Some("ab"), 200);

    assert_eq!(filter["until"], serde_json::json!(1_700_000_000));
    assert_eq!(filter["before_id"], serde_json::json!("ab"));
    assert_eq!(filter["limit"], serde_json::json!(200));
    assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
    assert!(
        !filter.contains_key("n"),
        "tiebreak must be `before_id`, not the `n` alias the relay ignores"
    );
}

#[test]
fn thread_replies_filter_carries_non_p_gated_kinds_to_clear_the_gate() {
    // The relay bridge p-gates EVERY filter before routing
    // (`p_gated_filters_authorized`): a kindless filter "could match" a
    // p-gated kind, so it demands a `#p` tag we don't send -> HTTP 403,
    // before the thread-subtree query runs. The headline Lane-1 fix
    // (`useThreadReplies` closing the descendant gap) then fails on every
    // call against a real relay. So the thread filter MUST carry `kinds`,
    // and every kind MUST be non-p-gated (else the gate still fires). The
    // Playwright mock does not model p-gating, so this unit test is the
    // only guard against the client/relay auth contract drifting.
    let filter = build_thread_replies_filter("root-hex", Some("channel-1"), 64, 200, None);

    let kinds = filter
        .get("kinds")
        .and_then(|v| v.as_array())
        .expect("thread filter must carry `kinds` so the p-gate passes");
    assert!(!kinds.is_empty(), "kinds must be non-empty");
    for kind in kinds {
        let k = kind.as_u64().expect("kind is a number") as u32;
        assert!(
            !buzz_core_pkg::kind::P_GATED_KINDS.contains(&k),
            "kind {k} is p-gated; a p-gated kind in the filter re-triggers the \
                 403 that this fix exists to prevent"
        );
    }
    assert_eq!(filter["#e"], serde_json::json!(["root-hex"]));
    assert_eq!(filter["depth_limit"], serde_json::json!(64));
    assert_eq!(filter["#h"], serde_json::json!(["channel-1"]));
}

#[test]
fn thread_replies_filter_pages_with_composite_cursor() {
    // When a cursor is supplied, both the timestamp and the event-id
    // tiebreak must be emitted (`thread_cursor` + `thread_cursor_id`), else
    // paging degrades to timestamp-only and drops same-second replies.
    let cursor = crate::models::ThreadCursor {
        created_at: 1_700_000_000,
        event_id: "abcd".to_string(),
    };
    let filter = build_thread_replies_filter("root-hex", None, 64, 200, Some(&cursor));
    assert_eq!(filter["thread_cursor"], serde_json::json!(1_700_000_000));
    assert_eq!(filter["thread_cursor_id"], serde_json::json!("abcd"));
    assert!(
        !filter.contains_key("#h"),
        "no channel_id -> no #h scope in the filter"
    );
}

#[test]
fn stored_managed_agent_auth_tag_trims_blank_values() {
    assert_eq!(
        stored_managed_agent_auth_tag(Some("  [\"auth\",\"owner\",\"\",\"sig\"]  ")),
        Some("[\"auth\",\"owner\",\"\",\"sig\"]".to_string())
    );
    assert_eq!(stored_managed_agent_auth_tag(Some("   ")), None);
    assert_eq!(stored_managed_agent_auth_tag(None), None);
}

#[test]
fn legacy_managed_agent_auth_tag_verifies_for_agent_pubkey() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();

    let tag = legacy_managed_agent_auth_tag(&owner_keys, &agent_keys.public_key())
        .expect("legacy auth tag should compute")
        .expect("legacy auth tag should be present");

    let owner = buzz_sdk_pkg::nip_oa::verify_auth_tag(&tag, &agent_keys.public_key())
        .expect("legacy auth tag should verify");
    assert_eq!(owner, owner_keys.public_key());
}

#[test]
fn legacy_managed_agent_auth_tag_skips_self_attestation() {
    let owner_keys = Keys::generate();

    let tag = legacy_managed_agent_auth_tag(&owner_keys, &owner_keys.public_key())
        .expect("self-attestation should be skipped");

    assert_eq!(tag, None);
}
