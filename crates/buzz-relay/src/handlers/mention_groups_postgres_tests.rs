//! NIP-MG admission regressions through the production ingest and REQ seams.
//! Run against the feature schema with BUZZ_TEST_DATABASE_URL set.

use std::sync::Arc;
use std::time::Duration;

use buzz_auth::Scope;
use buzz_core::kind::{
    KIND_MENTION_GROUP_STATE, RELAY_ADMIN_PUT_MENTION_GROUP, RELAY_ADMIN_SET_MENTION_GROUP_POLICY,
};
use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;
use uuid::Uuid;

use super::group_state_postgres_tests::{community, req_event_ids, state};
use super::ingest::{ingest_event, HttpAuthMethod, IngestAuth, IngestError};
use crate::state::AppState;

fn auth(keys: &Keys) -> IngestAuth {
    IngestAuth::Http {
        pubkey: keys.public_key(),
        scopes: vec![Scope::UsersWrite],
        auth_method: HttpAuthMethod::Nip98,
    }
}

fn put_at(keys: &Keys, id: Uuid, prev: i64, name: &str, timestamp: u64) -> Event {
    EventBuilder::new(Kind::Custom(RELAY_ADMIN_PUT_MENTION_GROUP as u16), "")
        .custom_created_at(Timestamp::from(timestamp))
        .allow_self_tagging()
        .tags([
            Tag::parse(["d", &id.to_string()]).unwrap(),
            Tag::parse(["prev", &prev.to_string()]).unwrap(),
            Tag::parse(["name", name]).unwrap(),
            Tag::public_key(keys.public_key()),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

fn put(keys: &Keys, id: Uuid, prev: i64, name: &str) -> Event {
    put_at(keys, id, prev, name, Timestamp::now().as_secs())
}

async fn head(state: &AppState, tenant: &TenantContext, id: Uuid) -> Option<Event> {
    let events = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![KIND_MENTION_GROUP_STATE as i32]),
            d_tag: Some(id.to_string()),
            limit: Some(1),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .unwrap();
    events.into_iter().next().map(|stored| stored.event)
}

async fn receipt(state: &AppState, tenant: &TenantContext, event: &Event) -> bool {
    state
        .db
        .mention_group_command_accepted(tenant.community(), event.id.as_bytes())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn signed_actor_global_token_and_users_write_scope_are_required_even_for_retries() {
    let state = state().await;
    let tenant = community(&state, "mg-admission").await;
    let actor = Keys::generate();
    let other = Keys::generate();
    let id = Uuid::new_v4();
    let command = put(&actor, id, 0, "Support");
    let scoped = IngestAuth::Nip42 {
        pubkey: actor.public_key(),
        scopes: vec![Scope::UsersWrite],
        channel_ids: Some(vec![Uuid::new_v4()]),
        conn_id: Uuid::new_v4(),
    };
    let no_users_write = IngestAuth::Http {
        pubkey: actor.public_key(),
        scopes: vec![Scope::MessagesWrite, Scope::AdminUsers],
        auth_method: HttpAuthMethod::Nip98,
    };
    for accepted in [false, true] {
        for (context, reason) in [
            (auth(&other), "authenticated identity"),
            (scoped.clone(), "global token"),
            (no_users_write.clone(), "users:write"),
        ] {
            let error = ingest_event(&state, &tenant, command.clone(), context)
                .await
                .err()
                .expect("expected refusal");
            assert!(
                matches!(error, IngestError::AuthFailed(ref message) if message.contains(reason)),
                "{error:?}"
            );
            assert_eq!(receipt(&state, &tenant, &command).await, accepted);
        }
        if !accepted {
            assert!(head(&state, &tenant, id).await.is_none());
            assert!(
                ingest_event(&state, &tenant, command.clone(), auth(&actor))
                    .await
                    .unwrap()
                    .accepted
            );
        }
    }
    let current = head(&state, &tenant, id).await.unwrap();
    assert_eq!(
        current
            .tags
            .iter()
            .find(|tag| tag.as_slice()[0] == "version")
            .unwrap()
            .content(),
        Some("1")
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn command_signature_is_verified_before_execution_or_receipt_acknowledgement() {
    let state = state().await;
    let tenant = community(&state, "mg-signature").await;
    let actor = Keys::generate();
    let id = Uuid::new_v4();
    let command = put(&actor, id, 0, "Support");
    let mut forged = command.clone();
    forged.sig = put(&Keys::generate(), id, 0, "Support").sig;
    for accepted in [false, true] {
        let error = ingest_event(&state, &tenant, forged.clone(), auth(&actor))
            .await
            .err()
            .expect("expected refusal");
        assert!(
            matches!(error, IngestError::Rejected(ref message) if message.contains("signature")),
            "{error:?}"
        );
        assert_eq!(receipt(&state, &tenant, &command).await, accepted);
        if !accepted {
            assert!(head(&state, &tenant, id).await.is_none());
            ingest_event(&state, &tenant, command.clone(), auth(&actor))
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn ban_and_current_group_policy_refusals_leave_no_receipt_and_have_valid_controls() {
    let state = state().await;
    let tenant = community(&state, "mg-authority").await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let _ = state
        .db
        .add_relay_member(
            tenant.community(),
            &owner.public_key().to_hex(),
            "owner",
            None,
            &state.relay_keypair,
        )
        .await
        .unwrap();
    let _ = state
        .db
        .add_relay_member(
            tenant.community(),
            &member.public_key().to_hex(),
            "member",
            None,
            &state.relay_keypair,
        )
        .await
        .unwrap();
    let policy = |manage: &str, prev: &str| {
        EventBuilder::new(
            Kind::Custom(RELAY_ADMIN_SET_MENTION_GROUP_POLICY as u16),
            "",
        )
        .tags([
            Tag::parse(["manage", manage]).unwrap(),
            Tag::parse(["prev", prev]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap()
    };
    ingest_event(&state, &tenant, policy("admins", "0"), auth(&owner))
        .await
        .unwrap();
    let command = put(&member, Uuid::new_v4(), 0, "Support");
    let error = ingest_event(&state, &tenant, command.clone(), auth(&member))
        .await
        .err()
        .expect("expected refusal");
    assert!(
        matches!(error, IngestError::AuthFailed(ref message) if message.starts_with("restricted:")),
        "{error:?}"
    );
    assert!(!receipt(&state, &tenant, &command).await);
    ingest_event(&state, &tenant, policy("everyone", "1"), auth(&owner))
        .await
        .unwrap();
    state
        .db
        .ban_community_member(
            tenant.community(),
            member.public_key().as_bytes(),
            owner.public_key().as_bytes(),
            None,
            None,
        )
        .await
        .unwrap();
    state.invalidate_restrictions_local(tenant.community());
    let error = ingest_event(&state, &tenant, command.clone(), auth(&member))
        .await
        .err()
        .expect("expected refusal")
        .into_legacy();
    assert!(
        matches!(error, IngestError::AuthFailed(ref message) if message.starts_with("blocked:")),
        "{error:?}"
    );
    assert!(!receipt(&state, &tenant, &command).await);
    state
        .db
        .unban_community_member(
            tenant.community(),
            member.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap();
    state.invalidate_restrictions_local(tenant.community());
    assert!(
        ingest_event(&state, &tenant, command.clone(), auth(&member))
            .await
            .unwrap()
            .accepted
    );
    assert!(receipt(&state, &tenant, &command).await);
}

#[tokio::test]
#[ignore = "requires Postgres; exercises one real command-expiry transition"]
async fn expired_exact_receipt_retry_survives_a_later_edit_but_unknown_expired_commands_fail() {
    let state = state().await;
    let tenant = community(&state, "mg-expiry").await;
    let actor = Keys::generate();
    let id = Uuid::new_v4();
    let timestamp = Timestamp::now().as_secs() - 105;
    let original = put_at(&actor, id, 0, "Original", timestamp);
    ingest_event(&state, &tenant, original.clone(), auth(&actor))
        .await
        .unwrap();
    let edit = put(&actor, id, 1, "Current");
    ingest_event(&state, &tenant, edit, auth(&actor))
        .await
        .unwrap();
    let current = head(&state, &tenant, id).await.unwrap();
    let wait = (timestamp + 121).saturating_sub(Timestamp::now().as_secs());
    assert!(wait <= 16, "expiry wait unexpectedly exceeded its bound");
    tokio::time::sleep(Duration::from_secs(wait)).await;
    assert!(Timestamp::now().as_secs() > timestamp + 120);
    let replay = ingest_event(&state, &tenant, original.clone(), auth(&actor))
        .await
        .unwrap();
    assert!(
        replay.accepted && replay.message.starts_with("duplicate:"),
        "retry refused: {}",
        replay.message
    );
    assert_eq!(replay.event_id, original.id.to_hex());
    let unknown = put_at(&actor, id, 2, "Expired overwrite", timestamp);
    let error = ingest_event(&state, &tenant, unknown.clone(), auth(&actor))
        .await
        .err()
        .expect("expected refusal");
    assert!(
        matches!(error, IngestError::Rejected(ref message) if message.contains("timestamp")),
        "{error:?}"
    );
    assert!(!receipt(&state, &tenant, &unknown).await);
    let future = put_at(
        &actor,
        id,
        2,
        "Future overwrite",
        Timestamp::now().as_secs() + 300,
    );
    let error = ingest_event(&state, &tenant, future.clone(), auth(&actor))
        .await
        .err()
        .expect("expected refusal");
    assert!(
        matches!(error, IngestError::Rejected(ref message) if message.contains("mention group command timestamp")),
        "{error:?}"
    );
    assert!(!receipt(&state, &tenant, &future).await);
    assert_eq!(head(&state, &tenant, id).await.unwrap().id, current.id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn recipient_cannot_delete_relay_group_state_by_event_or_address() {
    let state = state().await;
    let tenant = community(&state, "mg-head-deletion").await;
    let recipient = Keys::generate();
    let id = Uuid::new_v4();
    ingest_event(
        &state,
        &tenant,
        put(&recipient, id, 0, "Support"),
        auth(&recipient),
    )
    .await
    .unwrap();
    let current = head(&state, &tenant, id).await.unwrap();
    let deletion_auth = || IngestAuth::Http {
        pubkey: recipient.public_key(),
        scopes: vec![Scope::MessagesWrite, Scope::UsersWrite],
        auth_method: HttpAuthMethod::Nip98,
    };
    let deletion = |tag: Tag| {
        EventBuilder::new(Kind::EventDeletion, "")
            .tags([tag])
            .sign_with_keys(&recipient)
            .unwrap()
    };
    for tag in [
        Tag::parse(["e", &current.id.to_hex()]).unwrap(),
        Tag::parse(["a", &format!("39010:{}:{id}", current.pubkey.to_hex())]).unwrap(),
        // Relay-only state also cannot be targeted using the recipient as the
        // coordinate author: it must not store an accepted deletion of this kind.
        Tag::parse([
            "a",
            &format!("39010:{}:{id}", recipient.public_key().to_hex()),
        ])
        .unwrap(),
    ] {
        let request = deletion(tag);
        let error = ingest_event(&state, &tenant, request.clone(), deletion_auth())
            .await
            .err()
            .expect("relay-owned group state deletion must be refused");
        assert!(
            matches!(error, IngestError::Rejected(ref message)
            if message == "invalid: deletion target not found or not deletable by you"),
            "{error:?}"
        );
        assert!(state
            .db
            .get_event_by_id(tenant.community(), request.id.as_bytes())
            .await
            .unwrap()
            .is_none());
        assert_eq!(head(&state, &tenant, id).await.unwrap().id, current.id);
        assert_eq!(
            req_event_ids(
                &state,
                &tenant,
                &recipient,
                json!({"kinds":[39010], "#d":[id.to_string()]})
            )
            .await,
            vec![current.id.to_hex()]
        );
    }
    let note = EventBuilder::new(Kind::TextNote, "Ordinary deletable note")
        .sign_with_keys(&recipient)
        .unwrap();
    ingest_event(&state, &tenant, note.clone(), deletion_auth())
        .await
        .unwrap();
    assert!(
        ingest_event(
            &state,
            &tenant,
            deletion(Tag::parse(["e", &note.id.to_hex()]).unwrap()),
            deletion_auth()
        )
        .await
        .unwrap()
        .accepted
    );
    assert_eq!(head(&state, &tenant, id).await.unwrap().id, current.id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn relay_signed_state_is_visible_to_nonrecipient_members_and_cannot_be_forged() {
    let state = state().await;
    let tenant = community(&state, "mg-state").await;
    let actor = Keys::generate();
    let reader = Keys::generate();
    for keys in [&actor, &reader] {
        let _ = state
            .db
            .add_relay_member(
                tenant.community(),
                &keys.public_key().to_hex(),
                "member",
                None,
                &state.relay_keypair,
            )
            .await
            .unwrap();
    }
    let mut configured = (*state).clone();
    let mut config = (*state.config).clone();
    config.require_relay_membership = true;
    configured.config = Arc::new(config);
    let state = Arc::new(configured);
    let id = Uuid::new_v4();
    ingest_event(&state, &tenant, put(&actor, id, 0, "Support"), auth(&actor))
        .await
        .unwrap();
    let current = head(&state, &tenant, id).await.unwrap();
    assert!(current.verify_signature());
    assert_eq!(current.pubkey, state.relay_keypair.public_key());
    assert!(!current.tags.iter().any(|tag| tag.as_slice()[0] == "p"
        && tag.content() == Some(reader.public_key().to_hex().as_str())));
    let ids = req_event_ids(&state, &tenant, &reader, json!({"kinds":[39010], "authors":[state.relay_keypair.public_key().to_hex()], "#d":[id.to_string()]})).await;
    assert_eq!(ids, vec![current.id.to_hex()]);
    let forged = EventBuilder::new(Kind::Custom(KIND_MENTION_GROUP_STATE as u16), "")
        .tags(current.tags.iter().cloned())
        .sign_with_keys(&actor)
        .unwrap();
    let error = ingest_event(&state, &tenant, forged, auth(&actor))
        .await
        .err()
        .expect("expected refusal");
    assert!(
        matches!(error, IngestError::Rejected(ref message) if message.contains("relay-only")),
        "{error:?}"
    );
    assert_eq!(head(&state, &tenant, id).await.unwrap().id, current.id);
}
