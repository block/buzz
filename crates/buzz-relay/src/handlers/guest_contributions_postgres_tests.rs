//! Critical permission journey through the shared HTTP/WebSocket ingestion seam.
use buzz_auth::Scope;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use super::group_state_postgres_tests::{
    community, create_channel, group_state_tags, http_auth, state,
};
use super::ingest::{ingest_event, IngestAuth};

fn event(keys: &Keys, kind: u16, tags: &[Vec<String>]) -> Event {
    EventBuilder::new(
        Kind::Custom(kind),
        if kind == 9 { "Conversation" } else { "" },
    )
    .tags(
        tags.iter()
            .cloned()
            .chain([vec!["nonce".into(), Uuid::new_v4().to_string()]])
            .map(|tag| Tag::parse(tag).unwrap()),
    )
    .sign_with_keys(keys)
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn guests_contribute_only_through_explicit_reply_and_reaction_grants() {
    let state = state().await;
    let tenant = community(&state, "guest-contributions").await;
    let owner = Keys::generate();
    let guest = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = vec!["h".into(), channel.to_string()];
    let previous_root = event(&guest, 9, std::slice::from_ref(&h));
    assert!(
        ingest_event(&state, &tenant, previous_root.clone(), http_auth(&guest))
            .await
            .unwrap()
            .accepted
    );
    let other_channel = create_channel(&state, &tenant, &owner, &[]).await;
    let other_root = event(&owner, 9, &[vec!["h".into(), other_channel.to_string()]]);
    assert!(
        ingest_event(&state, &tenant, other_root.clone(), http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    let add = event(
        &owner,
        9000,
        &[
            h.clone(),
            vec!["p".into(), guest.public_key().to_hex()],
            vec!["role".into(), "guest".into()],
        ],
    );
    assert!(
        ingest_event(&state, &tenant, add, http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    assert_eq!(
        state
            .db
            .get_member_role(tenant.community(), channel, &guest.public_key().to_bytes())
            .await
            .unwrap()
            .as_deref(),
        Some("guest")
    );
    let initial = state
        .db
        .get_channel(tenant.community(), channel)
        .await
        .unwrap();
    assert!(!initial.guest_replies && !initial.guest_reactions);
    let root = event(&owner, 9, std::slice::from_ref(&h));
    assert!(
        ingest_event(&state, &tenant, root.clone(), http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    let reply_tags = vec![
        h.clone(),
        vec!["e".into(), root.id.to_hex(), "".into(), "reply".into()],
    ];
    for (replies, reactions) in [(false, false), (true, false), (false, true), (true, true)] {
        let settings = event(
            &owner,
            9002,
            &[
                h.clone(),
                vec!["guest_replies".into(), replies.to_string()],
                vec!["guest_reactions".into(), reactions.to_string()],
            ],
        );
        assert!(
            ingest_event(&state, &tenant, settings, http_auth(&owner))
                .await
                .unwrap()
                .accepted
        );
        let tags = group_state_tags(&state, &tenant, channel, 39000).await;
        assert!(tags.contains(&vec!["guest_replies".into(), replies.to_string()]));
        assert!(tags.contains(&vec!["guest_reactions".into(), reactions.to_string()]));
        for websocket in [false, true] {
            let auth = || {
                if websocket {
                    IngestAuth::Nip42 {
                        pubkey: guest.public_key(),
                        scopes: vec![
                            Scope::MessagesWrite,
                            Scope::ChannelsWrite,
                            Scope::AdminChannels,
                        ],
                        channel_ids: None,
                        conn_id: Uuid::new_v4(),
                    }
                } else {
                    http_auth(&guest)
                }
            };
            let reply = event(&guest, 9, &reply_tags);
            let outcome = ingest_event(&state, &tenant, reply.clone(), auth()).await;
            assert_eq!(
                outcome.is_ok_and(|r| r.accepted),
                replies,
                "reply {replies}/{reactions}/{websocket}"
            );
            if replies {
                let nested = event(
                    &guest,
                    9,
                    &[
                        h.clone(),
                        vec!["e".into(), root.id.to_hex(), "".into(), "root".into()],
                        vec!["e".into(), reply.id.to_hex(), "".into(), "reply".into()],
                    ],
                );
                assert!(
                    ingest_event(&state, &tenant, nested, auth())
                        .await
                        .unwrap()
                        .accepted
                );
                let edit = event(
                    &guest,
                    40003,
                    &[h.clone(), vec!["e".into(), reply.id.to_hex()]],
                );
                assert!(
                    ingest_event(&state, &tenant, edit, auth())
                        .await
                        .unwrap()
                        .accepted
                );
            }
            if replies {
                let deletion = event(&guest, 5, &[h.clone(), vec!["e".into(), reply.id.to_hex()]]);
                assert!(
                    ingest_event(&state, &tenant, deletion, auth())
                        .await
                        .unwrap()
                        .accepted
                );
            }
            let mut reaction_tags = vec![
                Tag::parse(["e", &root.id.to_hex()]).unwrap(),
                Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
            ];
            if websocket {
                reaction_tags.push(Tag::parse(h.clone()).unwrap());
            }
            let reaction = EventBuilder::new(Kind::Reaction, "👍")
                .tags(reaction_tags)
                .sign_with_keys(&guest)
                .unwrap();
            let outcome = ingest_event(&state, &tenant, reaction.clone(), auth()).await;
            assert_eq!(
                outcome.is_ok_and(|r| r.accepted),
                reactions,
                "reaction {replies}/{reactions}/{websocket}"
            );
            for forbidden in [
                event(
                    &guest,
                    40003,
                    &[h.clone(), vec!["e".into(), previous_root.id.to_hex()]],
                ),
                event(
                    &guest,
                    5,
                    &[h.clone(), vec!["e".into(), previous_root.id.to_hex()]],
                ),
                event(
                    &guest,
                    9,
                    &[
                        h.clone(),
                        vec![
                            "e".into(),
                            other_root.id.to_hex(),
                            "".into(),
                            "reply".into(),
                        ],
                    ],
                ),
                event(&guest, 9, std::slice::from_ref(&h)),
                event(
                    &guest,
                    9,
                    &[
                        h.clone(),
                        vec!["e".into(), root.id.to_hex(), "".into(), "root".into()],
                    ],
                ),
                event(
                    &guest,
                    9,
                    &[
                        reply_tags[0].clone(),
                        reply_tags[1].clone(),
                        vec!["broadcast".into(), "1".into()],
                    ],
                ),
                event(
                    &guest,
                    9,
                    &[
                        h.clone(),
                        vec!["e".into(), "f".repeat(64), "".into(), "reply".into()],
                    ],
                ),
                event(
                    &guest,
                    9002,
                    &[h.clone(), vec!["guest_replies".into(), "true".into()]],
                ),
                event(&guest, 40100, std::slice::from_ref(&h)),
            ] {
                assert!(ingest_event(&state, &tenant, forbidden, auth())
                    .await
                    .is_err());
            }
            if reactions {
                let fake_thread = event(
                    &guest,
                    9,
                    &[
                        h.clone(),
                        vec!["e".into(), reaction.id.to_hex(), "".into(), "reply".into()],
                    ],
                );
                assert!(ingest_event(&state, &tenant, fake_thread, auth())
                    .await
                    .is_err());
                let removal = event(
                    &guest,
                    5,
                    &[
                        vec!["e".into(), reaction.id.to_hex()],
                        vec!["k".into(), "7".into()],
                    ],
                );
                assert!(
                    ingest_event(&state, &tenant, removal, auth())
                        .await
                        .unwrap()
                        .accepted
                );
            }
        }
    }
    // Managing one's persisted reply still works after its parent was deleted.
    let orphan = event(&guest, 9, &reply_tags);
    assert!(
        ingest_event(&state, &tenant, orphan.clone(), http_auth(&guest))
            .await
            .unwrap()
            .accepted
    );
    let delete_root = event(&owner, 5, &[h.clone(), vec!["e".into(), root.id.to_hex()]]);
    assert!(
        ingest_event(&state, &tenant, delete_root, http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    for kind in [40003, 5] {
        let operation = event(
            &guest,
            kind,
            &[h.clone(), vec!["e".into(), orphan.id.to_hex()]],
        );
        assert!(
            ingest_event(&state, &tenant, operation, http_auth(&guest))
                .await
                .unwrap()
                .accepted
        );
    }
    // An unrelated edit preserves the grants; malformed and duplicate grants fail.
    let rename = event(
        &owner,
        9002,
        &[h.clone(), vec!["about".into(), "Updated".into()]],
    );
    assert!(
        ingest_event(&state, &tenant, rename, http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    assert!(
        state
            .db
            .get_channel(tenant.community(), channel)
            .await
            .unwrap()
            .guest_replies
    );
    for values in [vec!["yes"], vec!["true", "false"]] {
        let mut tags = vec![h.clone()];
        tags.extend(
            values
                .into_iter()
                .map(|v| vec!["guest_replies".into(), v.into()]),
        );
        assert!(ingest_event(
            &state,
            &tenant,
            event(&owner, 9002, &tags),
            http_auth(&owner)
        )
        .await
        .is_err());
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revoking_guest_grants_blocks_additions_but_preserves_own_reaction_removal() {
    let state = state().await;
    let tenant = community(&state, "guest-revocation").await;
    let owner = Keys::generate();
    let guest = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = vec!["h".into(), channel.to_string()];
    for command in [
        event(
            &owner,
            9000,
            &[
                h.clone(),
                vec!["p".into(), guest.public_key().to_hex(), "guest".into()],
            ],
        ),
        event(
            &owner,
            9002,
            &[
                h.clone(),
                vec!["guest_replies".into(), "true".into()],
                vec!["guest_reactions".into(), "true".into()],
            ],
        ),
    ] {
        assert!(
            ingest_event(&state, &tenant, command, http_auth(&owner))
                .await
                .unwrap()
                .accepted
        );
    }
    let root = event(&owner, 9, std::slice::from_ref(&h));
    assert!(
        ingest_event(&state, &tenant, root.clone(), http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    let reaction = EventBuilder::new(Kind::Reaction, "+")
        .tags([Tag::parse(["e", &root.id.to_hex()]).unwrap()])
        .sign_with_keys(&guest)
        .unwrap();
    assert!(
        ingest_event(&state, &tenant, reaction.clone(), http_auth(&guest))
            .await
            .unwrap()
            .accepted
    );
    let revoke = event(
        &owner,
        9002,
        &[
            h.clone(),
            vec!["guest_replies".into(), "false".into()],
            vec!["guest_reactions".into(), "false".into()],
        ],
    );
    assert!(
        ingest_event(&state, &tenant, revoke, http_auth(&owner))
            .await
            .unwrap()
            .accepted
    );
    let reply = event(
        &guest,
        9,
        &[
            h.clone(),
            vec!["e".into(), root.id.to_hex(), "".into(), "reply".into()],
        ],
    );
    assert!(ingest_event(&state, &tenant, reply, http_auth(&guest))
        .await
        .is_err());
    let addition = EventBuilder::new(Kind::Reaction, "👍")
        .tags([Tag::parse(["e", &root.id.to_hex()]).unwrap()])
        .sign_with_keys(&guest)
        .unwrap();
    assert!(ingest_event(&state, &tenant, addition, http_auth(&guest))
        .await
        .is_err());
    let removal = event(&guest, 5, &[vec!["e".into(), reaction.id.to_hex()]]);
    assert!(
        ingest_event(&state, &tenant, removal, http_auth(&guest))
            .await
            .unwrap()
            .accepted
    );
}
