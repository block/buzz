//! End-to-end tests for interactive elicitation question cards
//! (kind:44300 request / kind:44301 answer).
//!
//! Proves, against a live relay, the two things the in-process unit tests
//! cannot: (1) the relay accepts the new kinds (scope registration in
//! `required_scope_for_kind`), and (2) an answer is retrievable via the exact
//! filter the `buzz-acp` elicitation servicer polls with
//! (`kind:44301 & author & #e=<card id>`).
//!
//! Requires a running relay. Marked `#[ignore]` so plain `cargo test` skips it.
//!
//! # Running
//!
//! ```text
//! just relay                    # in another terminal
//! cargo test --test e2e_elicitation -- --ignored
//! # or point elsewhere:
//! RELAY_URL=ws://host:3000 cargo test --test e2e_elicitation -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Filter, Keys, Kind, Tag};

const KIND_ELICITATION_REQUEST: u16 = 44300;
const KIND_ELICITATION_RESPONSE: u16 = 44301;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Build a question card (44300) the way the pool servicer does: a JSON form in
/// `content` and a `p` tag locking it to the owner.
fn build_question_card(agent: &Keys, owner: &Keys) -> nostr::Event {
    let content = serde_json::json!({
        "v": 1,
        "questionKey": "question_0",
        "header": "Weight",
        "prompt": "How heavy is this?",
        "multiSelect": false,
        "allowCustom": true,
        "options": [
            { "label": "QUICK", "description": "hack/throwaway" },
            { "label": "STANDARD" },
            { "label": "FLAGSHIP" }
        ],
        "elicitationId": "1",
    })
    .to_string();
    EventBuilder::new(Kind::Custom(KIND_ELICITATION_REQUEST), content)
        .tags([Tag::parse(["p", &owner.public_key().to_hex()]).unwrap()])
        .sign_with_keys(agent)
        .unwrap()
}

/// Build an answer (44301) the way the desktop card does: `{action,answer,custom}`
/// in `content` and an `e` tag referencing the card.
fn build_answer(owner: &Keys, card_id: nostr::EventId, answer: &str) -> nostr::Event {
    let content =
        serde_json::json!({ "action": "accept", "answer": answer, "custom": "" }).to_string();
    EventBuilder::new(Kind::Custom(KIND_ELICITATION_RESPONSE), content)
        .tags([Tag::parse(["e", &card_id.to_hex(), "", "reply"]).unwrap()])
        .sign_with_keys(owner)
        .unwrap()
}

/// The relay accepts both new kinds (scope registration works).
#[tokio::test]
#[ignore]
async fn test_elicitation_kinds_accepted() {
    let url = relay_url();
    let agent = Keys::generate();
    let owner = Keys::generate();

    let mut agent_client = BuzzTestClient::connect(&url, &agent)
        .await
        .expect("agent connect");
    let card = build_question_card(&agent, &owner);
    let card_id = card.id;
    let ok = agent_client.send_event(card).await.expect("send card");
    assert!(ok.accepted, "relay must accept kind:44300: {}", ok.message);

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("owner connect");
    let answer = build_answer(&owner, card_id, "STANDARD");
    let ok = owner_client.send_event(answer).await.expect("send answer");
    assert!(ok.accepted, "relay must accept kind:44301: {}", ok.message);

    agent_client.disconnect().await.expect("agent disconnect");
    owner_client.disconnect().await.expect("owner disconnect");
}

/// The owner's answer is retrievable via the exact filter the acp servicer polls
/// with — and carries the expected answer content.
#[tokio::test]
#[ignore]
async fn test_answer_retrievable_by_servicer_poll_filter() {
    let url = relay_url();
    let agent = Keys::generate();
    let owner = Keys::generate();

    let mut agent_client = BuzzTestClient::connect(&url, &agent)
        .await
        .expect("agent connect");
    let card = build_question_card(&agent, &owner);
    let card_id = card.id;
    let ok = agent_client.send_event(card).await.expect("send card");
    assert!(ok.accepted, "relay must accept card: {}", ok.message);

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("owner connect");
    let answer = build_answer(&owner, card_id, "FLAGSHIP");
    let answer_id = answer.id;
    let ok = owner_client.send_event(answer).await.expect("send answer");
    assert!(ok.accepted, "relay must accept answer: {}", ok.message);

    // The exact filter `pool::fetch_card_answer` builds: kind 44301, authored by
    // the owner, `#e` = the card id.
    let sid = sub_id("poll");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_ELICITATION_RESPONSE))
        .author(owner.public_key())
        .event(card_id);
    agent_client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = agent_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    let found = events
        .iter()
        .find(|e| e.id == answer_id)
        .expect("servicer poll filter must return the owner's answer");
    let parsed: serde_json::Value =
        serde_json::from_str(&found.content).expect("answer content is JSON");
    assert_eq!(parsed["action"], "accept");
    assert_eq!(parsed["answer"], "FLAGSHIP");

    agent_client.disconnect().await.expect("agent disconnect");
    owner_client.disconnect().await.expect("owner disconnect");
}
