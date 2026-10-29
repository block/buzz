use super::*;

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn replacements_workflows_and_typing_refuse_fenced_users_and_agents() {
    let f = Fixture::new().await;
    let mut channels = Vec::new();
    for key in [&f.user, &f.agent, &f.peer] {
        channels.push(create_channel(&f.state, &f.tenant, key, &[]).await);
    }
    for phase in 0..2 {
        if phase == 1 {
            f.fence().await;
        }
        for websocket in [false, true] {
            for (actor, key) in [&f.user, &f.agent, &f.peer].into_iter().enumerate() {
                let expected = phase == 0 || actor == 2;
                let channel = channels[actor];
                let nonce = Uuid::new_v4().to_string();
                let timestamp = nostr::Timestamp::from(
                    nostr::Timestamp::now().as_secs() + phase * 4 + u64::from(websocket),
                );
                let cases = [
                    (0, format!("{{\"name\":\"{nonce}\"}}"), vec![]),
                    (10030, String::new(), vec![vec!["emoji".into(), "probe".into(), "https://example.invalid/probe.png".into()]]),
                    (30030, String::new(), vec![vec!["d".into(), nonce.clone()], vec!["emoji".into(), "probe".into(), "https://example.invalid/probe.png".into()]]),
                    (10003, String::new(), vec![vec!["e".into(), "ab".repeat(32)]]),
                    (30620, "name: Gate probe\ntrigger:\n  on: webhook\nsteps:\n  - id: noop\n    action: send_message\n    text: Never invoked\n".into(), vec![vec!["d".into(), Uuid::new_v4().to_string()], vec!["h".into(), channel.to_string()]]),
                    (20002, String::new(), vec![vec!["h".into(), channel.to_string()]]),
                ];
                for (kind, content, tags) in cases {
                    let event = EventBuilder::new(Kind::Custom(kind), content)
                        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
                        .custom_created_at(timestamp)
                        .sign_with_keys(key)
                        .unwrap();
                    let before = snapshot(&f).await;
                    let (accepted, reason) = f.send(key, &event, websocket).await;
                    assert_eq!(
                        accepted, expected,
                        "kind {kind}, actor {actor}, WS {websocket}: {reason}"
                    );
                    if !expected {
                        assert_eq!(
                            reason,
                            "restricted: identity is restricted for user deletion"
                        );
                        assert_eq!(snapshot(&f).await, before);
                    }
                    assert_eq!(
                        f.state
                            .db
                            .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                            .await
                            .unwrap()
                            .is_some(),
                        expected && kind != 20002
                    );
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn existing_workflow_actions_refuse_fenced_owners() {
    let f = Fixture::new().await;
    let action = buzz_workflow::ActionDef::Delay {
        duration: "0s".into(),
    };
    let step = buzz_workflow::Step {
        id: "probe".into(),
        name: None,
        if_expr: None,
        timeout_secs: None,
        action: action.clone(),
    };
    let mut runs = Vec::new();
    for key in [&f.user, &f.agent, &f.peer] {
        f.state
            .db
            .ensure_user(f.tenant.community(), key.public_key().as_bytes())
            .await
            .unwrap();
        let workflow = f
            .state
            .db
            .create_workflow(
                f.tenant.community(),
                Some(f.channel),
                key.public_key().as_bytes(),
                "existing",
                "{}",
                &[0; 32],
            )
            .await
            .unwrap();
        runs.push(
            f.state
                .db
                .create_workflow_run(f.tenant.community(), workflow, None, None)
                .await
                .unwrap(),
        );
    }
    for phase in 0..2 {
        if phase == 1 {
            f.fence().await;
        }
        for (index, run) in runs.iter().enumerate() {
            let result = buzz_workflow::executor::dispatch_action(
                &step,
                &action,
                &f.state.workflow_engine,
                f.tenant.community(),
                *run,
                &Default::default(),
            )
            .await;
            assert_eq!(result.is_ok(), phase == 0 || index == 2, "{result:?}");
            if phase == 1 && index != 2 {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("restricted for user deletion"));
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn upload_routes_check_actor_before_reading_body() {
    let f = Fixture::new().await;
    // Deliberately unsupported audio proves admission reached body validation
    // before fencing; after fencing the very same signed input must be 403.
    let body = b"ID3\x04\x00\x00\x00\x00\x00\x00";
    for phase in 0..2 {
        if phase == 1 {
            f.fence().await;
        }
        for path in ["/upload", "/media/upload"] {
            for (index, key) in [&f.user, &f.agent, &f.peer].iter().enumerate() {
                let sha = hex::encode(Sha256::digest(body));
                let event = EventBuilder::new(Kind::Custom(24242), Uuid::new_v4().to_string())
                    .tags([
                        Tag::parse(["t", "upload"]).unwrap(),
                        Tag::parse(["server", f.tenant.host()]).unwrap(),
                        Tag::parse(["x", &sha]).unwrap(),
                        Tag::parse([
                            "expiration",
                            &(nostr::Timestamp::now().as_secs() + 55).to_string(),
                        ])
                        .unwrap(),
                    ])
                    .sign_with_keys(key)
                    .unwrap();
                let response = reqwest::Client::new()
                    .put(format!("{}{path}", f.url))
                    .header("host", f.tenant.host())
                    .header("content-type", "audio/mpeg")
                    .header("x-sha-256", &sha)
                    .header(
                        "authorization",
                        format!(
                            "Nostr {}",
                            base64::engine::general_purpose::URL_SAFE_NO_PAD
                                .encode(serde_json::to_vec(&event).unwrap())
                        ),
                    )
                    .body(body.to_vec())
                    .send()
                    .await
                    .unwrap();
                let status = response.status().as_u16();
                let text = response.text().await.unwrap();
                assert_eq!(
                    status,
                    if phase == 1 && index != 2 { 403 } else { 415 },
                    "{path} {index}: {text}"
                );
                if status == 403 {
                    assert!(text.contains("restricted for user deletion"), "{text}");
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn personal_read_markers_refuse_fenced_actor_even_for_idempotent_retries() {
    use buzz_db::personal_read::{IntentOutcome, ReadIntent, ReadTarget};
    let f = Fixture::new().await;
    let message = f.message(&f.peer, 9, None);
    assert!(f.send(&f.peer, &message, false).await.0);
    let intent = ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: f.channel,
            root_id: None,
        },
        message_id: message.id.to_hex(),
    };
    for phase in 0..2 {
        if phase == 1 {
            f.fence().await;
        }
        for (index, key) in [&f.user, &f.agent, &f.peer].iter().enumerate() {
            let result = f
                .state
                .db
                .apply_personal_read_intent(f.tenant.community(), &key.public_key(), &intent)
                .await;
            if phase == 1 && index != 2 {
                assert!(
                    matches!(result, Err(buzz_db::DbError::UserDeletionRestricted)),
                    "{result:?}"
                );
            } else {
                assert_eq!(result.unwrap(), IntentOutcome::Applied);
            }
        }
    }
}
