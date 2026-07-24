//! End-to-end regression coverage for workflow action side effects.
//!
//! These tests require a running relay and are ignored by default.
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_workflow_actions -- --ignored
//! ```

use std::time::Duration;

use buzz_core::kind::{KIND_NIP29_CREATE_GROUP, KIND_REACTION, KIND_STREAM_MESSAGE};
use buzz_sdk::build_workflow_def;
use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use uuid::Uuid;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn workflow_yaml(name: &str, follow_up: &str) -> String {
    format!(
        "name: {name}\n\
         trigger:\n\
         \x20 on: message_posted\n\
         steps:\n\
         \x20 - id: acknowledge\n\
         \x20   action: add_reaction\n\
         \x20   emoji: \"👀\"\n\
         \x20 - id: follow_up\n\
         \x20   action: send_message\n\
         \x20   text: \"{follow_up}\"\n"
    )
}

fn has_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        tag.as_slice().first().map(String::as_str) == Some(name)
            && tag.as_slice().get(1).map(String::as_str) == Some(value)
    })
}

/// Regression for block/buzz#2395: `add_reaction` must emit kind:7 through the
/// in-process action sink, and a successful first step must not abort the next
/// workflow step.
#[tokio::test]
#[ignore = "requires a running relay"]
async fn workflow_add_reaction_emits_kind_7_and_continues() {
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    let channel_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let follow_up = format!("reaction-follow-up-{workflow_id}");

    let create_channel = EventBuilder::new(Kind::Custom(KIND_NIP29_CREATE_GROUP as u16), "")
        .tags([
            Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag"),
            Tag::parse(["name", format!("workflow-reaction-{channel_id}").as_str()])
                .expect("name tag"),
            Tag::parse(["channel_type", "stream"]).expect("channel type tag"),
            Tag::parse(["visibility", "open"]).expect("visibility tag"),
        ])
        .sign_with_keys(&keys)
        .expect("sign channel creation");
    let channel_ok = client
        .send_event(create_channel)
        .await
        .expect("send channel creation");
    assert!(
        channel_ok.accepted,
        "channel creation rejected: {}",
        channel_ok.message
    );

    let definition = build_workflow_def(
        channel_id,
        workflow_id,
        &workflow_yaml("reaction-action", &follow_up),
    )
    .expect("build workflow definition")
    .sign_with_keys(&keys)
    .expect("sign workflow definition");
    let definition_ok = client
        .send_event(definition)
        .await
        .expect("send workflow definition");
    assert!(
        definition_ok.accepted,
        "workflow definition rejected: {}",
        definition_ok.message
    );

    let target = EventBuilder::new(
        Kind::Custom(KIND_STREAM_MESSAGE as u16),
        "trigger reaction workflow",
    )
    .tags([Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag")])
    .sign_with_keys(&keys)
    .expect("sign target message");

    let subscription_id = format!("workflow-reaction-{workflow_id}");
    let reaction_filter = Filter::new()
        .kind(Kind::Custom(KIND_REACTION as u16))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::E), target.id.to_hex())
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            channel_id.to_string(),
        );
    let message_filter = Filter::new()
        .kind(Kind::Custom(KIND_STREAM_MESSAGE as u16))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            channel_id.to_string(),
        );
    client
        .subscribe(&subscription_id, vec![reaction_filter, message_filter])
        .await
        .expect("subscribe before trigger");
    let initial = client
        .collect_until_eose(&subscription_id, Duration::from_secs(5))
        .await
        .expect("initial EOSE");
    assert!(initial.is_empty(), "test channel should start empty");

    let target_ok = client
        .send_event(target.clone())
        .await
        .expect("send target message");
    assert!(
        target_ok.accepted,
        "target message rejected: {}",
        target_ok.message
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut reaction = None;
    let mut follow_up_message = None;
    while reaction.is_none() || follow_up_message.is_none() {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .expect("workflow action events timed out");
        match client
            .recv_event(remaining)
            .await
            .expect("receive workflow action event")
        {
            RelayMessage::Event {
                subscription_id: received_subscription,
                event,
            } if received_subscription == subscription_id => {
                if u32::from(event.kind.as_u16()) == KIND_REACTION {
                    reaction = Some(*event);
                } else if u32::from(event.kind.as_u16()) == KIND_STREAM_MESSAGE
                    && event.content == follow_up
                {
                    follow_up_message = Some(*event);
                }
            }
            _ => {}
        }
    }

    let reaction = reaction.expect("reaction event");
    assert_eq!(reaction.content, "👀");
    assert!(has_tag(&reaction, "e", &target.id.to_hex()));
    assert!(has_tag(&reaction, "actor", &keys.public_key().to_hex()));
    assert!(has_tag(&reaction, "buzz:workflow", "true"));

    let follow_up_message = follow_up_message.expect("follow-up message");
    assert!(has_tag(&follow_up_message, "buzz:workflow", "true"));
    assert!(
        has_tag(&follow_up_message, "p", &keys.public_key().to_hex()),
        "workflow follow-up must preserve owner attribution"
    );

    client
        .close_subscription(&subscription_id)
        .await
        .expect("close subscription");
    client.disconnect().await.expect("disconnect");
}
