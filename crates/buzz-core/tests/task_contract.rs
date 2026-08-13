use buzz_core::kind::{KIND_TASK_REQUESTED, KIND_TASK_RESOLVED, KIND_TASK_UPDATED};
use buzz_core::task::{
    TaskEventPayloadV1, TaskEventV1, TaskPriority, TaskResolution, TaskTarget, TaskType,
};
use buzz_core::CommunityId;
use chrono::{TimeZone, Utc};
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

fn task_event(
    kind: u32,
    agent: &Keys,
    owner_hex: &str,
    channel_id: Uuid,
    task_id: Uuid,
    source_id_hex: &str,
    content: &str,
) -> nostr::Event {
    let agent_hex = agent.public_key().to_hex();
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags([
            Tag::parse(["d", &task_id.to_string()]).unwrap(),
            Tag::parse(["p", owner_hex]).unwrap(),
            Tag::parse(["agent", &agent_hex]).unwrap(),
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["e", source_id_hex, "", "source"]).unwrap(),
        ])
        .sign_with_keys(agent)
        .unwrap()
}

fn requested_json() -> &'static str {
    r#"{
        "taskType":"approval",
        "title":"Approve campaign launch",
        "context":"Three variants are ready for review.",
        "priority":"high",
        "dueAt":"2026-08-13T10:00:00Z",
        "agentName":"Marketing Agent",
        "sourceVersion":1,
        "sourceUpdatedAt":"2026-08-13T08:18:00Z"
    }"#
}

#[test]
fn requested_event_parses_canonical_identity_and_payload() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let source_id = nostr::EventId::from_byte_array([7; 32]);
    let event = task_event(
        KIND_TASK_REQUESTED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        requested_json(),
    );

    let parsed = TaskEventV1::parse(&event).unwrap();
    assert_eq!(parsed.task_id, task_id);
    assert_eq!(parsed.owner_pubkey, owner.public_key());
    assert_eq!(parsed.agent_pubkey, agent.public_key());
    assert_eq!(parsed.channel_id, channel_id);
    assert_eq!(parsed.source_event_id, source_id);
    match parsed.payload {
        TaskEventPayloadV1::Requested(payload) => {
            assert_eq!(payload.task_type, TaskType::Approval);
            assert_eq!(payload.title, "Approve campaign launch");
            assert_eq!(
                payload.context.as_deref(),
                Some("Three variants are ready for review.")
            );
            assert_eq!(payload.priority, TaskPriority::High);
            assert_eq!(
                payload.due_at,
                Some(Utc.with_ymd_and_hms(2026, 8, 13, 10, 0, 0).unwrap())
            );
            assert_eq!(payload.source_version, 1);
        }
        other => panic!("expected requested payload, got {other:?}"),
    }
}

#[test]
fn updated_and_resolved_kinds_accept_only_their_payload_shapes() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let source_id = nostr::EventId::from_byte_array([8; 32]);
    let updated = task_event(
        KIND_TASK_UPDATED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        &requested_json().replace("\"sourceVersion\":1", "\"sourceVersion\":2"),
    );
    let resolved = task_event(
        KIND_TASK_RESOLVED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        r#"{
            "resolution":"withdrawn",
            "sourceVersion":3,
            "sourceUpdatedAt":"2026-08-13T09:00:00Z"
        }"#,
    );

    assert!(matches!(
        TaskEventV1::parse(&updated).unwrap().payload,
        TaskEventPayloadV1::Updated(payload) if payload.source_version == 2
    ));
    assert!(matches!(
        TaskEventV1::parse(&resolved).unwrap().payload,
        TaskEventPayloadV1::Resolved(payload)
            if payload.resolution == TaskResolution::Withdrawn && payload.source_version == 3
    ));

    let wrong_shape = task_event(
        KIND_TASK_RESOLVED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        requested_json(),
    );
    assert!(TaskEventV1::parse(&wrong_shape).is_err());
}

#[test]
fn envelope_rejects_agent_spoofing_and_duplicate_private_tags() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let other = Keys::generate();
    let channel_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let source_id = nostr::EventId::from_byte_array([9; 32]);

    let spoofed = EventBuilder::new(Kind::Custom(KIND_TASK_REQUESTED as u16), requested_json())
        .tags([
            Tag::parse(["d", &task_id.to_string()]).unwrap(),
            Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
            Tag::parse(["agent", &other.public_key().to_hex()]).unwrap(),
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["e", &source_id.to_hex(), "", "source"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    assert!(TaskEventV1::parse(&spoofed).is_err());

    let duplicate_owner =
        EventBuilder::new(Kind::Custom(KIND_TASK_REQUESTED as u16), requested_json())
            .tags([
                Tag::parse(["d", &task_id.to_string()]).unwrap(),
                Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
                Tag::parse(["p", &other.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
                Tag::parse(["h", &channel_id.to_string()]).unwrap(),
                Tag::parse(["e", &source_id.to_hex(), "", "source"]).unwrap(),
            ])
            .sign_with_keys(&agent)
            .unwrap();
    assert!(TaskEventV1::parse(&duplicate_owner).is_err());
}

#[test]
fn payload_rejects_zero_versions_and_unsafe_display_text() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    let source_id = nostr::EventId::from_byte_array([10; 32]);

    let zero_version = task_event(
        KIND_TASK_REQUESTED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        &requested_json().replace("\"sourceVersion\":1", "\"sourceVersion\":0"),
    );
    assert!(TaskEventV1::parse(&zero_version).is_err());

    let control_character = task_event(
        KIND_TASK_REQUESTED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        &requested_json().replace("Approve campaign launch", "Approve campaign\\u0000launch"),
    );
    assert!(TaskEventV1::parse(&control_character).is_err());

    let too_many_lines = task_event(
        KIND_TASK_REQUESTED,
        &agent,
        &owner.public_key().to_hex(),
        channel_id,
        task_id,
        &source_id.to_hex(),
        &requested_json().replace(
            "Three variants are ready for review.",
            "First\\nSecond\\nThird",
        ),
    );
    assert!(TaskEventV1::parse(&too_many_lines).is_err());
}

#[test]
fn task_target_builds_exact_native_url_only_after_identity_validation() {
    let community_id = CommunityId::from_uuid(Uuid::new_v4());
    let channel_id = Uuid::parse_str("1487447e-0f26-4bc5-8865-f5be07195579").unwrap();
    let source_id = [0xab; 32];
    let target = TaskTarget::from_bytes(community_id, channel_id, &source_id).unwrap();

    assert_eq!(target.community_id(), community_id);
    assert_eq!(target.channel_id(), channel_id);
    assert_eq!(target.source_event_id().as_bytes(), &source_id);
    assert_eq!(
        target.navigation_url(),
        "buzz://message?channel=1487447e-0f26-4bc5-8865-f5be07195579&id=abababababababababababababababababababababababababababababababab"
    );
    assert!(TaskTarget::from_bytes(community_id, channel_id, &[0xab; 31]).is_err());
}

#[test]
fn task_events_are_result_gated_to_the_signed_owner() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let event = task_event(
        KIND_TASK_REQUESTED,
        &agent,
        &owner.public_key().to_hex(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        &nostr::EventId::from_byte_array([11; 32]).to_hex(),
        requested_json(),
    );

    assert!(buzz_core::filter::reader_authorized_for_event(
        &event,
        &owner.public_key().to_hex()
    ));
    assert!(!buzz_core::filter::reader_authorized_for_event(
        &event,
        &stranger.public_key().to_hex()
    ));
}
