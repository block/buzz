//! End-to-end authorization and revision-binding coverage for agent-owned workflows.
//!
//! Run against a local relay with:
//! `cargo test -p buzz-test-client --test e2e_workflow_agent_owner -- --ignored`

use buzz_sdk::nip_oa;
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn workflow_yaml(text: &str) -> String {
    format!(
        "name: Agent-owned workflow\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: notify\n\
         \x20   action: send_message\n\
         \x20   text: {text}\n"
    )
}

async fn connect_agent_with_owner(agent: &Keys, owner: &Keys) -> BuzzTestClient {
    let tag_json =
        nip_oa::compute_auth_tag(owner, &agent.public_key(), "").expect("compute NIP-OA auth tag");
    let auth_tag = nip_oa::parse_auth_tag(&tag_json).expect("parse NIP-OA auth tag");
    let mut client = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("connect agent");
    client
        .authenticate_with_nip_oa(agent, &auth_tag)
        .await
        .expect("authenticate agent with NIP-OA");
    client
}

fn trigger(keys: &Keys, workflow_id: uuid::Uuid, revision: &str) -> nostr::Event {
    buzz_sdk::build_workflow_trigger(workflow_id, revision)
        .expect("build workflow trigger")
        .sign_with_keys(keys)
        .expect("sign workflow trigger")
}

#[tokio::test]
#[ignore]
async fn agent_and_human_owner_trigger_exact_revision_but_unrelated_member_cannot() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let unrelated = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let workflow_id = uuid::Uuid::new_v4();

    // NIP-OA authentication materializes the immutable community-scoped
    // agent→owner relationship used by workflow authorization.
    let mut agent_client = connect_agent_with_owner(&agent, &owner).await;

    let create_channel = EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", "workflow-agent-owner-e2e"]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    let response = agent_client
        .send_event(create_channel)
        .await
        .expect("create channel");
    assert!(response.accepted, "channel rejected: {}", response.message);

    let definition =
        buzz_sdk::build_workflow_def(channel_id, workflow_id, &workflow_yaml("owner-triggered"))
            .expect("build workflow definition")
            .sign_with_keys(&agent)
            .expect("sign workflow definition");
    let revision = definition.id.to_hex();
    let response = agent_client
        .send_event(definition)
        .await
        .expect("define workflow");
    assert!(
        response.accepted,
        "definition rejected: {}",
        response.message
    );

    let add_unrelated = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &unrelated.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .expect("sign add-member event");
    let response = agent_client
        .send_event(add_unrelated)
        .await
        .expect("add unrelated member");
    assert!(
        response.accepted,
        "add member rejected: {}",
        response.message
    );

    let response = agent_client
        .send_event(trigger(&agent, workflow_id, &revision))
        .await
        .expect("send agent trigger");
    assert!(
        response.accepted,
        "agent trigger rejected: {}",
        response.message
    );

    let mut unrelated_client = BuzzTestClient::connect(&relay_url(), &unrelated)
        .await
        .expect("connect unrelated user");
    let response = unrelated_client
        .send_event(trigger(&unrelated, workflow_id, &revision))
        .await
        .expect("send unrelated trigger");
    assert!(!response.accepted, "unrelated member triggered workflow");
    assert!(response.message.contains("not authorized to trigger"));

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect human owner");
    let response = owner_client
        .send_event(trigger(&owner, workflow_id, &revision))
        .await
        .expect("send owner trigger");
    assert!(
        response.accepted,
        "owner trigger rejected: {}",
        response.message
    );

    let missing_revision = EventBuilder::new(Kind::Custom(46020), "")
        .tags([Tag::parse(["d", &workflow_id.to_string()]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let response = owner_client
        .send_event(missing_revision)
        .await
        .expect("send unbound trigger");
    assert!(!response.accepted, "trigger without revision was accepted");
    assert!(response.message.contains("workflow revision e tag"));

    let wrong_revision = "ab".repeat(32);
    let response = owner_client
        .send_event(trigger(&owner, workflow_id, &wrong_revision))
        .await
        .expect("send wrong-revision trigger");
    assert!(!response.accepted, "wrong revision was accepted");
    assert!(response
        .message
        .contains("does not match current definition"));

    // A valid old revision becomes stale immediately after an agent-signed update.
    // NIP-33 revisions use second-granularity timestamps. Advance past the
    // creation second so this update is newer regardless of the event-ID
    // tie-breaker for equal timestamps.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let update = buzz_sdk::build_workflow_update(
        channel_id,
        workflow_id,
        &workflow_yaml("updated"),
        &revision,
    )
    .expect("build workflow update")
    .sign_with_keys(&agent)
    .expect("sign workflow update");
    let current_revision = update.id.to_hex();
    let response = agent_client
        .send_event(update)
        .await
        .expect("update workflow");
    assert!(response.accepted, "update rejected: {}", response.message);

    let response = owner_client
        .send_event(trigger(&owner, workflow_id, &revision))
        .await
        .expect("send stale trigger");
    assert!(!response.accepted, "stale revision was accepted");
    assert!(response
        .message
        .contains("does not match current definition"));

    let response = owner_client
        .send_event(trigger(&owner, workflow_id, &current_revision))
        .await
        .expect("send current trigger");
    assert!(
        response.accepted,
        "current revision rejected: {}",
        response.message
    );

    agent_client.disconnect().await.ok();
    unrelated_client.disconnect().await.ok();
    owner_client.disconnect().await.ok();
}
