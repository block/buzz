use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn conditional_canvas_and_deletion_actions_enforce_sender_fences() {
    for websocket in [false, true] {
        for actor in [0, 1, 2] {
            let f = Fixture::new().await;
            let key = match actor {
                0 => &f.user,
                1 => &f.agent,
                _ => &f.peer,
            };
            let mut phases = Vec::new();
            // Prepare both targets before fencing, so authorization/target checks
            // cannot accidentally make an unexercised guard look correct.
            for _ in 0..2 {
                let channel = create_channel(&f.state, &f.tenant, key, &[]).await;
                let root = action(key, channel, 9, "delete me", vec![]);
                assert!(f.send(key, &root, websocket).await.0);
                let workflow = Uuid::new_v4();
                f.state
                    .db
                    .ensure_user(f.tenant.community(), key.public_key().as_bytes())
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO workflows (community_id,id,name,owner_pubkey,channel_id,definition,definition_hash) VALUES ($1,$2,'fence-test',$3,$4,'{}','\\x00')")
                    .bind(f.tenant.community().as_uuid()).bind(workflow).bind(key.public_key().as_bytes()).bind(channel)
                    .execute(&f.pool).await.unwrap();
                let definition = action(
                    key,
                    channel,
                    30620,
                    "{}",
                    vec![vec!["d".into(), workflow.to_string()]],
                );
                f.state
                    .db
                    .replace_parameterized_event(
                        f.tenant.community(),
                        &definition,
                        &workflow.to_string(),
                        Some(channel),
                    )
                    .await
                    .unwrap();
                phases.push((channel, root, workflow));
            }
            for (phase, (channel, root, workflow)) in phases.into_iter().enumerate() {
                if phase == 1 {
                    f.fence().await;
                }
                let accepted = phase == 0 || actor == 2;
                let cases = [
                    (
                        "conditional canvas",
                        action(
                            key,
                            channel,
                            40100,
                            "canvas",
                            vec![vec!["expected-revision".into(), "none".into()]],
                        ),
                    ),
                    (
                        "message deletion",
                        action(
                            key,
                            channel,
                            5,
                            "",
                            vec![vec!["e".into(), root.id.to_hex()]],
                        ),
                    ),
                    (
                        "workflow deletion",
                        action(
                            key,
                            channel,
                            5,
                            "",
                            vec![vec![
                                "a".into(),
                                format!("30620:{}:{workflow}", key.public_key()),
                            ]],
                        ),
                    ),
                    ("channel deletion", action(key, channel, 9008, "", vec![])),
                ];
                for (name, event) in cases {
                    let before = snapshot(&f).await;
                    let (actual, reason) = f.send(key, &event, websocket).await;
                    assert_eq!(
                        actual, accepted,
                        "{name} actor={actor} phase={phase} ws={websocket}: {reason}"
                    );
                    if !accepted {
                        assert_eq!(
                            reason, "restricted: identity is restricted for user deletion",
                            "{name}"
                        );
                        assert_eq!(
                            snapshot(&f).await,
                            before,
                            "{name} changed state despite refusal"
                        );
                    } else if name == "workflow deletion" {
                        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workflows WHERE community_id=$1 AND id=$2)")
                            .bind(f.tenant.community().as_uuid()).bind(workflow).fetch_one(&f.pool).await.unwrap();
                        assert!(!exists, "positive control must really delete the workflow");
                    }
                }
            }
        }
    }
}
