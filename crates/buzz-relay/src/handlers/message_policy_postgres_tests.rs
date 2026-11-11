//! Policy is tested at ingest, not only through its pure predicate.
use super::group_state_postgres_tests::{
    community, create_channel, edit_channel, group_state_tags, http_auth, state,
};
use super::ingest::ingest_event;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

fn message(key: &Keys, channel: Uuid, kind: u16, extra: &[&[&str]]) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    tags.extend(
        extra
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).unwrap()),
    );
    EventBuilder::new(Kind::Custom(kind), Uuid::new_v4().to_string())
        .tags(tags)
        .sign_with_keys(key)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn publisher_policy_applies_to_existing_roles_threads_and_edits() {
    let state = state().await;
    let tenant = community(&state, "publisher").await;
    let publisher = Keys::generate();
    let other = Keys::generate();
    let channel = create_channel(&state, &tenant, &publisher, &[]).await;
    let root = message(&publisher, channel, 9, &[]);
    let old_other_root = message(&other, channel, 9, &[]);
    for (event, key) in [(&root, &publisher), (&old_other_root, &other)] {
        assert!(
            ingest_event(&state, &tenant, event.clone(), http_auth(key))
                .await
                .unwrap()
                .accepted
        );
    }
    edit_channel(
        &state,
        &tenant,
        &publisher,
        channel,
        &[
            &["message_publisher", &publisher.public_key().to_hex()],
            &["join_role", "guest"],
        ],
    )
    .await
    .unwrap();
    assert!(group_state_tags(&state, &tenant, channel, 39000)
        .await
        .contains(&vec![
            "message_publisher".into(),
            publisher.public_key().to_hex()
        ]));
    let root_hex = root.id.to_hex();
    let mut reply = None;
    for role in ["owner", "admin", "member", "bot", "guest", "non-member"] {
        let actor = Keys::generate();
        if role != "non-member" {
            state
                .db
                .add_member(
                    tenant.community(),
                    channel,
                    &actor.public_key().to_bytes(),
                    role.parse().unwrap(),
                    Some(&publisher.public_key().to_bytes()),
                )
                .await
                .unwrap();
        }
        let post = message(&actor, channel, 9, &[]);
        assert!(
            ingest_event(&state, &tenant, post, http_auth(&actor))
                .await
                .is_err(),
            "{role} root"
        );
        let event = message(&actor, channel, 9, &[&["e", &root_hex, "", "reply"]]);
        assert!(
            ingest_event(&state, &tenant, event.clone(), http_auth(&actor))
                .await
                .unwrap()
                .accepted,
            "{role} reply"
        );
        let edit = message(&actor, channel, 40003, &[&["e", &event.id.to_hex()]]);
        assert!(
            ingest_event(&state, &tenant, edit, http_auth(&actor))
                .await
                .unwrap()
                .accepted,
            "{role} edit"
        );
        reply = Some(event);
        for tags in [
            vec![vec!["e", &root_hex, "", "root"]],
            vec![vec!["e", &root_hex, "", "reply"], vec!["broadcast", "1"]],
        ] {
            let event = message(
                &actor,
                channel,
                9,
                &tags.iter().map(|tag| tag.as_slice()).collect::<Vec<_>>(),
            );
            assert!(
                ingest_event(&state, &tenant, event, http_auth(&actor))
                    .await
                    .is_err(),
                "{role} bypass"
            );
        }
        let old = message(
            &actor,
            channel,
            9,
            &[&["e", &old_other_root.id.to_hex(), "", "reply"]],
        );
        assert!(
            ingest_event(&state, &tenant, old, http_auth(&actor))
                .await
                .is_err(),
            "old other-author thread"
        );
    }
    let reply = reply.unwrap();
    let nested = message(
        &other,
        channel,
        9,
        &[
            &["e", &root_hex, "", "root"],
            &["e", &reply.id.to_hex(), "", "reply"],
        ],
    );
    assert!(
        ingest_event(&state, &tenant, nested, http_auth(&other))
            .await
            .unwrap()
            .accepted
    );
    assert!(
        ingest_event(
            &state,
            &tenant,
            message(&publisher, channel, 9, &[]),
            http_auth(&publisher)
        )
        .await
        .unwrap()
        .accepted
    );
    edit_channel(
        &state,
        &tenant,
        &publisher,
        channel,
        &[&["message_publisher", ""]],
    )
    .await
    .unwrap();
    assert!(state
        .db
        .get_channel(tenant.community(), channel)
        .await
        .unwrap()
        .message_publisher
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn publisher_policy_rejects_foreign_missing_forged_and_non_message_roots() {
    let state = state().await;
    let tenant = community(&state, "publisher-roots").await;
    let publisher = Keys::generate();
    let outsider = Keys::generate();
    let channel = create_channel(&state, &tenant, &publisher, &[]).await;
    let foreign = create_channel(&state, &tenant, &publisher, &[]).await;
    let root = message(&publisher, foreign, 9, &[]);
    ingest_event(&state, &tenant, root.clone(), http_auth(&publisher))
        .await
        .unwrap();
    edit_channel(
        &state,
        &tenant,
        &publisher,
        channel,
        &[&["message_publisher", &publisher.public_key().to_hex()]],
    )
    .await
    .unwrap();
    for root_id in [root.id.to_hex(), "a".repeat(64)] {
        let reply = message(
            &outsider,
            channel,
            9,
            &[
                &["e", &root_id, "", "reply"],
                &["actor", &publisher.public_key().to_hex()],
            ],
        );
        assert!(ingest_event(&state, &tenant, reply, http_auth(&outsider))
            .await
            .is_err());
    }
    let fake = message(
        &outsider,
        channel,
        9,
        &[&["actor", &publisher.public_key().to_hex()]],
    );
    assert!(ingest_event(&state, &tenant, fake, http_auth(&outsider))
        .await
        .is_err());
    // A readable same-channel non-message event is not a valid root.
    let metadata = message(&publisher, channel, 9002, &[&["about", "description"]]);
    ingest_event(&state, &tenant, metadata.clone(), http_auth(&publisher))
        .await
        .unwrap();
    let reply = message(
        &outsider,
        channel,
        9,
        &[&["e", &metadata.id.to_hex(), "", "reply"]],
    );
    assert!(ingest_event(&state, &tenant, reply, http_auth(&outsider))
        .await
        .is_err());
}
