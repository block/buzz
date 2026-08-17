//! End-to-end tests for NIP-09 deletion of workflow definitions (kind:30620).
//!
//! These tests require a running relay instance. By default they are marked
//! `#[ignore]` so that `cargo test` does not fail in CI when the relay is not
//! available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_workflow_delete -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_workflow_delete -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};

const KIND_WORKFLOW_DEF: u16 = 30620;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

fn workflow_yaml(name: &str) -> String {
    format!(
        "name: {name}\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n"
    )
}

/// Create an `open` stream channel owned by `keys` (kind:9007). The creator is
/// bootstrapped as an owner-member, which the kind:30620 membership check
/// requires.
async fn create_channel(client: &mut BuzzTestClient, keys: &Keys) -> uuid::Uuid {
    let channel_id = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["name", &format!("wf-del-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let ok = client.send_event(event).await.expect("send create-channel");
    assert!(ok.accepted, "channel should be created: {}", ok.message);
    channel_id
}

/// Query the live kind:30620 events for a workflow coordinate.
async fn query_workflow(
    client: &mut BuzzTestClient,
    keys: &Keys,
    workflow_id: &uuid::Uuid,
    label: &str,
) -> Vec<nostr::Event> {
    let sid = sub_id(label);
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEF))
        .author(keys.public_key())
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            workflow_id.to_string(),
        );
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect")
}

/// NIP-09 a-tag deletion: a kind:5 deletion targeting the addressable
/// coordinate `30620:<pubkey>:<workflow-id>` must soft-delete the live
/// definition event, so subsequent REQs no longer return it.
///
/// Regression test for issue #4864 — before the fix, the workflow branch of
/// `handle_a_tag_deletion` dropped the `workflows` row that drives execution
/// but left the kind:30620 definition event live. Every client reads
/// definitions by querying kind:30620 (`buzz workflows list`/`get`, the
/// desktop's `get_channel_workflows`), so a deleted workflow stayed fully
/// visible, and a later `workflows update` re-upserted the row from the
/// still-live event — resurrecting the workflow with a fresh webhook secret.
#[tokio::test]
#[ignore]
async fn test_workflow_a_tag_deletion_tombstones_definition_event() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let channel_id = create_channel(&mut client, &keys).await;

    // Publish a workflow definition keyed by a client-chosen d-tag — the same
    // shape `buzz_sdk::build_workflow_def` writes.
    let workflow_id = uuid::Uuid::new_v4();
    let def = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF),
        workflow_yaml("e2e-delete-me"),
    )
    .tags(vec![
        Tag::parse(["d", &workflow_id.to_string()]).unwrap(),
        Tag::parse(["h", &channel_id.to_string()]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();
    let ok = client.send_event(def).await.expect("send workflow def");
    assert!(
        ok.accepted,
        "workflow def should be accepted: {}",
        ok.message
    );

    // Sanity check: queryable before deletion.
    let pre = query_workflow(&mut client, &keys, &workflow_id, "wf-del-pre").await;
    assert!(
        !pre.is_empty(),
        "workflow definition should be queryable before deletion"
    );

    // Delete via the addressable coordinate.
    let a_coord = format!(
        "{}:{}:{}",
        KIND_WORKFLOW_DEF,
        keys.public_key().to_hex(),
        workflow_id
    );
    let del = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let ok_del = client.send_event(del).await.expect("send deletion");
    assert!(
        ok_del.accepted,
        "a-tag deletion should be accepted: {}",
        ok_del.message
    );

    // The definition event must no longer be returned.
    let post = query_workflow(&mut client, &keys, &workflow_id, "wf-del-post").await;
    assert!(
        post.is_empty(),
        "a-tag deletion should remove the workflow definition from REQ results (got {} events)",
        post.len()
    );

    client.disconnect().await.expect("disconnect");
}

/// A deletion whose a-tag names a *different* author's pubkey must not
/// tombstone that author's workflow definition. The coordinate delete is
/// scoped to the deleting author, so a crafted a-tag is a no-op.
#[tokio::test]
#[ignore]
async fn test_workflow_a_tag_deletion_cannot_delete_another_authors_workflow() {
    let url = relay_url();
    let owner = Keys::generate();
    let attacker = Keys::generate();

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let channel_id = create_channel(&mut owner_client, &owner).await;

    let workflow_id = uuid::Uuid::new_v4();
    let def = EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEF),
        workflow_yaml("e2e-keep-me"),
    )
    .tags(vec![
        Tag::parse(["d", &workflow_id.to_string()]).unwrap(),
        Tag::parse(["h", &channel_id.to_string()]).unwrap(),
    ])
    .sign_with_keys(&owner)
    .unwrap();
    let ok = client_send(&mut owner_client, def).await;
    assert!(ok, "owner workflow def should be accepted");

    // The attacker signs a deletion naming the OWNER's coordinate.
    let mut attacker_client = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let a_coord = format!(
        "{}:{}:{}",
        KIND_WORKFLOW_DEF,
        owner.public_key().to_hex(),
        workflow_id
    );
    let del = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&attacker)
        .unwrap();
    // Whether the relay accepts the envelope or not, the owner's definition
    // must survive — that is the property under test.
    let _ = attacker_client.send_event(del).await;

    let survived = query_workflow(&mut owner_client, &owner, &workflow_id, "wf-del-attack").await;
    assert!(
        !survived.is_empty(),
        "another author's a-tag deletion must not tombstone the owner's workflow definition"
    );

    owner_client.disconnect().await.expect("disconnect owner");
    attacker_client
        .disconnect()
        .await
        .expect("disconnect attacker");
}

async fn client_send(client: &mut BuzzTestClient, event: nostr::Event) -> bool {
    client
        .send_event(event)
        .await
        .map(|ok| ok.accepted)
        .unwrap_or(false)
}
