//! Expanded insertion coverage and owner-delegated HTTP regression tests.
use super::*;
use base64::Engine;
use sha2::{Digest, Sha256};

async fn signed_http(f: &Fixture, key: &Keys, event: &Event, owner: Option<&Keys>) -> (u16, Value) {
    let body = serde_json::to_vec(event).unwrap();
    let url = format!("https://{}/events", f.tenant.host());
    let auth = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
        ])
        .sign_with_keys(key)
        .unwrap();
    let mut request = reqwest::Client::new()
        .post(format!("{}/events", f.url))
        .header("host", f.tenant.host())
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&auth).unwrap())
            ),
        )
        .body(body);
    if let Some(owner) = owner {
        request = request.header(
            "x-auth-tag",
            buzz_sdk::nip_oa::compute_auth_tag(owner, &key.public_key(), "").unwrap(),
        );
    }
    let response = request.send().await.unwrap();
    (response.status().as_u16(), response.json().await.unwrap())
}

async fn direct_member(f: &Fixture, key: &Keys) {
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member') ON CONFLICT DO NOTHING")
        .bind(f.tenant.community().as_uuid()).bind(key.public_key().to_hex())
        .execute(&f.pool).await.unwrap();
}

fn action(key: &Keys, channel: Uuid, kind: u16, content: &str, extra: Vec<Vec<String>>) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    tags.extend(extra.into_iter().map(|tag| Tag::parse(tag).unwrap()));
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(key)
        .unwrap()
}

async fn snapshot(f: &Fixture) -> Vec<Value> {
    let mut rows = Vec::new();
    for table in [
        "events",
        "channels",
        "channel_members",
        "reactions",
        "thread_metadata",
        "event_mentions",
        "operator_listener_outbox",
        "artifact_heads",
        "artifact_revisions",
        "workflows",
    ] {
        let query = format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb) FROM {table} t WHERE community_id=$1");
        rows.push(
            sqlx::query_scalar(sqlx::AssertSqlSafe(query))
                .bind(f.tenant.community().as_uuid())
                .fetch_one(&f.pool)
                .await
                .unwrap(),
        );
    }
    rows
}

#[path = "user_deletion_lifecycle_postgres_tests.rs"]
mod lifecycle_postgres_tests;

/// Each case must succeed before fencing, then leave every relevant table
/// unchanged on rejection. Both real HTTP routing and the WS EVENT handler run.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn expanded_actions_reject_fenced_humans_and_agents_without_partial_writes() {
    let f = Fixture::new().await;
    let mut fixtures = Vec::new();
    for websocket in [false, true] {
        for key in [&f.user, &f.agent, &f.peer] {
            let channel = create_channel(&f.state, &f.tenant, key, &[]).await;
            let root = action(key, channel, 9, "edit target", vec![]);
            let post = action(key, channel, 45001, "vote target", vec![]);
            assert!(f.send(key, &root, websocket).await.0);
            assert!(f.send(key, &post, websocket).await.0);
            fixtures.push((websocket, key, channel, root, post));
        }
    }
    for fenced in [false, true] {
        if fenced {
            f.fence().await;
        }
        for (websocket, key, channel, root, post) in &fixtures {
            let text = Uuid::new_v4().to_string();
            let member = Keys::generate().public_key().to_hex();
            let new_channel = Uuid::new_v4();
            let dm = EventBuilder::new(Kind::Custom(41010), text.clone())
                .tags([Tag::parse(["p", &Keys::generate().public_key().to_hex()]).unwrap()])
                .sign_with_keys(key)
                .unwrap();
            let cases = vec![
                ("open DM", dm),
                (
                    "reaction",
                    action(
                        key,
                        *channel,
                        7,
                        if fenced { "🚀" } else { "👍" },
                        vec![vec!["e".into(), root.id.to_hex()]],
                    ),
                ),
                (
                    "edit",
                    action(
                        key,
                        *channel,
                        40003,
                        &text,
                        vec![vec!["e".into(), root.id.to_hex()]],
                    ),
                ),
                (
                    "vote",
                    action(
                        key,
                        *channel,
                        45002,
                        "+",
                        vec![
                            vec!["e".into(), post.id.to_hex()],
                            vec!["nonce".into(), text.clone()],
                        ],
                    ),
                ),
                ("canvas", action(key, *channel, 40100, &text, vec![])),
                (
                    "settings",
                    action(
                        key,
                        *channel,
                        9002,
                        "",
                        vec![
                            vec!["name".into(), text.clone()],
                            vec!["topic".into(), text.clone()],
                        ],
                    ),
                ),
                (
                    "ttl settings",
                    action(
                        key,
                        *channel,
                        9002,
                        "",
                        vec![vec![
                            "ttl".into(),
                            if fenced { "7200" } else { "3600" }.into(),
                        ]],
                    ),
                ),
                (
                    "add member",
                    action(
                        key,
                        *channel,
                        9000,
                        "",
                        vec![vec!["p".into(), member, "member".into()]],
                    ),
                ),
                (
                    "create channel",
                    action(
                        key,
                        new_channel,
                        9007,
                        "",
                        vec![vec!["name".into(), text.clone()]],
                    ),
                ),
                (
                    "artifact",
                    action(
                        key,
                        *channel,
                        45010,
                        &serde_json::json!({"text": text}).to_string(),
                        vec![
                            vec!["ar".into(), "1".into()],
                            vec!["d".into(), Uuid::new_v4().to_string()],
                            vec!["type".into(), "buzz.task".into()],
                            vec!["op".into(), "create".into()],
                            vec!["title".into(), "Task".into()],
                        ],
                    ),
                ),
            ];
            for (name, event) in cases {
                let should_accept = !fenced || key.public_key() == f.peer.public_key();
                let before = snapshot(&f).await;
                let (accepted, reason) = f.send(key, &event, *websocket).await;
                assert_eq!(
                    accepted, should_accept,
                    "{name} fenced={fenced} ws={websocket}: {reason}"
                );
                assert_eq!(
                    f.state
                        .db
                        .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                        .await
                        .unwrap()
                        .is_some(),
                    should_accept,
                    "{name}"
                );
                if !should_accept {
                    assert_eq!(
                        reason, "restricted: identity is restricted for user deletion",
                        "{name}"
                    );
                    assert_eq!(snapshot(&f).await, before, "{name} must roll back domain writes, counters, mentions, and notifications");
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn delegated_http_refuses_failed_owner_link_but_preserves_independent_members() {
    let f = Fixture::configured(true).await;
    direct_member(&f, &f.user).await;
    direct_member(&f, &f.peer).await;
    // All have channel membership, but only the last has independent relay membership.
    let fresh = Keys::generate();
    let allowed = Keys::generate();
    let independent = Keys::generate();
    let channel = create_channel(
        &f.state,
        &f.tenant,
        &f.peer,
        &[
            &["p", &fresh.public_key().to_hex()],
            &["p", &allowed.public_key().to_hex()],
            &["p", &independent.public_key().to_hex()],
        ],
    )
    .await;
    direct_member(&f, &independent).await;
    f.fence().await;
    for (key, owner, accepted) in [
        (&fresh, &f.user, false),
        (&allowed, &f.peer, true),
        (&independent, &f.user, true),
    ] {
        let event = EventBuilder::new(Kind::Custom(9), Uuid::new_v4().to_string())
            .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
            .sign_with_keys(key)
            .unwrap();
        let (status, body) = signed_http(&f, key, &event, Some(owner)).await;
        assert_eq!(
            status < 300 && body["accepted"] == true,
            accepted,
            "{status}: {body}"
        );
        assert_eq!(
            f.state
                .db
                .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                .await
                .unwrap()
                .is_some(),
            accepted
        );
        let stored: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(f.tenant.community().as_uuid())
        .bind(key.public_key().as_bytes())
        .fetch_optional(&f.pool)
        .await
        .unwrap()
        .flatten();
        if key.public_key() == allowed.public_key() {
            assert_eq!(
                stored.as_deref(),
                Some(f.peer.public_key().as_bytes().as_slice())
            );
        } else {
            assert!(
                stored.is_none(),
                "refused/independent identity must not be reassigned"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_expanded_writes_wait_for_fencing_commit_or_rollback() {
    for commit in [false, true] {
        let f = Arc::new(Fixture::new().await);
        let channel = create_channel(&f.state, &f.tenant, &f.user, &[]).await;
        let gate = f.pending_fence().await;
        let mut tasks = Vec::new();
        for (index, kind) in [9, 40100, 9002, 9000, 9007].into_iter().enumerate() {
            let f = f.clone();
            let channel = if kind == 9007 {
                Uuid::new_v4()
            } else {
                channel
            };
            let tags = match kind {
                9002 => vec![vec!["ttl".into(), "3600".into()]],
                9000 => vec![vec![
                    "p".into(),
                    Keys::generate().public_key().to_hex(),
                    "member".into(),
                ]],
                9007 => vec![vec!["name".into(), Uuid::new_v4().to_string()]],
                _ => vec![],
            };
            let event = action(&f.user, channel, kind, "concurrent test", tags);
            tasks.push(tokio::spawn(async move {
                let result = f.send(&f.user, &event, index % 2 == 0).await;
                (event, result)
            }));
        }
        // Wait for an actual database lock waiter, not a guessed sleep.
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE a.datname=current_database() AND l.locktype='advisory' AND NOT l.granted)")
                    .fetch_one(&f.pool).await.unwrap();
                if waiting { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("a real writer must wait on the fence");
        assert!(tasks.iter().all(|task| !task.is_finished()));
        assert!(
            f.send(&f.peer, &f.message(&f.peer, 9, None), false).await.0,
            "unrelated user must proceed"
        );
        if commit {
            gate.commit().await.unwrap();
        } else {
            gate.rollback().await.unwrap();
        }
        for task in tasks {
            let (event, (accepted, reason)) = tokio::time::timeout(Duration::from_secs(8), task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(accepted, !commit, "kind {}: {reason}", event.kind.as_u16());
            assert_eq!(
                f.state
                    .db
                    .get_event_by_id(f.tenant.community(), event.id.as_bytes())
                    .await
                    .unwrap()
                    .is_some(),
                !commit
            );
        }
        let deadlocks: i64 = sqlx::query_scalar(
            "SELECT deadlocks FROM pg_stat_database WHERE datname=current_database()",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(
            deadlocks, 0,
            "no deadlocks, including swallowed TTL-refresh failures"
        );
    }
}

#[path = "user_deletion_remaining_postgres_tests.rs"]
mod remaining_postgres_tests;
