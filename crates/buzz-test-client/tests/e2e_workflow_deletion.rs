//! End-to-end regression coverage for workflow lifecycle deletion.
//!
//! These tests require a running relay and are ignored by default.
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_workflow_deletion -- --ignored
//! ```

use std::time::Duration;

use buzz_core::kind::{KIND_NIP29_CREATE_GROUP, KIND_WORKFLOW_DEF};
use buzz_sdk::{build_workflow_def, build_workflow_update};
use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use uuid::Uuid;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn workflow_yaml(name: &str, text: &str) -> String {
    format!(
        "name: {name}\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: notify\n\
         \x20   name: Notify\n\
         \x20   action: send_message\n\
         \x20   text: \"{text}\"\n"
    )
}

async fn query_workflow_definitions(
    client: &mut BuzzTestClient,
    keys: &Keys,
    workflow_id: Uuid,
) -> Vec<nostr::Event> {
    let subscription_id = format!("workflow-delete-{}", Uuid::new_v4());
    let d_tag = workflow_id.to_string();
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEF as u16))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), d_tag);
    client
        .subscribe(&subscription_id, vec![filter])
        .await
        .expect("subscribe to workflow coordinate");
    let events = client
        .collect_until_eose(&subscription_id, Duration::from_secs(5))
        .await
        .expect("collect workflow coordinate");
    client
        .close_subscription(&subscription_id)
        .await
        .expect("close workflow subscription");
    events
}

fn workflow_delete_event(keys: &Keys, workflow_id: Uuid) -> nostr::Event {
    let coordinate = format!(
        "{KIND_WORKFLOW_DEF}:{}:{workflow_id}",
        keys.public_key().to_hex()
    );
    EventBuilder::new(Kind::EventDeletion, "")
        .tags([
            Tag::parse(["a", coordinate.as_str()]).expect("a tag"),
            Tag::parse(["nonce", Uuid::new_v4().to_string().as_str()]).expect("nonce tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign workflow deletion")
}

/// Regression for block/buzz#2390: a second a-tag deletion must tombstone the
/// recreated definition event even when the first definition was deleted.
#[tokio::test]
#[ignore = "requires a running relay"]
async fn workflow_a_tag_delete_removes_recreated_definition() {
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    let channel_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    let create_channel = EventBuilder::new(Kind::Custom(KIND_NIP29_CREATE_GROUP as u16), "")
        .tags([
            Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag"),
            Tag::parse(["name", format!("workflow-delete-{channel_id}").as_str()])
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

    let first_definition = build_workflow_def(
        channel_id,
        workflow_id,
        &workflow_yaml("lifecycle", "first"),
    )
    .expect("build first definition")
    .sign_with_keys(&keys)
    .expect("sign first definition");
    let first_id = first_definition.id;
    let first_ok = client
        .send_event(first_definition)
        .await
        .expect("send first definition");
    assert!(
        first_ok.accepted,
        "first definition rejected: {}",
        first_ok.message
    );

    let first_query = query_workflow_definitions(&mut client, &keys, workflow_id).await;
    assert_eq!(first_query.len(), 1);
    assert_eq!(first_query[0].id, first_id);

    let first_delete = client
        .send_event(workflow_delete_event(&keys, workflow_id))
        .await
        .expect("send first deletion");
    assert!(
        first_delete.accepted,
        "first deletion rejected: {}",
        first_delete.message
    );
    assert!(
        query_workflow_definitions(&mut client, &keys, workflow_id)
            .await
            .is_empty(),
        "first definition must be absent after coordinate deletion"
    );

    let recreated_definition = build_workflow_update(
        channel_id,
        workflow_id,
        &workflow_yaml("lifecycle", "second"),
    )
    .expect("build recreated definition")
    .sign_with_keys(&keys)
    .expect("sign recreated definition");
    let recreated_id = recreated_definition.id;
    let recreated_ok = client
        .send_event(recreated_definition)
        .await
        .expect("send recreated definition");
    assert!(
        recreated_ok.accepted,
        "recreated definition rejected: {}",
        recreated_ok.message
    );

    let recreated_query = query_workflow_definitions(&mut client, &keys, workflow_id).await;
    assert_eq!(recreated_query.len(), 1);
    assert_eq!(recreated_query[0].id, recreated_id);
    assert_ne!(recreated_id, first_id);

    let second_delete = client
        .send_event(workflow_delete_event(&keys, workflow_id))
        .await
        .expect("send second deletion");
    assert!(
        second_delete.accepted,
        "second deletion rejected: {}",
        second_delete.message
    );
    assert!(
        query_workflow_definitions(&mut client, &keys, workflow_id)
            .await
            .is_empty(),
        "recreated definition must be absent after the second coordinate deletion"
    );

    client.disconnect().await.expect("disconnect");
}
