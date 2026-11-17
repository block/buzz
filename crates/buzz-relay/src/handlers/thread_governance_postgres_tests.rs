//! Postgres-backed relay-enforcement tests for `buzz.thread-governance/v2`.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_STREAM_MESSAGE, KIND_THREAD_GOVERNANCE, THREAD_GOVERNANCE_V2_CAPABILITY,
};
use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

use super::group_state_postgres_tests::{community, create_channel, http_auth, state};
use super::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

async fn ingest(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    signer: &Keys,
) -> Result<IngestResult, IngestError> {
    ingest_event(state, tenant, event, http_auth(signer)).await
}

fn set_role(actor: &Keys, channel_id: Uuid, target: &Keys, role: &str) -> Event {
    EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
            Tag::parse(["p", &target.public_key().to_hex()]).expect("p tag"),
            Tag::parse(["role", role]).expect("role tag"),
        ])
        .sign_with_keys(actor)
        .expect("sign role change")
}

fn governance(
    signer: &Keys,
    channel_id: Uuid,
    root: &nostr::EventId,
    action: &str,
    created_at: Timestamp,
) -> Event {
    EventBuilder::new(
        Kind::Custom(KIND_THREAD_GOVERNANCE as u16),
        serde_json::json!({
            "version": 2,
            "reason": "test extension",
            "threshold": 20,
        })
        .to_string(),
    )
    .tags([
        Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
        Tag::parse(["e", &root.to_hex(), "", "root"]).expect("root tag"),
        Tag::parse(["type", THREAD_GOVERNANCE_V2_CAPABILITY]).expect("type tag"),
        Tag::parse(["action", action]).expect("action tag"),
    ])
    .custom_created_at(created_at)
    .sign_with_keys(signer)
    .expect("sign governance event")
}

/// The event's signed clock cannot restore authority after demotion. The first
/// record is accepted while the signer is an admin and remains durable. A new
/// event that is physically submitted after demotion is rejected even though
/// its `created_at` is backdated inside the relay's accepted ±900-second
/// window to a time before the role change.
///
/// Mutations: authorize outside the DB transaction, order by `created_at`, or
/// store before the membership-locked role check -> the backdated event exists
/// and this test is RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn demoted_admin_cannot_publish_a_backdated_management_record() {
    let state = state().await;
    let tenant = community(&state, "thread-governance-v2").await;
    let (owner, admin) = (Keys::generate(), Keys::generate());
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;

    ingest(
        &state,
        &tenant,
        set_role(&owner, channel_id, &admin, "admin"),
        &owner,
    )
    .await
    .expect("grant admin role");

    let root = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "thread root")
        .tags([Tag::parse(["h", &channel_id.to_string()]).expect("h tag")])
        .sign_with_keys(&owner)
        .expect("sign root");
    let root_id = root.id;
    ingest(&state, &tenant, root, &owner)
        .await
        .expect("store thread root");

    let now = Timestamp::now().as_secs();
    let accepted = governance(
        &admin,
        channel_id,
        &root_id,
        "extend",
        Timestamp::from(now - 120),
    );
    let accepted_id = accepted.id;
    ingest(&state, &tenant, accepted.clone(), &admin)
        .await
        .expect("current admin publishes management record");

    ingest(
        &state,
        &tenant,
        set_role(&owner, channel_id, &admin, "member"),
        &owner,
    )
    .await
    .expect("demote admin");

    assert!(
        state
            .db
            .get_event_by_id(tenant.community(), accepted_id.as_bytes())
            .await
            .expect("read historical management record")
            .is_some(),
        "a later demotion must not erase an accepted record"
    );
    let duplicate = ingest(&state, &tenant, accepted, &admin)
        .await
        .expect("historical record remains an accepted duplicate");
    assert!(duplicate.accepted);
    assert_eq!(duplicate.message, "duplicate:");

    let completed = governance(
        &admin,
        channel_id,
        &root_id,
        "completed",
        Timestamp::from(now),
    );
    ingest(&state, &tenant, completed, &admin)
        .await
        .expect("completed uses ordinary member write authority");

    let backdated = governance(
        &admin,
        channel_id,
        &root_id,
        "scope-change",
        Timestamp::from(now - 60),
    );
    let backdated_id = backdated.id;
    match ingest(&state, &tenant, backdated, &admin).await {
        Err(IngestError::Rejected(message)) => {
            assert!(message.contains("only channel owners/admins"), "{message}")
        }
        Err(error) => panic!("expected authorization rejection, got {error:?}"),
        Ok(result) => panic!("demoted admin was accepted: {}", result.message),
    }
    assert!(
        state
            .db
            .get_event_by_id(tenant.community(), backdated_id.as_bytes())
            .await
            .expect("query backdated record")
            .is_none(),
        "a rejected backdated record must not be stored"
    );
}
