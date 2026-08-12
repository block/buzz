use buzz_core::agent_activity::{
    AgentActivity, AgentActivityClass, AgentActivityFrame, AgentActivityStatus,
    AGENT_ACTIVITY_FRAME_VERSION,
};
use buzz_core::kind::KIND_AGENT_ACTIVITY_SUMMARY;
use buzz_sdk::{build_agent_activity_summary, SdkError};
use nostr::Keys;
use uuid::Uuid;

fn frame() -> AgentActivityFrame {
    AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![AgentActivity {
            activity_id: Uuid::new_v4(),
            occurred_at: "2026-08-12T00:00:00Z".parse().unwrap(),
            activity_class: AgentActivityClass::Turn,
            status: AgentActivityStatus::Started,
            tool_kind: None,
            duration_ms: None,
            usage: None,
        }],
    }
}

fn tag_values(event: &nostr::Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.len() == 2 && parts[0].as_str() == name).then(|| parts[1].as_str().to_owned())
        })
        .collect()
}

#[test]
fn builder_emits_exact_channel_and_agent_tags_with_validated_content() {
    let signer = Keys::generate();
    let channel_id = Uuid::new_v4();
    let activity = frame();

    let event = build_agent_activity_summary(channel_id, &signer.public_key().to_hex(), &activity)
        .unwrap()
        .sign_with_keys(&signer)
        .unwrap();

    assert_eq!(event.kind.as_u16(), KIND_AGENT_ACTIVITY_SUMMARY as u16);
    assert_eq!(tag_values(&event, "h"), vec![channel_id.to_string()]);
    assert_eq!(
        tag_values(&event, "agent"),
        vec![signer.public_key().to_hex()]
    );
    assert_eq!(AgentActivityFrame::parse(&event.content).unwrap(), activity);
}

#[test]
fn builder_rejects_invalid_agent_key_and_invalid_frame() {
    assert!(matches!(
        build_agent_activity_summary(Uuid::new_v4(), "not-a-key", &frame()),
        Err(SdkError::InvalidInput(_))
    ));

    let invalid = AgentActivityFrame {
        version: 99,
        activities: frame().activities,
    };
    assert!(matches!(
        build_agent_activity_summary(Uuid::new_v4(), &"a".repeat(64), &invalid),
        Err(SdkError::InvalidInput(_))
    ));
}

#[test]
fn builder_does_not_add_owner_or_observer_tags() {
    let signer = Keys::generate();
    let event =
        build_agent_activity_summary(Uuid::new_v4(), &signer.public_key().to_hex(), &frame())
            .unwrap()
            .sign_with_keys(&signer)
            .unwrap();

    assert!(tag_values(&event, "p").is_empty());
    assert!(tag_values(&event, "frame").is_empty());
    assert_eq!(event.pubkey, signer.public_key());
}
