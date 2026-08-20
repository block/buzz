//! Non-production relay canary for native decision cards and durable responses.

use std::time::Duration;

use buzz_core::decision_card::{DecisionCardChoice, DecisionCardPayload, DecisionResponsePayload};
use buzz_sdk::{build_decision_card, build_decision_response, ThreadRef};
use buzz_test_client::BuzzTestClient;
use nostr::{Filter, Keys};
use uuid::Uuid;

const TYLER_TEST_SECRET: &str = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const GENERAL_CHANNEL_ID: &str = "9f28288a-d724-587a-9709-92dc7f967110";

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

#[tokio::test]
#[ignore = "requires a seeded non-production relay"]
async fn signed_shadow_decision_round_trips_with_durable_receipt() {
    let keys = Keys::parse(TYLER_TEST_SECRET).expect("fixture key");
    let channel_id = Uuid::parse_str(GENERAL_CHANNEL_ID).expect("fixture channel");
    let payload = DecisionCardPayload {
        schema_version: 1,
        card_id: Uuid::new_v4(),
        title: "Approve corrected redraft".into(),
        situation: "Historical case #625 has a corrected shadow draft.".into(),
        recommendation: "Record approval intent in shadow mode.".into(),
        proposed_action: "Create a durable Buzz receipt only.".into(),
        risk: "No external send and no production write.".into(),
        record_url: Some("https://stomaton.example/cases/625".into()),
        choices: vec![
            DecisionCardChoice::Approve,
            DecisionCardChoice::Redraft,
            DecisionCardChoice::Escalate,
            DecisionCardChoice::Reject,
        ],
        expires_at: Some(2_100_000_000),
        shadow: true,
    };
    let payload_hash = payload.payload_hash().expect("payload hash");
    let card = build_decision_card(
        channel_id,
        &payload,
        "## Decision needed\nApprove corrected redraft\n\n**SHADOW — NOT DELIVERED**",
        None,
    )
    .expect("card builder")
    .sign_with_keys(&keys)
    .expect("signed card");

    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("authenticated relay connection");
    let accepted = client.send_event(card.clone()).await.expect("publish card");
    assert!(
        accepted.accepted,
        "relay rejected card: {}",
        accepted.message
    );

    client
        .subscribe("decision-card", vec![Filter::new().id(card.id)])
        .await
        .expect("query card");
    let stored_cards = client
        .collect_until_eose("decision-card", Duration::from_secs(5))
        .await
        .expect("stored card query");
    assert_eq!(stored_cards, vec![card.clone()]);

    let response_payload = DecisionResponsePayload {
        schema_version: 1,
        action_id: Uuid::new_v4(),
        card_id: payload.card_id,
        decision: DecisionCardChoice::Approve,
        payload_hash: payload_hash.clone(),
        note: Some("Non-production relay canary.".into()),
        shadow: true,
    };
    let response = build_decision_response(
        channel_id,
        &response_payload,
        "✅ Approved — SHADOW / NOT DELIVERED",
        &ThreadRef {
            root_event_id: card.id,
            parent_event_id: card.id,
        },
    )
    .expect("response builder")
    .sign_with_keys(&keys)
    .expect("signed response");
    let accepted = client
        .send_event(response.clone())
        .await
        .expect("publish response");
    assert!(
        accepted.accepted,
        "relay rejected response: {}",
        accepted.message
    );

    client
        .subscribe("decision-response", vec![Filter::new().id(response.id)])
        .await
        .expect("query response");
    let stored_responses = client
        .collect_until_eose("decision-response", Duration::from_secs(5))
        .await
        .expect("stored response query");
    assert_eq!(stored_responses, vec![response]);
    assert!(stored_responses[0]
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["payload_hash", payload_hash.as_str()]));
    assert!(stored_responses[0]
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["shadow", "1"]));

    client.disconnect().await.expect("clean disconnect");
}
