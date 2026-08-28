use std::collections::HashMap;

use super::*;

fn event(id: &str, pubkey: &str, created_at: u64, tags: &[&[&str]]) -> EventView {
    EventView {
        id: id.into(),
        kind: 9,
        pubkey: pubkey.into(),
        content: id.into(),
        created_at,
        tags: tags
            .iter()
            .map(|tag| tag.iter().map(|part| (*part).to_string()).collect())
            .collect(),
    }
}

fn request() -> UnreadCatchUpRequest {
    UnreadCatchUpRequest {
        channels: vec![],
        self_pubkey: "self".into(),
        muted_channel_ids: HashSet::new(),
    }
}

#[test]
fn pass_one_history_changes_later_classification() {
    let req = request();
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: Some(9),
        timeline_read_at: Some(9),
        discovery_at: None,
    };
    let fetched = vec![FetchedChannel {
        channel,
        events: vec![
            event(
                "self-reply",
                "self",
                10,
                &[&["e", "root", "", "reply"], &["h", "ch"]],
            ),
            event(
                "external-reply",
                "other",
                11,
                &[&["e", "root", "", "reply"], &["h", "ch"]],
            ),
        ],
        discovery_through: 20,
    }];
    let result = classify_batch(&req, fetched, &HashMap::new());
    let ChannelResult::Success {
        observed_events,
        discovered,
        ..
    } = &result[0]
    else {
        panic!("expected success")
    };
    assert_eq!(
        observed_events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["external-reply"]
    );
    assert_eq!(discovered.participated, ["root"]);
}

#[test]
fn same_second_marker_and_mutes_match_renderer_rules() {
    let req = request();
    let mut membership = HashMap::new();
    membership.insert("muted_root".into(), HashSet::from(["muted".into()]));
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: Some(10),
        timeline_read_at: Some(10),
        discovery_at: None,
    };
    let fetched = vec![FetchedChannel {
        channel,
        events: vec![
            event("boundary", "other", 10, &[&["h", "ch"]]),
            event(
                "muted",
                "other",
                11,
                &[&["e", "muted", "", "reply"], &["h", "ch"]],
            ),
            event(
                "broadcast",
                "other",
                12,
                &[&["broadcast", "1"], &["h", "ch"]],
            ),
        ],
        discovery_through: 20,
    }];
    let result = classify_batch(&req, fetched, &membership);
    let ChannelResult::Success {
        observed_events,
        max_trigger,
        ..
    } = &result[0]
    else {
        panic!("expected success")
    };
    assert_eq!(
        observed_events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["broadcast"]
    );
    assert_eq!(*max_trigger, 12);
}

#[test]
fn discovery_uses_its_device_local_watermark_independently_of_read_frontiers() {
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: Some(10),
        timeline_read_at: Some(900),
        discovery_at: Some(1_000_000),
    };
    let (authored, mentioned) = discovery_filters(&channel, "self", 2_000_000);
    assert_eq!(authored["since"], 395_200);
    assert_eq!(authored["authors"], serde_json::json!(["self"]));
    assert_eq!(mentioned["since"], 395_200);
    assert_eq!(mentioned["until"], 2_000_000);
    assert_eq!(mentioned["#p"], serde_json::json!(["self"]));

    let top_level = top_level_filter(&channel);
    assert_eq!(top_level["since"], 901);
    assert_eq!(top_level["top_level"], true);
}

#[test]
fn fresh_device_discovers_roots_before_synced_read_frontiers() {
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: Some(100),
        timeline_read_at: Some(100),
        discovery_at: None,
    };
    let (authored, mentioned) = discovery_filters(&channel, "self", 1_000);
    assert_eq!(authored["since"], 0);
    assert_eq!(mentioned["since"], 0);
}

#[test]
fn top_level_pagination_stops_after_crossing_the_read_frontier() {
    let cursor = |created_at| PageCursor {
        created_at,
        event_id: "ab".repeat(32),
    };

    assert_eq!(
        next_top_level_cursor(Some(cursor(901)), 901),
        Some(cursor(901))
    );
    assert_eq!(next_top_level_cursor(Some(cursor(900)), 901), None);
    assert_eq!(next_top_level_cursor(None, 901), None);
}

#[test]
fn markerless_top_level_recovery_uses_the_observed_unread_horizon() {
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: None,
        timeline_read_at: None,
        discovery_at: None,
    };
    let now = 2_000_000;
    let since = now - HORIZON_SECONDS as u64;
    let cursor = |created_at| PageCursor {
        created_at,
        event_id: "ab".repeat(32),
    };

    assert_eq!(top_level_since_at(&channel, now), since);
    assert_eq!(
        next_top_level_cursor(Some(cursor(since)), since),
        Some(cursor(since))
    );
    assert_eq!(next_top_level_cursor(Some(cursor(since - 1)), since), None);
}

#[test]
fn complete_paginated_batch_recovers_old_reply_behind_non_trigger_traffic() {
    let req = request();
    let channel = CatchUpChannel {
        id: "ch".into(),
        channel_type: "stream".into(),
        name: "Ch".into(),
        read_at: Some(0),
        timeline_read_at: Some(2_000),
        discovery_at: None,
    };
    let mut events = (0..=CATCH_UP_LIMIT)
        .map(|index| {
            event(
                &format!("noise-{index}"),
                if index % 2 == 0 { "self" } else { "other" },
                100 + index as u64,
                &[&["h", "ch"]],
            )
        })
        .collect::<Vec<_>>();
    events.push(event(
        "old-thread-reply",
        "other",
        5,
        &[&["e", "root", "", "reply"], &["h", "ch"]],
    ));
    let membership = HashMap::from([("participated".into(), HashSet::from(["root".into()]))]);
    let result = classify_batch(
        &req,
        vec![FetchedChannel {
            channel,
            events,
            discovery_through: 2_000,
        }],
        &membership,
    );
    let ChannelResult::Success {
        observed_events, ..
    } = &result[0]
    else {
        panic!("expected success")
    };
    assert_eq!(
        observed_events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["old-thread-reply"]
    );
}

/// Pins the SERIALIZED wire contract against `tauriUnreadCatchUp.ts`.
///
/// Asserts on serde's OUTPUT, not on `ChannelResult`: the renderer never
/// sees the Rust type, it sees bytes, through an `invokeTauri<T>` cast
/// that validates nothing. Every other test here inspects the enum before
/// serialization and the e2e bridge hand-writes the intended shape, so
/// without this nothing compares what Rust emits to what TypeScript
/// declares.
///
/// Whole-value rather than a key list, deliberately: a key-set assertion
/// passes a mutant that drops the variant rename and emits `"Success"`,
/// which the renderer's `status === "error"` branch silently misreads.
/// Failure here means the merge loop throws on the first success row and
/// catch-up yields nothing, silently.
#[test]
fn serialized_response_matches_the_typescript_contract() {
    let channels = vec![
        ChannelResult::Success {
            channel_id: "ch".into(),
            observed_events: vec![ObservedUnreadEvent {
                id: "evt".into(),
                created_at: 11,
                root_id: Some("root".into()),
                high_priority: true,
                counts_toward_badge: true,
                counts_toward_app_badge: false,
            }],
            max_trigger: 11,
            discovery_through: 12,
            activity_rows: vec![ActivityRow {
                id: "evt".into(),
                kind: 9,
                pubkey: "other".into(),
                content: "hi".into(),
                created_at: 11,
                channel_id: "ch".into(),
                channel_name: "Ch".into(),
                tags: vec![vec!["h".into(), "ch".into()]],
            }],
            discovered: DiscoveredRoots {
                participated: vec!["root".into()],
                authored: Vec::new(),
                mentioned: Vec::new(),
            },
        },
        ChannelResult::Error {
            channel_id: "ch-2".into(),
            error: "relay request timed out".into(),
        },
    ];

    let actual = serde_json::to_value(UnreadCatchUpResponse { channels }).unwrap();
    let expected = serde_json::json!({
        "channels": [
            {
                "status": "success",
                "channelId": "ch",
                "observedEvents": [{
                    "id": "evt",
                    "createdAt": 11,
                    "rootId": "root",
                    "highPriority": true,
                    "countsTowardBadge": true,
                    "countsTowardAppBadge": false,
                }],
                "maxTrigger": 11,
                "discoveryThrough": 12,
                "activityRows": [{
                    "id": "evt",
                    "kind": 9,
                    "pubkey": "other",
                    "content": "hi",
                    "createdAt": 11,
                    "channelId": "ch",
                    "channelName": "Ch",
                    "tags": [["h", "ch"]],
                }],
                "discovered": {
                    "participated": ["root"],
                    "authored": [],
                    "mentioned": [],
                },
            },
            {
                "status": "error",
                "channelId": "ch-2",
                "error": "relay request timed out",
            },
        ]
    });

    assert_eq!(actual, expected);
}
