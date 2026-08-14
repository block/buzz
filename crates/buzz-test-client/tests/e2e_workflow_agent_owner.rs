//! End-to-end authorization coverage for a human operating an agent-owned workflow.
//!
//! Run against a local relay with:
//! `cargo test -p buzz-test-client --test e2e_workflow_agent_owner -- --ignored`

use buzz_sdk::nip_oa;
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn workflow_yaml() -> String {
    "name: Agent-owned workflow\n\
     trigger:\n\
     \x20 on: webhook\n\
     steps:\n\
     \x20 - id: notify\n\
     \x20   action: send_message\n\
     \x20   text: owner-triggered\n"
        .to_string()
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

#[tokio::test]
#[ignore]
async fn agent_owner_can_trigger_but_unrelated_user_cannot() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let unrelated = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let workflow_id = uuid::Uuid::new_v4();

    // Authenticating the agent materializes the immutable community-scoped
    // agent→owner relationship used by workflow authorization.
    let mut agent_client = connect_agent_with_owner(&agent, &owner).await;

    let create_channel = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", "workflow-agent-owner-e2e"]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    let ok = agent_client
        .send_event(create_channel)
        .await
        .expect("create channel");
    assert!(
        ok.accepted,
        "agent channel creation rejected: {}",
        ok.message
    );

    let define = buzz_sdk::build_workflow_def(channel_id, workflow_id, &workflow_yaml())
        .expect("build workflow definition")
        .sign_with_keys(&agent)
        .expect("sign workflow definition");
    let ok = agent_client
        .send_event(define)
        .await
        .expect("define workflow");
    assert!(
        ok.accepted,
        "agent workflow definition rejected: {}",
        ok.message
    );

    // SEC-006 boundary: channel membership alone must not grant workflow
    // authority. Make the adversarial principal a real member before its
    // trigger attempt below.
    let add_unrelated = EventBuilder::new(Kind::Custom(9000), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["p", &unrelated.public_key().to_string()]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .expect("sign add-member event");
    let ok = agent_client
        .send_event(add_unrelated)
        .await
        .expect("add unrelated user as channel member");
    assert!(
        ok.accepted,
        "adding unrelated channel member rejected: {}",
        ok.message
    );

    let agent_trigger = buzz_sdk::build_workflow_trigger(workflow_id)
        .expect("build agent trigger")
        .sign_with_keys(&agent)
        .expect("sign agent trigger");
    let ok = agent_client
        .send_event(agent_trigger)
        .await
        .expect("send agent trigger");
    assert!(
        ok.accepted,
        "agent trigger of its own workflow rejected: {}",
        ok.message
    );

    let mut unrelated_client = BuzzTestClient::connect(&relay_url(), &unrelated)
        .await
        .expect("connect unrelated user");
    let unrelated_trigger = buzz_sdk::build_workflow_trigger(workflow_id)
        .expect("build unrelated trigger")
        .sign_with_keys(&unrelated)
        .expect("sign unrelated trigger");
    let ok = unrelated_client
        .send_event(unrelated_trigger)
        .await
        .expect("send unrelated trigger");
    assert!(
        !ok.accepted,
        "unrelated user triggered an agent-owned workflow"
    );
    assert!(
        ok.message
            .contains("not authorized to trigger this workflow"),
        "unexpected rejection: {}",
        ok.message
    );

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect owner");
    let owner_trigger = buzz_sdk::build_workflow_trigger(workflow_id)
        .expect("build owner trigger")
        .sign_with_keys(&owner)
        .expect("sign owner trigger");
    let ok = owner_client
        .send_event(owner_trigger)
        .await
        .expect("send owner trigger");
    assert!(
        ok.accepted,
        "owner trigger of agent-owned workflow rejected: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
    unrelated_client.disconnect().await.ok();
    owner_client.disconnect().await.ok();
}
