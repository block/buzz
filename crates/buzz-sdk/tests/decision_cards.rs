use buzz_core::decision_card::{DecisionCardChoice, DecisionCardPayload, DecisionResponsePayload};
use buzz_core::kind::{KIND_STREAM_DECISION_CARD, KIND_STREAM_DECISION_RESPONSE};
use buzz_sdk::{build_decision_card, build_decision_response, ThreadRef};
use nostr::{EventBuilder, EventId, Keys};
use uuid::Uuid;

fn sign(builder: EventBuilder) -> nostr::Event {
    builder
        .sign_with_keys(&Keys::generate())
        .expect("test event should sign")
}

fn tag_value<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some(name))
            .then(|| parts.get(1).map(String::as_str))
            .flatten()
    })
}

#[test]
fn decision_card_keeps_markdown_fallback_and_signed_structured_payload() {
    let channel_id = Uuid::new_v4();
    let payload = DecisionCardPayload {
        schema_version: 1,
        card_id: Uuid::new_v4(),
        title: "Approve corrected redraft".into(),
        situation: "Case #625 has a wording correction ready.".into(),
        recommendation: "Approve the corrected wording.".into(),
        proposed_action: "Record a shadow approval only.".into(),
        risk: "No external send and no production write.".into(),
        record_url: Some("https://stomaton.example/cases/625".into()),
        choices: vec![
            DecisionCardChoice::Approve,
            DecisionCardChoice::Redraft,
            DecisionCardChoice::Escalate,
            DecisionCardChoice::Reject,
        ],
        expires_at: Some(2_000_000_000),
        shadow: true,
    };
    let fallback = "## Decision needed\nApprove corrected redraft\n\n**SHADOW — NOT DELIVERED**";

    let event = sign(
        build_decision_card(channel_id, &payload, fallback, None)
            .expect("valid decision card should build"),
    );

    assert_eq!(event.kind.as_u16(), KIND_STREAM_DECISION_CARD as u16);
    assert_eq!(event.content, fallback);
    assert_eq!(
        tag_value(&event, "h"),
        Some(channel_id.to_string().as_str())
    );
    assert_eq!(tag_value(&event, "shadow"), Some("1"));
    assert_eq!(tag_value(&event, "expiration"), Some("2000000000"));

    let encoded = tag_value(&event, "decision_card").expect("decision_card tag");
    let decoded: DecisionCardPayload = serde_json::from_str(encoded).expect("valid payload JSON");
    assert_eq!(decoded, payload);

    let payload_hash = tag_value(&event, "payload_hash").expect("payload_hash tag");
    assert_eq!(payload_hash, payload.payload_hash().expect("payload hash"));
    assert_eq!(payload_hash.len(), 64);
}

#[test]
fn decision_response_references_card_and_preserves_originating_thread() {
    let channel_id = Uuid::new_v4();
    let root_id = EventId::from_hex(&"11".repeat(32)).expect("root event id");
    let card_event_id = EventId::from_hex(&"22".repeat(32)).expect("card event id");
    let action_id = Uuid::new_v4();
    let payload_hash = "ab".repeat(32);
    let payload = DecisionResponsePayload {
        schema_version: 1,
        action_id,
        card_id: Uuid::new_v4(),
        decision: DecisionCardChoice::Approve,
        payload_hash: payload_hash.clone(),
        note: Some("Proceed with the shadow receipt.".into()),
        shadow: true,
    };
    let fallback = "✅ Approved — SHADOW / NOT DELIVERED";
    let thread_ref = ThreadRef {
        root_event_id: root_id,
        parent_event_id: card_event_id,
    };

    let event = sign(
        build_decision_response(channel_id, &payload, fallback, &thread_ref)
            .expect("valid response should build"),
    );

    assert_eq!(event.kind.as_u16(), KIND_STREAM_DECISION_RESPONSE as u16);
    assert_eq!(event.content, fallback);
    assert_eq!(
        tag_value(&event, "h"),
        Some(channel_id.to_string().as_str())
    );
    assert_eq!(
        tag_value(&event, "payload_hash"),
        Some(payload_hash.as_str())
    );
    assert_eq!(tag_value(&event, "shadow"), Some("1"));

    let encoded = tag_value(&event, "decision_response").expect("decision_response tag");
    let decoded: DecisionResponsePayload =
        serde_json::from_str(encoded).expect("valid response JSON");
    assert_eq!(decoded, payload);

    let e_tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("e"))
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    assert_eq!(
        e_tags[0],
        vec!["e".into(), root_id.to_hex(), "".into(), "root".into()]
    );
    assert_eq!(
        e_tags[1],
        vec![
            "e".into(),
            card_event_id.to_hex(),
            "".into(),
            "reply".into(),
        ]
    );
}

#[test]
fn decision_card_rejects_empty_choices() {
    let payload = DecisionCardPayload {
        schema_version: 1,
        card_id: Uuid::new_v4(),
        title: "Choose".into(),
        situation: "A decision is required.".into(),
        recommendation: "Approve.".into(),
        proposed_action: "Record shadow intent.".into(),
        risk: "None.".into(),
        record_url: None,
        choices: vec![],
        expires_at: None,
        shadow: true,
    };

    assert!(build_decision_card(Uuid::new_v4(), &payload, "fallback", None).is_err());
}

#[test]
fn decision_card_rejects_non_http_record_url() {
    let payload = DecisionCardPayload {
        schema_version: 1,
        card_id: Uuid::new_v4(),
        title: "Choose".into(),
        situation: "A decision is required.".into(),
        recommendation: "Approve.".into(),
        proposed_action: "Record shadow intent.".into(),
        risk: "None.".into(),
        record_url: Some("javascript:alert(1)".into()),
        choices: vec![DecisionCardChoice::Approve],
        expires_at: None,
        shadow: true,
    };

    assert!(build_decision_card(Uuid::new_v4(), &payload, "fallback", None).is_err());
}
