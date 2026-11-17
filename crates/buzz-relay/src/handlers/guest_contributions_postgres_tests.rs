//! Critical permission journey through shared ingestion with both auth inputs.
use buzz_auth::Scope;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::group_state_postgres_tests::{
    community, create_channel, edit_channel, group_state_tags, http_auth, state,
};
use super::ingest::{ingest_event, IngestAuth, IngestError, IngestResult};

const POST_DENIED: &str = "only members can post in this channel";
const REACT_DENIED: &str = "reaction target event not found";

fn event(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
    let nonce = Uuid::new_v4().to_string();
    EventBuilder::new(Kind::from(kind), content)
        .tags(
            tags.iter()
                .copied()
                .chain([["nonce", nonce.as_str()].as_slice()])
                .map(|tag| Tag::parse(tag.iter().copied()).unwrap()),
        )
        .sign_with_keys(keys)
        .unwrap()
}

fn check(result: Result<IngestResult, IngestError>, denial: Option<&str>, case: &str) {
    match (result, denial) {
        (Ok(result), None) => assert!(result.accepted, "{case}: {}", result.message),
        (Err(IngestError::Rejected(reason)), Some(expected)) if reason.contains(expected) => {}
        (Ok(result), expected) => panic!("{case}: expected {expected:?}, got {}", result.message),
        (Err(error), expected) => panic!("{case}: expected {expected:?}, got {error:?}"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn guests_contribute_only_through_explicit_reply_and_reaction_grants() {
    let state = state().await;
    let tenant = community(&state, "guest-contributions").await;
    let owner = Keys::generate();
    let guest = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let channel_id = channel.to_string();
    let h = ["h", channel_id.as_str()];
    let guest_key = guest.public_key().to_hex();
    let submit = async |event, auth| ingest_event(&state, &tenant, event, auth).await;
    let post = |keys| event(keys, 9, "Conversation", &[&h]);

    let old_root = post(&guest);
    check(
        submit(old_root.clone(), http_auth(&guest)).await,
        None,
        "root published before becoming a Guest",
    );
    let add = event(&owner, 9000, "", &[&h, &["p", &guest_key, "guest"]]);
    check(submit(add, http_auth(&owner)).await, None, "add Guest");
    assert_eq!(
        state
            .db
            .get_member_role(tenant.community(), channel, &guest.public_key().to_bytes())
            .await
            .unwrap()
            .as_deref(),
        Some("guest")
    );

    let root = post(&owner);
    let root_id = root.id.to_hex();
    check(submit(root, http_auth(&owner)).await, None, "owner root");
    let reply_tag = ["e", root_id.as_str(), "", "reply"];
    let target_tag = ["e", root_id.as_str()];

    // Start with the actual channel defaults, then exercise each opt-in combination.
    let initial = state
        .db
        .get_channel(tenant.community(), channel)
        .await
        .unwrap();
    assert!(!initial.guest_replies && !initial.guest_reactions);
    for (replies, reactions) in [(false, false), (true, false), (false, true), (true, true)] {
        let reply_grant = ["guest_replies", if replies { "true" } else { "false" }];
        let react_grant = ["guest_reactions", if reactions { "true" } else { "false" }];
        if replies || reactions {
            edit_channel(
                &state,
                &tenant,
                &owner,
                channel,
                &[&reply_grant, &react_grant],
            )
            .await
            .unwrap();
        }
        let tags = group_state_tags(&state, &tenant, channel, 39000).await;
        for grant in [reply_grant, react_grant] {
            assert!(
                tags.iter()
                    .any(|tag| tag.iter().map(String::as_str).eq(grant)),
                "missing {grant:?}"
            );
        }
        check(
            submit(
                event(&guest, 9, "Reply", &[&h, &reply_tag]),
                http_auth(&guest),
            )
            .await,
            (!replies).then_some(POST_DENIED),
            &format!("reply with grants {replies}/{reactions}"),
        );
        // The relay deduplicates emoji per author and target, even with fresh event IDs.
        let emoji = if replies { "🎉" } else { "👍" };
        check(
            submit(event(&guest, 7, emoji, &[&target_tag]), http_auth(&guest)).await,
            (!reactions).then_some(REACT_DENIED),
            &format!("reaction with grants {replies}/{reactions}"),
        );
    }

    let websocket = || IngestAuth::Nip42 {
        pubkey: guest.public_key(),
        scopes: vec![Scope::MessagesWrite],
        channel_ids: None,
        conn_id: Uuid::new_v4(),
    };
    let parent = event(&guest, 9, "Reply", &[&h, &reply_tag]);
    let parent_id = parent.id.to_hex();
    check(submit(parent, websocket()).await, None, "NIP-42 reply");
    check(
        submit(post(&guest), websocket()).await,
        Some(POST_DENIED),
        "NIP-42 root",
    );
    let reaction = event(&guest, 7, "🔥", &[&h, &target_tag]);
    let reaction_id = reaction.id.to_hex();
    check(
        submit(reaction, websocket()).await,
        None,
        "NIP-42 reaction with h",
    );

    let other_channel = create_channel(&state, &tenant, &owner, &[]).await;
    let other_root = event(
        &owner,
        9,
        "Other channel",
        &[&["h", &other_channel.to_string()]],
    );
    let other_id = other_root.id.to_hex();
    check(
        submit(other_root, http_auth(&owner)).await,
        None,
        "other channel root",
    );
    let old_root_id = old_root.id.to_hex();
    let missing_id = "f".repeat(64);

    // With both grants on, each denial must reach the boundary it exercises.
    for (case, kind, tags, denial) in [
        ("root", 9, vec![h.as_slice()], POST_DENIED),
        (
            "lone root marker",
            9,
            vec![&h, &["e", &root_id, "", "root"]],
            POST_DENIED,
        ),
        (
            "broadcast reply",
            9,
            vec![&h, &reply_tag, &["broadcast", "1"]],
            POST_DENIED,
        ),
        (
            "missing parent",
            9,
            vec![&h, &["e", &missing_id, "", "reply"]],
            "reply parent not found",
        ),
        (
            "cross-channel parent",
            9,
            vec![&h, &["e", &other_id, "", "reply"]],
            "reply parent not found",
        ),
        (
            "reaction as parent",
            9,
            vec![&h, &["e", &reaction_id, "", "reply"]],
            POST_DENIED,
        ),
        (
            "old root edit",
            40003,
            vec![&h, &["e", &old_root_id]],
            "edit target not found or not editable",
        ),
        (
            "old root deletion",
            5,
            vec![&h, &["e", &old_root_id]],
            "deletion target not found or not deletable",
        ),
        (
            "privileged metadata",
            9002,
            vec![&h, &["guest_replies", "false"]],
            "actor not authorized",
        ),
        ("canvas write", 40100, vec![h.as_slice()], POST_DENIED),
    ] {
        let content = if kind == 9 { "Conversation" } else { "" };
        check(
            submit(event(&guest, kind, content, &tags), http_auth(&guest)).await,
            Some(denial),
            case,
        );
    }

    let nested = event(
        &guest,
        9,
        "Nested reply",
        &[
            &h,
            &["e", &root_id, "", "root"],
            &["e", &parent_id, "", "reply"],
        ],
    );
    let nested_id = nested.id.to_hex();
    check(
        submit(nested, http_auth(&guest)).await,
        None,
        "nested reply",
    );
    let delete_parent = event(&guest, 5, "", &[&h, &["e", &parent_id]]);
    check(
        submit(delete_parent, http_auth(&guest)).await,
        None,
        "delete own reply",
    );
    assert!(state
        .db
        .get_event_by_id_for_event_write(tenant.community(), &hex::decode(&parent_id).unwrap())
        .await
        .unwrap()
        .is_none());
    for kind in [40003, 5] {
        let operation = event(&guest, kind, "", &[&h, &["e", &nested_id]]);
        check(
            submit(operation, http_auth(&guest)).await,
            None,
            "manage own reply after parent deletion",
        );
    }

    edit_channel(&state, &tenant, &owner, channel, &[&["about", "Updated"]])
        .await
        .unwrap();
    let settings = state
        .db
        .get_channel(tenant.community(), channel)
        .await
        .unwrap();
    assert!(settings.guest_replies && settings.guest_reactions);
    for grants in [
        vec![["guest_replies", "yes"].as_slice()],
        vec![
            ["guest_replies", "true"].as_slice(),
            &["guest_replies", "false"],
        ],
    ] {
        let error = edit_channel(&state, &tenant, &owner, channel, &grants)
            .await
            .unwrap_err();
        assert!(
            error.contains("guest_replies must be one true/false tag"),
            "{error}"
        );
    }

    edit_channel(
        &state,
        &tenant,
        &owner,
        channel,
        &[&["guest_replies", "false"], &["guest_reactions", "false"]],
    )
    .await
    .unwrap();
    check(
        submit(
            event(&guest, 9, "Reply after revocation", &[&h, &reply_tag]),
            http_auth(&guest),
        )
        .await,
        Some(POST_DENIED),
        "revoked reply",
    );
    check(
        submit(event(&guest, 7, "👀", &[&target_tag]), http_auth(&guest)).await,
        Some(REACT_DENIED),
        "revoked reaction",
    );
    let removal = event(&guest, 5, "", &[&["e", &reaction_id], &["k", "7"]]);
    check(
        submit(removal, http_auth(&guest)).await,
        None,
        "remove own reaction after revocation",
    );
}
